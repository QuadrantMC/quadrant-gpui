//! The Settings page, after `SettingsPage` in the desktop app.
//!
//! Left out, because they configure the Tauri shell rather than Quadrant: the
//! update channel (the Tauri updater), the UI scale (webview zoom) and native
//! window decorations. The language picker is left out too: this app ships
//! only the English strings.

use std::path::PathBuf;

use gpui::{
    App, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, rgb,
};
use gpui_component::{h_flex, scroll::ScrollableElement as _, spinner::Spinner, v_flex};
use quadrant_host::QuadrantHost;
use serde_json::Value;

use crate::{
    backend::Backend,
    events,
    i18n::{describe, t, t_with},
    loadable::Loadable,
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

const PUBLIC_USAGE_URL: &str = "https://mrquantumoff.dev/projects/quadrant/analytics";
/// The runtime the version line names, as the desktop app names Tauri's.
const RUNTIME: (&str, &str) = ("GPUI", "0.3.9");

/// An on/off setting, in the order the page lists them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Toggle {
    CollectData,
    ClipIcons,
    CurseForge,
    Modrinth,
    AutoQuadrantSync,
    ModpackUpdateNotifications,
    SyncSettings,
    ShowUnupgradeableMods,
    ExperimentalFeatures,
    DevMode,
}

impl Toggle {
    pub const ALL: [Toggle; 10] = [
        Toggle::CollectData,
        Toggle::ClipIcons,
        Toggle::CurseForge,
        Toggle::Modrinth,
        Toggle::AutoQuadrantSync,
        Toggle::ModpackUpdateNotifications,
        Toggle::SyncSettings,
        Toggle::ShowUnupgradeableMods,
        Toggle::ExperimentalFeatures,
        Toggle::DevMode,
    ];

    /// The `config.json` key the desktop app keeps it under.
    pub fn key(self) -> &'static str {
        match self {
            Toggle::CollectData => "collectUserData",
            Toggle::ClipIcons => "clipIcons",
            Toggle::CurseForge => "curseforge",
            Toggle::Modrinth => "modrinth",
            Toggle::AutoQuadrantSync => "autoQuadrantSync",
            Toggle::ModpackUpdateNotifications => "showModpackUpdateNotifications",
            Toggle::SyncSettings => "syncSettings",
            Toggle::ShowUnupgradeableMods => "showUnupgradeableMods",
            Toggle::ExperimentalFeatures => "experimentalFeatures",
            Toggle::DevMode => "devMode",
        }
    }

    /// What the desktop app shows while the key is unset.
    fn default(self) -> bool {
        matches!(self, Toggle::ClipIcons | Toggle::ModpackUpdateNotifications)
    }

    fn label(self) -> SharedString {
        match self {
            Toggle::CurseForge => "CurseForge".into(),
            Toggle::Modrinth => "Modrinth".into(),
            Toggle::SyncSettings => t("quadrantSettingsSync"),
            Toggle::CollectData => t("collectData"),
            other => t(other.key()),
        }
    }
}

/// What the page shows, read from the host's config.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub mc_folder: String,
    /// `None` while Quadrant finds the Prism Launcher folder itself.
    pub prism_folder: Option<String>,
    /// One per [`Toggle::ALL`], in that order.
    pub toggles: [bool; 10],
}

impl Settings {
    pub fn read(host: &QuadrantHost) -> anyhow::Result<Self> {
        let mut toggles = [false; 10];
        for (slot, toggle) in toggles.iter_mut().zip(Toggle::ALL) {
            *slot = host
                .get_config_value(toggle.key())?
                .and_then(|value| value.as_bool())
                .unwrap_or(toggle.default());
        }
        Ok(Self {
            mc_folder: host.get_minecraft_folder()?.to_string_lossy().into_owned(),
            prism_folder: host
                .get_config_value("prismLauncherFolder")?
                .and_then(|value| value.as_str().map(str::to_string)),
            toggles,
        })
    }

    pub fn get(&self, toggle: Toggle) -> bool {
        let index = Toggle::ALL.iter().position(|each| *each == toggle);
        index.is_some_and(|index| self.toggles[index])
    }
}

