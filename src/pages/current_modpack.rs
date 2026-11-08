//! The Current modpack page: the applied modpack in a [`ModpackView`], after
//! `CurrentModpackPage` in the desktop app.

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Task, Window, div,
};
use gpui_component::{scroll::ScrollableElement as _, v_flex};
use quadrant_core::models::LocalModpack;

use crate::{
    backend::Backend,
    loadable::{Loadable, Reload},
    pages::{
        mod_ui::waiting,
        modpack_view::{ModpackView, ModpackViewEvent},
        modpacks::{digest, watch},
    },
    theme,
};

pub struct CurrentModpackPage {
    /// `Ready(None)` when no modpack is applied.
    modpack: Loadable<Option<LocalModpack>>,
    /// The applied modpack's view, kept while the same pack stays applied.
    view: Option<(Entity<ModpackView>, Subscription)>,
    /// The digest of the pack `modpack` holds, to tell a fresh answer
    /// that changes nothing.
    listed: Option<u64>,
    /// The newest listing; replacing it drops a stale answer.
    loading: Reload,
    /// The watch on the Minecraft folder and its modpacks, while shown, as
    /// the desktop app watches the `mods` folder.
    folders: Option<Task<()>>,
}

impl CurrentModpackPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            modpack: Loadable::Loading,
            view: None,
            listed: None,
            loading: Reload::default(),
            folders: None,
        };
        page.refresh(cx);
        page.folders = Some(watch::folders(cx, Self::folder_changed));
        page
    }

    /// Re-reads the applied modpack, redrawing only when it changed.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move {
            let modpacks = host.get_modpacks(false).await?;
            let applied = modpacks.into_iter().find(|modpack| modpack.is_applied);
            let digest = digest(applied.as_slice())?;
            Ok((applied, digest))
        });
        self.loading.start(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let digest = result.as_ref().ok().map(|(_, digest)| *digest);
                let fresh = Loadable::from_result(result.map(|(applied, _)| applied));
                if !unchanged((&this.modpack, this.listed), (&fresh, digest)) {
                    this.modpack = fresh;
                    this.listed = digest;
                    this.show_applied(cx);
                    cx.notify();
                }
                if this.loading.landed() {
                    this.refresh(cx);
                }
            });
        }));
    }

    /// Something changed in the Minecraft folder or under `modpacks/`.
    fn folder_changed(&mut self, cx: &mut Context<Self>) {
        if self.loading.ask() {
            self.refresh(cx);
        }
    }

    /// Re-reads the applied modpack as the page is shown again, and watches
    /// the folders while it is.
    pub fn revalidate(&mut self, cx: &mut Context<Self>) {
        if let Some((view, _)) = &self.view {
            view.update(cx, |view, cx| view.replay_entrance(cx));
        }
        if self.loading.ask() {
            self.refresh(cx);
        }
        self.folders = Some(watch::folders(cx, Self::folder_changed));
    }

    /// The page is no longer shown: it stops watching the folders.
    pub fn hidden(&mut self) {
        self.folders = None;
    }

    /// Points the view at the applied modpack: the same pack's view takes
    /// the new manifest, another pack gets a view of its own.
    fn show_applied(&mut self, cx: &mut Context<Self>) {
        let Loadable::Ready(Some(modpack)) = &self.modpack else {
            self.view = None;
            return;
        };
        if let Some((view, _)) = &self.view
            && view.read(cx).modpack().name == modpack.name
        {
            let modpack = modpack.clone();
            view.update(cx, |view, cx| view.set_modpack(modpack, cx));
            return;
        }
        let modpack = modpack.clone();
        let view = cx.new(|cx| ModpackView::new(modpack, false, cx));
        let changed = cx.subscribe(&view, |this, _, event, cx| match event {
            ModpackViewEvent::Changed => this.refresh(cx),
            ModpackViewEvent::Back => {}
        });
        self.view = Some((view, changed));
    }

    /// The Update button of the applied modpack's view, for screenshots.
    #[cfg(feature = "screenshot")]
    pub fn toggle_updates(&mut self, cx: &mut Context<Self>) {
        if let Some((view, _)) = &self.view {
            view.update(cx, |view, cx| view.toggle_updates(cx));
        }
    }
}

/// Whether a re-read found the same modpack, so the page need not redraw:
/// each side is what it holds and the digest of a pack it read.
fn unchanged(
    (old, listed): (&Loadable<Option<LocalModpack>>, Option<u64>),
    (new, digest): (&Loadable<Option<LocalModpack>>, Option<u64>),
) -> bool {
    match (old, new) {
        (Loadable::Ready(_), Loadable::Ready(_)) => listed.is_some() && listed == digest,
        (Loadable::Failed(old), Loadable::Failed(new)) => old == new,
        _ => false,
    }
}

