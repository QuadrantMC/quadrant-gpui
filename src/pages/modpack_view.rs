//! One local modpack's mods, after the desktop app's `ModpackView`: the
//! Current modpack page shows the applied pack in it, and a pack's Details
//! on the Apply page opens it. A header with the pack's summary, the update
//! check and the unknown file identification, then a card per mod. Each
//! card shows what the manifest records until Modrinth or CurseForge answers
//! for it through the [`ModCatalog`].

use std::{collections::HashMap, rc::Rc};

use chrono::{Local, TimeZone as _};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FontWeight, IntoElement, ParentElement as _,
    Pixels, Render, SharedString, Styled as _, Subscription, Task, Window, div, list,
    prelude::FluentBuilder as _, px, rems, rgb,
};
use gpui_component::{h_flex, scroll::ScrollableElement as _, v_flex};
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
    mod_catalog::{ModCatalog, ModKey},
    motion::{self, CardEntrance},
    pages::{
        mod_grid::{self, CardSlot, GridList, Item, RowHeights, Rows, Shape},
        mod_ui::{CardPress, CardStatus, OnCardPress, Primary, card_key, mod_card},
    },
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

/// About one row of cards, for the scrollbar until rows are measured.
const ROW_HINT: Pixels = px(190.);

/// A change to one mod of the modpack. Changes to different mods run side by
/// side; a mod with a change running takes no other until it finishes.
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

/// What the view lists.
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

/// An installed mod as a card, from what its manifest entry records: what
/// the card shows until its provider answers.
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

/// The modpack's mods as cards, most downloaded first.
pub fn installed_cards(modpack: &LocalModpack) -> Vec<Mod> {
    let mut cards: Vec<Mod> = modpack
        .mods
        .iter()
        .map(|installed| installed_card(installed, &modpack.name))
        .collect();
    cards.sort_by_key(|card| std::cmp::Reverse(card.download_count));
    cards
}

/// `card` with its provider's record, when there is one: the provider's
/// title, icon, downloads and description, and the card's own actions.
pub fn with_record(card: &Mod, record: Option<&Mod>) -> Mod {
    match record {
        Some(record) => Mod {
            id: card.id.clone(),
            downloadable: card.downloadable,
            show_previous_version: card.show_previous_version,
            new_version: card.new_version.clone(),
            deleteable: card.deleteable,
            autoinstallable: card.autoinstallable,
            selectable: card.selectable,
            modpack: card.modpack.clone(),
            select_url: card.select_url.clone(),
            ..record.clone()
        },
        None => card.clone(),
    }
}

