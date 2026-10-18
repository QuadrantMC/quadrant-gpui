//! One modpack's mods, after the desktop app's `ModpackView` (a local pack's
//! Details) and `SharedModpackView` (a Quadrant Share or Sync pack).
//!
//! The desktop app fetches each mod's icon and description from Modrinth or
//! CurseForge. This view shows what the manifest records, so it never waits
//! on the network.

use gpui::{
    AnyElement, Context, FontWeight, IntoElement, ParentElement as _, SharedString, Styled as _,
    div, prelude::FluentBuilder as _, rgb,
};
use gpui_component::{h_flex, v_flex};
use quadrant_core::models::{InstalledMod, ModSource};

use super::{ModpacksPage, View};
use crate::{i18n::t, theme};

/// How many mod cards share a row.
const COLUMNS: u16 = 3;

/// The header bar with a way back to the list, `action` beside the summary,
/// and a card per mod.
pub fn render(
    summary: SharedString,
    action: Option<AnyElement>,
    mods: &[InstalledMod],
    cx: &mut Context<ModpacksPage>,
) -> impl IntoElement {
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
                        .children(action),
                ),
        )
        .when(!mods.is_empty(), |this| {
            this.child(
                div()
                    .ml_2()
                    .mr_4()
                    .mb_12()
                    .p_4()
                    .grid()
                    .grid_cols(COLUMNS)
                    .gap_4()
                    .rounded(theme::radius_card())
                    .bg(rgb(theme::SURFACE))
                    .children(mods.iter().map(mod_card)),
            )
        })
}

fn mod_card(installed: &InstalledMod) -> impl IntoElement {
    let name = if installed.name.is_empty() {
        installed.id.clone()
    } else {
        installed.name.clone()
    };
    let source = match installed.source {
        ModSource::CurseForge => "CurseForge",
        ModSource::Modrinth => "Modrinth",
        ModSource::Online => "Online",
    };
    let mut details = vec![source.to_string()];
    if !installed.version.is_empty() {
        details.insert(0, installed.version.clone());
    }
    v_flex()
        .min_w_0()
        .p_4()
        .gap_1()
        .rounded(theme::radius_card())
        .bg(rgb(theme::PAGE))
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::EXTRA_BOLD)
                .child(name),
        )
        .child(
            div()
                .text_color(rgb(theme::TEXT_MUTED))
                .child(details.join(" · ")),
        )
        .when(!installed.description.is_empty(), |this| {
            this.child(div().child(installed.description.clone()))
        })
}
