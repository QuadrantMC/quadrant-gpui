//! The install page for one mod, after `ModInstallPage` in the desktop app:
//! its details and dependencies, and where to install it.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FontWeight, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription, Task, Window,
    div, prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{
    Disableable as _, Icon, h_flex,
    select::{SearchableVec, Select, SelectEvent, SelectState},
    spinner::Spinner,
    v_flex,
};
use quadrant_core::{
    mc_mod::{Mod, ModType},
    models::{LocalModpack, ModLoader, ModSource},
};
use quadrant_host::{QuadrantHost, get_user_url};

use crate::{
    assets::MdIcon,
    backend::Backend,
    events::{ModProgress, ModStage, watch_mod_progress},
    i18n::{describe, t, t_with},
    loadable::Loadable,
    pages::{
        install_location::{self, LocationPicker, initial_location, is_pack},
        mod_grid,
        mod_ui::{CardPress, CardStatus, Choice, OnCardPress, mod_card, source_label},
        search::logic::{Provider, compact_count, installed_in, loader_options},
    },
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

/// One install of one mod, the only change the install and search pages
/// make.
#[derive(Clone, Debug, PartialEq)]
pub struct InstallRequest {
    pub id: String,
    pub source: ModSource,
    pub mod_type: ModType,
    pub version: String,
    pub loader: ModLoader,
    /// The modpack a mod goes into. A pack follows it to wherever it is
    /// applied, or without one goes to the Minecraft folder.
    pub modpack: Option<String>,
    /// A content location a pack goes to instead, from the "Install to"
    /// picker. Never set for a mod.
    pub content_location: Option<String>,
}

impl InstallRequest {
    /// A one-click install from a search result into `target`, as the
    /// desktop app's card does when the result is auto-installable. A pack
    /// goes to `location` when the user picked one.
    pub fn into_target(item: &Mod, target: &LocalModpack, location: Option<String>) -> Self {
        Self {
            id: item.id.clone(),
            source: item.source.clone(),
            mod_type: item.mod_type,
            version: target.version.clone(),
            loader: target.mod_loader,
            modpack: Some(target.name.clone()),
            content_location: location.filter(|_| is_pack(item.mod_type)),
        }
    }

    pub async fn perform(self, host: QuadrantHost) -> anyhow::Result<()> {
        host.install_mod(
            self.id,
            self.version,
            self.loader,
            self.source,
            self.modpack,
            self.mod_type,
            None,
            self.content_location,
        )
        .await
    }
}

/// The install choices saved by the last install or search.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Saved {
    pub version: Option<String>,
    pub loader: Option<ModLoader>,
    pub modpack: Option<String>,
}

impl Saved {
    fn load(host: &QuadrantHost) -> Self {
        let read = |key: &str| {
            host.get_config_value(key)
                .ok()
                .flatten()
                .and_then(|value| value.as_str().map(str::to_string))
                .filter(|value| !value.is_empty())
        };
        Self {
            version: read("lastUsedVersion"),
            loader: read("lastUsedAPI")
                .map(ModLoader::from)
                .filter(|loader| *loader != ModLoader::Unknown),
            modpack: read("lastUsedModpack"),
        }
    }
}

/// Where the page proposes to install, before the user changes anything.
#[derive(Clone, Debug, PartialEq)]
pub struct Targets {
    pub version: String,
    pub loader: ModLoader,
    pub modpack: Option<String>,
}

/// Reconciles the opener's target and the saved choices with what exists,
/// so a deleted pack or an unlisted version never reaches the install. A
/// mod falls back to the saved, then the applied, then the first modpack;
/// packs follow only the opener's target.
pub fn initial_targets(
    mod_type: ModType,
    modpacks: &[LocalModpack],
    versions: &[String],
    explicit: Option<&str>,
    saved: &Saved,
) -> Targets {
    let find = |name: Option<&str>| {
        name.and_then(|name| modpacks.iter().find(|modpack| modpack.name == name))
    };
    let explicit = find(explicit);
    let target = if mod_type == ModType::Mod {
        explicit
            .or_else(|| find(saved.modpack.as_deref()))
            .or_else(|| modpacks.iter().find(|modpack| modpack.is_applied))
            .or_else(|| modpacks.first())
    } else {
        explicit
    };
    let wanted = explicit
        .map(|modpack| modpack.version.clone())
        .or_else(|| saved.version.clone());
    let version = wanted
        .filter(|wanted| versions.contains(wanted))
        .or_else(|| target.map(|modpack| modpack.version.clone()))
        .or_else(|| versions.first().cloned())
        .unwrap_or_default();
    let loader = explicit
        .map(|modpack| modpack.mod_loader)
        .or(saved.loader)
        .or_else(|| target.map(|modpack| modpack.mod_loader))
        .unwrap_or(ModLoader::Unknown);
    Targets {
        version,
        loader,
        modpack: target.map(|modpack| modpack.name.clone()),
    }
}