/// The keys the catalog knows `cards` by.
pub fn catalog_keys(cards: &[Mod]) -> Vec<ModKey> {
    cards
        .iter()
        .filter_map(|card| ModKey::new(&card.source, &card.id))
        .collect()
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

async fn check_updates(
    host: QuadrantHost,
    modpack: LocalModpack,
    cards: Vec<Mod>,
) -> anyhow::Result<Vec<Mod>> {
    let mut updates = Vec::new();
    for card in cards {
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

/// What a [`ModpackView`] tells the page it sits in.
pub enum ModpackViewEvent {
    /// The back button was pressed.
    Back,
    /// A change to the modpack finished; its manifest should be re-read.
    Changed,
}

pub struct ModpackView {
    modpack: LocalModpack,
    /// Shows a way back, for a Details view.
    back: bool,
    /// The manifest's mods as cards, most downloaded first.
    cards: Vec<Mod>,
    mode: Mode,
    updates: Loadable<Vec<Mod>>,
    unknowns: Loadable<Vec<Unknown>>,
    running: Vec<Op>,
    /// Download progress of running updates, by mod id.
    progress: HashMap<String, i32>,
    /// Updates installed from here, whose cards go away.
    finished: Vec<String>,
    /// The newest update check or identification; replacing it drops a
    /// stale answer.
    checking: Task<()>,
    list: GridList,
    /// The grid's row heights, so an unchanged row lays out once.
    row_heights: RowHeights,
    /// The cards the grid shows this frame, as indices into the mode's
    /// cards.
    shown: Vec<usize>,
    /// The cards' `Mod.tsx` entrance, and where it puts them this frame.
    entrance: CardEntrance,
    card_frame: Option<(Pixels, f32)>,
    catalog: Entity<ModCatalog>,
    _catalog_changed: Subscription,
    _progress: Task<()>,
}

impl EventEmitter<ModpackViewEvent> for ModpackView {}

impl ModpackView {
    pub fn new(modpack: LocalModpack, back: bool, cx: &mut Context<Self>) -> Self {
        let catalog = ModCatalog::global(cx);
        let catalog_changed = cx.observe(&catalog, |_, _, cx| cx.notify());
        let progress = watch_mod_progress(cx, |this: &mut Self, progress, cx| {
            if progress.stage == ModStage::Download
                && this.running.iter().any(|op| op.id() == progress.mod_id)
            {
                this.progress.insert(progress.mod_id, progress.percent);
                cx.notify();
            }
        });
        let cards = installed_cards(&modpack);
        let keys = catalog_keys(&cards);
        catalog.update(cx, |catalog, cx| catalog.want(keys, cx));
        Self {
            modpack,
            back,
            cards,
            mode: Mode::Mods,
            updates: Loadable::Idle,
            unknowns: Loadable::Idle,
            running: Vec::new(),
            progress: HashMap::new(),
            finished: Vec::new(),
            checking: Task::ready(()),
            list: GridList::new(ROW_HINT),
            row_heights: RowHeights::default(),
            shown: Vec::new(),
            entrance: CardEntrance::Pending,
            card_frame: None,
            catalog,
            _catalog_changed: catalog_changed,
            _progress: progress,
        }
    }

    pub fn modpack(&self) -> &LocalModpack {
        &self.modpack
    }

    /// Scrolls the mods down by `distance`, for screenshots.
    #[cfg(feature = "screenshot")]
    pub fn scroll_by(&mut self, distance: Pixels, cx: &mut Context<Self>) {
        self.list.state().scroll_by(distance);
        cx.notify();
    }

    /// Plays the cards' entrance again, as the desktop app's cards mount
    /// again each time their page is shown.
    pub fn replay_entrance(&mut self, cx: &mut Context<Self>) {
        self.entrance.restart();
        cx.notify();
    }

    /// Shows the re-read manifest of the same modpack.
    pub fn set_modpack(&mut self, modpack: LocalModpack, cx: &mut Context<Self>) {
        self.cards = installed_cards(&modpack);
        self.modpack = modpack;
        let keys = catalog_keys(&self.cards);
        self.catalog
            .update(cx, |catalog, cx| catalog.want(keys, cx));
        cx.notify();
    }

    /// The Update button: checks every mod, or goes back to the list.
    pub fn toggle_updates(&mut self, cx: &mut Context<Self>) {
        if self.mode == Mode::Updates {
            self.mode = Mode::Mods;
            self.entrance.restart();
            cx.notify();
            return;
        }
        self.mode = Mode::Updates;
        self.updates = Loadable::Loading;
        self.finished.clear();
        let catalog = self.catalog.read(cx);
        let cards: Vec<Mod> = self
            .cards
            .iter()
            .map(|card| with_record(card, catalog.get(&card.source, &card.id)))
            .collect();
        let modpack = self.modpack.clone();
        let task = Backend::global(cx).run(cx, move |host| check_updates(host, modpack, cards));
        self.checking = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if matches!(&result, Ok(updates) if updates.is_empty()) {
                    this.mode = Mode::Mods;
                    snackbar::show(cx, SnackbarKind::Neutral, t("allModsUpToDate"));
                }
                this.updates = Loadable::from_result(result);
                this.entrance.restart();
                cx.notify();
            });
        });
        cx.notify();
    }

    pub fn toggle_identify(&mut self, cx: &mut Context<Self>) {
        if self.mode == Mode::Identify {
            self.mode = Mode::Mods;
            self.entrance.restart();
            cx.notify();
            return;
        }
        self.mode = Mode::Identify;
        self.unknowns = Loadable::Loading;
        let modpack = self.modpack.clone();
        let task = Backend::global(cx).run(cx, move |host| identify(host, modpack));
        self.checking = cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if matches!(&result, Ok(unknowns) if unknowns.is_empty()) {
                    this.mode = Mode::Mods;
                }
                this.unknowns = Loadable::from_result(result);
                this.entrance.restart();
                cx.notify();
            });
        });
        cx.notify();
    }

    fn running_on(&self, id: &str) -> bool {
        self.running.iter().any(|op| op.id() == id)
    }

    pub fn run(&mut self, op: Op, cx: &mut Context<Self>) {
        if self.running_on(op.id()) {
            return;
        }
        self.running.push(op.clone());
        let done_text = op.done_text();
        let task = Backend::global(cx).run(cx, {
            let op = op.clone();
            move |host| op.perform(host)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running.retain(|running| running.id() != op.id());
                this.progress.remove(op.id());
                match result {
                    Ok(()) => {
                        this.settle(&op);
                        snackbar::show(cx, SnackbarKind::Success, done_text);
                    }
                    Err(error) => snackbar::show(cx, SnackbarKind::Error, describe(error)),
                }
                cx.emit(ModpackViewEvent::Changed);
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
        let modpack = self.modpack.name.clone();
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

    /// The cards of the current mode the grid lays out.
    fn grid_cards(&self) -> Option<&[Mod]> {
        match (self.mode, &self.updates) {
            (Mode::Mods, _) if !self.cards.is_empty() => Some(&self.cards),
            (Mode::Updates, Loadable::Ready(updates)) => Some(updates),
            _ => None,
        }
    }

    /// `item` as its provider knows it, and how its card is drawn.
    fn shown(&self, item: &Mod, cx: &App) -> (Mod, CardStatus) {
        let item = with_record(item, self.catalog.read(cx).get(&item.source, &item.id));
        let status = CardStatus {
            progress: self
                .running_on(&item.id)
                .then(|| self.progress.get(&item.id).copied().unwrap_or(0)),
            ..CardStatus::default()
        };
        (item, status)
    }

    fn card(
        &self,
        index: usize,
        (item, status): &(Mod, CardStatus),
        slot: Option<CardSlot>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let on_press: OnCardPress<Self> =
            Rc::new(|this, press, item, _, _, cx| this.press(press, item, cx));
        motion::card_at(
            mod_card(index, item, *status, slot, on_press, cx),
            self.card_frame,
        )
        .into_any_element()
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = match (self.mode, &self.updates) {
            (Mode::Updates, Loadable::Ready(updates)) => {
                updates.iter().filter(|update| update.downloadable).count()
            }
            _ => self.modpack.mods.len(),
        };
        h_flex()
            .w_full()
            .mt_2()
            .mb_5()
            .p_4()
            .items_center()
            .rounded(theme::radius_card())
            .bg(rgb(theme::CONTROL))
            .font_weight(FontWeight::BOLD)
            .when(self.back, |this| {
                this.child(
                    theme::cancel_button("back", t("cancel"), cx)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(ModpackViewEvent::Back))),
                )
            })
            .child(
                h_flex()
                    .flex_1()
                    .justify_center()
                    .items_center()
                    .gap_4()
                    .child(div().text_center().child(summary(&self.modpack, count)))
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
                    .when(self.modpack.unknown_mods, |this| {
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
                    }),
            )
    }

    /// The body when it isn't a grid: a waiting panel or the identification.
    fn render_single(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.mode {
            Mode::Mods => waiting("-".into_any_element()),
            Mode::Updates => match &self.updates {
                Loadable::Failed(error) => waiting(error.clone().into_any_element()),
                _ => waiting(spinner()),
            },
            Mode::Identify => match &self.unknowns {
                Loadable::Ready(unknowns) => {
                    let mut offset = 0;
                    let groups: Vec<AnyElement> = unknowns
                        .iter()
                        .map(|unknown| {
                            let cards: Vec<AnyElement> = unknown
                                .proposals
                                .iter()
                                .enumerate()
                                .map(|(index, item)| {
                                    self.card(offset + index, &self.shown(item, cx), None, cx)
                                })
                                .collect();
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

    fn render_item(&mut self, ix: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let shape = self.list.shape();
        match shape.item(ix) {
            Item::Header(_) => v_flex()
                .w_full()
                .px_4()
                .pt_4()
                .child(self.render_header(cx))
                .into_any_element(),
            Item::Row { row, cards } => {
                let gap = if self.mode == Mode::Updates {
                    rems(1.5)
                } else {
                    rems(1.)
                };
                let source = self.grid_cards().unwrap_or_default();
                let shown: Vec<(Mod, CardStatus)> = self.shown[cards.clone()]
                    .iter()
                    .map(|&card| self.shown(&source[card], cx))
                    .collect();
                let key = self.row_heights.key(
                    shape.rows.columns,
                    shown.iter().map(|(item, status)| card_key(item, *status)),
                    u64::from(self.mode == Mode::Updates),
                );
                let elements: Vec<AnyElement> = shown
                    .iter()
                    .zip(cards)
                    .enumerate()
                    .map(|(column, (card, index))| {
                        let slot = CardSlot {
                            heights: &self.row_heights,
                            row: key,
                            first: column == 0,
                        };
                        self.card(index, card, Some(slot), cx)
                    })
                    .collect();
                let first = row == 0;
                let last = row + 1 == shape.rows.len();
                v_flex()
                    .w_full()
                    .px_4()
                    .when(last, |this| this.pb_4())
                    .child(
                        div()
                            .ml_2()
                            .mr_4()
                            .when(last, |this| this.mb_12())
                            .px_4()
                            .bg(rgb(theme::SURFACE))
                            .map(|this| if last { this.pb_4() } else { this.pb(gap) })
                            .when(first, |this| this.pt_4().rounded_t(theme::radius_card()))
                            .when(last, |this| this.rounded_b(theme::radius_card()))
                            .child(mod_grid::row(shape.rows.columns, gap, elements)),
                    )
                    .into_any_element()
            }
            Item::Trailer(_) => v_flex()
                .w_full()
                .px_4()
                .pb_4()
                .child(self.render_single(cx))
                .into_any_element(),
        }
    }
}

fn spinner() -> AnyElement {
    theme::circular_progress("progress").into_any_element()
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

impl Render for ModpackView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width;
        self.row_heights.next_frame(width);
        self.card_frame = self.entrance.frame(window);
        let grid = self.grid_cards().map(|cards| {
            let columns = match self.mode {
                Mode::Updates => mod_grid::update_columns(width),
                Mode::Mods | Mode::Identify => mod_grid::modpack_columns(width),
            };
            let shown: Vec<usize> = cards
                .iter()
                .enumerate()
                .filter(|(_, card)| !self.finished.contains(&card.id))
                .map(|(index, _)| index)
                .collect();
            (Rows::new(shown.len(), columns), shown)
        });
        let shape = match grid {
            Some((rows, shown)) => {
                self.shown = shown;
                Shape {
                    headers: 1,
                    rows,
                    trailers: 0,
                }
            }
            None => {
                self.shown.clear();
                Shape {
                    headers: 1,
                    rows: Rows::default(),
                    trailers: 1,
                }
            }
        };
        self.list.set_shape(shape);
        div()
            .size_full()
            .relative()
            .child(
                list(
                    self.list.state().clone(),
                    cx.processor(|this, ix, window, cx| this.render_item(ix, window, cx)),
                )
                .size_full(),
            )
            .child(self.list.probe(cx.entity_id()))
            .vertical_scrollbar(self.list.state())
    }
}
