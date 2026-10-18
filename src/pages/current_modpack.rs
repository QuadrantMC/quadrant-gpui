//! The Current modpack page: the applied modpack's mods, after
//! `CurrentModpackPage` and `ModpackView` in the desktop app.

use std::{rc::Rc, time::Duration};

use chrono::{Local, TimeZone as _};
use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Task, Window, div, prelude::FluentBuilder as _, rgb,
};
use gpui_component::{
    Sizable as _, h_flex, scroll::ScrollableElement as _, spinner::Spinner, v_flex,
};
use quadrant_core::{
    mc_mod::{GetModArgs, Mod, ModType, UniversalModFile},
    models::{InstalledMod, LocalModpack, ModSource},
};
use quadrant_host::{QuadrantHost, get_mod_url};

use crate::{
    assets::MdIcon,
    backend::Backend,
    events::{ModStage, watch_mod_progress},
    i18n::{describe, t, t_with},
    loadable::Loadable,
    pages::mod_ui::{CardPress, CardStatus, OnCardPress, Primary, mod_card},
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

/// How often the page looks for changes made outside it, as the desktop
/// app's watch on the `mods` folder does.
const POLL: Duration = Duration::from_secs(2);

/// A change to the applied modpack. At most one runs at a time.
#[derive(Clone, Debug)]
pub enum Op {
    Delete {
        modpack: String,
        id: String,
    },
    /// Installs the newer file an update check found.
    Update {
        modpack: String,
        id: String,
        source: ModSource,
        mod_type: ModType,
        file: UniversalModFile,
    },
    /// Records an unknown file as the mod the user picked for it.
    Register {
        modpack: String,
        id: String,
        source: ModSource,
        download_url: String,
    },
}

impl Op {
    /// The update an update check's result offers, if it has a file.
    pub fn update(modpack: &str, item: &Mod) -> Option<Self> {
        Some(Op::Update {
            modpack: modpack.to_string(),
            id: item.id.clone(),
            source: item.source.clone(),
            mod_type: item.mod_type,
            file: item.new_version.clone()?,
        })
    }

    pub async fn perform(self, host: QuadrantHost) -> anyhow::Result<()> {
        match self {
            Op::Delete { modpack, id } => host.delete_mod(modpack, id).await,
            Op::Update {
                modpack,
                id,
                source,
                mod_type,
                file,
            } => {
                host.install_remote_file(file, mod_type, Some(modpack), source, id)
                    .await
            }
            Op::Register {
                modpack,
                id,
                source,
                download_url,
            } => {
                host.register_mod(InstalledMod::minimal(id, source, download_url), modpack)
                    .await
            }
        }
    }

    fn id(&self) -> &str {
        match self {
            Op::Delete { id, .. } | Op::Update { id, .. } | Op::Register { id, .. } => id,
        }
    }

    fn done_text(&self) -> SharedString {
        match self {
            Op::Delete { .. } => "Mod deleted".into(),
            Op::Update { .. } => t("downloadSuccess"),
            Op::Register { .. } => "Mod identified".into(),
        }
    }
}

/// What the page lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Mods,
    Updates,
    Identify,
}

/// An unknown file and the mods it may be.
#[derive(Clone, Debug)]
pub struct Unknown {
    pub file_name: String,
    pub proposals: Vec<Mod>,
}

/// An installed mod as a card, from what its manifest entry records, so the
/// list needs no provider calls.
pub fn installed_card(installed: &InstalledMod, modpack: &str) -> Mod {
    let mod_type = ModType::from(installed.mod_type.clone());
    let mod_type = if mod_type == ModType::Unknown {
        ModType::Mod
    } else {
        mod_type
    };
    let url = if installed.slug.is_empty() || installed.source == ModSource::Online {
        String::new()
    } else {
        get_mod_url(installed.slug.clone(), mod_type, installed.source.clone())
    };
    Mod {
        name: if installed.name.is_empty() {
            installed.id.clone()
        } else {
            installed.name.clone()
        },
        id: installed.id.clone(),
        download_count: installed.download_count,
        version: installed.version.clone(),
        date_modified: String::new(),
        mod_type,
        source: installed.source.clone(),
        slug: installed.slug.clone(),
        thumbnail_urls: installed.thumbnail_urls.clone(),
        url,
        description: installed.description.clone(),
        license: installed.license.clone(),
        mod_icon_url: installed.mod_icon_url.clone(),
        downloadable: false,
        show_previous_version: false,
        new_version: None,
        deleteable: true,
        autoinstallable: false,
        selectable: false,
        modpack: Some(modpack.to_string()),
        select_url: None,
    }
}

