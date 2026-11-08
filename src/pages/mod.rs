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
    } else if let Ok(page) = page.clone().downcast::<SettingsPage>() {
        page.update(cx, |page, cx| page.revalidate(cx));
    }
}

/// `page`'s route is no longer shown.
pub fn hidden(page: &AnyView, cx: &mut App) {
    if let Ok(page) = page.clone().downcast::<ModpacksPage>() {
        page.update(cx, |page, _| page.hidden());
    } else if let Ok(page) = page.clone().downcast::<CurrentModpackPage>() {
        page.update(cx, |page, _| page.hidden());
    }
}

/// What a file dialog's answer comes to: what the user chose, or `None` for a
/// cancel. A dialog the platform couldn't open (no desktop portal on a
/// minimal Linux setup) is an error to show, not a cancel.
pub fn dialog_choice<T>(
    answer: Result<anyhow::Result<Option<T>>, futures::channel::oneshot::Canceled>,
) -> anyhow::Result<Option<T>> {
    answer.unwrap_or(Ok(None))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_that_failed_to_open_is_not_a_cancel() {
        let (_, dropped) = futures::channel::oneshot::channel::<anyhow::Result<Option<u8>>>();
        let canceled = futures::executor::block_on(dropped);
        assert_eq!(dialog_choice(canceled).unwrap(), None);
        assert_eq!(dialog_choice(Ok(Ok(None::<u8>))).unwrap(), None);
        assert_eq!(dialog_choice(Ok(Ok(Some(3)))).unwrap(), Some(3));
        let failed = dialog_choice(Ok(Err::<Option<u8>, _>(anyhow::anyhow!("no portal"))));
        assert!(failed.is_err());
    }
}
