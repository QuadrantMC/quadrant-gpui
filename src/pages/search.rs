//! The Search page, after `SearchPage` in the desktop app: a query, the
//! filters sidebar, and a paged grid of results from every enabled provider.

pub mod logic;

use std::{collections::HashMap, rc::Rc, time::Instant};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Bounds, Context, Entity,
    FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    Window, div, point, prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{
    Disableable as _, Icon, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    v_flex,
};
use quadrant_core::{
    mc_mod::{Mod, SearchCategory},
    models::{LocalModpack, ModLoader},
};
use quadrant_host::QuadrantHost;
use serde_json::{Value, json};

use self::logic::{
    Chip, ContentType, Filters, MergedCategory, PAGE_SIZE, Provider, Sort, installed_in,
    loader_options, merge_categories, order_results, page_count, sections,
};
use crate::{
    assets::MdIcon,
    backend::Backend,
    events::{ModStage, watch_mod_progress},
    i18n::{describe, t, t_with},
    loadable::{Loadable, Reload},
    motion::{self, CardEntrance},
    pages::{
        install_location::LocationPicker,
        mod_grid::{self, CardSlot, RowHeights, WidthProbe},
        mod_install::{InstallRequest, InstallViewEvent, ModInstallView, remember},
        mod_ui::{CardPress, CardStatus, Choice, OnCardPress, Primary, card_key, mod_card},
    },
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

/// Whether "search further" can fetch more.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum More {
    Available,
    Loading,
    Exhausted,
}

struct Results {
    /// Each searched provider's answers, in request order, so a further
    /// page lines up with the list it extends.
    lists: Vec<Vec<Mod>>,
    /// `lists` merged in the sort order.
    shown: Vec<Mod>,
    more: More,
}

impl Results {
    fn new(lists: Vec<Vec<Mod>>, sort: Sort) -> Self {
        let shown = order_results(&lists, sort);
        Self {
            lists,
            shown,
            more: More::Available,
        }
    }
}

/// What the page restores on start: the saved filters and the lists the
/// pickers offer.
struct Boot {
    filters: Filters,
    filters_open: bool,
    versions: Vec<String>,
    modpacks: Vec<LocalModpack>,
}

type Picker<V> = Entity<SelectState<SearchableVec<Choice<V>>>>;