/// The applied modpack's mods as cards, most downloaded first.
pub fn installed_cards(modpack: &LocalModpack) -> Vec<Mod> {
    let mut cards: Vec<Mod> = modpack
        .mods
        .iter()
        .map(|installed| installed_card(installed, &modpack.name))
        .collect();
    cards.sort_by_key(|card| std::cmp::Reverse(card.download_count));
    cards
}

/// "Alpha | Fabric | 1.21.1 | 3 Mod(s)", plus the sync date once there is
/// one.
pub fn summary(modpack: &LocalModpack, count: usize) -> String {
    let mut line = format!(
        "{} | {} | {} | {}",
        modpack.name,
        modpack.mod_loader,
        modpack.version,
        t_with("modCount", &[("amount", &count.to_string())])
    );
    if modpack.last_synced > 0
        && let Some(date) = Local.timestamp_millis_opt(modpack.last_synced).single()
    {
        let date = date.format("%a, %D, %H:%M").to_string();
        line.push_str(" | ");
        line.push_str(&t_with("localSyncDate", &[("date", &date)]));
    }
    line
}

async fn check_updates(host: QuadrantHost, modpack: LocalModpack) -> anyhow::Result<Vec<Mod>> {
    let mut updates = Vec::new();
    for card in installed_cards(&modpack) {
        match host
            .check_mod_updates(
                card,
                modpack.version.clone(),
                modpack.mod_loader,
                modpack.name.clone(),
            )
            .await
        {
            Ok(Some(update)) => updates.push(update),
            Ok(None) => {}
            Err(error) => log::warn!("update check failed: {error:#}"),
        }
    }
    updates.sort_by_key(|update| std::cmp::Reverse(update.download_count));
    Ok(updates)
}

async fn identify(host: QuadrantHost, modpack: LocalModpack) -> anyhow::Result<Vec<Unknown>> {
    let mut unknowns: Vec<Unknown> = Vec::new();
    for found in host.identify_modpack(modpack.name.clone()).await? {
        let installed = found.installed_mod;
        let args = GetModArgs {
            id: installed.id.clone(),
            downloadable: false,
            show_previous_version: false,
            deletable: false,
            version_target: String::new(),
            mod_loader: modpack.mod_loader,
            modpack: modpack.name.clone(),
            selectable: true,
            select_url: Some(installed.download_url.clone()),
        };
        let proposal = match installed.source {
            ModSource::Modrinth => host.get_mod_modrinth(args).await,
            #[cfg(feature = "curseforge")]
            ModSource::CurseForge => host.get_mod_curseforge(args).await,
            _ => continue,
        };
        let Ok(proposal) = proposal else {
            continue;
        };
        match unknowns
            .iter_mut()
            .find(|unknown| unknown.file_name == found.file_name)
        {
            Some(unknown) => unknown.proposals.push(proposal),
            None => unknowns.push(Unknown {
                file_name: found.file_name,
                proposals: vec![proposal],
            }),
        }
    }
    Ok(unknowns)
}

pub struct CurrentModpackPage {
    /// `Ready(None)` when no modpack is applied.
    modpack: Loadable<Option<LocalModpack>>,
    mode: Mode,
    updates: Loadable<Vec<Mod>>,
    unknowns: Loadable<Vec<Unknown>>,
    running: Option<Op>,
    progress: Option<i32>,
    /// Updates installed from here, whose cards go away.
    finished: Vec<String>,
    /// The newest listing or check; replacing it drops a stale answer.
    loading: Task<()>,
    checking: Task<()>,
    _poll: Task<()>,
    _progress: Task<()>,
}

