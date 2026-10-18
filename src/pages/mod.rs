use gpui::{AnyView, App, AppContext as _, Window};

mod account;
mod current_modpack;
mod installed_content;
mod mod_install;
mod mod_ui;
mod modpacks;
mod search;
mod settings;

pub use account::AccountPage;
pub use current_modpack::CurrentModpackPage;
pub use installed_content::InstalledContentPage;
pub use modpacks::ModpacksPage;
pub use search::SearchPage;
#[cfg(feature = "screenshot")]
pub use search::logic::ContentType;
pub use settings::SettingsPage;

use crate::route::Route;

/// Builds the view for `route`. Called once per route; the app keeps the
/// view alive across navigation.
pub fn build(route: Route, window: &mut Window, cx: &mut App) -> AnyView {
    match route {
        Route::Modpacks => cx.new(|cx| ModpacksPage::new(window, cx)).into(),
        Route::Search => cx.new(|cx| SearchPage::new(window, cx)).into(),
        Route::CurrentModpack => cx.new(CurrentModpackPage::new).into(),
        Route::Settings => cx.new(SettingsPage::new).into(),
        Route::InstalledContent => cx.new(InstalledContentPage::new).into(),
        Route::Account => cx.new(AccountPage::new).into(),
    }
}
