//! A Quadrant Share or Sync pack's mods, after the desktop app's
//! `SharedModpackView`: a header with its download, then a card per mod in
//! `ModpackView`'s grid. Cards show the manifest's record until the mod's
//! provider answers through the [`ModCatalog`](crate::mod_catalog::ModCatalog).

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, FontWeight, IntoElement, ParentElement as _, Pixels, SharedString,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{h_flex, scroll::ScrollableElement as _, v_flex};
use quadrant_core::{mc_mod::Mod, models::InstalledModpack};

use super::{ModpacksPage, SyncTarget, View};
use crate::{
    i18n::{t, t_with},
    mod_catalog::{ModCatalog, ModKey, Tally},
    motion,
    pages::{
        mod_grid::{self, CardSlot, GridList, Item, RowHeights, Rows, Shape},
        mod_ui::{CardStatus, OnCardPress, card_key, mod_card},
        modpack_view::{catalog_keys, grid_row, installed_card, with_record},
    },
    theme,
};

/// About one row of cards, for the scrollbar until rows are measured.
const ROW_HINT: Pixels = px(190.);

/// The gap between cards, across and down.
fn gap() -> gpui::Rems {
    gpui::rems(1.)
}

/// An open shared pack: what the view draws from, made once when it opens.
pub struct SharedPack {
    pub modpack: Rc<InstalledModpack>,
    pub sync: Option<SyncTarget>,
    /// The pack's mods as cards that only open in the web.
    cards: Vec<Mod>,
    /// `cards` as their providers know them, kept as the catalog notifies.
    merged: Vec<Rc<Mod>>,
    keys: Vec<ModKey>,
    /// How many of `keys` the catalog has answered, kept as it notifies.
    tally: Tally,
    list: GridList,
    row_heights: RowHeights,
}

impl SharedPack {
    pub fn new(modpack: InstalledModpack, sync: Option<SyncTarget>) -> Self {
        let cards = cards(&modpack);
        let keys = catalog_keys(&cards);
        Self {
            modpack: Rc::new(modpack),
            sync,
            cards,
            merged: Vec::new(),
            keys,
            tally: Tally::default(),
            list: GridList::new(ROW_HINT),
            row_heights: RowHeights::default(),
        }
    }

    pub fn keys(&self) -> &[ModKey] {
        &self.keys
    }

    /// Takes in the catalog's answers, after it changed.
    pub fn retally(&mut self, cx: &mut App) {
        let catalog = ModCatalog::global(cx);
        let catalog = catalog.read(cx);
        self.tally = catalog.tally(&self.keys);
        self.merged = self
            .cards
            .iter()
            .map(|card| Rc::new(with_record(card, catalog.get(&card.source, &card.id))))
            .collect();
    }

    /// Lays the list out for this frame's width.
    pub fn prepare(&mut self, window: &Window) {
        let width = window.viewport_size().width;
        self.row_heights.next_frame(width);
        self.list.set_shape(Shape {
            headers: 1,
            rows: Rows::new(self.cards.len(), mod_grid::modpack_columns(width)),
            trailers: 0,
        });
    }
}

/// The pack's mods as cards that only open in the web.
fn cards(modpack: &InstalledModpack) -> Vec<Mod> {
    modpack
        .mods
        .iter()
        .map(|installed| Mod {
            deleteable: false,
            ..installed_card(installed, &modpack.name)
        })
        .collect()
}

impl ModpacksPage {
    /// The open shared pack, or the one fading out as `leaving`.
    fn shared(&self, leaving: bool) -> Option<&SharedPack> {
        let view = if leaving {
            &self.leaving.as_ref()?.0
        } else {
            &self.view
        };
        match view {
            View::Shared(shared) => Some(shared),
            View::List | View::Details { .. } => None,
        }
    }

    /// The header bar with a way back to the list and the download beside
    /// the summary, then the cards, only the rows on screen drawn.
    pub(super) fn render_shared(
        &self,
        shared: &SharedPack,
        leaving: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .size_full()
            .relative()
            .child(
                gpui::list(
                    shared.list.state().clone(),
                    cx.processor(move |this, ix, window, cx| {
                        this.render_shared_item(leaving, ix, window, cx)
                    }),
                )
                .size_full(),
            )
            .child(shared.list.probe(cx.entity_id()))
            .vertical_scrollbar(shared.list.state())
            .into_any_element()
    }

