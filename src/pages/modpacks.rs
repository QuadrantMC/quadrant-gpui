//! The Apply page: local modpacks, after `ApplyPage` in the desktop app.

use std::path::PathBuf;

use chrono::{Local, TimeZone as _};
use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FontWeight, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{
    Disableable as _, IndexPath, WindowExt as _,
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement as _,
    select::{SearchableVec, Select, SelectItem, SelectState},
    spinner::Spinner,
    v_flex,
};
use quadrant_core::models::{LocalModpack, ModLoader};
use quadrant_host::QuadrantHost;

use crate::{
    assets::MdIcon,
    backend::Backend,
    i18n::{describe, t, t_with},
    loadable::Loadable,
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

mod details;

/// Loader choices in the order the desktop app offers them.
const LOADERS: [ModLoader; 13] = [
    ModLoader::Fabric,
    ModLoader::Forge,
    ModLoader::NeoForge,
    ModLoader::Babric,
    ModLoader::BtaBabric,
    ModLoader::JavaAgent,
    ModLoader::LegacyFabric,
    ModLoader::LiteLoader,
    ModLoader::RisugamisModLoader,
    ModLoader::NilLoader,
    ModLoader::Ornithe,
    ModLoader::Quilt,
    ModLoader::Rift,
];

/// The scratch modpack "Clear" swaps in so no mods are applied.
const FREE_MODPACK: &str = "free";

/// What the create dialog submits.
#[derive(Clone, Debug, PartialEq)]
pub struct NewModpack {
    pub name: String,
    pub version: String,
    pub loader: ModLoader,
}

impl NewModpack {
    /// Checks what the backend can't: that every field was filled in. Whether
    /// the name is usable is the backend's call.
    pub fn parse(
        name: &str,
        version: &str,
        loader: Option<ModLoader>,
    ) -> Result<Self, &'static str> {
        let name = name.trim();
        let version = version.trim();
        if name.is_empty() {
            return Err("Give the modpack a name.");
        }
        if version.is_empty() {
            return Err("Choose a Minecraft version.");
        }
        let loader = loader.ok_or("Choose a mod loader.")?;
        Ok(Self {
            name: name.to_string(),
            version: version.to_string(),
            loader,
        })
    }
}

/// A change to the modpacks folder. At most one runs at a time.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Apply(String),
    Delete(String),
    Create(NewModpack),
    /// Replaces the applied modpack with an empty one, like the desktop app's
    /// "Clear" button. Carries the Minecraft version the empty pack targets.
    Clear(String),
    /// Renames a modpack or changes its version or loader.
    Update {
        from: String,
        to: NewModpack,
    },
    Export {
        name: String,
        to: PathBuf,
    },
}

impl Op {
    pub async fn perform(self, host: QuadrantHost) -> anyhow::Result<()> {
        match self {
            Op::Apply(name) => host.frontend_apply_modpack(name),
            Op::Delete(name) => host.delete_modpack(name).await,
            Op::Create(new) => host.create_modpack(new.name, new.version, new.loader).await,
            Op::Clear(version) => {
                // A missing scratch pack is the normal first-run case.
                let _ = host.delete_modpack(FREE_MODPACK.into()).await;
                host.create_modpack(FREE_MODPACK.into(), version, ModLoader::Unknown)
                    .await?;
                host.frontend_apply_modpack(FREE_MODPACK.into())
            }
            Op::Update { from, to } => {
                host.update_modpack(from, Some(to.name), Some(to.version), Some(to.loader))
                    .await
            }
            Op::Export { name, to } => host.export_modpack_to(name, to).await,
        }
    }

    fn done_text(&self) -> SharedString {
        match self {
            Op::Apply(_) => t("setModpackSuccess"),
            Op::Delete(name) => format!("Deleted \"{name}\"").into(),
            Op::Create(new) => format!("Created \"{}\"", new.name).into(),
            Op::Clear(_) => "Cleared the applied modpack".into(),
            // The desktop app's own summary of the saved details.
            Op::Update { to, .. } => format!("{} | {} | {}", to.name, to.version, to.loader).into(),
            Op::Export { name, .. } => format!("Exported \"{name}\"").into(),
        }
    }