pub struct SearchPage {
    query: Entity<InputState>,
    /// The query the shown results answer, kept for "search further".
    submitted: String,
    filters: Filters,
    filters_open: bool,
    /// Facet sections the user folded away, by header.
    folded: Vec<String>,
    categories: Loadable<Vec<MergedCategory>>,
    modpacks: Loadable<Vec<LocalModpack>>,
    results: Loadable<Results>,
    page: usize,
    /// Installs in flight, by mod id, with their download progress once it
    /// is reported. Each card installs on its own.
    installs: HashMap<String, Option<i32>>,
    /// Results installed from here, whose download button retires.
    finished: Vec<String>,
    /// Install pages opened from a result, the newest on top.
    details: Vec<(Entity<ModInstallView>, Subscription)>,
    /// Counts install pages opened and closed: each change plays the
    /// arriving content's entrance, as a page switch does.
    transitions: usize,
    target_picker: Picker<Option<String>>,
    version_picker: Picker<Option<String>>,
    sort_picker: Picker<Sort>,
    /// Where a quick-installed pack goes. Not saved: it follows the
    /// content type.
    install_to: Entity<LocationPicker>,
    results_scroll: ScrollHandle,
    /// How wide the results panel was last frame, for its column count.
    results_width: WidthProbe,
    /// The result grid's row heights, so an unchanged grid lays out once.
    row_heights: RowHeights,
    /// The filters sidebar folding away or out, its clock starting at the
    /// first frame that draws it, and its width share and panel opacity this
    /// frame.
    sidebar: Option<Option<Instant>>,
    sidebar_frame: Option<(f32, f32)>,
    /// Changes as the sidebar starts and stops moving. Part of the result
    /// rows' height keys, so the cards are measured again for the width the
    /// sidebar leaves them, but not on every frame while it moves.
    sidebar_epoch: u64,
    /// The result cards' `Mod.tsx` entrance, and where it puts them this
    /// frame.
    entrance: CardEntrance,
    card_frame: Option<(Pixels, f32)>,
    filters_scroll: ScrollHandle,
    /// The newest search; replacing it drops a stale one's answer.
    searching: Task<()>,
    loading_categories: Task<()>,
    /// The newest re-read of the target picker's modpacks.
    listing_modpacks: Reload,
    /// Until the saved filters load, changes don't search or save.
    booted: bool,
    _progress: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SearchPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filters = Filters::default();
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder(filters.content)));
        let target_picker =
            cx.new(|cx| SelectState::new(SearchableVec::new(vec![no_target()]), None, window, cx));
        let version_picker = cx.new(|cx| {
            SelectState::new(SearchableVec::new(vec![any_version()]), None, window, cx)
                .searchable(true)
        });
        let sorts = Sort::ALL.map(|sort| Choice::new(t(sort.label_key()), sort));
        let sort_picker =
            cx.new(|cx| SelectState::new(SearchableVec::new(sorts.to_vec()), None, window, cx));
        let subscriptions = vec![
            cx.subscribe(&query, |this, _, event: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.search(false, cx);
                }
            }),
            cx.subscribe_in(&target_picker, window, |this, _, event, window, cx| {
                let SelectEvent::Confirm(Some(name)) = event else {
                    return;
                };
                this.pick_target(name.clone(), window, cx);
            }),
            cx.subscribe_in(&version_picker, window, |this, _, event, window, cx| {
                let SelectEvent::Confirm(Some(version)) = event else {
                    return;
                };
                let version = version.clone();
                remember(
                    cx,
                    vec![("lastUsedVersion", version.clone().unwrap_or_default())],
                );
                let target = this.target_modpack().cloned();
                this.change(window, cx, |filters| {
                    filters.set_version(version, target.as_ref())
                });
            }),
            cx.subscribe_in(&sort_picker, window, |this, _, event, window, cx| {
                let SelectEvent::Confirm(Some(sort)) = event else {
                    return;
                };
                let sort = *sort;
                this.change(window, cx, |filters| filters.sort = sort);
            }),
        ];
        let progress = watch_mod_progress(cx, |this: &mut Self, progress, cx| {
            if progress.stage == ModStage::Download
                && let Some(done) = this.installs.get_mut(&progress.mod_id)
            {
                *done = Some(progress.percent);
                cx.notify();
            }
        });
        let mut page = Self {
            query,
            submitted: String::new(),
            filters,
            filters_open: true,
            folded: Vec::new(),
            categories: Loadable::Idle,
            modpacks: Loadable::Loading,
            results: Loadable::Loading,
            page: 0,
            installs: HashMap::new(),
            finished: Vec::new(),
            details: Vec::new(),
            transitions: 0,
            target_picker,
            version_picker,
            sort_picker,
            install_to: cx.new(|cx| LocationPicker::new(window, cx)),
            results_scroll: ScrollHandle::new(),
            results_width: WidthProbe::default(),
            row_heights: RowHeights::default(),
            sidebar: None,
            sidebar_frame: None,
            sidebar_epoch: 0,
            entrance: CardEntrance::Rest,
            card_frame: None,
            filters_scroll: ScrollHandle::new(),
            searching: Task::ready(()),
            loading_categories: Task::ready(()),
            listing_modpacks: Reload::default(),
            booted: false,
            _progress: progress,
            _subscriptions: subscriptions,
        };
        page.boot(window, cx);
        page
    }

    fn boot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move {
            // A version list that fails to load leaves only "Any version".
            let versions = host
                .get_versions()
                .await
                .map(|versions| versions.into_iter().map(|v| v.version).collect())
                .unwrap_or_default();
            let modpacks = host.get_modpacks(true).await?;
            let config = |key: &str| host.get_config_value(key).ok().flatten();
            let (filters, filters_open) = restore_filters(
                config("searchFilters").as_ref(),
                config("curseforge").and_then(|v| v.as_bool()),
                config("modrinth").and_then(|v| v.as_bool()),
            );
            Ok(Boot {
                filters,
                filters_open,
                versions,
                modpacks,
            })
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(boot) => this.finish_boot(boot, window, cx),
                Err(error) => {
                    this.modpacks = Loadable::Failed(describe(error));
                    this.booted = true;
                    this.load_categories(cx);
                    this.search(false, cx);
                }
            });
        })
        .detach();
    }

    fn finish_boot(&mut self, boot: Boot, window: &mut Window, cx: &mut Context<Self>) {
        self.filters = boot.filters;
        self.filters_open = boot.filters_open;
        if self
            .filters
            .target
            .as_ref()
            .is_some_and(|name| !boot.modpacks.iter().any(|modpack| &modpack.name == name))
        {
            self.filters.target = None;
        }
        let versions: Vec<_> = std::iter::once(any_version())
            .chain(
                boot.versions
                    .into_iter()
                    .map(|version| Choice::new(version.clone(), Some(version))),
            )
            .collect();
        self.version_picker.update(cx, |picker, cx| {
            picker.set_items(SearchableVec::new(versions), window, cx)
        });
        self.set_modpacks(boot.modpacks, window, cx);
        self.query.update(cx, |query, cx| {
            query.set_placeholder(placeholder(self.filters.content), window, cx)
        });
        self.booted = true;
        self.load_locations(window, cx);
        self.sync_pickers(window, cx);
        self.load_categories(cx);
        self.search(false, cx);
    }

    fn set_modpacks(
        &mut self,
        modpacks: Vec<LocalModpack>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = t("currentModpack");
        let choices: Vec<_> = std::iter::once(no_target())
            .chain(modpacks.iter().map(|modpack| {
                let mut title = format!(
                    "{} · {} · {}",
                    modpack.name, modpack.mod_loader, modpack.version
                );
                if modpack.is_applied {
                    title.push_str(&format!(" ({current})"));
                }
                Choice::new(title, Some(modpack.name.clone()))
            }))
            .collect();
        self.target_picker.update(cx, |picker, cx| {
            picker.set_items(SearchableVec::new(choices), window, cx)
        });
        self.modpacks = Loadable::Ready(modpacks);
        self.sync_pickers(window, cx);
    }

    pub fn refresh_modpacks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move { host.get_modpacks(true).await });
        self.listing_modpacks
            .start(cx.spawn_in(window, async move |this, cx| {
                let result = task.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Ok(modpacks) = result {
                        this.set_modpacks(modpacks, window, cx);
                        cx.notify();
                    }
                    if this.listing_modpacks.landed() {
                        this.refresh_modpacks(window, cx);
                    }
                });
            }));
    }

    /// Re-reads the target picker's modpacks and the "Install to"
    /// locations as the page is shown again, once the first load is done.
    pub fn revalidate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.booted {
            return;
        }
        // The desktop app mounts the page, and its cards, again.
        self.entrance.restart();
        if self.listing_modpacks.ask() {
            self.refresh_modpacks(window, cx);
        }
        let mod_type = self.filters.content.mod_type();
        self.install_to
            .update(cx, |picker, cx| picker.revalidate(mod_type, window, cx));
    }

    fn categories(&self) -> &[MergedCategory] {
        match &self.categories {
            Loadable::Ready(categories) => categories,
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => &[],
        }
    }

    fn target_modpack(&self) -> Option<&LocalModpack> {
        let name = self.filters.target.as_ref()?;
        match &self.modpacks {
            Loadable::Ready(modpacks) => modpacks.iter().find(|modpack| &modpack.name == name),
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => None,
        }
    }

    /// Applies a filter change, then saves the filters and searches again.
    fn change(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut Filters),
    ) {
        let before = self.filters.clone();
        edit(&mut self.filters);
        if self.filters == before {
            return;
        }
        if self.filters.content != before.content {
            self.query.update(cx, |query, cx| {
                query.set_placeholder(placeholder(self.filters.content), window, cx)
            });
            self.load_locations(window, cx);
        }
        if self.filters.content != before.content
            || self.filters.curseforge != before.curseforge
            || self.filters.modrinth != before.modrinth
        {
            self.load_categories(cx);
        }
        self.sync_pickers(window, cx);
        self.save(cx);
        self.search(false, cx);
    }

    fn load_locations(&self, window: &mut Window, cx: &mut Context<Self>) {
        let mod_type = self.filters.content.mod_type();
        self.install_to.update(cx, |picker, cx| {
            picker.load_for_search(mod_type, window, cx)
        });
    }

    fn pick_target(&mut self, name: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let modpack = name.and_then(|name| match &self.modpacks {
            Loadable::Ready(modpacks) => modpacks.iter().find(|m| m.name == name).cloned(),
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => None,
        });
        if let Some(modpack) = &modpack {
            remember(
                cx,
                vec![
                    ("lastUsedModpack", modpack.name.clone()),
                    ("lastUsedVersion", modpack.version.clone()),
                    ("lastUsedAPI", modpack.mod_loader.to_string()),
                ],
            );
        }
        self.change(window, cx, |filters| {
            filters.select_target(modpack.as_ref())
        });
    }

    fn pick_loader(
        &mut self,
        loader: Option<ModLoader>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        remember(
            cx,
            vec![(
                "lastUsedAPI",
                loader.map(|loader| loader.to_string()).unwrap_or_default(),
            )],
        );
        let target = self.target_modpack().cloned();
        self.change(window, cx, |filters| {
            filters.set_loader(loader, target.as_ref())
        });
    }

    /// Points the pickers at the filters after a change made elsewhere,
    /// such as a removed chip.
    fn sync_pickers(&self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.filters.target.clone();
        self.target_picker.update(cx, |picker, cx| {
            picker.set_selected_value(&target, window, cx)
        });
        let version = self.filters.version.clone();
        self.version_picker.update(cx, |picker, cx| {
            picker.set_selected_value(&version, window, cx)
        });
        let sort = self.filters.sort;
        self.sort_picker.update(cx, |picker, cx| {
            picker.set_selected_value(&sort, window, cx)
        });
    }

    fn save(&self, cx: &mut Context<Self>) {
        if !self.booted {
            return;
        }
        let saved = save_filters(&self.filters, self.filters_open);
        Backend::global(cx)
            .run(cx, move |host| async move {
                host.set_config_value("searchFilters", saved)
            })
            .detach();
    }

    fn load_categories(&mut self, cx: &mut Context<Self>) {
        let curseforge =
            self.filters.curseforge && Provider::available().contains(&Provider::CurseForge);
        let modrinth = self.filters.modrinth;
        let mod_type = self.filters.content.mod_type().to_string();
        self.categories = Loadable::Loading;
        let task = Backend::global(cx).run(cx, move |host| async move {
            let fetch = |enabled: bool, provider: Provider| {
                let host = host.clone();
                let mod_type = mod_type.clone();
                async move {
                    if !enabled {
                        return Vec::new();
                    }
                    // A provider without facets leaves the sidebar without them.
                    host.get_categories(provider.source(), mod_type)
                        .await
                        .unwrap_or_default()
                }
            };
            let curseforge: Vec<SearchCategory> = fetch(curseforge, Provider::CurseForge).await;
            let modrinth = fetch(modrinth, Provider::Modrinth).await;
            Ok(merge_categories(&curseforge, &modrinth))
        });
        self.loading_categories = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.categories = Loadable::from_result(result);
                // Picked categories gain or lose provider ids with the
                // taxonomy, which changes the requests.
                if !this.filters.selected.is_empty() {
                    this.search(false, cx);
                }
                cx.notify();
            });
        });
    }

    /// Searches every active provider. `further` asks each for the page
    /// after the results it already returned and appends them.
    fn search(&mut self, further: bool, cx: &mut Context<Self>) {
        if !self.booted {
            return;
        }
        let offsets: Vec<usize> = match (&self.results, further) {
            (Loadable::Ready(results), true) => results.lists.iter().map(Vec::len).collect(),
            _ => Vec::new(),
        };
        if further {
            match &mut self.results {
                Loadable::Ready(results) if results.more == More::Available => {
                    results.more = More::Loading
                }
                _ => return,
            }
        } else {
            self.submitted = self.query.read(cx).value().to_string();
            self.results = Loadable::Loading;
            self.page = 0;
            self.results_scroll.set_offset(point(px(0.), px(0.)));
        }
        let requests = self.filters.requests(
            &self.submitted,
            self.categories(),
            self.target_modpack(),
            &offsets,
        );
        let sort = self.filters.sort;
        let task = Backend::global(cx).run(cx, move |host| search_all(host, requests));
        self.searching = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match (&mut this.results, result) {
                    (Loadable::Ready(results), Ok(lists)) if further => {
                        if lists.iter().all(Vec::is_empty) {
                            results.more = More::Exhausted;
                        } else {
                            for (list, more) in results.lists.iter_mut().zip(lists) {
                                list.extend(more);
                            }
                            results.shown = order_results(&results.lists, sort);
                            results.more = More::Available;
                        }
                    }
                    (Loadable::Ready(results), Err(_)) if further => {
                        results.more = More::Available;
                    }
                    (_, result) => {
                        this.results =
                            Loadable::from_result(result.map(|lists| Results::new(lists, sort)));
                        this.entrance.restart();
                    }
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn press(
        &mut self,
        press: CardPress,
        item: &Mod,
        origin: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match press {
            CardPress::Primary(Primary::Download) if item.autoinstallable => {
                if let Some(target) = self.target_modpack() {
                    let location = self.install_to.read(cx).selected(cx);
                    let request = InstallRequest::into_target(item, target, location);
                    self.install(request, window, cx);
                } else {
                    self.open_details(item.clone(), Some(origin), window, cx);
                }
            }
            CardPress::Primary(Primary::Download | Primary::Installed) | CardPress::Open => {
                if item.downloadable {
                    self.open_details(item.clone(), Some(origin), window, cx);
                }
            }
            CardPress::Primary(Primary::Update) | CardPress::Delete | CardPress::Select => {}
        }
    }

    fn install(&mut self, request: InstallRequest, window: &mut Window, cx: &mut Context<Self>) {
        if self.installs.contains_key(&request.id) {
            return;
        }
        self.installs.insert(request.id.clone(), None);
        let id = request.id.clone();
        let task = Backend::global(cx).run(cx, move |host| request.perform(host));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.installs.remove(&id);
                match result {
                    Ok(()) => {
                        this.finished.push(id);
                        snackbar::show(cx, SnackbarKind::Success, t("downloadSuccess"));
                        this.refresh_modpacks(window, cx);
                    }
                    Err(error) => snackbar::show(cx, SnackbarKind::Error, describe(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens `item`'s install page. With the `origin` of the card pressed,
    /// the page's details grow out of the card, as the desktop app's
    /// `expanded` page does, rather than the page entering from below.
    fn open_details(
        &mut self,
        item: Mod,
        origin: Option<Bounds<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.filters.target.clone();
        let location = self.install_to.read(cx).selected(cx);
        let view = cx.new(|cx| ModInstallView::new(item, target, location, origin, window, cx));
        let subscription = cx.subscribe_in(&view, window, |this, _, event, window, cx| {
            match event {
                InstallViewEvent::Close => {
                    this.details.pop();
                    this.transitions += 1;
                    this.entrance.restart();
                }
                InstallViewEvent::Open(item, origin) => {
                    this.open_details((**item).clone(), Some(*origin), window, cx)
                }
                InstallViewEvent::Installed => this.refresh_modpacks(window, cx),
            }
            cx.notify();
        });
        self.details.push((view, subscription));
        if origin.is_none() {
            self.transitions += 1;
        }
        cx.notify();
    }

    /// Shows the first result not in the target pack as installing at
    /// `progress` percent, or no install with `None`, for screenshots.
    /// Nothing is installed.
    #[cfg(feature = "screenshot")]
    pub fn preview_installing(&mut self, progress: Option<i32>, cx: &mut Context<Self>) {
        self.installs.clear();
        let target = self.target_modpack();
        if let (Some(progress), Loadable::Ready(results)) = (progress, &self.results)
            && let Some(first) = results
                .shown
                .iter()
                .find(|item| installed_in(item, target).is_none())
        {
            self.installs.insert(first.id.clone(), Some(progress));
        }
        cx.notify();
    }

    /// Shows the results as loading, for screenshots. Nothing is asked.
    #[cfg(feature = "screenshot")]
    pub fn preview_searching(&mut self, cx: &mut Context<Self>) {
        self.searching = Task::ready(());
        self.results = Loadable::Loading;
        cx.notify();
    }

    /// Closes every open install page, for screenshots.
    #[cfg(feature = "screenshot")]
    pub fn close_details(&mut self, cx: &mut Context<Self>) {
        self.details.clear();
        self.transitions += 1;
        cx.notify();
    }

    /// The open install page's mod, which the title bar names as the desktop
    /// app does.
    pub fn title(&self, cx: &App) -> Option<SharedString> {
        self.details()
            .map(|view| view.read(cx).item().name.clone().into())
    }

    /// Closes the install page on top, which may first shrink back into
    /// its card; false when none is open.
    pub fn back(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.details() else {
            return false;
        };
        view.clone().update(cx, |view, cx| view.close(cx));
        true
    }

    /// The install page on top, if one is open.
    fn details(&self) -> Option<&Entity<ModInstallView>> {
        self.details.last().map(|(view, _)| view)
    }

    pub fn set_filters_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if open != self.filters_open {
            self.sidebar = Some(None);
            self.sidebar_epoch += 1;
        }
        self.filters_open = open;
        self.save(cx);
        cx.notify();
    }

    #[cfg(any(feature = "screenshot", feature = "perf"))]
    pub fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |input, cx| {
            input.set_value(query.to_string(), window, cx)
        });
        self.search(false, cx);
    }

    /// Whether the page has booted and its search has answered.
    #[cfg(feature = "perf")]
    pub fn results_settled(&self) -> bool {
        self.booted && !matches!(self.results, Loadable::Loading)
    }

    /// The icons of the results the page shows.
    #[cfg(feature = "perf")]
    pub fn icon_urls(&self) -> Vec<SharedString> {
        match &self.results {
            Loadable::Ready(results) => results
                .shown
                .iter()
                .map(|item| item.mod_icon_url.clone().into())
                .collect(),
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => Vec::new(),
        }
    }

    #[cfg(feature = "screenshot")]
    pub fn set_content(
        &mut self,
        content: ContentType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.change(window, cx, |filters| filters.select_content(content));
    }

    #[cfg(feature = "screenshot")]
    pub fn set_target(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_target(Some(name.to_string()), window, cx);
    }

    #[cfg(feature = "screenshot")]
    pub fn open_first_result(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Loadable::Ready(results) = &self.results
            && let Some(first) = results.shown.first().cloned()
        {
            self.open_details(first, None, window, cx);
        }
    }

    fn set_page(&mut self, page: usize, cx: &mut Context<Self>) {
        self.page = page;
        self.entrance.restart();
        self.results_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_none()
            .px_6()
            .pt_5()
            .pb_4()
            .gap_2p5()
            .items_center()
            .child(
                div().flex_1().child(
                    Input::new(&self.query)
                        .appearance(false)
                        .prefix(
                            Icon::new(MdIcon::Search)
                                .size_4()
                                .text_color(rgb(theme::TEXT_SUBTLE)),
                        )
                        .h_10()
                        .pl_3p5()
                        .pr_4()
                        .rounded_full()
                        .bg(rgb(theme::SURFACE))
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM),
                ),
            )
            .child(
                theme::search_button("search", t("search"), cx)
                    .on_click(cx.listener(|this, _, _, cx| this.search(false, cx))),
            )
    }

    fn render_filters(&self, chips: usize, cx: &mut Context<Self>) -> gpui::Div {
        let header = h_flex()
            .flex_none()
            .justify_between()
            .items_center()
            .px_6()
            .pt_5()
            .pb_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(MdIcon::FilterAlt)
                            .size(px(18.9))
                            .text_color(rgb(theme::TEXT_BADGE)),
                    )
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child(t("filter")),
                    )
                    .when(chips > 0, |this| this.child(count_badge(chips))),
            )
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .when(chips > 0, |this| {
                        this.child(
                            div()
                                .id("clear-all")
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(theme::TEXT_MUTED))
                                .cursor_pointer()
                                .hover(|this| this.text_color(rgb(theme::TEXT_DANGER)))
                                .child(t("clearAll"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.change(window, cx, Filters::clear_all)
                                })),
                        )
                    })
                    .child(
                        div()
                            .id("collapse-filters")
                            .text_color(rgb(theme::TEXT_MUTED))
                            .cursor_pointer()
                            .hover(|this| this.text_color(rgb(theme::TEXT_BRIGHT)))
                            .child(Icon::new(MdIcon::ChevronLeft).size_5())
                            .on_click(
                                cx.listener(|this, _, _, cx| this.set_filters_open(false, cx)),
                            ),
                    ),
            );

        theme::panel()
            .w(FILTERS_WIDTH)
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("filters-scroll")
                            .size_full()
                            .track_scroll(&self.filters_scroll)
                            .overflow_y_scroll()
                            .px_6()
                            .pb_6()
                            .child(self.render_sources(cx))
                            .child(self.render_content_types(cx))
                            .child(self.render_target())
                            .children(self.render_install_to(cx))
                            .child(self.render_version())
                            .children(self.render_loaders(cx))
                            .children(self.render_facets(cx))
                            .child(self.render_open_source(cx)),
                    )
                    .vertical_scrollbar(&self.filters_scroll),
            )
    }

    fn render_sources(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let categories = self.categories();
        let filters = &self.filters;
        let lock = filters.lock(categories);
        v_flex()
            .mb_6()
            .child(theme::filter_label(t("source")))
            .child(
                h_flex()
                    .gap_2()
                    .children(Provider::available().iter().map(|&provider| {
                        let disabled = filters.source_disabled(provider, categories);
                        let dot = match provider {
                            Provider::CurseForge => theme::CURSEFORGE_DOT,
                            Provider::Modrinth => theme::MODRINTH_DOT,
                        };
                        theme::source_toggle(
                            SharedString::from(format!("source-{}", provider.label())),
                            provider.label(),
                            dot,
                            filters.searches(provider, categories),
                            disabled,
                        )
                        .when(!disabled, |this| {
                            this.on_click(cx.listener(move |this, _, window, cx| {
                                this.change(window, cx, |filters| filters.toggle_source(provider))
                            }))
                        })
                    })),
            )
            .when_some(lock, |this, lock| {
                this.child(div().mt_2p5().child(theme::warning_note(t_with(
                    "sourceLockHint",
                    &[("source", lock.label())],
                ))))
            })
    }

    fn render_content_types(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let filters = &self.filters;
        v_flex()
            .mb_6()
            .child(theme::filter_label(t("contentType")))
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1p5()
                    .children(ContentType::ALL.map(|content| {
                        theme::option_pill(
                            SharedString::from(format!("content-{content:?}")),
                            t(content.label_key()),
                            Some(content.icon()),
                            filters.content == content,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.change(window, cx, |filters| filters.select_content(content))
                            },
                        ))
                    })),
            )
    }

    fn render_target(&self) -> impl IntoElement {
        let filters = &self.filters;
        let no_modpacks =
            !matches!(&self.modpacks, Loadable::Ready(modpacks) if !modpacks.is_empty());
        let target_hint = if filters.target.is_some() {
            t("installIntoHint")
        } else {
            t("noTargetModpackHint")
        };
        let target_color = if filters.target.is_some() {
            theme::SELECTED
        } else {
            theme::CONTROL
        };
        v_flex()
            .mb_6()
            .child(theme::filter_label(t("installInto")))
            .child(
                picker(Select::new(&self.target_picker).disabled(no_modpacks))
                    .bg(rgb(target_color)),
            )
            .child(
                div()
                    .mt_1p5()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme::TEXT_SUBTLE))
                    .text_center()
                    .child(target_hint),
            )
    }

    /// Which location a quick-installed pack goes to, when there is a choice.
    fn render_install_to(&self, cx: &gpui::App) -> Option<impl IntoElement> {
        let select = self.install_to.read(cx).select()?;
        Some(
            v_flex()
                .mb_6()
                .child(theme::filter_label(t("installedContentInstallTo")))
                .child(picker(select).bg(rgb(theme::CONTROL))),
        )
    }

    fn render_version(&self) -> impl IntoElement {
        v_flex()
            .mb_6()
            .child(theme::filter_label(t("minecraftVersion")))
            .child(picker(Select::new(&self.version_picker)).bg(rgb(theme::CONTROL)))
    }

    /// Only mods have a loader, so the section is absent for packs.
    fn render_loaders(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let categories = self.categories();
        let filters = &self.filters;
        filters.content.has_loader().then(|| {
            let providers = filters.active_providers(categories);
            v_flex()
                .mb_6()
                .child(theme::filter_label(t("loader")))
                .child(h_flex().flex_wrap().gap_1p5().children(
                    loader_options(&providers).into_iter().map(|loader| {
                        let picked = filters.loader == Some(loader);
                        theme::option_pill(
                            SharedString::from(format!("loader-{loader:?}")),
                            loader.to_string(),
                            None,
                            picked,
                            cx,
                        )
                        .px_3p5()
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.pick_loader((!picked).then_some(loader), window, cx)
                            },
                        ))
                    }),
                ))
        })
    }

    fn render_facets(&self, cx: &mut Context<Self>) -> Vec<impl IntoElement> {
        let categories = self.categories();
        sections(categories)
            .into_iter()
            .map(|section| {
                let folded = self.folded.contains(&section.header);
                let header = section.header.clone();
                let options: Vec<AnyElement> = section
                    .options
                    .iter()
                    .map(|category| self.render_category(category, section.pills, cx))
                    .collect();
                v_flex()
                    .border_t_1()
                    .border_color(gpui::rgba(theme::RULE))
                    .pt_4()
                    .mb_3p5()
                    .child(
                        h_flex()
                            .id(SharedString::from(format!("section-{header}")))
                            .w_full()
                            .pb_3()
                            .justify_between()
                            .items_center()
                            .cursor_pointer()
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::EXTRA_BOLD)
                                    .child(section.title.clone()),
                            )
                            .child(
                                Icon::new(MdIcon::ExpandMore)
                                    .size(px(17.85))
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .when(folded, |icon| {
                                        icon.rotate(gpui::radians(std::f32::consts::PI))
                                    }),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                match this.folded.iter().position(|h| h == &header) {
                                    Some(index) => {
                                        this.folded.remove(index);
                                    }
                                    None => this.folded.push(header.clone()),
                                }
                                cx.notify();
                            })),
                    )
                    .when(!folded, |this| {
                        if section.pills {
                            this.child(h_flex().flex_wrap().gap_1p5().children(options))
                        } else {
                            this.child(v_flex().gap(px(1.)).children(options))
                        }
                    })
            })
            .collect()
    }

    fn render_open_source(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let open_source = self.filters.open_source;
        v_flex()
            .border_t_1()
            .border_color(gpui::rgba(theme::RULE))
            .pt_4()
            .child(
                h_flex()
                    .id("open-source")
                    .w_full()
                    .justify_between()
                    .items_center()
                    .cursor_pointer()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme::TEXT_SOFT))
                            .child(t("openSourceOnly")),
                    )
                    .child(theme::switch(open_source))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let categories = this.categories().to_vec();
                        this.change(window, cx, |filters| {
                            filters.toggle_open_source(&categories)
                        })
                    })),
            )
    }

    fn render_category(
        &self,
        category: &MergedCategory,
        pill: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let picked = self.filters.selected.contains(&category.key);
        let disabled = self.filters.category_disabled(category, self.categories());
        let key = category.key.clone();
        let toggle = cx.listener(move |this, _, window, cx| {
            this.change(window, cx, |filters| filters.toggle_category(&key))
        });
        let id = SharedString::from(format!("category-{}", category.key));
        if pill {
            return theme::option_pill(id, category.name.clone(), None, picked && !disabled, cx)
                .px_3p5()
                .disabled(disabled)
                .on_click(toggle)
                .into_any_element();
        }
        let label_color = if disabled {
            theme::TEXT_DISABLED
        } else if picked {
            theme::TEXT
        } else {
            theme::TEXT_SOFT
        };
        h_flex()
            .id(id)
            .w_full()
            .px_2()
            .py_1p5()
            .gap_2p5()
            .items_center()
            .rounded_xl()
            .when(disabled, |this| this.opacity(0.55))
            .when(!disabled, |this| {
                this.cursor_pointer()
                    .hover(|this| this.bg(gpui::rgba(theme::OPTION_HOVER)))
                    .on_click(toggle)
            })
            .child(theme::check_box(picked, disabled))
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(label_color))
                    .child(category.name.clone()),
            )
            .when_some(category.only_on(), |this, provider| {
                let curseforge = provider == Provider::CurseForge;
                this.child(
                    theme::source_pill(if curseforge { "CF" } else { "MR" }, curseforge)
                        .text_size(px(9.5))
                        .px_1p5()
                        .rounded_md(),
                )
            })
            .into_any_element()
    }

    fn render_results(&self, chips: &[Chip], cx: &mut Context<Self>) -> gpui::Div {
        let content_label = t(self.filters.content.label_key()).to_lowercase();
        let count = match &self.results {
            Loadable::Ready(results) => results.shown.len(),
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => 0,
        };
        let header = h_flex()
            .flex_none()
            .mb_3()
            .gap_3()
            .items_center()
            .when(!self.filters_open, |this| {
                this.child(
                    div()
                        .id("expand-filters")
                        .relative()
                        .flex_none()
                        .size_9()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(rgb(theme::SURFACE))
                        .text_color(rgb(theme::TEXT_BADGE))
                        .cursor_pointer()
                        .child(Icon::new(MdIcon::FilterAlt).size(px(18.9)))
                        .when(!chips.is_empty(), |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .top(px(-4.))
                                    .right(px(-4.))
                                    .child(count_badge(chips.len())),
                            )
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.set_filters_open(true, cx))),
                )
            })
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(t_with(
                        "resultCount",
                        &[("count", &count.to_string()), ("type", &content_label)],
                    )),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::TEXT_SUBTLE))
                            .child(t("sortBy")),
                    )
                    .child(
                        Select::new(&self.sort_picker)
                            .appearance(false)
                            .w(px(11. * 16.8))
                            .h_9()
                            .px_2()
                            .rounded_full()
                            .bg(rgb(theme::SURFACE))
                            .text_size(px(13.5))
                            .font_weight(FontWeight::BOLD),
                    ),
            );

        let chip_row = (!chips.is_empty()).then(|| {
            h_flex().flex_none().flex_wrap().gap_2().mb_3p5().children(
                chips.iter().enumerate().map(|(index, chip)| {
                    let removed = chip.clone();
                    h_flex()
                        .id(("chip", index))
                        .h_8()
                        .pl_3p5()
                        .pr_2()
                        .gap_1p5()
                        .items_center()
                        .rounded_full()
                        .bg(rgb(theme::CONTROL))
                        .text_color(rgb(theme::TEXT_BADGE))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::BOLD)
                        .cursor_pointer()
                        .hover(|this| {
                            this.bg(rgb(theme::CHIP_REMOVE))
                                .text_color(rgb(theme::CHIP_REMOVE_TEXT))
                        })
                        .child(chip.label())
                        .child(Icon::new(MdIcon::Close).size_3p5())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let removed = removed.clone();
                            this.change(window, cx, move |filters| filters.remove_chip(&removed))
                        }))
                }),
            )
        });

        let any_source = !self.filters.active_providers(self.categories()).is_empty();
        let body = if !any_source {
            empty_state(
                MdIcon::FilterAlt,
                t("noActiveSourceTitle"),
                t("noActiveSourceHint"),
                !chips.is_empty(),
                cx,
            )
        } else {
            match &self.results {
                Loadable::Idle | Loadable::Loading => h_flex()
                    .py_16()
                    .justify_center()
                    .child(theme::circular_progress("progress"))
                    .into_any_element(),
                Loadable::Failed(error) => div()
                    .p_4()
                    .rounded(theme::radius_card())
                    .bg(rgb(theme::ERROR_FILL))
                    .font_weight(FontWeight::BOLD)
                    .child(error.clone())
                    .into_any_element(),
                Loadable::Ready(results) if results.shown.is_empty() => empty_state(
                    MdIcon::Search,
                    t("noResultsTitle"),
                    t("noResultsHint"),
                    !chips.is_empty(),
                    cx,
                ),
                Loadable::Ready(results) => self.render_grid(results, cx),
            }
        };

        v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(header)
            .children(chip_row)
            .child(
                theme::panel()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("results-scroll")
                            .size_full()
                            .track_scroll(&self.results_scroll)
                            .overflow_y_scroll()
                            .p(RESULTS_PADDING)
                            .child(body),
                    )
                    .vertical_scrollbar(&self.results_scroll)
                    .child(self.results_width.element(cx.entity_id(), result_columns)),
            )
    }

    fn render_grid(&self, results: &Results, cx: &mut Context<Self>) -> AnyElement {
        let pages = page_count(results.shown.len());
        let page = self.page.min(pages - 1);
        let target = self.target_modpack();
        let on_press: OnCardPress<Self> = Rc::new(|this, press, item, origin, window, cx| {
            this.press(press, item, origin, window, cx)
        });
        let columns = result_columns(self.results_width.width());
        let shown: Vec<(&Mod, CardStatus)> = results
            .shown
            .iter()
            .skip(page * PAGE_SIZE)
            .take(PAGE_SIZE)
            .map(|item| {
                let status = CardStatus {
                    installed: installed_in(item, target).is_some(),
                    finished: self.finished.contains(&item.id),
                    progress: self.installs.get(&item.id).map(|done| done.unwrap_or(0)),
                };
                (item, status)
            })
            .collect();
        let further = page + 1 >= pages && results.more != More::Exhausted;
        let row_keys: Vec<u64> = shown
            .chunks(columns)
            .enumerate()
            .map(|(row, cards)| {
                let last = (row + 1) * columns >= shown.len();
                let keys = cards.iter().map(|(item, status)| card_key(item, *status));
                self.row_heights.key(
                    columns,
                    keys,
                    u64::from(last && further) | self.sidebar_epoch << 1,
                )
            })
            .collect();
        let mut cells: Vec<AnyElement> = shown
            .iter()
            .enumerate()
            .map(|(index, (item, status))| {
                let slot = CardSlot {
                    heights: &self.row_heights,
                    row: row_keys[index / columns],
                    first: index % columns == 0,
                };
                motion::card_at(
                    mod_card(
                        page * PAGE_SIZE + index,
                        item,
                        *status,
                        Some(slot),
                        on_press.clone(),
                        cx,
                    ),
                    self.card_frame,
                )
                .into_any_element()
            })
            .collect();
        if further {
            let loading = results.more == More::Loading;
            cells.push(
                div()
                    .id("search-further")
                    .min_h_32()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(!loading, |this| {
                        this.cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| this.search(true, cx)))
                    })
                    .child({
                        let arrow = div()
                            .relative()
                            .size_10()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(rgb(theme::POSITIVE_FILL))
                            .child(Icon::new(MdIcon::ArrowForward).size_5());
                        // The desktop app's arrow slides away right while
                        // more results load, rather than turning into a
                        // spinner.
                        if loading {
                            arrow
                                .with_animation(
                                    "search-further-slide",
                                    Animation::new(motion::FURTHER_SLIDE),
                                    |this, progress| {
                                        let eased = motion::EASE_IN.ease(progress);
                                        this.left(motion::FURTHER_SLIDE_BY * eased)
                                            .opacity(1. - eased)
                                    },
                                )
                                .into_any_element()
                        } else {
                            arrow.into_any_element()
                        }
                    })
                    .into_any_element(),
            );
        }
        v_flex()
            .child(
                div()
                    .grid()
                    .grid_cols(result_columns(self.results_width.width()) as u16)
                    .gap_3()
                    .children(cells),
            )
            .when(pages > 1, |this| {
                this.child(
                    h_flex()
                        .mt_5()
                        .gap_2()
                        .justify_center()
                        .items_center()
                        .child(
                            page_icon_button("first-page", MdIcon::FirstPage, cx)
                                .tooltip(t("firstPage"))
                                .disabled(page == 0)
                                .on_click(cx.listener(|this, _, _, cx| this.set_page(0, cx))),
                        )
                        .child(
                            page_button("previous-page", t("previous"), cx)
                                .disabled(page == 0)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_page(page.saturating_sub(1), cx)
                                })),
                        )
                        .child(
                            div()
                                .px_2()
                                .text_size(px(13.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(theme::TEXT_MUTED))
                                .child(t_with(
                                    "pageOf",
                                    &[
                                        ("current", &(page + 1).to_string()),
                                        ("total", &pages.to_string()),
                                    ],
                                )),
                        )
                        .child(
                            page_button("next-page", t("next"), cx)
                                .disabled(page + 1 >= pages)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.set_page(page + 1, cx)),
                                ),
                        )
                        .child(
                            page_icon_button("last-page", MdIcon::LastPage, cx)
                                .tooltip(t("lastPage"))
                                .disabled(page + 1 >= pages)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.set_page(pages - 1, cx)),
                                ),
                        ),
                )
            })
            .into_any_element()
    }
}

