//! The Apply page: local modpacks, after `ApplyPage` in the desktop app.

use std::path::PathBuf;

use quadrant_core::{
    account::{id::AccountInfo, quadrant_sync::SyncedModpack},
    models::InstalledModpack,
};
use quadrant_host::ErrorCode;

use chrono::{Local, TimeZone as _};
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, ClickEvent, ClipboardItem,
    Context, Entity, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
    rgb,
};
use gpui_component::{
    IndexPath, WindowExt as _,
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
    events,
    i18n::{describe, t, t_with},
    loadable::Loadable,
    mod_catalog::ModCatalog,
    motion,
    pages::modpack_view::{ModpackView, ModpackViewEvent, catalog_keys},
    session::Session,
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

mod cloud;
mod details;
mod prism;

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

/// The Quadrant Sync record a download is paired with, so the installed pack
/// shows as synced.
#[derive(Clone, Debug)]
pub struct SyncTarget {
    /// The cloud copy's sync time, in the unit Quadrant Sync gave it.
    pub synced_at: i64,
    pub modpack_id: String,
}

impl SyncTarget {
    fn of(synced: &SyncedModpack) -> Self {
        Self {
            synced_at: synced.last_synced,
            modpack_id: synced.modpack_id.clone(),
        }
    }
}

/// A change to the modpacks folder or to Quadrant Sync. Changes run side by
/// side unless they share a [`Lane`].
#[derive(Clone, Debug)]
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
    /// Shares an installed modpack on Quadrant Share.
    Share(String),
    /// Shares a Quadrant Sync pack that isn't installed.
    ShareRaw(Box<InstalledModpack>),
    /// Pushes a modpack to Quadrant Sync; `overwrite` replaces a newer cloud copy.
    Sync {
        modpack: Box<LocalModpack>,
        overwrite: bool,
    },
    /// Downloads a pack from Quadrant Sync or Quadrant Share.
    Install {
        modpack: Box<InstalledModpack>,
        sync: Option<SyncTarget>,
    },
    Kick {
        modpack_id: String,
        username: String,
    },
    Invite {
        modpack_id: String,
        username: String,
        admin: bool,
    },
    DeleteSynced(String),
    /// Looks a Quadrant Share code up.
    ResolveShare(String),
}

/// What a finished op hands back to the page.
#[derive(Debug)]
pub enum Outcome {
    Done,
    /// A Quadrant Share link for the clipboard.
    Copy(String),
    /// The modpack a share code stands for.
    Shared(Box<InstalledModpack>),
}

impl Op {
    pub async fn perform(self, host: QuadrantHost) -> anyhow::Result<Outcome> {
        match self {
            Op::Apply(name) => host.frontend_apply_modpack(name)?,
            Op::Delete(name) => host.delete_modpack(name).await?,
            Op::Create(new) => {
                host.create_modpack(new.name, new.version, new.loader)
                    .await?
            }
            Op::Clear(version) => {
                // A missing scratch pack is the normal first-run case.
                let _ = host.delete_modpack(FREE_MODPACK.into()).await;
                host.create_modpack(FREE_MODPACK.into(), version, ModLoader::Unknown)
                    .await?;
                host.frontend_apply_modpack(FREE_MODPACK.into())?
            }
            Op::Update { from, to } => {
                host.update_modpack(from, Some(to.name), Some(to.version), Some(to.loader))
                    .await?
            }
            Op::Export { name, to } => host.export_modpack_to(name, to).await?,
            Op::Share(name) => {
                let shared = host.share_modpack(name).await?;
                return Ok(Outcome::Copy(format!(
                    "{}{}",
                    cloud::SHARE_URL,
                    shared.code
                )));
            }
            Op::ShareRaw(modpack) => {
                let shared = host.share_modpack_raw(*modpack).await?;
                return Ok(Outcome::Copy(format!(
                    "{}{}",
                    cloud::SHARE_URL,
                    shared.code
                )));
            }
            Op::Sync { modpack, overwrite } => host.sync_modpack(*modpack, overwrite).await?,
            Op::Install { modpack, sync } => {
                let name = modpack.name.clone();
                host.install_modpack(*modpack).await?;
                if let Some(sync) = sync {
                    host.set_modpack_sync_date(
                        sync.synced_at.max(0) as u64,
                        name,
                        Some(sync.modpack_id),
                    )?;
                }
            }
            Op::Kick {
                modpack_id,
                username,
            } => host.kick_member(modpack_id, username).await?,
            Op::Invite {
                modpack_id,
                username,
                admin,
            } => host.invite_member(modpack_id, username, admin).await?,
            Op::DeleteSynced(modpack_id) => host.delete_synced_modpack(modpack_id).await?,
            Op::ResolveShare(code) => {
                let modpack = host.get_quadrant_share_modpack(code).await?;
                return Ok(Outcome::Shared(Box::new(modpack)));
            }
        }
        Ok(Outcome::Done)
    }