    /// The modpack whose card shows this op as running.
    fn target(&self) -> Option<&str> {
        match self {
            Op::Apply(name)
            | Op::Delete(name)
            | Op::Update { from: name, .. }
            | Op::Export { name, .. } => Some(name),
            Op::Create(_) | Op::Clear(_) => None,
        }
    }
}

/// The applied pack first, then the most recently synced, then by name.
pub fn sort_modpacks(mut modpacks: Vec<LocalModpack>) -> Vec<LocalModpack> {
    modpacks.sort_by(|a, b| {
        b.is_applied
            .cmp(&a.is_applied)
            .then(b.last_synced.cmp(&a.last_synced))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    modpacks
}

/// The packs whose name, version or loader contains `query`, ignoring case.
pub fn matches_query(modpack: &LocalModpack, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || modpack.name.to_lowercase().contains(&query)
        || modpack.version.to_lowercase().contains(&query)
        || modpack
            .mod_loader
            .to_string()
            .to_lowercase()
            .contains(&query)
}

/// "1.21.1 | Fabric | 3 Mod(s)", plus the local sync date once there is one.
pub fn metadata_line(modpack: &LocalModpack) -> String {
    let mut line = format!(
        "{} | {} | {}",
        modpack.version,
        modpack.mod_loader,
        t_with("modCount", &[("amount", &modpack.mods.len().to_string())])
    );
    if let Some(date) = Local.timestamp_millis_opt(modpack.last_synced).single()
        && modpack.last_synced != 0
    {
        let date = date.format("%A, %D, %H:%M").to_string();
        line.push_str(" | ");
        line.push_str(&t_with("localSyncDate", &[("date", &date)]));
    }
    line
}

#[derive(Clone)]
struct LoaderItem(ModLoader);

impl SelectItem for LoaderItem {
    type Value = ModLoader;

    fn title(&self) -> SharedString {
        self.0.to_string().into()
    }

    fn value(&self) -> &ModLoader {
        &self.0
    }
}

/// The version field: a list when the release list loaded, otherwise free
/// text so a modpack can still be created offline.
#[derive(Clone)]
enum VersionField {
    Pick(Entity<SelectState<SearchableVec<SharedString>>>),
    Type(Entity<InputState>),
}

impl VersionField {
    fn value(&self, cx: &App) -> String {
        match self {
            VersionField::Pick(select) => select
                .read(cx)
                .selected_value()
                .map(ToString::to_string)
                .unwrap_or_default(),
            VersionField::Type(input) => input.read(cx).value().to_string(),
        }
    }
}

#[derive(Clone)]
struct ModpackForm {
    name: Entity<InputState>,
    version: VersionField,
    loader: Entity<SelectState<Vec<LoaderItem>>>,
}

impl ModpackForm {
    fn submit(&self, cx: &App) -> Result<NewModpack, &'static str> {
        NewModpack::parse(
            &self.name.read(cx).value(),
            &self.version.value(cx),
            self.loader.read(cx).selected_value().copied(),
        )
    }
}

/// What the page shows: the list, or one modpack's mods.
enum View {
    List,
    Details(LocalModpack),
}

pub struct ModpacksPage {
    view: View,
    modpacks: Loadable<Vec<LocalModpack>>,
    /// Minecraft release versions, newest first.
    versions: Loadable<Vec<SharedString>>,
    running: Option<Op>,
    search: Entity<InputState>,
    /// The newest listing request; replacing it drops a stale one's answer.
    listing: Task<()>,
    _search_changed: Subscription,
}

impl ModpacksPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t("searchModpacksPlaceholder")));
        let search_changed = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let mut page = Self {
            view: View::List,
            modpacks: Loadable::Idle,
            versions: Loadable::Idle,
            running: None,
            search,
            listing: Task::ready(()),
            _search_changed: search_changed,
        };
        page.refresh(cx);
        page.load_versions(cx);
        page
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        // A loaded list stays on screen while it reloads, so acting on a card
        // doesn't blank the page.
        if !matches!(self.modpacks, Loadable::Ready(_)) {
            self.modpacks = Loadable::Loading;
        }
        let task = Backend::global(cx).run(cx, |host| async move { host.get_modpacks(true).await });
        self.listing = cx.spawn(async move |this, cx| {
            let result = task.await.map(sort_modpacks);
            let _ = this.update(cx, |this, cx| {
                this.modpacks = Loadable::from_result(result);
                cx.notify();
            });
        });
        cx.notify();
    }

    fn load_versions(&mut self, cx: &mut Context<Self>) {
        self.versions = Loadable::Loading;
        let task = Backend::global(cx).run(cx, |host| async move {
            let versions = host.get_versions().await?;
            Ok(versions
                .into_iter()
                .map(|version| SharedString::from(version.version))
                .collect::<Vec<_>>())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.versions = Loadable::from_result(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn run(&mut self, op: Op, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        self.running = Some(op.clone());
        let done_text = op.done_text();
        let task = Backend::global(cx).run(cx, move |host| op.perform(host));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                match result {
                    Ok(()) => snackbar::show(cx, SnackbarKind::Success, done_text),
                    Err(error) => snackbar::show(cx, SnackbarKind::Error, describe(error)),
                }
                this.refresh(cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        let newest = match &self.versions {
            Loadable::Ready(versions) => versions.first().cloned(),
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => None,
        };
        match newest {
            Some(version) => self.run(Op::Clear(version.to_string()), cx),
            None => snackbar::show(
                cx,
                SnackbarKind::Error,
                "Clearing needs the Minecraft version list, which hasn't loaded.",
            ),
        }
    }

    fn open_folder(&mut self, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move {
            let folder = host.get_modpacks_folder()?;
            std::fs::create_dir_all(&folder)?;
            Ok(folder)
        });
        cx.spawn(async move |_, cx| {
            if let Ok(folder) = task.await {
                cx.update(|cx| cx.open_with_system(&folder));
            }
        })
        .detach();
    }

    pub fn open_create_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_form(None, window, cx);
    }

    /// The desktop app's `ModpackEditDialog` on an existing modpack.
    pub fn open_update_dialog(
        &mut self,
        modpack: LocalModpack,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_form(Some(modpack), window, cx);
    }

    /// Creates a modpack, or updates `editing` when there is one.
    fn open_form(
        &mut self,
        editing: Option<LocalModpack>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let version = match &self.versions {
            Loadable::Ready(versions) => {
                let selected = editing
                    .as_ref()
                    .and_then(|modpack| versions.iter().position(|each| *each == modpack.version))
                    .unwrap_or_default();
                let versions = SearchableVec::new(versions.clone());
                VersionField::Pick(cx.new(|cx| {
                    SelectState::new(
                        versions,
                        Some(IndexPath::default().row(selected)),
                        window,
                        cx,
                    )
                    .searchable(true)
                }))
            }
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => {
                let typed = editing.as_ref().map(|modpack| modpack.version.clone());
                VersionField::Type(cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("1.21.1")
                        .default_value(typed.unwrap_or_default())
                }))
            }
        };
        let loader = editing
            .as_ref()
            .and_then(|modpack| LOADERS.iter().position(|each| *each == modpack.mod_loader))
            .map(|row| IndexPath::default().row(row));
        let name = editing.as_ref().map(|modpack| modpack.name.clone());
        let form = ModpackForm {
            name: cx.new(|cx| InputState::new(window, cx).default_value(name.unwrap_or_default())),
            version,
            loader: cx
                .new(|cx| SelectState::new(LOADERS.map(LoaderItem).to_vec(), loader, window, cx)),
        };
        let original = editing.map(|modpack| modpack.name);
        let page = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let version_field = match &form.version {
                VersionField::Pick(select) => {
                    pill(Select::new(select).appearance(false)).into_any_element()
                }
                VersionField::Type(input) => {
                    pill(Input::new(input).appearance(false)).into_any_element()
                }
            };
            let submit = {
                let form = form.clone();
                let page = page.clone();
                let original = original.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| match form.submit(cx) {
                    Ok(new) => {
                        let op = match &original {
                            Some(from) => Op::Update {
                                from: from.clone(),
                                to: new,
                            },
                            None => Op::Create(new),
                        };
                        page.update(cx, |page, cx| page.run(op, cx));
                        window.close_dialog(cx);
                    }
                    Err(problem) => snackbar::show(cx, SnackbarKind::Error, problem),
                }
            };
            let (title, name_label) = match original {
                Some(_) => (t("update"), t("chooseModpack")),
                None => (t("createModpack"), t("name")),
            };
            theme::dialog_surface(dialog)
                .w(px(560.))
                .title(theme::dialog_title(title.clone()))
                .child(
                    v_flex()
                        .gap_4()
                        .when(original.is_some(), |this| {
                            this.child(t("updateModpackDetails"))
                        })
                        .child(field(t("chooseVersion"), version_field))
                        .child(field(
                            t("choosePreferredAPI"),
                            pill(Select::new(&form.loader).placeholder("-").appearance(false)),
                        ))
                        .child(field(
                            name_label,
                            pill(Input::new(&form.name).appearance(false)),
                        )),
                )
                .footer(
                    DialogFooter::new()
                        .justify_start()
                        .child(
                            theme::pill_button("create", Tone::Positive, title, None, cx)
                                .on_click(submit),
                        )
                        .child(
                            theme::cancel_button("cancel", t("cancel"), cx)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
        });
    }

    /// Asks where to save the export, then exports there.
    fn export(&mut self, name: String, cx: &mut Context<Self>) {
        let folder = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let chosen = cx.prompt_for_new_path(&folder, Some(&format!("{name}.quadrantExport.zip")));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(to))) = chosen.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.run(Op::Export { name, to }, cx));
        })
        .detach();
    }

    /// Opens `name`'s Details or edit dialog, or goes back to the list, for
    /// screenshots.
    #[cfg(feature = "screenshot")]
    pub fn preview(
        &mut self,
        name: Option<&str>,
        edit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let found = match (&self.modpacks, name) {
            (Loadable::Ready(modpacks), Some(name)) => modpacks
                .iter()
                .find(|modpack| modpack.name == name)
                .cloned(),
            _ => None,
        };
        match found {
            Some(modpack) if edit => self.open_update_dialog(modpack, window, cx),
            Some(modpack) => self.show(View::Details(modpack), cx),
            None => self.show(View::List, cx),
        }
    }

    fn show(&mut self, view: View, cx: &mut Context<Self>) {
        self.view = view;
        cx.notify();
    }

    pub fn confirm_delete(
        &mut self,
        name: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let page = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let delete = {
                let page = page.clone();
                let name = name.clone();
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    page.update(cx, |page, cx| page.run(Op::Delete(name.to_string()), cx));
                    window.close_dialog(cx);
                }
            };
            theme::dialog_surface(dialog)
                .w(px(520.))
                .title(theme::dialog_title(format!("Delete \"{name}\"?")))
                .child("This removes the modpack folder and every mod in it.")
                .footer(
                    DialogFooter::new()
                        .justify_start()
                        .child(
                            theme::pill_button(
                                "confirm-delete",
                                Tone::Danger,
                                t("delete"),
                                Some(MdIcon::Delete),
                                cx,
                            )
                            .on_click(delete),
                        )
                        .child(
                            theme::cancel_button("cancel", t("cancel"), cx)
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
        });
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.running.is_some();
        h_flex()
            .mx_8()
            .my_4()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .child(pill(Input::new(&self.search).appearance(false)).px_4()),
            )
            .child(
                theme::pill_button("create", Tone::Positive, t("createModpack"), None, cx)
                    .icon(gpui_component::Icon::new(MdIcon::Add).size_5())
                    .disabled(busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_create_dialog(window, cx)),
                    ),
            )
            .child(
                theme::toolbar_button("clear", Tone::Control, MdIcon::Clear, cx)
                    .tooltip(t("clear"))
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
            )
            .child(
                theme::toolbar_button("open-folder", Tone::Control, MdIcon::Folder, cx)
                    .tooltip(t("openModpacksFolder"))
                    .on_click(cx.listener(|this, _, _, cx| this.open_folder(cx))),
            )
            .child(
                theme::toolbar_button("refresh", Tone::Control, MdIcon::Refresh, cx)
                    .tooltip("Refresh")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
    }

    fn render_card(
        &self,
        index: usize,
        modpack: &LocalModpack,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = SharedString::from(modpack.name.clone());
        let running_here =
            self.running.as_ref().and_then(Op::target) == Some(modpack.name.as_str());
        let busy = self.running.is_some();
        let pending = |id: &'static str, tone, label: &str, icon| {
            theme::pill_button((id, index), tone, t(label), Some(icon), cx)
                .tooltip("Not available in this build yet")
                .disabled(true)
        };
        let apply = if modpack.is_applied {
            theme::pill_button(
                ("apply", index),
                Tone::Done,
                t("applied"),
                Some(MdIcon::Check),
                cx,
            )
        } else {
            let name = name.clone();
            theme::pill_button(
                ("apply", index),
                Tone::Positive,
                t("apply"),
                Some(MdIcon::Check),
                cx,
            )
            .loading(running_here && matches!(self.running, Some(Op::Apply(_))))
            .disabled(busy)
            .on_click(cx.listener(move |this, _, _, cx| this.run(Op::Apply(name.to_string()), cx)))
        };
        let delete =
            {
                let name = name.clone();
                theme::pill_button(
                    ("delete", index),
                    Tone::Destructive,
                    t("delete"),
                    Some(MdIcon::Delete),
                    cx,
                )
                .loading(running_here && matches!(self.running, Some(Op::Delete(_))))
                .disabled(busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.confirm_delete(name.clone(), window, cx)
                }))
            };
        let update = {
            let modpack = modpack.clone();
            theme::pill_button(
                ("update", index),
                Tone::Primary,
                t("update"),
                Some(MdIcon::Edit),
                cx,
            )
            .loading(running_here && matches!(self.running, Some(Op::Update { .. })))
            .disabled(busy)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_update_dialog(modpack.clone(), window, cx)
            }))
        };
        let export = {
            let name = name.clone();
            theme::pill_button(
                ("export", index),
                Tone::Neutral,
                t("export"),
                Some(MdIcon::Archive),
                cx,
            )
            .loading(running_here && matches!(self.running, Some(Op::Export { .. })))
            .disabled(busy)
            .on_click(cx.listener(move |this, _, _, cx| this.export(name.to_string(), cx)))
        };
        let details = {
            let modpack = modpack.clone();
            theme::pill_button(
                ("details", index),
                Tone::Neutral,
                t("details"),
                Some(MdIcon::Info),
                cx,
            )
            .on_click(
                cx.listener(move |this, _, _, cx| this.show(View::Details(modpack.clone()), cx)),
            )
        };
        v_flex()
            .mx_5()
            .p_4()
            .rounded(theme::radius_card())
            .bg(rgb(theme::PAGE))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::EXTRA_BOLD)
                            .child(name),
                    )
                    .child(theme::badge(t("badgeLocal"), MdIcon::Computer)),
            )
            .child(
                div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(metadata_line(modpack)),
            )
            .child(
                h_flex()
                    .my_2()
                    .flex_wrap()
                    .gap_4()
                    .child(apply)
                    .child(pending("share", Tone::Primary, "share", MdIcon::Share))
                    .child(pending("sync", Tone::Positive, "sync", MdIcon::Sync))
                    .child(update)
                    .child(export)
                    .child(delete)
                    .child(details),
            )
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value();
        let body = match &self.modpacks {
            Loadable::Idle | Loadable::Loading => status_row(Spinner::new().into_any_element()),
            Loadable::Failed(error) => status_row(
                v_flex()
                    .items_center()
                    .gap_2()
                    .child(error.clone())
                    .child(
                        theme::pill_button(
                            "retry",
                            Tone::Neutral,
                            "Try again",
                            Some(MdIcon::Refresh),
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .into_any_element(),
            ),
            Loadable::Ready(modpacks) => {
                let shown: Vec<_> = modpacks
                    .iter()
                    .filter(|modpack| matches_query(modpack, &query))
                    .enumerate()
                    .map(|(index, modpack)| self.render_card(index, modpack, cx).into_any_element())
                    .collect();
                if shown.is_empty() {
                    status_row("No modpacks here yet.".into_any_element())
                } else {
                    v_flex().py_5().gap_5().children(shown).into_any_element()
                }
            }
        };
        div()
            .mx_6()
            .my_4()
            .rounded(theme::radius_card())
            .bg(rgb(theme::SURFACE))
            .child(body)
    }
}