/// `w-79`: the filters panel, and its sidebar when open.
const FILTERS_WIDTH: Pixels = px(19.75 * theme::FONT_SIZE);

/// The results panel's padding on each side.
const RESULTS_PADDING: Pixels = px(18.9);

/// `grid-cols-[repeat(auto-fill,minmax(300px,1fr))] gap-3` in a results
/// panel `panel_width` wide.
fn result_columns(panel_width: Pixels) -> usize {
    mod_grid::auto_fill_columns(
        panel_width - RESULTS_PADDING * 2.,
        mod_grid::SEARCH_CARD_MIN,
        px(0.75 * theme::FONT_SIZE),
    )
}

/// Runs each request in turn. A failed provider contributes an empty list
/// so a further page still lines up; only all failing is an error.
async fn search_all(
    host: QuadrantHost,
    requests: Vec<(Provider, quadrant_core::mc_mod::GlobalSearchModsArgs)>,
) -> anyhow::Result<Vec<Vec<Mod>>> {
    let mut lists = Vec::with_capacity(requests.len());
    let mut first_error = None;
    for (provider, args) in requests {
        match host.search_mods(args).await {
            Ok(list) => lists.push(list),
            Err(error) => {
                log::warn!("{} search failed: {error:#}", provider.label());
                first_error.get_or_insert(error);
                lists.push(Vec::new());
            }
        }
    }
    match first_error {
        Some(error) if lists.iter().all(Vec::is_empty) => Err(error),
        _ => Ok(lists),
    }
}