/// Saves install choices for the next install and search, as the desktop
/// app's `lastUsed*` settings.
pub fn remember(cx: &App, entries: Vec<(&'static str, String)>) {
    Backend::global(cx)
        .run(cx, move |host| async move {
            for (key, value) in entries {
                host.set_config_value(key, value.into())?;
            }
            Ok(())
        })
        .detach();
}

/// What the install page asks of the page that shows it.
pub enum InstallViewEvent {
    Close,
    /// Open the install page of a dependency.
    Open(Box<Mod>),
    /// An install finished; the opener refreshes what it shows.
    Installed,
}

struct Setup {
    modpacks: Vec<LocalModpack>,
}

type PickVersion = SelectState<SearchableVec<Choice<String>>>;
type PickLoader = SelectState<SearchableVec<Choice<ModLoader>>>;

pub struct ModInstallView {
    item: Mod,
    explicit_target: Option<String>,
    setup: Loadable<Setup>,
    owners: Loadable<Vec<String>>,
    deps: Loadable<Vec<Mod>>,
    version: Entity<PickVersion>,
    loader: Entity<PickLoader>,
    modpack: Entity<PickVersion>,
    /// The location the opener picked, while it is still listed.
    opener_location: Option<String>,
    install_to: Entity<LocationPicker>,
    running: Option<InstallRequest>,
    download_progress: i32,
    install_progress: i32,
    /// Its own, so each install page starts at the top.
    scroll: ScrollHandle,
    _progress: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InstallViewEvent> for ModInstallView {}

impl ModInstallView {
    pub fn new(
        item: Mod,
        explicit_target: Option<String>,
        opener_location: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let empty = || SearchableVec::new(Vec::new());
        let version: Entity<PickVersion> =
            cx.new(|cx| SelectState::new(empty(), None, window, cx).searchable(true));
        let loader: Entity<PickLoader> =
            cx.new(|cx| SelectState::new(SearchableVec::new(Vec::new()), None, window, cx));
        let modpack: Entity<PickVersion> = cx.new(|cx| SelectState::new(empty(), None, window, cx));
        let install_to = cx.new(|cx| LocationPicker::new(window, cx));
        let subscriptions = vec![
            cx.subscribe(&version, |_, _, event: &SelectEvent<_>, cx| {
                let SelectEvent::Confirm(Some(version)) = event else {
                    return;
                };
                let version: &String = version;
                remember(cx, vec![("lastUsedVersion", version.clone())]);
                cx.notify();
            }),
            cx.subscribe(&loader, |_, _, event: &SelectEvent<_>, cx| {
                let SelectEvent::Confirm(Some(loader)) = event else {
                    return;
                };
                let loader: &ModLoader = loader;
                remember(cx, vec![("lastUsedAPI", loader.to_string())]);
                cx.notify();
            }),
            cx.subscribe_in(&modpack, window, Self::modpack_picked),
        ];
        let id = item.id.clone();
        let progress = watch_mod_progress(cx, move |this: &mut Self, progress, cx| {
            if progress.mod_id == id {
                this.record(progress);
                cx.notify();
            }
        });
        let mut view = Self {
            item,
            explicit_target,
            setup: Loadable::Loading,
            owners: Loadable::Loading,
            deps: Loadable::Loading,
            version,
            loader,
            modpack,
            opener_location,
            install_to,
            running: None,
            download_progress: 0,
            install_progress: 0,
            scroll: ScrollHandle::new(),
            _progress: progress,
            _subscriptions: subscriptions,
        };
        view.load_setup(window, cx);
        view.load_details(cx);
        view
    }

    pub fn item(&self) -> &Mod {
        &self.item
    }

    fn record(&mut self, progress: ModProgress) {
        match progress.stage {
            ModStage::Download => self.download_progress = progress.percent,
            ModStage::Install => self.install_progress = progress.percent,
        }
    }

    fn load_setup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mod_type = self.item.mod_type;
        let task = Backend::global(cx).run(cx, move |host| async move {
            let versions = host
                .get_versions()
                .await?
                .into_iter()
                .map(|version| version.version)
                .collect::<Vec<_>>();
            let modpacks = host.get_modpacks(false).await?;
            let locations = install_location::list_for(&host, mod_type).await;
            Ok((versions, modpacks, Saved::load(&host), locations))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.setup =
                    Loadable::from_result(result.map(|(versions, modpacks, saved, locations)| {
                        this.fill_pickers(&versions, &modpacks, &saved, window, cx);
                        this.fill_locations(locations, &modpacks, window, cx);
                        Setup { modpacks }
                    }));
                cx.notify();
            });
        })
        .detach();
    }

    fn fill_pickers(
        &self,
        versions: &[String],
        modpacks: &[LocalModpack],
        saved: &Saved,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let targets = initial_targets(
            self.item.mod_type,
            modpacks,
            versions,
            self.explicit_target.as_deref(),
            saved,
        );
        let mut version_choices: Vec<_> = versions
            .iter()
            .map(|version| Choice::new(version.clone(), version.clone()))
            .collect();
        if !targets.version.is_empty() && !versions.contains(&targets.version) {
            version_choices.insert(
                0,
                Choice::new(targets.version.clone(), targets.version.clone()),
            );
        }
        self.version.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(version_choices), window, cx);
            select.set_selected_value(&targets.version, window, cx);
        });
        let providers = match self.item.source {
            ModSource::CurseForge => vec![Provider::CurseForge],
            ModSource::Modrinth => vec![Provider::Modrinth],
            ModSource::Online => Vec::new(),
        };
        let mut loaders = loader_options(&providers);
        if !loaders.contains(&targets.loader) {
            loaders.insert(0, targets.loader);
        }
        self.loader.update(cx, |select, cx| {
            let choices: Vec<_> = loaders
                .into_iter()
                .map(|loader| Choice::new(loader.to_string(), loader))
                .collect();
            select.set_items(SearchableVec::new(choices), window, cx);
            select.set_selected_value(&targets.loader, window, cx);
        });
        self.modpack.update(cx, |select, cx| {
            let choices: Vec<_> = modpacks
                .iter()
                .map(|modpack| {
                    Choice::new(
                        format!(
                            "{} · {} · {}",
                            modpack.name, modpack.mod_loader, modpack.version
                        ),
                        modpack.name.clone(),
                    )
                })
                .collect();
            select.set_items(SearchableVec::new(choices), window, cx);
            if let Some(name) = &targets.modpack {
                select.set_selected_value(name, window, cx);
            }
        });
    }

    /// Offers the content locations a pack can go to. Following the target
    /// modpack is a choice only when the opener named one that exists.
    fn fill_locations(
        &self,
        locations: Vec<quadrant_core::content::ContentLocation>,
        modpacks: &[LocalModpack],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .explicit_target
            .as_deref()
            .filter(|name| modpacks.iter().any(|modpack| &modpack.name == name));
        let selected = initial_location(
            &locations,
            target.is_some(),
            self.opener_location.as_deref(),
        );
        let automatic =
            target.map(|name| t_with("installedContentInstallAutomatic", &[("modpack", name)]));
        self.install_to.update(cx, |picker, cx| {
            picker.set_locations(locations, automatic, selected, window, cx)
        });
    }

    /// Picking a modpack adopts its version and loader, and remembers all
    /// three.
    fn modpack_picked(
        &mut self,
        _: &Entity<PickVersion>,
        event: &SelectEvent<SearchableVec<Choice<String>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(name)) = event else {
            return;
        };
        let Loadable::Ready(setup) = &self.setup else {
            return;
        };
        let Some(modpack) = setup.modpacks.iter().find(|modpack| &modpack.name == name) else {
            return;
        };
        let (version, loader) = (modpack.version.clone(), modpack.mod_loader);
        self.version.update(cx, |select, cx| {
            select.set_selected_value(&version, window, cx)
        });
        self.loader.update(cx, |select, cx| {
            select.set_selected_value(&loader, window, cx)
        });
        remember(
            cx,
            vec![
                ("lastUsedVersion", version),
                ("lastUsedAPI", loader.to_string()),
                ("lastUsedModpack", name.clone()),
            ],
        );
        cx.notify();
    }

    fn load_details(&mut self, cx: &mut Context<Self>) {
        let source = self.item.source.clone();
        let id = self.item.id.clone();
        let task = Backend::global(cx).run(cx, move |host| async move {
            match source {
                ModSource::Modrinth => Ok((
                    host.get_mod_owners_modrinth(id.clone()).await?,
                    host.get_mod_deps_modrinth(id).await?,
                )),
                #[cfg(feature = "curseforge")]
                ModSource::CurseForge => Ok((
                    host.get_mod_owners_curseforge(id.clone()).await?,
                    host.get_mod_deps_curseforge(id).await?,
                )),
                _ => Err(anyhow::anyhow!("No details for mods from this source.")),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((owners, deps)) => {
                        this.owners = Loadable::Ready(owners);
                        this.deps = Loadable::Ready(deps);
                    }
                    Err(error) => {
                        let error = describe(error);
                        this.owners = Loadable::Failed(error.clone());
                        this.deps = Loadable::Failed(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn selected_modpack(&self, cx: &App) -> Option<&LocalModpack> {
        let Loadable::Ready(setup) = &self.setup else {
            return None;
        };
        let name = self.modpack.read(cx).selected_value()?;
        setup.modpacks.iter().find(|modpack| &modpack.name == name)
    }

    /// The install the form describes, or `None` while it can't be made:
    /// still loading, no version, or a mod without a modpack.
    fn request(&self, cx: &App) -> Option<InstallRequest> {
        if !matches!(self.setup, Loadable::Ready(_)) {
            return None;
        }
        let version = self.version.read(cx).selected_value()?.clone();
        let modpack = self.modpack.read(cx).selected_value().cloned();
        if version.is_empty() || (self.item.mod_type == ModType::Mod && modpack.is_none()) {
            return None;
        }
        Some(InstallRequest {
            id: self.item.id.clone(),
            source: self.item.source.clone(),
            mod_type: self.item.mod_type,
            version,
            loader: self
                .loader
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or(ModLoader::Unknown),
            // A pack's is the opener's target, which it follows when no
            // location is picked.
            modpack,
            content_location: self
                .install_to
                .read(cx)
                .selected(cx)
                .filter(|_| is_pack(self.item.mod_type)),
        })
    }

    pub fn install(&mut self, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        let Some(request) = self.request(cx) else {
            return;
        };
        self.running = Some(request.clone());
        self.download_progress = 0;
        self.install_progress = 0;
        let task = Backend::global(cx).run(cx, move |host| request.perform(host));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                match result {
                    Ok(()) => {
                        snackbar::show(cx, SnackbarKind::Success, t("downloadSuccess"));
                        cx.emit(InstallViewEvent::Installed);
                        this.reload_modpacks(cx);
                    }
                    Err(error) => snackbar::show(cx, SnackbarKind::Error, describe(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Refreshes the "already installed" note after an install without
    /// touching the pickers.
    fn reload_modpacks(&mut self, cx: &mut Context<Self>) {
        let task =
            Backend::global(cx).run(cx, |host| async move { host.get_modpacks(false).await });
        cx.spawn(async move |this, cx| {
            let Ok(modpacks) = task.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Loadable::Ready(setup) = &mut this.setup {
                    setup.modpacks = modpacks;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_details(&self, window_width: Pixels) -> impl IntoElement {
        let item = &self.item;
        let source = source_label(&item.source);
        let type_key = match item.mod_type {
            ModType::ResourcePack => "resourcepack",
            ModType::ShaderPack => "shaderpack",
            _ => "mod",
        };
        let owners = match &self.owners {
            Loadable::Ready(owners) => owners.clone(),
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => Vec::new(),
        };
        theme::panel()
            .p_5()
            .min_w_0()
            .child(
                h_flex()
                    .items_start()
                    .gap_4()
                    .child(theme::mod_icon(
                        &item.mod_icon_url,
                        &item.name,
                        px(4. * theme::FONT_SIZE),
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::BLACK)
                                    .child(item.name.clone()),
                            )
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(theme::TEXT_MUTED))
                                    .child(theme::source_pill(
                                        t_with(type_key, &[("source", source)]),
                                        item.source == ModSource::CurseForge,
                                    ))
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .child(Icon::new(MdIcon::Download).size_3p5())
                                            .child(compact_count(item.download_count)),
                                    )
                                    .when(
                                        item.source == ModSource::Modrinth
                                            && !item.license.is_empty(),
                                        |this| {
                                            this.child(t_with(
                                                "licensedUnder",
                                                &[("license", &item.license)],
                                            ))
                                        },
                                    ),
                            )
                            .when(!item.description.trim().is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(rgb(theme::TEXT_SOFT))
                                        .child(item.description.clone()),
                                )
                            })
                            .when(!owners.is_empty(), |this| {
                                this.child(h_flex().mt_1().flex_wrap().gap_1p5().children(
                                    owners.into_iter().enumerate().map(|(index, owner)| {
                                        let url = get_user_url(owner.clone(), item.source.clone());
                                        h_flex()
                                            .id(("owner", index))
                                            .h_7()
                                            .px_2p5()
                                            .gap_1()
                                            .items_center()
                                            .rounded_full()
                                            .bg(rgb(theme::CONTROL))
                                            .hover(|this| this.bg(rgb(theme::CONTROL_HOVER)))
                                            .cursor_pointer()
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(rgb(theme::TEXT_BADGE))
                                            .child(Icon::new(MdIcon::Person).size_3p5())
                                            .child(owner)
                                            .on_click(move |_, _, cx| cx.open_url(&url))
                                    }),
                                ))
                            }),
                    ),
            )
            .when(!item.thumbnail_urls.is_empty(), |this| {
                this.child(
                    div()
                        .mt_4()
                        .grid()
                        .grid_cols(mod_grid::screenshot_columns(window_width) as u16)
                        .gap_2()
                        .children(item.thumbnail_urls.iter().enumerate().map(|(index, url)| {
                            let open = url.clone();
                            div()
                                .id(("thumbnail", index))
                                .w_full()
                                .aspect_ratio(16. / 9.)
                                .overflow_hidden()
                                .rounded_2xl()
                                .bg(rgb(theme::PAGE))
                                .cursor_pointer()
                                .on_click(move |_, _, cx| cx.open_url(&open))
                                .child(
                                    gpui::img(SharedString::from(url.clone()))
                                        .size_full()
                                        .rounded_2xl()
                                        .object_fit(gpui::ObjectFit::Cover),
                                )
                        })),
                )
            })
    }

    fn render_deps(
        &self,
        window_width: Pixels,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let Loadable::Ready(deps) = &self.deps else {
            return None;
        };
        if deps.is_empty() {
            return None;
        }
        let selected = self.selected_modpack(cx).cloned();
        let on_press: OnCardPress<Self> = Rc::new(|_, press, item, _, cx| match press {
            CardPress::Primary(_) | CardPress::Open => {
                cx.emit(InstallViewEvent::Open(Box::new(item.clone())))
            }
            CardPress::Delete | CardPress::Select => {}
        });
        let cards: Vec<_> = deps
            .iter()
            .enumerate()
            .map(|(index, dep)| {
                let status = CardStatus {
                    installed: installed_in(dep, selected.as_ref()).is_some(),
                    ..CardStatus::default()
                };
                mod_card(index, dep, status, None, on_press.clone(), cx).into_any_element()
            })
            .collect();
        Some(
            theme::panel()
                .p_5()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_size(px(13.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(Icon::new(MdIcon::Extension).size_4())
                        .child(
                            t("dependencies")
                                .trim_end()
                                .trim_end_matches(':')
                                .to_string(),
                        )
                        .child(count_badge(deps.len())),
                )
                .child(
                    div()
                        .grid()
                        .grid_cols(mod_grid::dependency_columns(window_width) as u16)
                        .gap_2()
                        .children(cards),
                ),
        )
    }

    fn render_form(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let item = &self.item;
        let is_mod = item.mod_type == ModType::Mod;
        let selected = self.selected_modpack(cx);
        let installed = installed_in(item, selected);
        let reinstall = installed.is_some();
        let modpack_name = selected.map(|modpack| modpack.name.clone());
        let can_install = self.running.is_none() && self.request(cx).is_some();
        let showing_progress = self.running.is_some() || self.download_progress > 0;
        let note = installed.map(|installed_from| {
            let modpack = modpack_name.clone().unwrap_or_default();
            if installed_from == item.source {
                t_with("alreadyInstalledIn", &[("modpack", &modpack)])
            } else {
                t_with(
                    "alreadyInstalledOtherProvider",
                    &[
                        ("modpack", &modpack),
                        ("source", source_label(&installed_from)),
                    ],
                )
            }
        });
        let field = |label: &str, control: gpui::AnyElement| {
            v_flex()
                .child(
                    div()
                        .mb_1p5()
                        .text_size(px(13.))
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .text_color(rgb(theme::TEXT_MUTED))
                        .child(t(label)),
                )
                .child(control)
        };
        let body = match &self.setup {
            Loadable::Idle | Loadable::Loading => h_flex()
                .p_4()
                .justify_center()
                .child(Spinner::new())
                .into_any_element(),
            Loadable::Failed(error) => div().child(error.clone()).into_any_element(),
            Loadable::Ready(_) => v_flex()
                .gap_3()
                .child(field("chooseVersion", picker(Select::new(&self.version))))
                .when(is_mod, |this| {
                    this.child(field(
                        "choosePreferredAPI",
                        picker(Select::new(&self.loader)),
                    ))
                    .child(field("chooseModpack", picker(Select::new(&self.modpack))))
                })
                .children(
                    self.install_to
                        .read(cx)
                        .select()
                        .map(|select| field("installedContentInstallTo", picker(select))),
                )
                .into_any_element(),
        };
        let install_tone = if can_install {
            Tone::Positive
        } else {
            Tone::Control
        };
        theme::panel()
            .p_5()
            .flex()
            .flex_col()
            .gap_3()
            .child(body)
            .when_some(note, |this, note| this.child(theme::warning_note(note)))
            .when(showing_progress, |this| {
                this.child(
                    v_flex()
                        .gap_1p5()
                        .child(theme::progress_bar(self.download_progress))
                        .child(theme::progress_bar(self.install_progress)),
                )
            })
            .child(
                h_flex()
                    .mt_1()
                    .gap_2()
                    .child(
                        theme::wide_button(
                            "install",
                            install_tone,
                            t(if reinstall { "reinstall" } else { "download" }),
                            MdIcon::Download,
                            cx,
                        )
                        .flex_1()
                        .loading(self.running.is_some())
                        .disabled(!can_install)
                        .on_click(cx.listener(|this, _, _, cx| this.install(cx))),
                    )
                    .child({
                        let url = item.url.clone();
                        theme::wide_button(
                            "open-web",
                            Tone::Control,
                            t("openInTheWeb"),
                            MdIcon::OpenInNew,
                            cx,
                        )
                        .flex_1()
                        .on_click(move |_, _, cx| cx.open_url(&url))
                    }),
            )
    }
}

fn picker<E: gpui::Styled + IntoElement>(select: E) -> gpui::AnyElement {
    select
        .h_10()
        .rounded_full()
        .bg(rgb(theme::CONTROL))
        .text_sm()
        .font_weight(FontWeight::BOLD)
        .into_any_element()
}

fn count_badge(count: usize) -> impl IntoElement {
    div()
        .min_w_5()
        .h_5()
        .px_1p5()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(theme::SELECTED))
        .text_color(rgb(theme::TEXT))
        .text_size(px(11.))
        .child(count.to_string())
}

impl Render for ModInstallView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let window_width = window.viewport_size().width;
        let wide = mod_grid::at_least(window_width, 64.);
        v_flex()
            .id("mod-install")
            .size_full()
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .gap_3()
            .px_4()
            .pt_2()
            .pb_5()
            .child(
                h_flex().child(
                    theme::cancel_button("cancel", t("cancel"), cx)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(InstallViewEvent::Close))),
                ),
            )
            // `lg:grid-cols-[minmax(0,1fr)_22rem]`: the form beside the
            // details from Tailwind's `lg` up, below them before that.
            .child(
                div()
                    .flex()
                    .gap_3()
                    .map(|this| {
                        if wide {
                            this.flex_row().items_start()
                        } else {
                            this.flex_col()
                        }
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_3()
                            .child(self.render_details(window_width))
                            .children(self.render_deps(window_width, cx)),
                    )
                    .child(
                        div()
                            .when(wide, |this| this.w(px(22. * theme::FONT_SIZE)).flex_none())
                            .child(self.render_form(cx)),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::search::logic::tests::{modpack, result};

    fn versions() -> Vec<String> {
        ["1.21.1", "1.20.1"].map(String::from).to_vec()
    }

    #[test]
    fn the_openers_target_wins() {
        let packs = [
            modpack("Alpha", "1.21.1", ModLoader::Fabric),
            modpack("Beta", "1.20.1", ModLoader::Forge),
        ];
        let saved = Saved {
            version: Some("1.21.1".into()),
            loader: Some(ModLoader::Quilt),
            modpack: Some("Alpha".into()),
        };
        assert_eq!(
            initial_targets(ModType::Mod, &packs, &versions(), Some("Beta"), &saved),
            Targets {
                version: "1.20.1".into(),
                loader: ModLoader::Forge,
                modpack: Some("Beta".into()),
            }
        );
    }

    #[test]
    fn a_mod_falls_back_to_saved_then_applied_then_first() {
        let mut packs = vec![
            modpack("Alpha", "1.21.1", ModLoader::Fabric),
            modpack("Beta", "1.20.1", ModLoader::Forge),
        ];
        let saved = Saved {
            modpack: Some("Gone".into()),
            ..Saved::default()
        };
        let first = initial_targets(ModType::Mod, &packs, &versions(), None, &saved);
        assert_eq!(first.modpack.as_deref(), Some("Alpha"));
        packs[1].is_applied = true;
        let applied = initial_targets(ModType::Mod, &packs, &versions(), Some("Gone"), &saved);
        assert_eq!(
            applied,
            Targets {
                version: "1.20.1".into(),
                loader: ModLoader::Forge,
                modpack: Some("Beta".into()),
            }
        );
    }

    #[test]
    fn an_unlisted_saved_version_gives_way() {
        let packs = [modpack("Alpha", "1.21.1", ModLoader::Fabric)];
        let saved = Saved {
            version: Some("0.0.1".into()),
            loader: Some(ModLoader::Quilt),
            modpack: None,
        };
        let targets = initial_targets(ModType::Mod, &packs, &versions(), None, &saved);
        assert_eq!(targets.version, "1.21.1");
        assert_eq!(targets.loader, ModLoader::Quilt);
    }

    #[test]
    fn packs_follow_only_the_openers_target() {
        let mut packs = vec![modpack("Alpha", "1.20.1", ModLoader::Fabric)];
        packs[0].is_applied = true;
        let targets = initial_targets(
            ModType::ShaderPack,
            &packs,
            &versions(),
            None,
            &Saved::default(),
        );
        assert_eq!(
            targets,
            Targets {
                version: "1.21.1".into(),
                loader: ModLoader::Unknown,
                modpack: None,
            }
        );
        assert_eq!(
            initial_targets(ModType::Mod, &[], &[], None, &Saved::default()).version,
            ""
        );
    }

    #[test]
    fn only_a_pack_goes_to_a_picked_location() {
        let target = modpack("Alpha", "1.21.1", ModLoader::Fabric);
        let mut item = result("Bliss", ModSource::Modrinth, 0, "");
        let location = || Some("prism:survival".to_string());
        assert_eq!(
            InstallRequest::into_target(&item, &target, location()).content_location,
            None
        );
        item.mod_type = ModType::ShaderPack;
        let request = InstallRequest::into_target(&item, &target, location());
        assert_eq!(request.content_location, location());
        assert_eq!(request.modpack.as_deref(), Some("Alpha"));
    }
}
