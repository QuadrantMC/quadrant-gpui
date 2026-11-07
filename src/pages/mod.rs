use gpui::{AnyView, App, AppContext as _, SharedString, Window};

mod account;
mod current_modpack;
mod install_location;
mod installed_content;
mod mod_grid;
mod mod_install;
mod mod_ui;
mod modpack_view;
mod modpacks;
mod search;
mod settings;

pub use account::AccountPage;
pub use current_modpack::CurrentModpackPage;
pub use installed_content::InstalledContentPage;
pub use modpacks::{ModpacksPage, parse_share_code};
pub use search::SearchPage;
#[cfg(feature = "screenshot")]
pub use search::logic::ContentType;
pub use settings::SettingsPage;

use crate::route::Route;

/// What the title bar names while `page` shows content of its own (a pack's
/// mods, a mod's install page); `None` for the route's own label.
pub fn title(page: &AnyView, cx: &App) -> Option<SharedString> {
    if let Ok(modpacks) = page.clone().downcast::<ModpacksPage>() {
        return modpacks.read(cx).title(cx);
    }
    if let Ok(search) = page.clone().downcast::<SearchPage>() {
        return search.read(cx).title(cx);
    }
    None
}

/// Closes the content `page` shows of its own, as the desktop app's back
/// does before it leaves the page; false when there is none.
pub fn back(page: &AnyView, cx: &mut App) -> bool {
    if let Ok(modpacks) = page.clone().downcast::<ModpacksPage>() {
        return modpacks.update(cx, |page, cx| page.back(cx));
    }
    if let Ok(search) = page.clone().downcast::<SearchPage>() {
        return search.update(cx, |page, cx| page.back(cx));
    }
    false
}

/// Re-reads the host data `page` shows, now that its route is shown again:
/// the desktop app remounts a page on every visit, so its data reloads.
/// What the page shows stays until the answers replace it.
pub fn shown(page: &AnyView, window: &mut Window, cx: &mut App) {
    if let Ok(page) = page.clone().downcast::<ModpacksPage>() {
        page.update(cx, |page, cx| page.revalidate(cx));
    } else if let Ok(page) = page.clone().downcast::<CurrentModpackPage>() {
        page.update(cx, |page, cx| page.revalidate(cx));
    } else if let Ok(page) = page.clone().downcast::<InstalledContentPage>() {
        page.update(cx, |page, cx| page.revalidate(cx));
    } else if let Ok(page) = page.clone().downcast::<AccountPage>() {
        page.update(cx, |page, cx| page.revalidate(cx));
    } else if let Ok(page) = page.clone().downcast::<SearchPage>() {
        page.update(cx, |page, cx| page.revalidate(window, cx));
    }
}

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