    /// What the snackbar says once it worked, if anything. A share announces
    /// itself through the host's `quadrantShareSubmission` event.
    fn done_text(&self) -> Option<SharedString> {
        Some(match self {
            Op::Apply(_) => t("setModpackSuccess"),
            Op::Delete(name) => format!("Deleted \"{name}\"").into(),
            Op::Create(new) => format!("Created \"{}\"", new.name).into(),
            Op::Clear(_) => "Cleared the applied modpack".into(),
            // The desktop app's own summary of the saved details.
            Op::Update { to, .. } => format!("{} | {} | {}", to.name, to.version, to.loader).into(),
            Op::Export { name, .. } => format!("Exported \"{name}\"").into(),
            Op::Sync { .. } => t("modpackUpdated"),
            Op::Install { .. } => t("downloadSuccess"),
            Op::Invite { .. } => t("invite"),
            Op::DeleteSynced(_) => t("delete"),
            Op::Share(_) | Op::ShareRaw(_) | Op::Kick { .. } | Op::ResolveShare(_) => return None,
        })
    }

    /// What this op works on. Of two ops that share a lane, the second
    /// waits for the first.
    fn lanes(&self) -> Vec<Lane<'_>> {
        match self {
            Op::Apply(name) => vec![Lane::Modpack(name), Lane::Applied],
            Op::Clear(_) => vec![Lane::Modpack(FREE_MODPACK), Lane::Applied],
            Op::Delete(name) | Op::Export { name, .. } | Op::Share(name) => {
                vec![Lane::Modpack(name)]
            }
            Op::Create(new) => vec![Lane::Modpack(&new.name)],
            Op::Update { from, to } => vec![Lane::Modpack(from), Lane::Modpack(&to.name)],
            Op::Sync { modpack, .. } => vec![Lane::Modpack(&modpack.name)],
            Op::ShareRaw(modpack) => vec![Lane::Modpack(&modpack.name)],
            // One download at a time: its progress events don't say whose.
            Op::Install { modpack, .. } => vec![Lane::Modpack(&modpack.name), Lane::Download],
            Op::Kick { modpack_id, .. }
            | Op::Invite { modpack_id, .. }
            | Op::DeleteSynced(modpack_id) => vec![Lane::Cloud(modpack_id)],
            Op::ResolveShare(_) => vec![Lane::ShareCode],
        }
    }

    fn conflicts(&self, other: &Op) -> bool {
        let lanes = other.lanes();
        self.lanes().iter().any(|lane| lanes.contains(lane))
    }

    /// The modpack whose card shows this op as running.
    fn target(&self) -> Option<&str> {
        match self {
            Op::Apply(name)
            | Op::Delete(name)
            | Op::Update { from: name, .. }
            | Op::Export { name, .. }
            | Op::Share(name) => Some(name),
            Op::Sync { modpack, .. } => Some(&modpack.name),
            Op::Create(_)
            | Op::Clear(_)
            | Op::ShareRaw(_)
            | Op::Install { .. }
            | Op::Kick { .. }
            | Op::Invite { .. }
            | Op::DeleteSynced(_)
            | Op::ResolveShare(_) => None,
        }
    }
}

