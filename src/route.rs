use gpui::{ElementId, SharedString};

use crate::{assets::MdIcon, i18n::t};

/// A top-level page, in rail order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Route {
    Modpacks,
    CurrentModpack,
    Search,
    InstalledContent,
    Account,
    Settings,
}

impl Route {
    pub const ALL: [Route; 6] = [
        Route::Modpacks,
        Route::CurrentModpack,
        Route::Search,
        Route::InstalledContent,
        Route::Account,
        Route::Settings,
    ];

    /// The desktop app's name for the page, as `lastPageName` keeps it. It
    /// is also the key of the page's title.
    pub fn name(self) -> &'static str {
        match self {
            Route::Modpacks => "apply",
            Route::CurrentModpack => "currentModpack",
            Route::Search => "search",
            Route::InstalledContent => "installedContent",
            Route::Account => "account",
            Route::Settings => "settings",
        }
    }

    pub fn from_name(name: &str) -> Option<Route> {
        Route::ALL.into_iter().find(|route| route.name() == name)
    }

    /// The page title, from the desktop app's strings.
    pub fn label(self) -> SharedString {
        t(self.name())
    }

    pub fn icon(self) -> MdIcon {
        match self {
            Route::Modpacks => MdIcon::Check,
            Route::CurrentModpack => MdIcon::Description,
            Route::Search => MdIcon::Search,
            Route::InstalledContent => MdIcon::Folder,
            Route::Account => MdIcon::AccountCircle,
            Route::Settings => MdIcon::Settings,
        }
    }

    pub fn id(self) -> ElementId {
        ElementId::Name(format!("route-{self:?}").into())
    }
}

/// Back/forward navigation over pages, like a browser's: opening a page drops
/// whatever was ahead of the current one.
#[derive(Debug)]
pub struct History {
    entries: Vec<Route>,
    cursor: usize,
}

impl History {
    pub fn new(start: Route) -> Self {
        Self {
            entries: vec![start],
            cursor: 0,
        }
    }

    pub fn current(&self) -> Route {
        self.entries[self.cursor]
    }

    /// Opens `route`, returning false when it is already the current page.
    pub fn navigate(&mut self, route: Route) -> bool {
        if route == self.current() {
            return false;
        }
        self.entries.truncate(self.cursor + 1);
        self.entries.push(route);
        self.cursor += 1;
        true
    }

    pub fn back(&mut self) -> Option<Route> {
        self.cursor = self.cursor.checked_sub(1)?;
        Some(self.current())
    }

    pub fn forward(&mut self) -> Option<Route> {
        if self.cursor + 1 >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        Some(self.current())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_names_round_trip_as_the_desktop_app_writes_them() {
        for route in Route::ALL {
            assert_eq!(Route::from_name(route.name()), Some(route));
        }
        assert_eq!(
            Route::from_name("currentModpack"),
            Some(Route::CurrentModpack)
        );
        // A legacy numeric `lastPage` matches no name.
        assert_eq!(Route::from_name("2"), None);
    }

    #[test]
    fn back_and_forward_walk_the_visited_pages() {
        let mut history = History::new(Route::Modpacks);
        assert!(history.navigate(Route::Search));
        assert!(history.navigate(Route::Settings));
        assert_eq!(history.back(), Some(Route::Search));
        assert_eq!(history.back(), Some(Route::Modpacks));
        assert_eq!(history.back(), None);
        assert_eq!(history.current(), Route::Modpacks);
        assert_eq!(history.forward(), Some(Route::Search));
        assert_eq!(history.forward(), Some(Route::Settings));
        assert_eq!(history.forward(), None);
    }

    #[test]
    fn navigating_after_going_back_drops_the_forward_pages() {
        let mut history = History::new(Route::Modpacks);
        history.navigate(Route::Search);
        history.navigate(Route::Settings);
        history.back();
        assert!(history.navigate(Route::Account));
        assert_eq!(history.forward(), None);
        assert_eq!(history.back(), Some(Route::Search));
    }

    #[test]
    fn reopening_the_current_page_is_not_a_step() {
        let mut history = History::new(Route::Modpacks);
        assert!(!history.navigate(Route::Modpacks));
        assert_eq!(history.back(), None);
    }

    #[test]
    fn titles_come_from_the_desktop_strings() {
        let titles = Route::ALL.map(|route| route.label().to_string());
        assert_eq!(
            titles,
            [
                "Apply",
                "Current modpack",
                "Search",
                "Installed content",
                "Account",
                "Settings"
            ]
        );
    }
}
