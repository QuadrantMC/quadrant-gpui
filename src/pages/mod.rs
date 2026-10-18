use gpui::{
    AnyView, App, AppContext as _, Context, FontWeight, IntoElement, ParentElement as _, Render,
    Styled as _, Window, rgb,
};
use gpui_component::v_flex;

mod current_modpack;
mod mod_install;
mod mod_ui;
mod modpacks;
mod search;

pub use current_modpack::CurrentModpackPage;
pub use modpacks::ModpacksPage;
pub use search::SearchPage;
#[cfg(feature = "screenshot")]
pub use search::logic::ContentType;

use crate::{route::Route, theme};

/// Builds the view for `route`. Called once per route; the app keeps the
/// view alive across navigation.
pub fn build(route: Route, window: &mut Window, cx: &mut App) -> AnyView {
    match route {
        Route::Modpacks => cx.new(|cx| ModpacksPage::new(window, cx)).into(),
        Route::Search => cx.new(|cx| SearchPage::new(window, cx)).into(),
        Route::CurrentModpack => cx.new(CurrentModpackPage::new).into(),
        Route::InstalledContent | Route::Account | Route::Settings => {
            cx.new(|_| Placeholder { route }).into()
        }
    }
}

/// Stands in for a page that has not been ported yet.
pub struct Placeholder {
    route: Route,
}

impl Render for Placeholder {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                gpui::div()
                    .text_2xl()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(self.route.label()),
            )
            .child(
                gpui::div()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child("Not yet implemented"),
            )
    }
}