/// A change to the settings. At most one runs at a time.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Set(Toggle, bool),
    /// A folder to use, or `None` for the platform's `.minecraft`.
    MinecraftFolder(Option<PathBuf>),
    /// A folder to use, or `None` to find it automatically.
    PrismFolder(Option<PathBuf>),
}

impl Op {
    pub async fn perform(self, host: QuadrantHost) -> anyhow::Result<()> {
        match self {
            Op::Set(toggle, on) => {
                host.set_config_value(toggle.key(), Value::Bool(on))?;
                // Like the desktop app: the choice is kept even when the
                // telemetry server can't be reached, and the failure reported.
                match (toggle, on) {
                    (Toggle::CollectData, true) => host.send_telemetry().await,
                    (Toggle::CollectData, false) => host.remove_telemetry().await,
                    _ => Ok(()),
                }
            }
            Op::MinecraftFolder(folder) => {
                let folder = match folder {
                    Some(folder) => folder,
                    None => quadrant_core::config::get_mc_folder()?.ok_or(anyhow::Error::from(
                        quadrant_host::ErrorCode::NoMinecraftFolder,
                    ))?,
                };
                host.set_config_value("mcFolder", folder.to_string_lossy().into())
            }
            Op::PrismFolder(Some(folder)) => {
                host.set_config_value("prismLauncherFolder", folder.to_string_lossy().into())
            }
            Op::PrismFolder(None) => host.remove_config_value("prismLauncherFolder"),
        }
    }
}

pub struct SettingsPage {
    settings: Loadable<Settings>,
    running: Option<Op>,
}

