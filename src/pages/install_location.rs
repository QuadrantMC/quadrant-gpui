//! The "Install to" picker for resource packs and shaders, after
//! `contentLocations.ts` in the desktop app and its use on the Search and
//! mod install pages. A pack can go to the Minecraft folder or to a Prism
//! Launcher instance; the picker shows only when there is more than one, which
//! takes experimental features and a Prism Launcher install.

use gpui::{App, AppContext as _, Context, Entity, SharedString, Task, Window};
use gpui_component::select::{SearchableVec, Select, SelectState};
use quadrant_core::{
    content::{ContentLocation, ContentLocationKind},
    mc_mod::ModType,
};
use quadrant_host::QuadrantHost;

use crate::{
    backend::Backend,
    i18n::{t, t_with},
    pages::{installed_content::location_title, mod_ui::Choice},
};

/// Whether this kind of content can be sent to a location of its own.
pub fn is_pack(mod_type: ModType) -> bool {
    matches!(mod_type, ModType::ResourcePack | ModType::ShaderPack)
}

/// How a location reads in the picker, saying which ones Prism Launcher owns.
pub fn option_label(location: &ContentLocation) -> SharedString {
    let title = location_title(location);
    match location.kind {
        ContentLocationKind::Minecraft => title,
        ContentLocationKind::Prism => t_with("installedContentPrismOption", &[("name", &title)]),
    }
}

/// The location the install page starts on. A single location is not a
/// choice, so nothing is sent. With a target modpack, following it stays the
/// default; without one, the first location. The opener's choice wins while
/// that location is still listed.
pub fn initial_location(
    locations: &[ContentLocation],
    has_target: bool,
    opener: Option<&str>,
) -> Option<String> {
    let opened = opener.and_then(|id| locations.iter().find(|location| location.id == id));
    let fallback = (locations.len() > 1 && !has_target).then(|| &locations[0]);
    opened.or(fallback).map(|location| location.id.clone())
}

/// The picker's choices: `automatic` first, when the page offers following
/// the modpack, then every location.
pub fn choices(
    locations: &[ContentLocation],
    automatic: Option<SharedString>,
) -> Vec<Choice<Option<String>>> {
    automatic
        .map(|label| Choice::new(label, None))
        .into_iter()
        .chain(
            locations
                .iter()
                .map(|location| Choice::new(option_label(location), Some(location.id.clone()))),
        )
        .collect()
}

type PickLocation = SelectState<SearchableVec<Choice<Option<String>>>>;

pub struct LocationPicker {
    /// The locations' names only; their packs are never listed here.
    locations: Vec<ContentLocation>,
    select: Entity<PickLocation>,
    loading: Task<()>,
}

impl LocationPicker {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            locations: Vec::new(),
            select: cx.new(|cx| SelectState::new(SearchableVec::new(Vec::new()), None, window, cx)),
            loading: Task::ready(()),
        }
    }

    /// Offers `locations`, starting on `selected`.
    pub fn set_locations(
        &mut self,
        locations: Vec<ContentLocation>,
        automatic: Option<SharedString>,
        selected: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choices = choices(&locations, automatic);
        self.select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(choices), window, cx);
            select.set_selected_value(&selected, window, cx);
        });
        self.locations = locations;
        cx.notify();
    }

    /// Search's picker: for a pack, every location after "Automatic", which
    /// it starts on; for a mod, nothing. A host that can't list the
    /// locations costs the picker, not the install.
    pub fn load_for_search(
        &mut self,
        mod_type: ModType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_locations(Vec::new(), None, None, window, cx);
        self.loading = Task::ready(());
        if is_pack(mod_type) {
            self.list_for_search(None, window, cx);
        }
    }

    /// Lists Search's locations again, keeping the one picked while it is
    /// still there, unless a listing is on its way.
    pub fn revalidate(&mut self, mod_type: ModType, window: &mut Window, cx: &mut Context<Self>) {
        if is_pack(mod_type) && self.loading.is_ready() {
            let picked = self.selected(cx);
            self.list_for_search(picked, window, cx);
        }
    }

    fn list_for_search(
        &mut self,
        picked: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let task =
            Backend::global(cx).run(cx, |host| async move { host.get_installed_content(false) });
        self.loading = cx.spawn_in(window, async move |this, cx| {
            let locations = task.await.unwrap_or_else(|error| {
                log::warn!("Listing content locations failed: {error:#}");
                Vec::new()
            });
            let picked = picked.filter(|id| locations.iter().any(|location| &location.id == id));
            let _ = this.update_in(cx, |this, window, cx| {
                let automatic = Some(t("installedContentInstallAuto"));
                this.set_locations(locations, automatic, picked, window, cx)
            });
        });
    }

    /// The location the user picked, or `None` for the automatic placement.
    pub fn selected(&self, cx: &App) -> Option<String> {
        if self.locations.len() < 2 {
            return None;
        }
        self.select.read(cx).selected_value().cloned().flatten()
    }

    /// The select, while there is more than one location to choose from.
    pub fn select(&self) -> Option<Select<SearchableVec<Choice<Option<String>>>>> {
        (self.locations.len() > 1).then(|| Select::new(&self.select))
    }
}

/// Lists the location names for an install page that will offer them.
pub async fn list_for(host: &QuadrantHost, mod_type: ModType) -> Vec<ContentLocation> {
    if !is_pack(mod_type) {
        return Vec::new();
    }
    host.get_installed_content(false).unwrap_or_else(|error| {
        log::warn!("Listing content locations failed: {error:#}");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(id: &str, kind: ContentLocationKind, name: &str) -> ContentLocation {
        ContentLocation {
            id: id.into(),
            kind,
            name: name.into(),
            path: String::new(),
            sections: Vec::new(),
        }
    }

    fn two() -> Vec<ContentLocation> {
        vec![
            location("minecraft", ContentLocationKind::Minecraft, ""),
            location("prism:survival", ContentLocationKind::Prism, "Survival"),
        ]
    }

    #[test]
    fn only_packs_have_a_location() {
        assert!(is_pack(ModType::ResourcePack));
        assert!(is_pack(ModType::ShaderPack));
        assert!(!is_pack(ModType::Mod));
    }

    #[test]
    fn prism_locations_say_so() {
        let titles: Vec<_> = choices(&two(), Some(t("installedContentInstallAuto")))
            .into_iter()
            .map(|choice| (choice.title, choice.value))
            .collect();
        assert_eq!(
            titles,
            [
                ("Automatic".into(), None),
                ("Minecraft folder".into(), Some("minecraft".into())),
                (
                    "Survival (Prism Launcher)".into(),
                    Some("prism:survival".into())
                ),
            ]
        );
        assert_eq!(choices(&two(), None).len(), 2, "no automatic choice");
    }

    #[test]
    fn the_install_page_starts_where_the_original_does() {
        let locations = two();
        assert_eq!(
            initial_location(&locations, true, None),
            None,
            "follows the target"
        );
        assert_eq!(
            initial_location(&locations, false, None).as_deref(),
            Some("minecraft"),
            "without a target, the first location"
        );
        assert_eq!(
            initial_location(&locations, true, Some("prism:survival")).as_deref(),
            Some("prism:survival"),
            "the opener's choice wins"
        );
        assert_eq!(
            initial_location(&locations, true, Some("prism:gone")),
            None,
            "unless it is gone"
        );
        assert_eq!(
            initial_location(&locations[..1], false, None),
            None,
            "one location is no choice"
        );
    }
}