/// What an [`Op`] works on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane<'a> {
    /// A local modpack, by name.
    Modpack(&'a str),
    /// Which modpack is applied.
    Applied,
    /// A Quadrant Sync pack, by id.
    Cloud(&'a str),
    Download,
    ShareCode,
}

/// `account`, if it may use Quadrant Sync: signed in, with a sync limit.
fn sync_account(account: Option<&AccountInfo>) -> Option<AccountInfo> {
    account
        .filter(|account| account.quadrant_sync_limit != 0)
        .cloned()
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
    /// A local pack's Details.
    Details {
        view: Entity<ModpackView>,
        _events: Subscription,
    },
    /// A Quadrant Share or Quadrant Sync pack, with its download.
    Shared {
        modpack: Box<InstalledModpack>,
        sync: Option<SyncTarget>,
    },
}

pub struct ModpacksPage {
    view: View,
    modpacks: Loadable<Vec<LocalModpack>>,
    /// Minecraft release versions, newest first.
    versions: Loadable<Vec<SharedString>>,
    /// Changes in flight, none sharing a lane.
    running: Vec<Op>,
    search: Entity<InputState>,
    /// The newest listing request; replacing it drops a stale one's answer.
    listing: Task<()>,
    session: Entity<Session>,
    /// The signed-in account's Quadrant Sync packs; `Idle` while signed out.
    synced: Loadable<Vec<SyncedModpack>>,
    synced_listing: Task<()>,
    /// Cards whose Quadrant Sync panel is unfolded, by cloud modpack id.
    open_panels: Vec<String>,
    /// How far the running download is, from the host's progress events.
    install_progress: Option<f64>,
    /// A sync the cloud refused for being older, waiting for the dialog.
    sync_conflict: Option<LocalModpack>,
    prism: Entity<prism::PrismMenus>,
    _search_changed: Subscription,
    /// The view the last change of view left, fading out under the new one
    /// until the crossfade ends, and the crossfade's number.
    leaving: Option<(View, usize)>,
    leaving_done: Task<()>,
    _session_changed: Subscription,
    _prism_changed: Subscription,
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
        let session = Session::global(cx);
        let session_changed = cx.observe(&session, |this, _, cx| this.load_synced(cx));
        // A shared pack's cards fill in as their providers answer.
        let catalog = ModCatalog::global(cx);
        cx.observe(&catalog, |this, _, cx| {
            if matches!(this.view, View::Shared { .. }) {
                cx.notify();
            }
        })
        .detach();
        events::listen(&["refreshSyncedModpacks"], cx, |this, _, cx| {
            this.load_synced(cx)
        });
        events::listen(&["modpackDownloadProgress"], cx, |this, event, cx| {
            if this.running_any(|op| matches!(op, Op::Install { .. })) {
                // Only the op's own answer confirms success.
                this.install_progress = event.payload.as_f64().map(|done| done.min(0.99));
                cx.notify();
            }
        });
        let prism = cx.new(prism::PrismMenus::new);
        let prism_changed =
            cx.subscribe(&prism, |this, _, _: &prism::Changed, cx| this.refresh(cx));
        let mut page = Self {
            view: View::List,
            modpacks: Loadable::Idle,
            versions: Loadable::Idle,
            running: Vec::new(),
            search,
            listing: Task::ready(()),
            session,
            synced: Loadable::Idle,
            synced_listing: Task::ready(()),
            open_panels: Vec::new(),
            install_progress: None,
            sync_conflict: None,
            leaving: None,
            leaving_done: Task::ready(()),
            prism,
            _search_changed: search_changed,
            _session_changed: session_changed,
            _prism_changed: prism_changed,
        };
        page.refresh(cx);
        page.load_versions(cx);
        page.load_synced(cx);
        page
    }

    /// The signed-in account, when it has Quadrant Sync. Without one, every
    /// Sync control is hidden rather than disabled.
    fn sync_account(&self, cx: &App) -> Option<AccountInfo> {
        sync_account(self.session.read(cx).signed_in())
    }

    /// Reads the account's Quadrant Sync packs, or forgets them once signed out.
    fn load_synced(&mut self, cx: &mut Context<Self>) {
        if self.sync_account(cx).is_none() {
            self.synced = Loadable::Idle;
            self.synced_listing = Task::ready(());
            self.open_panels.clear();
            cx.notify();
            return;
        }
        if !matches!(self.synced, Loadable::Ready(_)) {
            self.synced = Loadable::Loading;
        }
        let task = Backend::global(cx).run(cx, |host| async move {
            host.get_synced_modpacks(true, None).await
        });
        self.synced_listing = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.synced = Loadable::from_result(result);
                cx.notify();
            });
        });
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        // A loaded list stays on screen while it reloads, so acting on a card
        // doesn't blank the page.
        if !matches!(self.modpacks, Loadable::Ready(_)) {
            self.modpacks = Loadable::Loading;
        }
        self.prism.update(cx, |prism, cx| prism.refresh(cx));
        let task = Backend::global(cx).run(cx, |host| async move { host.get_modpacks(true).await });
        self.listing = cx.spawn(async move |this, cx| {
            let result = task.await.map(sort_modpacks);
            let _ = this.update(cx, |this, cx| {
                this.modpacks = Loadable::from_result(result);
                this.refresh_details(cx);
                cx.notify();
            });
        });
        cx.notify();
    }

    /// Hands an open Details view its pack's re-read manifest.
    fn refresh_details(&mut self, cx: &mut Context<Self>) {
        let (View::Details { view, .. }, Loadable::Ready(modpacks)) = (&self.view, &self.modpacks)
        else {
            return;
        };
        let name = &view.read(cx).modpack().name;
        if let Some(modpack) = modpacks.iter().find(|modpack| &modpack.name == name) {
            let modpack = modpack.clone();
            view.update(cx, |view, cx| view.set_modpack(modpack, cx));
        }
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

    fn running_any(&self, matches: impl Fn(&Op) -> bool) -> bool {
        self.running.iter().any(matches)
    }

    /// Whether an op of the kind `matches` picks runs on modpack `name`.
    fn running_on(&self, name: &str, matches: impl Fn(&Op) -> bool) -> bool {
        self.running_any(|op| op.target() == Some(name) && matches(op))
    }

    fn run(&mut self, op: Op, cx: &mut Context<Self>) {
        if self.running_any(|running| running.conflicts(&op)) {
            return;
        }
        if matches!(op, Op::Install { .. }) {
            self.install_progress = None;
        }
        self.running.push(op.clone());
        let task = Backend::global(cx).run(cx, {
            let op = op.clone();
            move |host| op.perform(host)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running.retain(|running| !running.conflicts(&op));
                if matches!(op, Op::Install { .. }) {
                    this.install_progress = None;
                }
                match result {
                    Ok(outcome) => {
                        match outcome {
                            Outcome::Done => {}
                            Outcome::Copy(link) => {
                                cx.write_to_clipboard(ClipboardItem::new_string(link))
                            }
                            Outcome::Shared(modpack) => this.show(
                                View::Shared {
                                    modpack,
                                    sync: None,
                                },
                                cx,
                            ),
                        }
                        if let Some(text) = op.done_text() {
                            snackbar::show(cx, SnackbarKind::Success, text);
                        }
                    }
                    Err(error) => match op {
                        Op::Sync {
                            modpack,
                            overwrite: false,
                        } if is_error(&error, ErrorCode::CloudSyncNewer) => {
                            this.sync_conflict = Some(*modpack);
                        }
                        Op::ResolveShare(_) => {
                            log::warn!("Quadrant Share lookup failed: {}", describe(error));
                            snackbar::show(cx, SnackbarKind::Error, t("unsupportedDownload"));
                        }
                        _ => snackbar::show(cx, SnackbarKind::Error, describe(error)),
                    },
                }
                this.refresh(cx);
                this.load_synced(cx);
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
            Some(modpack) => self.open_details(modpack, cx),
            None => self.show(View::List, cx),
        }
    }

    /// Shows `synced` as the account's Quadrant Sync packs with the first
    /// one's panel open, `search` in the search box, and `conflict`'s sync
    /// refused, for screenshots. Nothing is asked of Quadrant Sync.
    #[cfg(feature = "screenshot")]
    pub fn preview_cloud(
        &mut self,
        synced: Vec<SyncedModpack>,
        search: &str,
        conflict: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.synced_listing = Task::ready(());
        self.open_panels = synced
            .first()
            .map(|modpack| modpack.modpack_id.clone())
            .into_iter()
            .collect();
        self.synced = Loadable::Ready(synced);
        self.search.update(cx, |input, cx| {
            input.set_value(search.to_string(), window, cx)
        });
        if let (Loadable::Ready(modpacks), Some(name)) = (&self.modpacks, conflict) {
            self.sync_conflict = modpacks
                .iter()
                .find(|modpack| modpack.name == name)
                .cloned();
        }
        cx.notify();
    }

    fn show(&mut self, view: View, cx: &mut Context<Self>) {
        if let View::Shared { modpack, .. } = &view {
            let keys = catalog_keys(&details::cards(modpack));
            ModCatalog::global(cx).update(cx, |catalog, cx| catalog.want(keys, cx));
        }
        let left = std::mem::replace(&mut self.view, view);
        if std::mem::discriminant(&left) != std::mem::discriminant(&self.view) {
            let crossfade = self.leaving.as_ref().map_or(0, |(_, id)| id + 1);
            self.leaving = Some((left, crossfade));
            self.leaving_done = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(motion::CROSSFADE).await;
                let _ = this.update(cx, |this, cx| {
                    this.leaving = None;
                    cx.notify();
                });
            });
        }
        cx.notify();
    }

    /// What the title bar names while a pack's mods are open, as the desktop
    /// app titles that content with the pack's name.
    pub fn title(&self, cx: &App) -> Option<SharedString> {
        match &self.view {
            View::List => None,
            View::Details { view, .. } => Some(view.read(cx).modpack().name.clone().into()),
            View::Shared { modpack, .. } => Some(modpack.name.clone().into()),
        }
    }

    /// Shows `name` as exporting, or nothing running with `None`, for
    /// screenshots. Nothing is exported.
    #[cfg(feature = "screenshot")]
    pub fn preview_exporting(&mut self, name: Option<&str>, cx: &mut Context<Self>) {
        self.running = name
            .map(|name| Op::Export {
                name: name.to_string(),
                to: PathBuf::new(),
            })
            .into_iter()
            .collect();
        cx.notify();
    }

    /// Scrolls an open Details view down by `distance`, for screenshots.
    #[cfg(feature = "screenshot")]
    pub fn scroll_details(&mut self, distance: gpui::Pixels, cx: &mut Context<Self>) {
        if let View::Details { view, .. } = &self.view {
            view.update(cx, |view, cx| view.scroll_by(distance, cx));
        }
    }

    /// Goes back from a pack's mods to the list; false when already there.
    pub fn back(&mut self, cx: &mut Context<Self>) -> bool {
        if matches!(self.view, View::List) {
            return false;
        }
        self.show(View::List, cx);
        true
    }

    fn open_details(&mut self, modpack: LocalModpack, cx: &mut Context<Self>) {
        let view = cx.new(|cx| ModpackView::new(modpack, true, cx));
        let events = cx.subscribe(&view, |this, _, event, cx| match event {
            ModpackViewEvent::Back => this.show(View::List, cx),
            ModpackViewEvent::Changed => this.refresh(cx),
        });
        self.show(
            View::Details {
                view,
                _events: events,
            },
            cx,
        );
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
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_create_dialog(window, cx)),
                    ),
            )
            .child(
                theme::toolbar_button("clear", Tone::Control, MdIcon::Clear, cx)
                    .tooltip(t("clear"))
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
        synced: Option<&SyncedModpack>,
        account: Option<&AccountInfo>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let name = SharedString::from(modpack.name.clone());
        let share = {
            let name = name.clone();
            theme::pill_button(
                ("share", index),
                Tone::Primary,
                t("share"),
                Some(MdIcon::Share),
                cx,
            )
            .loading(self.running_on(&modpack.name, |op| matches!(op, Op::Share(_))))
            .on_click(cx.listener(move |this, _, _, cx| this.run(Op::Share(name.to_string()), cx)))
        };
        let sync = account.is_some().then(|| {
            let modpack = modpack.clone();
            theme::pill_button(
                ("sync", index),
                Tone::Positive,
                t("sync"),
                Some(MdIcon::Sync),
                cx,
            )
            .loading(self.running_on(&modpack.name, |op| matches!(op, Op::Sync { .. })))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run(
                    Op::Sync {
                        modpack: Box::new(modpack.clone()),
                        overwrite: false,
                    },
                    cx,
                )
            }))
        });
        let mut line = metadata_line(modpack);
        if let Some(synced) = synced {
            line.push_str(" | ");
            line.push_str(&t_with(
                "cloudSyncDate",
                &[("date", &cloud::format_sync_date(synced.last_synced))],
            ));
        }
        let panel = synced.zip(account).map(|(synced, account)| {
            self.render_sync_panel(synced, Some(&modpack.name), account, cx)
                .into_any_element()
        });
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
            .loading(self.running_on(&modpack.name, |op| matches!(op, Op::Apply(_))))
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
                .loading(self.running_on(&modpack.name, |op| matches!(op, Op::Delete(_))))
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
            .loading(self.running_on(&modpack.name, |op| matches!(op, Op::Update { .. })))
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
            .loading(self.running_on(&modpack.name, |op| matches!(op, Op::Export { .. })))
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
            .on_click(cx.listener(move |this, _, _, cx| this.open_details(modpack.clone(), cx)))
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
                    .child(theme::badge(t("badgeLocal"), MdIcon::Computer))
                    .when(synced.is_some(), |this| {
                        this.child(theme::synced_badge(t("badgeSynced")))
                    }),
            )
            .child(div().text_color(rgb(theme::TEXT_MUTED)).child(line))
            .children(self.prism.read(cx).applied_line(&modpack.name))
            .child(
                h_flex()
                    .my_2()
                    .flex_wrap()
                    .gap_4()
                    .child(apply)
                    .children(prism::PrismMenus::menu(&self.prism, &modpack.name, cx))
                    .child(share)
                    .children(sync)
                    .child(update)
                    .child(export)
                    .child(delete)
                    .child(details),
            )
            .children(panel)
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
                let account = self.sync_account(cx);
                let synced = match &self.synced {
                    Loadable::Ready(synced) => synced.as_slice(),
                    Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => &[],
                };
                let shown: Vec<_> = cloud::merge_modpacks(modpacks, synced, &query)
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, row)| match (row.local, row.synced, &account) {
                        (Some(local), synced, account) => Some(
                            self.render_card(index, local, synced, account.as_ref(), cx)
                                .into_any_element(),
                        ),
                        (None, Some(synced), Some(account)) => Some(
                            self.render_cloud_card(index, synced, account, cx)
                                .into_any_element(),
                        ),
                        (None, _, _) => None,
                    })
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