impl SettingsPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Settings sync can change keys behind the page's back.
        events::listen(&["configChanged"], cx, |this, _, cx| this.reload(cx));
        let mut page = Self {
            settings: Loadable::Idle,
            running: None,
        };
        page.reload(cx);
        page
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.settings, Loadable::Ready(_)) {
            self.settings = Loadable::Loading;
        }
        let task = Backend::global(cx).run(cx, |host| async move { Settings::read(&host) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.settings = Loadable::from_result(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn run(&mut self, op: Op, window: &mut Window, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        // Shown at once, as the desktop app's switches flip before saving.
        if let (Op::Set(toggle, on), Loadable::Ready(settings)) = (&op, &mut self.settings)
            && let Some(index) = Toggle::ALL.iter().position(|each| each == toggle)
        {
            settings.toggles[index] = *on;
        }
        self.running = Some(op.clone());
        let task = Backend::global(cx).run(cx, move |host| op.perform(host));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.running = None;
                if let Err(error) = result {
                    snackbar::show(cx, SnackbarKind::Error, describe(error));
                }
                this.reload(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Turns the experimental features on, so a screenshot shows the Prism
    /// Launcher folder row.
    #[cfg(feature = "screenshot")]
    pub fn preview_experimental(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(Op::Set(Toggle::ExperimentalFeatures, true), window, cx);
    }

    /// Asks for a folder, then runs the op `pick` makes of it.
    fn pick_folder(
        &mut self,
        title: SharedString,
        pick: fn(PathBuf) -> Op,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(title),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(folder) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| this.run(pick(folder), window, cx));
        })
        .detach();
    }

    fn open_own_usage(&mut self, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move {
            Ok(host
                .get_config_value("hardwareId")?
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default())
        });
        cx.spawn(async move |_, cx| {
            if let Ok(id) = task.await {
                cx.update(|cx| cx.open_url(&format!("{PUBLIC_USAGE_URL}/{id}")));
            }
        })
        .detach();
    }

    fn render_toggle(&self, toggle: Toggle, on: bool, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .my_4()
            .items_center()
            .font_weight(FontWeight::BOLD)
            .child(theme::setting_switch(toggle.key(), on).on_click(
                cx.listener(move |this, _, window, cx| this.run(Op::Set(toggle, !on), window, cx)),
            ))
            .child(div().ml_4().child(toggle.label()))
    }

    fn render_settings(&self, settings: &Settings, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle = |this: &Self, toggle: Toggle, cx: &mut Context<Self>| {
            this.render_toggle(toggle, settings.get(toggle), cx)
                .into_any_element()
        };
        let usage_button = |id: &'static str, tone, label: SharedString, cx: &mut Context<Self>| {
            theme::pill_button(id, tone, label, None, cx).px_2().my_4()
        };
        v_flex()
            .child(panel(t_with(
                "currentVersion",
                &[
                    ("version", env!("CARGO_PKG_VERSION")),
                    ("runtimeName", RUNTIME.0),
                    ("runtimeVersion", RUNTIME.1),
                ],
            )))
            .child(
                panel(settings.mc_folder.clone()).mt_4().child(
                    h_flex()
                        .w_full()
                        .gap_8()
                        .child(
                            theme::pill_button(
                                "reset-mc-folder",
                                Tone::Control,
                                t("resetMinecraftFolder"),
                                None,
                                cx,
                            )
                            .flex_1()
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.run(Op::MinecraftFolder(None), window, cx)
                                },
                            )),
                        )
                        .child(
                            theme::pill_button(
                                "override-mc-folder",
                                Tone::Control,
                                t("overrideMinecraftFolder"),
                                None,
                                cx,
                            )
                            .flex_1()
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.pick_folder(
                                        t("overrideMinecraftFolder"),
                                        |folder| Op::MinecraftFolder(Some(folder)),
                                        window,
                                        cx,
                                    )
                                },
                            )),
                        ),
                ),
            )
            .child(toggle(self, Toggle::CollectData, cx))
            .child(
                v_flex()
                    .items_start()
                    .child(
                        usage_button("collect-data", Tone::Neutral, t("collectData"), cx).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.run(Op::Set(Toggle::CollectData, true), window, cx)
                            }),
                        ),
                    )
                    .child(
                        usage_button(
                            "public-usage",
                            Tone::Neutral,
                            t_with("viewPublicUsage", &[("productName", &t("productName"))]),
                            cx,
                        )
                        .on_click(|_, _, cx| cx.open_url(PUBLIC_USAGE_URL)),
                    )
                    .child(
                        usage_button("own-usage", Tone::Neutral, t("viewYourUsageData"), cx)
                            .on_click(cx.listener(|this, _, _, cx| this.open_own_usage(cx))),
                    )
                    .child(
                        usage_button(
                            "delete-usage",
                            Tone::Destructive,
                            t("deleteYourUsageData"),
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run(Op::Set(Toggle::CollectData, false), window, cx)
                        })),
                    ),
            )
            .children(
                [
                    Toggle::ClipIcons,
                    Toggle::CurseForge,
                    Toggle::Modrinth,
                    Toggle::AutoQuadrantSync,
                    Toggle::ModpackUpdateNotifications,
                    Toggle::SyncSettings,
                    Toggle::ShowUnupgradeableMods,
                    Toggle::ExperimentalFeatures,
                ]
                .map(|each| toggle(self, each, cx)),
            )
            .when(settings.get(Toggle::ExperimentalFeatures), |this| {
                this.child(self.render_prism_folder(settings, cx))
            })
            .child(toggle(self, Toggle::DevMode, cx))
    }

    fn render_prism_folder(&self, settings: &Settings, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_4()
            .w_full()
            .p_4()
            .my_4()
            .bg(rgb(theme::SURFACE))
            .rounded(theme::radius_card())
            .child(
                div()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(t("prismLauncherFolder")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_4()
                    .py_2()
                    .bg(rgb(theme::CONTROL))
                    .rounded(theme::radius_card())
                    .child(
                        settings
                            .prism_folder
                            .clone()
                            .map(SharedString::from)
                            .unwrap_or_else(|| t("prismLauncherFolderAuto")),
                    ),
            )
            .child(
                theme::pill_button(
                    "choose-prism-folder",
                    Tone::Control,
                    t("choosePrismLauncherFolder"),
                    None,
                    cx,
                )
                .px_2()
                .on_click(cx.listener(|this, _, window, cx| {
                    this.pick_folder(
                        t("prismLauncherFolder"),
                        |folder| Op::PrismFolder(Some(folder)),
                        window,
                        cx,
                    )
                })),
            )
            .child(
                theme::pill_button(
                    "reset-prism-folder",
                    Tone::Control,
                    t("resetPrismLauncherFolder"),
                    None,
                    cx,
                )
                .px_2()
                .on_click(
                    cx.listener(|this, _, window, cx| this.run(Op::PrismFolder(None), window, cx)),
                ),
            )
    }
}

