//! A Quadrant Share or Sync pack's mods, after the desktop app's
//! `SharedModpackView`: a header with its download, then a card per mod in
//! `ModpackView`'s grid. Cards show the manifest's record until the mod's
//! provider answers through the [`ModCatalog`](crate::mod_catalog::ModCatalog).

use std::rc::Rc;

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _, rgb,
};
use gpui_component::{h_flex, v_flex};
use quadrant_core::{mc_mod::Mod, models::InstalledModpack};

use super::{ModpacksPage, View};
use crate::{
    i18n::t,
    mod_catalog::ModCatalog,
    pages::{
        mod_grid::{self, Rows},
        mod_ui::{CardStatus, OnCardPress, mod_card},
        modpack_view::{installed_card, with_record},
    },
    theme,
};

/// The pack's mods as cards that only open in the web.
pub fn cards(modpack: &InstalledModpack) -> Vec<Mod> {
    modpack
        .mods
        .iter()
        .map(|installed| Mod {
            deleteable: false,
            ..installed_card(installed, &modpack.name)
        })
        .collect()
}

/// The header bar with a way back to the list and `action` beside the
/// summary, then the cards.
pub fn render(
    summary: SharedString,
    action: AnyElement,
    cards: &[Mod],
    window: &Window,
    cx: &mut Context<ModpacksPage>,
) -> impl IntoElement {
    let rows = Rows::new(
        cards.len(),
        mod_grid::modpack_columns(window.viewport_size().width),
    );
    let on_press: OnCardPress<ModpacksPage> = Rc::new(|_, _, _, _, _| {});
    let catalog = ModCatalog::global(cx);
    let grid: Vec<AnyElement> = (0..rows.len())
        .map(|row| {
            let elements = rows
                .cards(row)
                .map(|index| {
                    let card = &cards[index];
                    let card = with_record(card, catalog.read(cx).get(&card.source, &card.id));
                    mod_card(index, &card, CardStatus::default(), on_press.clone(), cx)
                })
                .collect();
            mod_grid::row(rows.columns, gpui::rems(1.), elements).into_any_element()
        })
        .collect();
    v_flex()
        .p_4()
        .child(
            h_flex()
                .mt_2()
                .mb_5()
                .p_4()
                .items_center()
                .rounded(theme::radius_card())
                .bg(rgb(theme::CONTROL))
                .font_weight(FontWeight::BOLD)
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
                        .child(action),
                ),
        )
        .when_some((!grid.is_empty()).then_some(grid), |this, grid| {
            this.child(
                v_flex()
                    .ml_2()
                    .mr_4()
                    .mb_12()
                    .p_4()
                    .gap_4()
                    .rounded(theme::radius_card())
                    .bg(rgb(theme::SURFACE))
                    .children(grid),
            )
        })
}