/// Reads the desktop app's `searchFilters` setting and source toggles.
/// Anything missing or malformed keeps its default.
pub fn restore_filters(
    saved: Option<&Value>,
    curseforge: Option<bool>,
    modrinth: Option<bool>,
) -> (Filters, bool) {
    let mut filters = Filters {
        curseforge: curseforge.unwrap_or(true) && cfg!(feature = "curseforge"),
        modrinth: modrinth.unwrap_or(true),
        ..Filters::default()
    };
    let Some(saved) = saved else {
        return (filters, true);
    };
    let text = |key: &str| saved.get(key).and_then(Value::as_str);
    filters.content = match text("contentType") {
        Some("ResourcePack") => ContentType::ResourcePacks,
        Some("ShaderPack" | "Shader") => ContentType::Shaders,
        _ => ContentType::Mods,
    };
    if let Some(sort) = Sort::ALL
        .into_iter()
        .find(|sort| Some(sort.value()) == text("sortBy"))
    {
        filters.sort = sort;
    }
    filters.version = text("version")
        .filter(|version| !version.is_empty() && *version != "any")
        .map(str::to_string);
    filters.loader = text("loader")
        .map(|loader| ModLoader::from(loader.to_string()))
        .filter(|loader| *loader != ModLoader::Unknown);
    filters.selected = saved
        .get("selected")
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    filters.open_source = saved
        .get("openSource")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    filters.target = text("targetModpack")
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    let collapsed = saved
        .get("filtersCollapsed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    (filters, !collapsed)
}

/// The `searchFilters` setting the desktop app reads back.
pub fn save_filters(filters: &Filters, filters_open: bool) -> Value {
    let content = match filters.content {
        ContentType::Mods => "Mod",
        ContentType::ResourcePacks => "ResourcePack",
        ContentType::Shaders => "ShaderPack",
    };
    json!({
        "contentType": content,
        "sortBy": filters.sort.value(),
        "version": filters.version.as_deref().unwrap_or("any"),
        "loader": filters.loader.map(|loader| loader.to_string()).unwrap_or_default(),
        "selected": filters.selected,
        "openSource": filters.open_source,
        "targetModpack": filters.target.as_deref().unwrap_or(""),
        "filtersCollapsed": !filters_open,
    })
}

fn placeholder(content: ContentType) -> SharedString {
    let label = t(content.label_key()).to_lowercase();
    t_with("searchPlaceholder", &[("type", &label)])
}

fn no_target() -> Choice<Option<String>> {
    Choice::new(t("noTargetModpack"), None)
}

fn any_version() -> Choice<Option<String>> {
    Choice::new(t("anyVersion"), None)
}

/// A sidebar select: an `h-11` pill.
fn picker<E: gpui::Styled>(select: E) -> E {
    select
        .h(px(2.75 * 16.8))
        .px_2()
        .rounded_full()
        .text_sm()
        .font_weight(FontWeight::BOLD)
}

fn count_badge(count: usize) -> impl IntoElement {
    div()
        .min_w(px(22.))
        .h(px(22.))
        .px(px(7.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(theme::SELECTED))
        .text_xs()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(count.to_string())
}

/// A round pager button for the first and last page.
fn page_icon_button(
    id: &'static str,
    icon: MdIcon,
    cx: &mut Context<SearchPage>,
) -> gpui_component::button::Button {
    theme::option_pill(id, "", Some(icon), false, cx)
        .size_9()
        .px_0()
}

/// A pager pill for the previous and next page.
fn page_button(
    id: &'static str,
    label: SharedString,
    cx: &mut Context<SearchPage>,
) -> gpui_component::button::Button {
    theme::option_pill(id, label, None, false, cx).h_9().px_4()
}

fn empty_state(
    icon: MdIcon,
    title: SharedString,
    hint: SharedString,
    can_clear: bool,
    cx: &mut Context<SearchPage>,
) -> AnyElement {
    v_flex()
        .py(px(73.5))
        .px_5()
        .items_center()
        .justify_center()
        .text_center()
        .text_color(rgb(theme::TEXT_SUBTLE))
        .child(Icon::new(icon).size_12().text_color(rgb(theme::CONTROL)))
        .child(
            div()
                .mt_4()
                .text_size(px(17.))
                .font_weight(FontWeight::EXTRA_BOLD)
                .text_color(rgb(theme::TEXT_MUTED))
                .child(title),
        )
        .child(
            div()
                .mt_1p5()
                .max_w(px(22.5 * 16.8))
                .text_size(px(13.5))
                .child(hint),
        )
        .when(can_clear, |this| {
            this.child(
                theme::pill_button(
                    "clear-all-filters",
                    Tone::Control,
                    t("clearAllFilters"),
                    None,
                    cx,
                )
                .mt_4()
                .on_click(
                    cx.listener(|this, _, window, cx| this.change(window, cx, Filters::clear_all)),
                ),
            )
        })
        .into_any_element()
}

impl Render for SearchPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.row_heights.next_frame(window.viewport_size().width);
        self.card_frame = match self.details() {
            Some(_) => None,
            None => self.entrance.frame(window),
        };
        self.sidebar_frame = match &mut self.sidebar {
            Some(start) => {
                let elapsed = start.get_or_insert_with(Instant::now).elapsed();
                let frame = motion::sidebar(elapsed, self.filters_open);
                match frame {
                    Some(_) => window.request_animation_frame(),
                    None => {
                        self.sidebar = None;
                        self.sidebar_epoch += 1;
                    }
                }
                frame
            }
            None => None,
        };
        let content = self.render_content(cx);
        if self.transitions == 0 {
            return content;
        }
        // An install page enters with ModInstallPage's `initial {y: 24,
        // opacity: 0}`, and the results enter the same way on the way back.
        motion::entering(
            div().size_full().relative().child(content),
            ("search-entrance", self.transitions),
            false,
        )
        .into_any_element()
    }
}