/// A `bg-slate-800` panel around a `bg-slate-700` line of text.
fn panel(text: impl Into<SharedString>) -> gpui::Div {
    v_flex()
        .w_full()
        .items_center()
        .p_4()
        .bg(rgb(theme::SURFACE))
        .rounded(theme::radius_card())
        .child(
            div()
                .my_2()
                .p_4()
                .bg(rgb(theme::CONTROL))
                .rounded(theme::radius_card())
                .font_weight(FontWeight::EXTRA_BOLD)
                .child(text.into()),
        )
}

impl Render for SettingsPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.settings {
            Loadable::Idle | Loadable::Loading => h_flex()
                .p_8()
                .justify_center()
                .child(Spinner::new())
                .into_any_element(),
            Loadable::Failed(error) => h_flex()
                .p_8()
                .justify_center()
                .text_color(rgb(theme::TEXT_MUTED))
                .child(error.clone())
                .into_any_element(),
            Loadable::Ready(settings) => self.render_settings(settings, cx).into_any_element(),
        };
        div()
            .id("settings")
            .size_full()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .mt_2()
                    .mx_8()
                    .pb_4()
                    .child(
                        div().my_4().flex().child(
                            div()
                                .p_3()
                                .rounded(theme::radius_card())
                                .bg(rgb(theme::SURFACE))
                                .text_2xl()
                                .font_weight(FontWeight::EXTRA_BOLD)
                                .child(t("someSettingsRequireReload")),
                        ),
                    )
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_host::QuadrantHostOptions;

    fn sandbox_host(root: &std::path::Path) -> QuadrantHost {
        let mut options = QuadrantHostOptions::new(root.join("data"), "", "", "");
        options.mc_folder = Some(root.join("mc"));
        options.keyring_service_name = "quadrant-gpui-tests".into();
        let host = QuadrantHost::new(options).unwrap();
        host.init_config().unwrap();
        host
    }

    #[test]
    fn every_toggle_has_its_own_key_and_a_desktop_label() {
        let mut keys: Vec<_> = Toggle::ALL.map(Toggle::key).to_vec();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), Toggle::ALL.len());
        for toggle in Toggle::ALL {
            let label = toggle.label();
            assert!(
                label != toggle.key() && label != "collectData",
                "{toggle:?} has no English label"
            );
        }
    }

    #[test]
    fn ops_persist_through_the_host_config() {
        let temp = tempfile::tempdir().unwrap();
        let host = sandbox_host(temp.path());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let run = |op: Op| runtime.block_on(op.perform(host.clone()));
        let read = || Settings::read(&host).unwrap();

        let before = read();
        assert!(before.get(Toggle::ClipIcons));
        assert!(!before.get(Toggle::ExperimentalFeatures));
        assert_eq!(before.prism_folder, None);

        run(Op::Set(Toggle::ExperimentalFeatures, true)).unwrap();
        run(Op::Set(Toggle::ClipIcons, false)).unwrap();
        assert!(read().get(Toggle::ExperimentalFeatures));
        assert!(!read().get(Toggle::ClipIcons));

        let elsewhere = temp.path().join("other-mc");
        run(Op::MinecraftFolder(Some(elsewhere.clone()))).unwrap();
        assert_eq!(read().mc_folder, elsewhere.to_string_lossy());
        assert_eq!(host.get_minecraft_folder().unwrap(), elsewhere);

        let prism = temp.path().join("prism");
        run(Op::PrismFolder(Some(prism.clone()))).unwrap();
        assert_eq!(read().prism_folder, Some(prism.to_string_lossy().into()));
        run(Op::PrismFolder(None)).unwrap();
        assert_eq!(read().prism_folder, None);

        let config = std::fs::read_to_string(temp.path().join("data/config.json")).unwrap();
        assert!(
            config.contains("\"experimentalFeatures\": true"),
            "{config}"
        );
    }
}