impl CurrentModpackPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
            }
        });
        let progress = watch_mod_progress(cx, |this: &mut Self, progress, cx| {
            let ours = this
                .running
                .as_ref()
                .is_some_and(|running| running.id() == progress.mod_id);
            if ours && progress.stage == ModStage::Download {
                this.progress = Some(progress.percent);
                cx.notify();
            }
        });
        let mut page = Self {
            modpack: Loadable::Loading,
            mode: Mode::Mods,
            updates: Loadable::Idle,
            unknowns: Loadable::Idle,
            running: None,
            progress: None,
            finished: Vec::new(),
            loading: Task::ready(()),
            checking: Task::ready(()),
            _poll: poll,
            _progress: progress,
        };
        page.refresh(cx);
        page
    }

    /// Re-reads the applied modpack, redrawing only when it changed.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move {
            let modpacks = host.get_modpacks(false).await?;
            Ok(modpacks.into_iter().find(|modpack| modpack.is_applied))
        });
        self.loading = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let fresh = Loadable::from_result(result);
                if !same_modpack(&this.modpack, &fresh) {
                    this.modpack = fresh;
                    cx.notify();
                }
            });
        });
    }

    fn applied(&self) -> Option<&LocalModpack> {
        match &self.modpack {
            Loadable::Ready(modpack) => modpack.as_ref(),
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => None,
        }
    }

    /// The Update button: checks every mod, or goes back to the list.
    pub fn toggle_updates(&mut self, cx: &mut Context<Self>) {
        if self.mode == Mode::Updates {
            self.mode = Mode::Mods;
            cx.notify();
            return;
        }
        let Some(modpack) = self.applied().cloned() else {
            return;
        };
        self.mode = Mode::Updates;
        self.updates = Loadable::Loading;
        self.finished.clear();
        let task = Backend::global(cx).run(cx, move |host| check_updates(host, modpack));
        self.checking = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if matches!(&result, Ok(updates) if updates.is_empty()) {
                    this.mode = Mode::Mods;
                    snackbar::show(cx, SnackbarKind::Neutral, "Every mod is up to date.");
                }
                this.updates = Loadable::from_result(result);
                cx.notify();
            });
        });
        cx.notify();
    }

    pub fn toggle_identify(&mut self, cx: &mut Context<Self>) {
        if self.mode == Mode::Identify {
            self.mode = Mode::Mods;
            cx.notify();
            return;
        }
        let Some(modpack) = self.applied().cloned() else {
            return;
        };
        self.mode = Mode::Identify;
        self.unknowns = Loadable::Loading;
        let task = Backend::global(cx).run(cx, move |host| identify(host, modpack));
        self.checking = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if matches!(&result, Ok(unknowns) if unknowns.is_empty()) {
                    this.mode = Mode::Mods;
                }
                this.unknowns = Loadable::from_result(result);
                cx.notify();
            });
        });
        cx.notify();
    }

    pub fn run(&mut self, op: Op, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        self.running = Some(op.clone());
        self.progress = None;
        let done_text = op.done_text();
        let task = Backend::global(cx).run(cx, {
            let op = op.clone();
            move |host| op.perform(host)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                this.progress = None;
                match result {
                    Ok(()) => {
                        this.settle(&op);
                        snackbar::show(cx, SnackbarKind::Success, done_text);
                    }
                    Err(error) => snackbar::show(cx, SnackbarKind::Error, describe(error)),
                }
                this.refresh(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Drops what a finished op made stale from the update and identify
    /// lists.
    fn settle(&mut self, op: &Op) {
        match op {
            Op::Update { id, .. } => self.finished.push(id.clone()),
            Op::Register { id, .. } => {
                if let Loadable::Ready(unknowns) = &mut self.unknowns {
                    unknowns.retain(|unknown| {
                        !unknown.proposals.iter().any(|proposal| &proposal.id == id)
                    });
                    if unknowns.is_empty() {
                        self.mode = Mode::Mods;
                    }
                }
            }
            Op::Delete { .. } => {}
        }
    }

    fn press(&mut self, press: CardPress, item: &Mod, cx: &mut Context<Self>) {
        let Some(modpack) = self.applied().map(|modpack| modpack.name.clone()) else {
            return;
        };
        let op = match press {
            CardPress::Delete => Some(Op::Delete {
                modpack,
                id: item.id.clone(),
            }),
            CardPress::Primary(Primary::Update) => Op::update(&modpack, item),
            CardPress::Select => Some(Op::Register {
                modpack,
                id: item.id.clone(),
                source: item.source.clone(),
                download_url: item.select_url.clone().unwrap_or_default(),
            }),
            CardPress::Primary(Primary::Download | Primary::Installed) | CardPress::Open => None,
        };
        if let Some(op) = op {
            self.run(op, cx);
        }
    }

    fn cards(&self, items: &[Mod], offset: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let on_press: OnCardPress<Self> =
            Rc::new(|this, press, item, _, cx| this.press(press, item, cx));
        items
            .iter()
            .filter(|item| !self.finished.contains(&item.id))
            .enumerate()
            .map(|(index, item)| {
                let running_here = self
                    .running
                    .as_ref()
                    .is_some_and(|running| running.id() == item.id);
                let status = CardStatus {
                    progress: running_here.then(|| self.progress.unwrap_or(0)),
                    busy: self.running.is_some(),
                    ..CardStatus::default()
                };
                mod_card(offset + index, item, status, on_press.clone(), cx)
            })
            .collect()
    }

    fn render_header(
        &self,
        modpack: &LocalModpack,
        count: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        h_flex()
            .w_full()
            .mt_2()
            .mb_5()
            .p_4()
            .justify_center()
            .items_center()
            .gap_4()
            .rounded(theme::radius_card())
            .bg(rgb(theme::CONTROL))
            .font_weight(FontWeight::BOLD)
            .child(div().text_center().child(summary(modpack, count)))
            .child(
                theme::pill_button(
                    "updates",
                    Tone::Primary,
                    t("update"),
                    Some(MdIcon::Update),
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_updates(cx))),
            )
            .when(modpack.unknown_mods, |this| {
                this.child(
                    theme::pill_button(
                        "identify",
                        Tone::Neutral,
                        t("identifyUnknownMods"),
                        Some(MdIcon::PermIdentity),
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_identify(cx))),
                )
            })
    }

    fn render_body(&self, modpack: &LocalModpack, cx: &mut Context<Self>) -> AnyElement {
        let grid = |columns: u16, cards: Vec<AnyElement>| {
            div()
                .mr_4()
                .ml_2()
                .mb_12()
                .p_4()
                .rounded(theme::radius_card())
                .bg(rgb(theme::SURFACE))
                .grid()
                .grid_cols(columns)
                .gap_4()
                .children(cards)
                .into_any_element()
        };
        match self.mode {
            Mode::Mods => {
                let cards = installed_cards(modpack);
                if cards.is_empty() {
                    return waiting("-".into_any_element());
                }
                grid(2, self.cards(&cards, 0, cx))
            }
            Mode::Updates => match &self.updates {
                Loadable::Ready(updates) => grid(3, self.cards(updates, 0, cx)),
                Loadable::Failed(error) => waiting(error.clone().into_any_element()),
                Loadable::Idle | Loadable::Loading => waiting(spinner()),
            },
            Mode::Identify => match &self.unknowns {
                Loadable::Ready(unknowns) => {
                    let mut offset = 0;
                    let groups: Vec<AnyElement> = unknowns
                        .iter()
                        .map(|unknown| {
                            let cards = self.cards(&unknown.proposals, offset, cx);
                            offset += unknown.proposals.len();
                            v_flex()
                                .my_2()
                                .p_4()
                                .rounded(theme::radius_card())
                                .bg(rgb(theme::SURFACE))
                                .child(
                                    div()
                                        .font_weight(FontWeight::BOLD)
                                        .child(format!("{} ->", unknown.file_name)),
                                )
                                .child(h_flex().mt_8().gap_4().children(cards))
                                .into_any_element()
                        })
                        .collect();
                    v_flex().children(groups).into_any_element()
                }
                Loadable::Failed(error) => waiting(error.clone().into_any_element()),
                Loadable::Idle | Loadable::Loading => waiting(spinner()),
            },
        }
    }
}

/// Whether a re-read found the same modpack, so the page need not redraw.
fn same_modpack(
    old: &Loadable<Option<LocalModpack>>,
    new: &Loadable<Option<LocalModpack>>,
) -> bool {
    match (old, new) {
        (Loadable::Ready(old), Loadable::Ready(new)) => {
            serde_json::to_value(old).ok() == serde_json::to_value(new).ok()
        }
        (Loadable::Failed(old), Loadable::Failed(new)) => old == new,
        _ => false,
    }
}

fn spinner() -> AnyElement {
    Spinner::new().large().into_any_element()
}

/// The desktop app's waiting panel: a slate-800 strip with one thing in it.
fn waiting(content: AnyElement) -> AnyElement {
    h_flex()
        .w_full()
        .p_4()
        .justify_center()
        .rounded(theme::radius_card())
        .bg(rgb(theme::SURFACE))
        .child(content)
        .into_any_element()
}

impl Render for CurrentModpackPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.modpack {
            Loadable::Idle | Loadable::Loading => waiting(spinner()),
            Loadable::Failed(error) => waiting(error.clone().into_any_element()),
            Loadable::Ready(None) => div().child("-").into_any_element(),
            Loadable::Ready(Some(modpack)) => {
                let count = match (self.mode, &self.updates) {
                    (Mode::Updates, Loadable::Ready(updates)) => {
                        updates.iter().filter(|update| update.downloadable).count()
                    }
                    _ => modpack.mods.len(),
                };
                v_flex()
                    .child(self.render_header(modpack, count, cx))
                    .child(self.render_body(modpack, cx))
                    .into_any_element()
            }
        };
        v_flex()
            .size_full()
            .overflow_y_scrollbar()
            .child(div().p_4().child(content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_core::models::ModLoader;
    use quadrant_host::QuadrantHostOptions;

    fn installed(id: &str, name: &str, downloads: i64) -> InstalledMod {
        let mut entry = InstalledMod::minimal(
            id.into(),
            ModSource::Modrinth,
            format!("https://cdn.modrinth.com/data/{id}/{name}.jar"),
        );
        entry.name = name.into();
        entry.slug = name.to_lowercase();
        entry.mod_type = "Mod".into();
        entry.download_count = downloads;
        entry
    }

    #[test]
    fn cards_come_from_the_manifest_most_downloaded_first() {
        let mut pack =
            crate::pages::search::logic::tests::modpack("Alpha", "1.21.1", ModLoader::Fabric);
        pack.mods = vec![installed("a", "Lithium", 10), installed("b", "Sodium", 99)];
        let cards = installed_cards(&pack);
        assert_eq!(cards[0].name, "Sodium");
        assert_eq!(cards[0].url, "https://modrinth.com/mod/sodium");
        assert!(cards[0].deleteable && !cards[0].downloadable);
        assert_eq!(cards[0].modpack.as_deref(), Some("Alpha"));
        assert_eq!(summary(&pack, 2), "Alpha | Fabric | 1.21.1 | 2 Mod(s)");
    }

    #[test]
    fn an_update_needs_a_file() {
        let mut card = installed_card(&installed("a", "Iris", 0), "Alpha");
        assert!(Op::update("Alpha", &card).is_none());
        card.new_version = Some(UniversalModFile {
            id: None,
            file_name: "iris.jar".into(),
            download_url: "https://cdn.modrinth.com/iris.jar".into(),
            sha1: String::new(),
            size: 0,
        });
        assert!(matches!(
            Op::update("Alpha", &card),
            Some(Op::Update { .. })
        ));
    }

    /// Deletes and registers mods in a throwaway Minecraft folder through
    /// the same host calls the page makes.
    #[test]
    fn ops_change_the_applied_modpack() {
        let temp = tempfile::tempdir().unwrap();
        let mc = temp.path().join("mc");
        let mut options = QuadrantHostOptions::new(temp.path().join("data"), "", "", "");
        options.mc_folder = Some(mc.clone());
        let host = QuadrantHost::new(options).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(host.create_modpack("Alpha".into(), "1.21.1".into(), ModLoader::Fabric))
            .unwrap();
        let folder = mc.join("modpacks/Alpha");
        std::fs::write(folder.join("unknown.jar"), b"jar").unwrap();
        let ids = || -> Vec<String> {
            let modpacks = runtime.block_on(host.get_modpacks(false)).unwrap();
            let alpha = modpacks.iter().find(|m| m.name == "Alpha").unwrap();
            alpha.mods.iter().map(|m| m.id.clone()).collect()
        };

        runtime
            .block_on(
                Op::Register {
                    modpack: "Alpha".into(),
                    id: "fixture-unknown".into(),
                    source: ModSource::Online,
                    download_url: "https://example.com/files/unknown.jar".into(),
                }
                .perform(host.clone()),
            )
            .unwrap();
        assert_eq!(ids(), ["fixture-unknown"]);

        runtime
            .block_on(
                Op::Delete {
                    modpack: "Alpha".into(),
                    id: "fixture-unknown".into(),
                }
                .perform(host.clone()),
            )
            .unwrap();
        assert!(ids().is_empty());
        assert!(!folder.join("unknown.jar").exists());
    }
}