impl SearchPage {
    fn render_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(view) = self.details() {
            return div().size_full().child(view.clone()).into_any_element();
        }
        let chips = self.filters.chips(self.categories());
        // The `aside` grows or shrinks around the panel, which keeps its full
        // width and is clipped; the gap beside it follows the state at once.
        let sidebar = match self.sidebar_frame {
            Some((width, opacity)) => Some(
                div()
                    .flex_none()
                    .h_full()
                    .overflow_hidden()
                    .w(FILTERS_WIDTH * width)
                    .when(opacity > 0., |this| {
                        this.child(self.render_filters(chips.len(), cx).opacity(opacity))
                    })
                    .into_any_element(),
            ),
            None if self.filters_open => {
                Some(self.render_filters(chips.len(), cx).into_any_element())
            }
            None => None,
        };
        v_flex()
            .size_full()
            .min_h_0()
            .child(self.render_search_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .px_6()
                    .pb_6()
                    .when(self.filters_open, |this| this.gap_4())
                    .children(sidebar)
                    .child(self.render_results(&chips, cx).h_full()),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_survive_a_save_and_restore() {
        let mut filters = Filters {
            content: ContentType::Shaders,
            sort: Sort::Updated,
            version: Some("1.21.1".into()),
            open_source: true,
            target: Some("Alpha".into()),
            ..Filters::default()
        };
        filters.toggle_category("categories:worldgen");
        let saved = save_filters(&filters, false);
        assert_eq!(saved["contentType"], "ShaderPack");
        assert_eq!(saved["loader"], "");
        let (restored, open) = restore_filters(Some(&saved), Some(true), Some(true));
        assert_eq!(restored, filters);
        assert!(!open);
    }

    #[test]
    fn restoring_reads_the_desktop_apps_legacy_values() {
        let saved = json!({ "contentType": "Shader", "version": "any", "loader": "Fabric" });
        let (filters, open) = restore_filters(Some(&saved), None, Some(false));
        assert_eq!(filters.content, ContentType::Shaders);
        assert_eq!(filters.version, None);
        assert_eq!(filters.loader, Some(ModLoader::Fabric));
        assert!(!filters.modrinth);
        assert!(open);
        let (defaults, open) = restore_filters(None, None, None);
        assert_eq!(defaults, Filters::default());
        assert!(open);
    }

    /// Searches the real Modrinth API the way the page does.
    #[test]
    #[ignore = "needs the network"]
    fn searches_modrinth() {
        let temp = tempfile::tempdir().unwrap();
        let mut options =
            quadrant_host::QuadrantHostOptions::new(temp.path().join("data"), "", "", "");
        options.mc_folder = Some(temp.path().join("mc"));
        let host = QuadrantHost::new(options).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let filters = Filters {
            content: ContentType::Shaders,
            ..Filters::default()
        };
        let requests = filters.requests("complementary", &[], None, &[]);
        let lists = runtime.block_on(search_all(host, requests)).unwrap();
        let results = order_results(&lists, Sort::Relevance);
        assert!(
            results
                .iter()
                .any(|item| item.name.contains("Complementary")),
            "{:?}",
            results.iter().map(|item| &item.name).collect::<Vec<_>>()
        );
    }
}