/// Whether `error` is the classified failure `code`.
fn is_error(error: &anyhow::Error, code: ErrorCode) -> bool {
    error
        .chain()
        .any(|cause| cause.downcast_ref::<ErrorCode>() == Some(&code))
}

impl Render for ModpacksPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(modpack) = self.sync_conflict.take() {
            let page = cx.entity();
            window.defer(cx, move |window, cx| {
                page.update(cx, |page, cx| page.confirm_sync_anyway(modpack, window, cx))
            });
        }
        let current = self.render_view(&self.view, window, cx);
        let Some((leaving, crossfade)) = &self.leaving else {
            return current;
        };
        // The browser's view transition: the old view fades out as the new
        // one fades in over it.
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(self.render_view(leaving, window, cx)),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(rgb(theme::PAGE))
                    .occlude()
                    .child(current)
                    .with_animation(
                        ("crossfade", *crossfade),
                        Animation::new(motion::CROSSFADE),
                        |this, progress| this.opacity(motion::CROSSFADE_EASE.ease(progress)),
                    ),
            )
            .into_any_element()
    }
}

impl ModpacksPage {
    fn render_view(&self, view: &View, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let page = v_flex().size_full().overflow_y_scrollbar();
        match view {
            View::List => {
                let banner = cloud::parse_share_code(&self.search.read(cx).value())
                    .map(|code| self.render_share_banner(code, cx).into_any_element());
                page.child(self.render_toolbar(cx))
                    .children(banner)
                    .child(self.render_list(cx))
                    .into_any_element()
            }
            View::Details { view, .. } => view.clone().into_any_element(),
            View::Shared { modpack, sync } => {
                let header = format!(
                    "{} | {} | {} | {}",
                    modpack.name,
                    modpack.mod_loader,
                    modpack.version,
                    t_with("modCount", &[("amount", &modpack.mods.len().to_string())])
                );
                let download = self
                    .download_button(
                        "shared-download",
                        t("download"),
                        (**modpack).clone(),
                        sync.clone(),
                        cx,
                    )
                    .into_any_element();
                let cards = details::cards(modpack);
                page.child(details::render(header.into(), download, &cards, window, cx))
                    .into_any_element()
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
    fn ops_wait_only_for_ops_on_the_same_thing() {
        let export = |name: &str| Op::Export {
            name: name.into(),
            to: PathBuf::new(),
        };
        assert!(!export("Alpha").conflicts(&Op::Delete("Beta".into())));
        assert!(export("Alpha").conflicts(&Op::Delete("Alpha".into())));
        // Applying swaps the mods folder, whichever pack it applies.
        assert!(Op::Apply("Alpha".into()).conflicts(&Op::Apply("Beta".into())));
        assert!(Op::Clear("1.21.1".into()).conflicts(&Op::Apply("Beta".into())));
        assert!(!Op::Apply("Alpha".into()).conflicts(&export("Beta")));
    }

    #[test]
    fn sync_needs_an_account_with_a_sync_limit() {
        let account = |limit| AccountInfo {
            id: "id".into(),
            name: "Alex".into(),
            email: "alex@example.com".into(),
            quadrant_sync_limit: limit,
            quadrant_share_limit: 50,
            login: "alex".into(),
            notifications: Vec::new(),
        };
        assert!(sync_account(None).is_none());
        assert!(sync_account(Some(&account(0))).is_none());
        assert!(sync_account(Some(&account(10))).is_some());
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