impl Render for CurrentModpackPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if let Some((view, _)) = &self.view {
            return div().size_full().child(view.clone()).into_any_element();
        }
        let content = match &self.modpack {
            Loadable::Idle | Loadable::Loading => {
                waiting(theme::circular_progress("progress").into_any_element())
            }
            Loadable::Failed(error) => waiting(error.clone().into_any_element()),
            Loadable::Ready(_) => div().child("-").into_any_element(),
        };
        v_flex()
            .size_full()
            .overflow_y_scrollbar()
            .child(div().p_4().child(content))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use crate::pages::modpack_view::{Op, installed_card, installed_cards, summary};
    use quadrant_core::{
        mc_mod::UniversalModFile,
        models::{InstalledMod, ModLoader, ModSource},
    };
    use quadrant_host::{QuadrantHost, QuadrantHostOptions};

    fn installed(id: &str, name: &str, downloads: i64) -> InstalledMod {
        let mut entry = InstalledMod::minimal(
            id.into(),
            ModSource::Modrinth,
            format!("https://cdn.modrinth.com/data/{id}/{name}.jar"),
        );
        entry.name = name.into();
        entry.slug = name.to_lowercase();
        entry.mod_type = "Mod".into();
        entry.download_count = downloads;
        entry
    }

    #[test]
    fn a_reread_with_the_same_digest_changes_nothing() {
        use super::unchanged;
        use crate::loadable::Loadable;
        let ready = Loadable::Ready(None);
        let failed = |error: &'static str| Loadable::Failed(error.into());
        assert!(unchanged((&ready, Some(1)), (&ready, Some(1))));
        assert!(!unchanged((&ready, Some(1)), (&ready, Some(2))));
        assert!(!unchanged((&Loadable::Loading, None), (&ready, Some(1))));
        assert!(unchanged(
            (&failed("offline"), None),
            (&failed("offline"), None)
        ));
        assert!(!unchanged((&failed("offline"), None), (&ready, Some(1))));
    }

    #[test]
    fn cards_come_from_the_manifest_most_downloaded_first() {
        let mut pack =
            crate::pages::search::logic::tests::modpack("Alpha", "1.21.1", ModLoader::Fabric);
        pack.mods = vec![installed("a", "Lithium", 10), installed("b", "Sodium", 99)];
        let cards = installed_cards(&pack);
        assert_eq!(cards[0].name, "Sodium");
        assert_eq!(cards[0].url, "https://modrinth.com/mod/sodium");
        assert!(cards[0].deleteable && !cards[0].downloadable);
        assert_eq!(cards[0].modpack.as_deref(), Some("Alpha"));
        assert_eq!(summary(&pack, 2), "Alpha | Fabric | 1.21.1 | 2 Mod(s)");
    }

    #[test]
    fn a_provider_record_keeps_the_cards_own_actions() {
        let card = installed_card(&installed("a", "Iris", 3), "Alpha");
        let mut record = card.clone();
        record.name = "Iris Shaders".into();
        record.download_count = 9_000_000;
        record.deleteable = false;
        record.modpack = Some(String::new());
        let shown = crate::pages::modpack_view::with_record(&card, Some(&record));
        assert_eq!(shown.name, "Iris Shaders");
        assert_eq!(shown.download_count, 9_000_000);
        assert!(shown.deleteable);
        assert_eq!(shown.modpack.as_deref(), Some("Alpha"));
    }

    #[test]
    fn an_update_needs_a_file() {
        let mut card = installed_card(&installed("a", "Iris", 0), "Alpha");
        assert!(Op::update("Alpha", &card).is_none());
        card.new_version = Some(UniversalModFile {
            id: None,
            file_name: "iris.jar".into(),
            download_url: "https://cdn.modrinth.com/iris.jar".into(),
            sha1: String::new(),
            sha512: None,
            size: 0,
        });
        assert!(matches!(
            Op::update("Alpha", &card),
            Some(Op::Update { .. })
        ));
    }

    /// Deletes and registers mods in a throwaway Minecraft folder through
    /// the same host calls the page makes.
    #[test]
    fn ops_change_the_applied_modpack() {
        let temp = tempfile::tempdir().unwrap();
        let mc = temp.path().join("mc");
        let mut options = QuadrantHostOptions::new(temp.path().join("data"), "", "", "");
        options.mc_folder = Some(mc.clone());
        let host = QuadrantHost::new(options).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(host.create_modpack("Alpha".into(), "1.21.1".into(), ModLoader::Fabric))
            .unwrap();
        let folder = mc.join("modpacks/Alpha");
        std::fs::write(folder.join("unknown.jar"), b"jar").unwrap();
        let ids = || -> Vec<String> {
            let modpacks = runtime.block_on(host.get_modpacks(false)).unwrap();
            let alpha = modpacks.iter().find(|m| m.name == "Alpha").unwrap();
            alpha.mods.iter().map(|m| m.id.clone()).collect()
        };

        runtime
            .block_on(
                Op::Register {
                    modpack: "Alpha".into(),
                    id: "fixture-unknown".into(),
                    source: ModSource::Online,
                    download_url: "https://example.com/files/unknown.jar".into(),
                }
                .perform(host.clone()),
            )
            .unwrap();
        assert_eq!(ids(), ["fixture-unknown"]);

        runtime
            .block_on(
                Op::Delete {
                    modpack: "Alpha".into(),
                    id: "fixture-unknown".into(),
                }
                .perform(host.clone()),
            )
            .unwrap();
        assert!(ids().is_empty());
        assert!(!folder.join("unknown.jar").exists());
    }
}