    fn render_shared_item(
        &mut self,
        leaving: bool,
        ix: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(shared) = self.shared(leaving) else {
            return div().into_any_element();
        };
        let shape = shared.list.shape();
        match shape.item(ix) {
            Item::Header(_) => v_flex()
                .w_full()
                .px_4()
                .pt_4()
                .child(self.render_shared_header(shared, cx))
                .into_any_element(),
            Item::Row { row, cards } => {
                let shown = shared.merged.get(cards.clone()).unwrap_or_default();
                let status = CardStatus::default();
                let key = shared.row_heights.key(
                    shape.rows.columns,
                    shown.iter().map(|card| card_key(card, status)),
                    0,
                );
                let on_press: OnCardPress<ModpacksPage> = Rc::new(|_, _, _, _, _, _| {});
                let frame = if leaving { None } else { self.shared_frame };
                let elements = shown
                    .iter()
                    .zip(cards)
                    .enumerate()
                    .map(|(column, (card, index))| {
                        let slot = CardSlot {
                            heights: &shared.row_heights,
                            row: key,
                            first: column == 0,
                        };
                        motion::card_at(
                            mod_card(index, card, status, Some(slot), on_press.clone(), cx),
                            frame,
                        )
                        .into_any_element()
                    })
                    .collect();
                grid_row(
                    row,
                    shape.rows.len(),
                    gap(),
                    mod_grid::row(shape.rows.columns, gap(), elements),
                )
            }
            Item::Trailer(_) => div().into_any_element(),
        }
    }

    fn render_shared_header(
        &self,
        shared: &SharedPack,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let modpack = &shared.modpack;
        let summary: SharedString = format!(
            "{} | {} | {} | {}",
            modpack.name,
            modpack.mod_loader,
            modpack.version,
            t_with("modCount", &[("amount", &modpack.mods.len().to_string())])
        )
        .into();
        let download = self.download_button(
            "shared-download",
            t("download"),
            modpack.clone(),
            shared.sync.clone(),
            cx,
        );
        v_flex()
            .mt_2()
            .mb_5()
            .p_4()
            .rounded(theme::radius_card())
            .bg(theme::palette().control)
            .font_weight(FontWeight::BOLD)
            .child(
                h_flex()
                    .items_center()
                    .child(
                        theme::cancel_button("back", t("cancel"), cx)
                            .on_click(cx.listener(|this, _, _, cx| this.show(View::List, cx))),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .justify_center()
                            .items_center()
                            .gap_4()
                            .child(div().text_center().child(summary))
                            .child(download),
                    ),
            )
            .when(shared.tally.pending > 0, |this| {
                this.child(loading_status(shared.tally.found, shared.cards.len()))
            })
    }
}

/// "Loading mods… 3 / 12" under the header while providers are still being
/// asked, as `SharedModpackView` shows it.
fn loading_status(loaded: usize, total: usize) -> impl IntoElement {
    h_flex()
        .mt_4()
        .justify_center()
        .items_center()
        .gap_3()
        .text_sm()
        .text_color(theme::palette().text_soft)
        .child(theme::spinner("loading-mods"))
        .child(t_with(
            "loadingMods",
            &[
                ("loaded", &loaded.to_string()),
                ("total", &total.to_string()),
            ],
        ))
}

#[cfg(test)]
mod tests {
    use quadrant_core::models::{InstalledMod, InstalledModpack, ModLoader, ModSource};

    use super::SharedPack;

    #[test]
    fn a_shared_pack_makes_its_cards_and_keys_once() {
        let mods = ["sodium", "lithium"].map(|id| {
            InstalledMod::minimal(
                id.into(),
                ModSource::Modrinth,
                format!("https://cdn.modrinth.com/data/{id}/{id}.jar"),
            )
        });
        let shared = SharedPack::new(
            InstalledModpack {
                mod_config_version: "2".into(),
                quadrant_version: String::new(),
                name: "Alpha".into(),
                version: "1.21.1".into(),
                mod_loader: ModLoader::Fabric,
                mods: mods.to_vec(),
            },
            None,
        );
        assert_eq!(shared.cards.len(), 2);
        assert!(shared.cards.iter().all(|card| !card.deleteable));
        assert_eq!(shared.keys().len(), 2);
    }
}
