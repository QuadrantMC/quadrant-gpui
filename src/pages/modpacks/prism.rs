//! The Prism Launcher menu on a local modpack card, after `PrismInstanceMenu`
//! in the desktop app: apply the modpack to an instance, or unlink an
//! instance it is applied to. The host lists no instances unless experimental
//! features are on, and without instances the card shows neither the menu nor
//! the "Applied to" line.

use std::collections::HashMap;

use gpui::{
    Anchor, AnyElement, App, Context, ElementId, Entity, EventEmitter, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    Task, Window, div, prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{Icon, h_flex, popover::Popover};
use quadrant_core::prism::{PrismInstance, PrismSyncPlan};
use quadrant_host::QuadrantHost;

use crate::{
    assets::MdIcon,
    backend::Backend,
    i18n::{describe, t, t_with},
    loadable::Loadable,
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

/// One change to one instance. Both carry the modpack whose menu it came
/// from, so only that card's button shows it running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    Apply { modpack: String, instance: String },
    Detach { modpack: String, instance: String },
}

impl Op {
    /// What picking `instance` in `modpack`'s menu does: unlink it when it
    /// runs this modpack, apply the modpack to it otherwise.
    pub fn toggle(modpack: &str, instance: &PrismInstance) -> Self {
        let (modpack, id) = (modpack.to_string(), instance.id.clone());
        if is_linked(instance, &modpack) {
            Op::Detach {
                modpack,
                instance: id,
            }
        } else {
            Op::Apply {
                modpack,
                instance: id,
            }
        }
    }

    pub async fn perform(self, host: QuadrantHost) -> anyhow::Result<()> {
        match self {
            Op::Apply { modpack, instance } => {
                host.apply_modpack_to_prism_instance(modpack, instance)
                    .await
            }
            Op::Detach { instance, .. } => host.detach_prism_instance(instance),
        }
    }

    fn modpack(&self) -> &str {
        match self {
            Op::Apply { modpack, .. } | Op::Detach { modpack, .. } => modpack,
        }
    }

    fn done_text(&self) -> SharedString {
        match self {
            Op::Apply { .. } => t("prismApplySuccess"),
            Op::Detach { .. } => t("prismDetachSuccess"),
        }
    }
}

pub fn is_linked(instance: &PrismInstance, modpack: &str) -> bool {
    instance.applied_modpack.as_deref() == Some(modpack)
}

/// The line under a card naming the instances that run its modpack.
pub fn applied_to(instances: &[PrismInstance], modpack: &str) -> Option<SharedString> {
    let names: Vec<&str> = instances
        .iter()
        .filter(|instance| is_linked(instance, modpack))
        .map(|instance| instance.name.as_str())
        .collect();
    (!names.is_empty()).then(|| t_with("prismAppliedTo", &[("instances", &names.join(", "))]))
}

/// An instance's Minecraft version and loader, under its name in the menu.
pub fn summary(instance: &PrismInstance) -> String {
    let version = instance
        .minecraft_version
        .clone()
        .unwrap_or_else(|| t("unknown").to_string());
    format!("{version} | {}", instance.mod_loader)
}

/// What applying would move an unlinked instance to, from the host's plan
/// for it. Nothing for a linked instance, one without a plan, or one already
/// on the modpack's version and loader.
pub fn switch_hint(
    instance: &PrismInstance,
    modpack: &str,
    plans: &[PrismSyncPlan],
) -> Option<SharedString> {
    if is_linked(instance, modpack) {
        return None;
    }
    let plan = plans.iter().find(|plan| plan.instance_id == instance.id)?;
    let parts: Vec<String> = plan
        .minecraft_version
        .iter()
        .cloned()
        .chain(plan.mod_loader.map(|loader| loader.to_string()))
        .collect();
    (!parts.is_empty())
        .then(|| t_with("prismInstanceSwitchHint", &[("target", &parts.join(" · "))]))
}

/// Emitted after an op succeeds, so the page reloads its modpacks and,
/// through them, the instances.
pub struct Changed;

/// The instances every card's menu offers, and the menu that is open. One
/// per Apply page.
pub struct PrismMenus {
    instances: Loadable<Vec<PrismInstance>>,
    /// By modpack name, asked for when its menu first opens. Applying
    /// changes the instances, so they are forgotten whenever those reload.
    plans: HashMap<String, Loadable<Vec<PrismSyncPlan>>>,
    /// The modpack whose menu is open.
    open: Option<String>,
    running: Option<Op>,
    listing: Task<()>,
    /// Plan requests in flight; a reload drops them with the plans.
    planning: Vec<Task<()>>,
}

impl EventEmitter<Changed> for PrismMenus {}

impl PrismMenus {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut menus = Self {
            instances: Loadable::Idle,
            plans: HashMap::new(),
            open: None,
            running: None,
            listing: Task::ready(()),
            planning: Vec::new(),
        };
        menus.refresh(cx);
        menus
    }

    fn instances(&self) -> &[PrismInstance] {
        match &self.instances {
            Loadable::Ready(instances) => instances,
            Loadable::Idle | Loadable::Loading | Loadable::Failed(_) => &[],
        }
    }

    /// Lists the instances again. The list only changes outside the folders
    /// Quadrant watches, so the page calls this when it reloads its modpacks.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.instances.reload();
        self.plans.clear();
        self.planning.clear();
        let task = Backend::global(cx).run(cx, |host| async move { host.get_prism_instances() });
        self.listing = cx.spawn(async move |this, cx| {
            let result = task.await;
            if let Err(error) = &result {
                // Never keep offering instances the host can no longer list.
                log::warn!("Listing Prism Launcher instances failed: {error:#}");
            }
            let _ = this.update(cx, |this, cx| {
                this.instances = Loadable::from_result(result);
                if let Some(modpack) = this.open.clone() {
                    this.load_plans(modpack, cx);
                }
                cx.notify();
            });
        });
    }

    fn load_plans(&mut self, modpack: String, cx: &mut Context<Self>) {
        self.plans.insert(modpack.clone(), Loadable::Loading);
        let task = Backend::global(cx).run(cx, {
            let modpack = modpack.clone();
            move |host| async move { host.get_prism_sync_plans(modpack).await }
        });
        self.planning.push(cx.spawn(async move |this, cx| {
            let result = task.await;
            if let Err(error) = &result {
                // The hint is an extra; losing it must not cost the menu.
                log::warn!("Planning Prism Launcher changes for \"{modpack}\" failed: {error:#}");
            }
            let _ = this.update(cx, |this, cx| {
                this.plans.insert(modpack, Loadable::from_result(result));
                cx.notify();
            });
        }));
    }

    /// Opens `modpack`'s menu, or closes the open one with `None`.
    pub fn set_open(&mut self, modpack: Option<String>, cx: &mut Context<Self>) {
        if let Some(modpack) = &modpack
            && !self.plans.contains_key(modpack)
        {
            self.load_plans(modpack.clone(), cx);
        }
        self.open = modpack;
        cx.notify();
    }

    fn run(&mut self, op: Op, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        self.open = None;
        self.running = Some(op.clone());
        let task = Backend::global(cx).run(cx, {
            let op = op.clone();
            move |host| op.perform(host)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                match result {
                    Ok(()) => {
                        cx.emit(Changed);
                        snackbar::show(cx, SnackbarKind::Success, op.done_text());
                    }
                    Err(error) => snackbar::show(cx, SnackbarKind::Error, describe(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The "Applied to Prism Launcher" line under a card's metadata.
    pub fn applied_line(&self, modpack: &str) -> Option<impl IntoElement + use<>> {
        applied_to(self.instances(), modpack)
            .map(|text| div().text_color(theme::palette().text_muted).child(text))
    }

    /// The card's "Prism Launcher" button with its menu, after Apply.
    pub fn menu(this: &Entity<Self>, modpack: &str, cx: &App) -> Option<AnyElement> {
        let menus = this.read(cx);
        if menus.instances().is_empty() {
            return None;
        }
        let open = menus.open.as_deref() == Some(modpack);
        let running_here = menus
            .running
            .as_ref()
            .is_some_and(|op| op.modpack() == modpack);
        let button = theme::pill_button(
            element_id("prism-button", modpack),
            Tone::Neutral,
            t("prismLauncher"),
            Some(MdIcon::ViewList),
            cx,
        )
        .loading(running_here);
        let on_open_change = {
            let this = this.clone();
            let modpack = modpack.to_string();
            move |open: &bool, _: &mut Window, cx: &mut App| {
                let modpack = open.then(|| modpack.clone());
                this.update(cx, |menus, cx| menus.set_open(modpack, cx));
            }
        };
        let content = {
            let this = this.clone();
            let modpack = modpack.to_string();
            move |_: &mut _, _: &mut Window, cx: &mut Context<_>| render_items(&this, &modpack, cx)
        };
        let popover = Popover::new(element_id("prism-menu", modpack))
            .anchor(Anchor::TopLeft)
            .offset(px(8.))
            .open(open)
            .on_open_change(on_open_change)
            .trigger(button)
            .content(content);
        Some(theme::menu_panel(popover).into_any_element())
    }
}

fn element_id(kind: &str, modpack: &str) -> ElementId {
    ElementId::Name(format!("{kind}:{modpack}").into())
}

fn render_items(this: &Entity<PrismMenus>, modpack: &str, cx: &App) -> impl IntoElement + use<> {
    let menus = this.read(cx);
    let plans = match menus.plans.get(modpack) {
        Some(Loadable::Ready(plans)) => plans.as_slice(),
        Some(Loadable::Idle | Loadable::Loading | Loadable::Failed(_)) | None => &[],
    };
    let busy = menus.running.is_some();
    let items: Vec<_> = menus
        .instances()
        .iter()
        .map(|instance| {
            let op = Op::toggle(modpack, instance);
            let this = this.clone();
            theme::menu_item(element_id("prism-instance", &instance.id), busy)
                .flex()
                .flex_col()
                .items_start()
                .when(!busy, |item| {
                    item.on_click(move |_, _, cx| {
                        this.update(cx, |menus, cx| menus.run(op.clone(), cx))
                    })
                })
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(instance.name.clone())
                        .when(is_linked(instance, modpack), |row| {
                            row.child(Icon::new(MdIcon::Check).size_5().ml_auto().flex_none())
                        }),
                )
                .child(muted(summary(instance)))
                .children(switch_hint(instance, modpack, plans).map(muted))
        })
        .collect();
    div()
        .id("prism-instances")
        .flex()
        .flex_col()
        .max_h(rems(20.))
        .overflow_y_scroll()
        .children(items)
}

fn muted(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_color(theme::palette().text_muted)
        .child(text.into())
}

#[cfg(feature = "screenshot")]
impl super::ModpacksPage {
    /// Opens `modpack`'s Prism Launcher menu, as a click on its button does,
    /// or closes the open one.
    pub fn preview_prism_menu(&mut self, modpack: Option<&str>, cx: &mut Context<Self>) {
        let modpack = modpack.map(str::to_string);
        self.prism
            .update(cx, |menus, cx| menus.set_open(modpack, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_core::models::ModLoader;

    fn instance(id: &str, version: Option<&str>, applied: Option<&str>) -> PrismInstance {
        PrismInstance {
            id: id.into(),
            name: format!("{id} instance"),
            minecraft_version: version.map(Into::into),
            mod_loader: ModLoader::Unknown,
            mod_loader_version: None,
            applied_modpack: applied.map(Into::into),
        }
    }

    fn plan(id: &str, version: Option<&str>, loader: Option<ModLoader>) -> PrismSyncPlan {
        PrismSyncPlan {
            instance_id: id.into(),
            minecraft_version: version.map(Into::into),
            mod_loader: loader,
        }
    }

    #[test]
    fn the_card_names_the_instances_running_its_modpack() {
        let instances = [
            instance("a", None, Some("Alpha")),
            instance("b", None, Some("Beta")),
            instance("c", None, Some("Alpha")),
        ];
        assert_eq!(
            applied_to(&instances, "Alpha").unwrap(),
            "Applied to Prism Launcher: a instance, c instance"
        );
        assert_eq!(applied_to(&instances, "Gamma"), None);
    }

    #[test]
    fn picking_a_linked_instance_unlinks_it() {
        let linked = instance("a", None, Some("Alpha"));
        let elsewhere = instance("b", None, Some("Beta"));
        assert_eq!(
            Op::toggle("Alpha", &linked),
            Op::Detach {
                modpack: "Alpha".into(),
                instance: "a".into()
            }
        );
        assert_eq!(
            Op::toggle("Alpha", &elsewhere),
            Op::Apply {
                modpack: "Alpha".into(),
                instance: "b".into()
            }
        );
    }

    #[test]
    fn the_summary_says_unknown_for_a_missing_version() {
        let mut fabric = instance("a", Some("1.21.1"), None);
        fabric.mod_loader = ModLoader::Fabric;
        assert_eq!(summary(&fabric), "1.21.1 | Fabric");
        assert_eq!(summary(&instance("b", None, None)), "Unknown | Unknown");
    }

    #[test]
    fn the_hint_names_what_applying_would_switch() {
        let plans = [
            plan("a", Some("1.20.1"), Some(ModLoader::Forge)),
            plan("b", None, Some(ModLoader::Fabric)),
            plan("c", None, None),
        ];
        let hint = |instance: &PrismInstance| switch_hint(instance, "Alpha", &plans);
        assert_eq!(
            hint(&instance("a", None, None)).unwrap(),
            "Will be switched to 1.20.1 · Forge"
        );
        assert_eq!(
            hint(&instance("b", None, None)).unwrap(),
            "Will be switched to Fabric"
        );
        assert_eq!(hint(&instance("c", None, None)), None, "nothing to switch");
        assert_eq!(hint(&instance("d", None, None)), None, "no plan");
        assert_eq!(
            hint(&instance("a", None, Some("Alpha"))),
            None,
            "already linked"
        );
    }

    fn write_instance(prism: &std::path::Path, id: &str, components: &str) -> std::path::PathBuf {
        let dir = prism.join("instances").join(id);
        std::fs::create_dir_all(dir.join("minecraft")).unwrap();
        std::fs::write(dir.join("instance.cfg"), format!("name={id}\n")).unwrap();
        let pack = format!(r#"{{"formatVersion":1,"components":[{components}]}}"#);
        std::fs::write(dir.join("mmc-pack.json"), pack).unwrap();
        dir
    }

    /// Applies a modpack to a throwaway Prism Launcher instance and unlinks
    /// it again through the host calls the menu makes. The instance already
    /// runs the modpack's loader, so applying never asks Prism's metadata
    /// server for a loader build.
    #[test]
    fn ops_link_and_unlink_an_instance() {
        let temp = tempfile::tempdir().unwrap();
        let mc = temp.path().join("mc");
        // Never the real Prism Launcher folder.
        let prism = temp.path().join("prism");
        let fabric = r#"{"uid":"net.minecraft","version":"1.21.1"},{"uid":"net.fabricmc.fabric-loader","version":"0.16.5"}"#;
        let survival = write_instance(&prism, "survival", fabric);
        std::fs::create_dir_all(survival.join("minecraft/mods")).unwrap();
        std::fs::write(survival.join("minecraft/mods/own.jar"), "own").unwrap();
        write_instance(
            &prism,
            "old",
            r#"{"uid":"net.minecraft","version":"1.20.1"}"#,
        );
        let mut options =
            quadrant_host::QuadrantHostOptions::new(temp.path().join("data"), "", "", "");
        options.mc_folder = Some(mc.clone());
        options.keyring_service_name = "quadrant-gpui-tests".into();
        let host = QuadrantHost::new(options).unwrap();
        host.init_config().unwrap();
        crate::config::set(&host, "prismLauncherFolder", prism.to_string_lossy().into()).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(host.create_modpack("Alpha".into(), "1.21.1".into(), ModLoader::Fabric))
            .unwrap();
        let run = |op: Op| runtime.block_on(op.perform(host.clone()));
        let apply = Op::Apply {
            modpack: "Alpha".into(),
            instance: "survival".into(),
        };

        assert!(
            host.get_prism_instances().unwrap().is_empty(),
            "off without experimental features"
        );
        assert!(
            run(apply.clone()).is_err(),
            "refused without experimental features"
        );
        crate::config::set(&host, "experimentalFeatures", true.into()).unwrap();

        let plans = runtime
            .block_on(host.get_prism_sync_plans("Alpha".into()))
            .unwrap();
        let hints: Vec<_> = host
            .get_prism_instances()
            .unwrap()
            .iter()
            .map(|instance| switch_hint(instance, "Alpha", &plans))
            .collect();
        assert_eq!(
            hints,
            [Some("Will be switched to 1.21.1 · Fabric".into()), None],
            "old needs both, survival is already on 1.21.1 Fabric"
        );

        run(apply).unwrap();
        let instances = host.get_prism_instances().unwrap();
        assert_eq!(
            applied_to(&instances, "Alpha").unwrap(),
            "Applied to Prism Launcher: survival"
        );
        let mods = survival.join("minecraft/mods");
        assert_eq!(
            std::fs::read_link(&mods).unwrap(),
            mc.join("modpacks").join("Alpha")
        );

        let survival_instance = instances.iter().find(|each| each.id == "survival").unwrap();
        run(Op::toggle("Alpha", survival_instance)).unwrap();
        assert_eq!(
            applied_to(&host.get_prism_instances().unwrap(), "Alpha"),
            None
        );
        assert!(!mods.is_symlink());
        assert!(mods.join("own.jar").is_file(), "its own mods come back");
    }
}