fn status_row(content: gpui::AnyElement) -> gpui::AnyElement {
    h_flex()
        .p_8()
        .justify_center()
        .text_color(rgb(theme::TEXT_MUTED))
        .child(content)
        .into_any_element()
}

/// The desktop app's `.input`: an `h-10` slate-700 pill.
fn pill<E: gpui::Styled>(control: E) -> E {
    control.h_10().rounded_full().bg(rgb(theme::CONTROL))
}

fn field(label: SharedString, control: impl IntoElement) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(div().font_weight(FontWeight::BOLD).child(label))
        .child(control)
}

impl Render for ModpacksPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = v_flex().size_full().overflow_y_scrollbar();
        match &self.view {
            View::List => page
                .child(self.render_toolbar(cx))
                .child(self.render_list(cx)),
            View::Details(modpack) => {
                let header = format!("{} | {}", modpack.name, metadata_line(modpack));
                let mods = modpack.mods.clone();
                page.child(details::render(header.into(), None, &mods, cx))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_core::models::InstalledMod;
    use quadrant_host::QuadrantHostOptions;

    fn modpack(name: &str, applied: bool, last_synced: i64) -> LocalModpack {
        LocalModpack {
            name: name.into(),
            version: "1.21.1".into(),
            mod_loader: ModLoader::Fabric,
            mods: Vec::new(),
            unknown_mods: false,
            is_applied: applied,
            last_synced,
            modpack_id: None,
        }
    }

    #[test]
    fn applied_first_then_recently_synced_then_by_name() {
        let sorted = sort_modpacks(vec![
            modpack("beta", false, 0),
            modpack("Alpha", false, 0),
            modpack("synced", false, 5),
            modpack("applied", true, 0),
        ]);
        let names: Vec<_> = sorted.iter().map(|modpack| modpack.name.as_str()).collect();
        assert_eq!(names, ["applied", "synced", "Alpha", "beta"]);
    }

    #[test]
    fn search_matches_name_version_and_loader() {
        let pack = modpack("Alpha Pack", false, 0);
        assert!(matches_query(&pack, ""));
        assert!(matches_query(&pack, "alpha"));
        assert!(matches_query(&pack, "1.21"));
        assert!(matches_query(&pack, "FABRIC"));
        assert!(!matches_query(&pack, "forge"));
    }

    #[test]
    fn metadata_line_counts_mods_and_hides_unsynced_dates() {
        let mut pack = modpack("Alpha Pack", false, 0);
        pack.mods.push(InstalledMod::minimal(
            "id".into(),
            quadrant_core::models::ModSource::Modrinth,
            "https://cdn.modrinth.com/x.jar".into(),
        ));
        assert_eq!(metadata_line(&pack), "1.21.1 | Fabric | 1 Mod(s)");
        pack.last_synced = 1_700_000_000_000;
        assert!(metadata_line(&pack).contains("| Local version: "));
    }

    #[test]
    fn create_form_needs_every_field() {
        assert_eq!(
            NewModpack::parse("  Pack ", " 1.21.1 ", Some(ModLoader::Quilt)),
            Ok(NewModpack {
                name: "Pack".into(),
                version: "1.21.1".into(),
                loader: ModLoader::Quilt,
            })
        );
        assert!(NewModpack::parse(" ", "1.21.1", Some(ModLoader::Quilt)).is_err());
        assert!(NewModpack::parse("Pack", "", Some(ModLoader::Quilt)).is_err());
        assert!(NewModpack::parse("Pack", "1.21.1", None).is_err());
    }

    /// Runs each op against a throwaway Minecraft folder through the same host
    /// calls the page makes, and checks the folder afterwards. Set
    /// `QUADRANT_GPUI_KEEP_OPS_DIR` to an empty folder to keep the result for
    /// inspection.
    #[test]
    fn ops_change_the_modpacks_folder() {
        let temp = tempfile::tempdir().unwrap();
        let root = std::env::var_os("QUADRANT_GPUI_KEEP_OPS_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| temp.path().to_path_buf());
        let mc = root.join("mc");
        let mut options = QuadrantHostOptions::new(root.join("data"), "", "", "");
        options.mc_folder = Some(mc.clone());
        let host = QuadrantHost::new(options).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let run = |op: Op| runtime.block_on(op.perform(host.clone()));
        let names = || -> Vec<String> {
            let mut names: Vec<_> = runtime
                .block_on(host.get_modpacks(false))
                .unwrap()
                .into_iter()
                .map(|modpack| {
                    format!(
                        "{}{}",
                        modpack.name,
                        if modpack.is_applied { "*" } else { "" }
                    )
                })
                .collect();
            names.sort();
            names
        };

        run(Op::Create(NewModpack {
            name: "Alpha".into(),
            version: "1.21.1".into(),
            loader: ModLoader::Fabric,
        }))
        .unwrap();
        let manifest = std::fs::read_to_string(mc.join("modpacks/Alpha/modConfigV2.json")).unwrap();
        assert!(manifest.contains("\"modLoader\": \"Fabric\""), "{manifest}");
        assert!(
            run(Op::Create(NewModpack {
                name: "Alpha".into(),
                version: "1.20.1".into(),
                loader: ModLoader::Forge,
            }))
            .is_err()
        );

        run(Op::Apply("Alpha".into())).unwrap();
        assert_eq!(names(), ["Alpha*"]);
        assert_eq!(
            std::fs::read_link(mc.join("mods")).unwrap(),
            mc.join("modpacks").join("Alpha")
        );

        run(Op::Clear("1.21.1".into())).unwrap();
        assert_eq!(names(), ["Alpha", "free*"]);

        run(Op::Update {
            from: "Alpha".into(),
            to: NewModpack {
                name: "Beta".into(),
                version: "1.20.1".into(),
                loader: ModLoader::Forge,
            },
        })
        .unwrap();
        assert_eq!(names(), ["Beta", "free*"]);
        let manifest = std::fs::read_to_string(mc.join("modpacks/Beta/modConfigV2.json")).unwrap();
        assert!(manifest.contains("\"modLoader\": \"Forge\""), "{manifest}");

        let archive = root.join("Beta.quadrantExport.zip");
        run(Op::Export {
            name: "Beta".into(),
            to: archive.clone(),
        })
        .unwrap();
        assert!(std::fs::metadata(&archive).unwrap().len() > 0);

        run(Op::Delete("Beta".into())).unwrap();
        assert_eq!(names(), ["free*"]);
        assert!(!mc.join("modpacks/Beta").exists());
    }
}
