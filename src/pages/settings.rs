//! The Settings page, after `SettingsPage` in the desktop app.
//!
//! The update channel shows while the self-updater runs (feature `updater`).
//! The language, UI scale, color theme, icon shape and window decorations
//! are [`Prefs`], which apply to the whole window at once.

use std::path::PathBuf;

use gpui::{
    App, AppContext as _, Context, Entity, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, rems,
};
use gpui_component::{
    IndexPath, h_flex,
    scroll::ScrollableElement as _,
    select::{Select, SelectEvent, SelectItem, SelectState},
    v_flex,
};
use quadrant_host::QuadrantHost;
use serde_json::Value;

use crate::{
    assets::MdIcon,
    autostart::Autostart,
    backend::Backend,
    color_theme::{self, ThemeId, Themes},
    config, events,
    i18n::{Locale, describe, t, t_with},
    loadable::Loadable,
    prefs::{self, Prefs},
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

const PUBLIC_USAGE_URL: &str = "https://mrquantumoff.dev/projects/quadrant/analytics";
/// The runtime the version line names, as the desktop app names Tauri's.
const RUNTIME: (&str, &str) = ("GPUI", "0.3.11");

/// An on/off setting, in the order the page lists them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Toggle {
    CollectData,
    ClipIcons,
    CurseForge,
    Modrinth,
    NativeDecorations,
    AutoQuadrantSync,
    ModpackUpdateNotifications,
    SyncSettings,
    ShowUnupgradeableMods,
    ExperimentalFeatures,
    DevMode,
}

impl Toggle {
    pub const ALL: [Toggle; 11] = [
        Toggle::CollectData,
        Toggle::ClipIcons,
        Toggle::CurseForge,
        Toggle::Modrinth,
        Toggle::NativeDecorations,
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
            Toggle::NativeDecorations => "nativeDecorations",
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
    pub toggles: [bool; Toggle::ALL.len()],
}

impl Settings {
    pub fn read(host: &QuadrantHost) -> anyhow::Result<Self> {
        let mut toggles = [false; Toggle::ALL.len()];
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
                config::set(&host, toggle.key(), Value::Bool(on))?;
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
                config::set(&host, "mcFolder", folder.to_string_lossy().into())
            }
            Op::PrismFolder(Some(folder)) => config::set(
                &host,
                "prismLauncherFolder",
                folder.to_string_lossy().into(),
            ),
            Op::PrismFolder(None) => config::unset(&host, "prismLauncherFolder"),
        }
    }
}

pub struct SettingsPage {
    settings: Loadable<Settings>,
    running: Option<Op>,
    /// Whether the login item is registered, read from the OS; `None`
    /// where this platform has none.
    autostart: Option<bool>,
    /// The language picker, made on the first render, which has the window.
    language: Option<Entity<SelectState<Vec<LanguageItem>>>>,
    _language_picked: Option<Subscription>,
    /// The color theme picker and the themes it lists, made like `language`.
    color_theme: Option<Entity<SelectState<Vec<ThemeItem>>>>,
    listed_themes: Vec<ThemeItem>,
    /// [`Themes`] changed since the picker last listed it.
    themes_changed: bool,
    _color_theme_picked: Option<Subscription>,
}

#[derive(Clone)]
struct LanguageItem(Locale);

impl SelectItem for LanguageItem {
    type Value = Locale;

    fn title(&self) -> SharedString {
        self.0.native_name().into()
    }

    fn value(&self) -> &Locale {
        &self.0
    }
}

#[derive(Clone, PartialEq)]
struct ThemeItem {
    id: ThemeId,
    name: SharedString,
}

impl SelectItem for ThemeItem {
    type Value = ThemeId;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &ThemeId {
        &self.id
    }
}

fn theme_items(cx: &App) -> Vec<ThemeItem> {
    Themes::global(cx)
        .all()
        .map(|theme| ThemeItem {
            id: theme.id.clone(),
            name: theme.name.clone(),
        })
        .collect()
}

/// Puts a fresh scan of the user's themes in place and shows the chosen
/// theme as it now reads.
fn rescanned(themes: Themes, cx: &mut App) {
    cx.set_global(themes);
    let id = Prefs::global(cx).color_theme.clone();
    prefs::show_color_theme(&id, cx);
}

impl SettingsPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Settings sync can change keys behind the page's back.
        events::listen_bursts(&["configChanged"], cx, |this, cx| this.reload(false, cx));
        // The UI scale changes from the zoom shortcuts too.
        cx.observe_global::<Prefs>(|_, cx| cx.notify()).detach();
        cx.observe_global::<Themes>(|this, cx| {
            this.themes_changed = true;
            cx.notify();
        })
        .detach();
        let mut page = Self {
            settings: Loadable::Idle,
            running: None,
            autostart: Autostart::global(cx).map(Autostart::is_enabled),
            language: None,
            _language_picked: None,
            color_theme: None,
            listed_themes: Vec::new(),
            themes_changed: false,
            _color_theme_picked: None,
        };
        page.reload(true, cx);
        page
    }

    /// Reads the settings again, now that the page is shown again, and the
    /// user's themes, which may have changed in their folder meanwhile.
    pub fn revalidate(&mut self, cx: &mut Context<Self>) {
        self.reload(true, cx);
    }

    /// Reads the settings, and with `rescan` the themes folder. Config
    /// changes don't rescan: no theme file is in the config.
    fn reload(&mut self, rescan: bool, cx: &mut Context<Self>) {
        self.settings.reload();
        let task = Backend::global(cx).run(cx, move |host| async move {
            let themes =
                rescan.then(|| Themes::scan(&color_theme::folder(&host.options().data_dir)));
            Ok((Settings::read(&host), themes))
        });
        cx.spawn(async move |this, cx| {
            let Ok((result, themes)) = task.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(themes) = themes {
                    rescanned(themes, cx);
                }
                this.settings = Loadable::from_result(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Makes the themes folder, with an example in it while it has no theme,
    /// reads it again and opens it in the file manager.
    fn open_themes_folder(&mut self, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move {
            let folder = color_theme::folder(&host.options().data_dir);
            color_theme::prepare_folder(&folder)?;
            Ok((Themes::scan(&folder), folder))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |_, cx| match result {
                Ok((themes, folder)) => {
                    rescanned(themes, cx);
                    cx.open_with_system(&folder);
                    cx.notify();
                }
                Err(error) => snackbar::show(cx, SnackbarKind::Error, describe(error)),
            });
        })
        .detach();
    }

    fn run(&mut self, op: Op, cx: &mut Context<Self>) {
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
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                if let Err(error) = result {
                    snackbar::show(cx, SnackbarKind::Error, describe(error));
                }
                // The icon shape and window decorations apply app-wide.
                Prefs::reload(cx);
                this.reload(false, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Asks for a folder, then runs the op `pick` makes of it.
    fn pick_folder(
        &mut self,
        title: SharedString,
        pick: fn(PathBuf) -> Op,
        cx: &mut Context<Self>,
    ) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(title),
        });
        cx.spawn(
            async move |this, cx| match super::dialog_choice(chosen.await) {
                Ok(Some(paths)) => {
                    if let Some(folder) = paths.into_iter().next() {
                        let _ = this.update(cx, |this, cx| this.run(pick(folder), cx));
                    }
                }
                Ok(None) => {}
                Err(error) => cx.update(|cx| {
                    snackbar::show(cx, SnackbarKind::Error, describe(error));
                }),
            },
        )
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

    /// Registers or removes the login item, then shows what the OS says.
    fn set_autostart(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(autostart) = Autostart::global(cx) else {
            return;
        };
        let result = autostart.set(on);
        self.autostart = Some(autostart.is_enabled());
        if let Err(error) = result {
            snackbar::show(cx, SnackbarKind::Error, describe(error));
        }
        cx.notify();
    }

    fn render_autostart(&self, on: bool, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .my_4()
            .items_center()
            .font_weight(FontWeight::BOLD)
            .child(
                theme::setting_switch("launchAtLogin", on)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_autostart(!on, cx))),
            )
            .child(div().ml_4().child(t("launchAtLogin")))
    }

    fn language_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<Vec<LanguageItem>>> {
        let current = Prefs::global(cx).locale;
        if let Some(select) = self.language.clone() {
            // Kept in step with a language set elsewhere.
            if select.read(cx).selected_value() != Some(&current) {
                select.update(cx, |select, cx| {
                    select.set_selected_value(&current, window, cx)
                });
            }
            return select;
        }
        let row = Locale::ALL.iter().position(|each| *each == current);
        let select = cx.new(|cx| {
            SelectState::new(
                Locale::ALL.map(LanguageItem).to_vec(),
                row.map(|row| IndexPath::default().row(row)),
                window,
                cx,
            )
        });
        self._language_picked = Some(cx.subscribe(
            &select,
            |_, _, event: &SelectEvent<Vec<LanguageItem>>, cx| {
                if let SelectEvent::Confirm(Some(locale)) = event {
                    Prefs::set_locale(*locale, cx);
                }
            },
        ));
        self.language = Some(select.clone());
        select
    }

    fn color_theme_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<Vec<ThemeItem>>> {
        let current = Themes::global(cx)
            .resolve(&Prefs::global(cx).color_theme)
            .id
            .clone();
        if let Some(select) = self.color_theme.clone() {
            // A rescan changes the list, settings sync the choice.
            let relisted = std::mem::take(&mut self.themes_changed)
                .then(|| theme_items(cx))
                .filter(|items| *items != self.listed_themes);
            if let Some(items) = &relisted {
                self.listed_themes = items.clone();
            }
            select.update(cx, |select, cx| {
                if let Some(items) = relisted {
                    select.set_items(items, window, cx);
                }
                if select.selected_value() != Some(&current) {
                    select.set_selected_value(&current, window, cx);
                }
            });
            return select;
        }
        let items = theme_items(cx);
        let row = items.iter().position(|item| item.id == current);
        let select = cx.new(|cx| {
            SelectState::new(
                items.clone(),
                row.map(|row| IndexPath::default().row(row)),
                window,
                cx,
            )
        });
        self._color_theme_picked = Some(cx.subscribe(
            &select,
            |_, _, event: &SelectEvent<Vec<ThemeItem>>, cx| {
                if let SelectEvent::Confirm(Some(id)) = event {
                    Prefs::set_color_theme(id.clone(), cx);
                }
            },
        ));
        self.color_theme = Some(select.clone());
        self.listed_themes = items;
        select
    }

    /// The color theme picker, the language picker's twin, with the button
    /// to the user's themes beside it and their broken files under it.
    fn render_color_theme(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let select = self.color_theme_select(window, cx);
        let broken: Vec<_> = Themes::global(cx)
            .broken
            .iter()
            .map(|broken| {
                theme::warning_note(t_with(
                    "brokenColorTheme",
                    &[("file", &broken.file), ("error", &broken.error)],
                ))
            })
            .collect();
        v_flex()
            .my_4()
            .font_weight(FontWeight::BOLD)
            .child(t("colorTheme"))
            .child(
                h_flex()
                    .my_4()
                    .gap_4()
                    .flex_wrap()
                    .items_center()
                    .child(
                        div().w_full().max_w(rems(20.)).child(
                            Select::new(&select)
                                .appearance(false)
                                .h_10()
                                .rounded_full()
                                .bg(theme::palette().control),
                        ),
                    )
                    .child(
                        theme::pill_button(
                            "open-themes-folder",
                            Tone::Control,
                            t("openThemesFolder"),
                            Some(MdIcon::Folder),
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.open_themes_folder(cx))),
                    ),
            )
            .when(!broken.is_empty(), |this| {
                this.child(v_flex().gap_2().max_w(rems(40.)).children(broken))
            })
    }

    /// The language picker, as the desktop app's: a bold label over a
    /// `max-w-xs` select.
    fn render_language(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let select = self.language_select(window, cx);
        v_flex()
            .my_4()
            .font_weight(FontWeight::BOLD)
            .child(t("language"))
            .child(
                div().my_4().w_full().max_w(rems(20.)).child(
                    Select::new(&select)
                        .appearance(false)
                        .h_10()
                        .rounded_full()
                        .bg(theme::palette().control),
                ),
            )
    }

    /// The UI scale: a bold label over minus, the percent and plus.
    fn render_ui_scale(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let scale = i64::from(Prefs::global(cx).ui_scale);
        let step = i64::from(prefs::UI_SCALE_STEP);
        v_flex()
            .my_4()
            .font_weight(FontWeight::BOLD)
            .child(t("uiScale"))
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .my_4()
                    .gap_2()
                    .child(
                        theme::icon_button("ui-scale-down", Tone::Control, MdIcon::Remove, cx)
                            .on_click(move |_, _, cx| Prefs::set_ui_scale(scale - step, cx)),
                    )
                    .child(
                        div()
                            .w_20()
                            .px_4()
                            .py_2()
                            .bg(theme::palette().surface)
                            .rounded(theme::radius_card())
                            .text_center()
                            .child(format!("{scale}%")),
                    )
                    .child(
                        theme::icon_button("ui-scale-up", Tone::Control, MdIcon::Add, cx)
                            .on_click(move |_, _, cx| Prefs::set_ui_scale(scale + step, cx)),
                    ),
            )
    }

    fn render_toggle(&self, toggle: Toggle, on: bool, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .my_4()
            .items_center()
            .font_weight(FontWeight::BOLD)
            .child(
                theme::setting_switch(toggle.key(), on).on_click(
                    cx.listener(move |this, _, _, cx| this.run(Op::Set(toggle, !on), cx)),
                ),
            )
            .child(div().ml_4().child(toggle.label()))
    }

    fn render_settings(
        &mut self,
        settings: &Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
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
            .children(update_channel(window, cx))
            .child(self.render_language(window, cx))
            .child(self.render_color_theme(window, cx))
            .child(self.render_ui_scale(cx))
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
                            .on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.run(Op::MinecraftFolder(None), cx)
                                }),
                            ),
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pick_folder(
                                    t("overrideMinecraftFolder"),
                                    |folder| Op::MinecraftFolder(Some(folder)),
                                    cx,
                                )
                            })),
                        ),
                ),
            )
            .child(toggle(self, Toggle::CollectData, cx))
            .child(
                v_flex()
                    .items_start()
                    .child(
                        usage_button("collect-data", Tone::Neutral, t("collectData"), cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.run(Op::Set(Toggle::CollectData, true), cx)
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
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.run(Op::Set(Toggle::CollectData, false), cx)
                        })),
                    ),
            )
            .children(
                [
                    Toggle::ClipIcons,
                    Toggle::CurseForge,
                    Toggle::Modrinth,
                    Toggle::NativeDecorations,
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
            .children(self.autostart.map(|on| self.render_autostart(on, cx)))
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
            .bg(theme::palette().surface)
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
                    .bg(theme::palette().control)
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
                .on_click(cx.listener(|this, _, _, cx| {
                    this.pick_folder(
                        t("prismLauncherFolder"),
                        |folder| Op::PrismFolder(Some(folder)),
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
                .on_click(cx.listener(|this, _, _, cx| this.run(Op::PrismFolder(None), cx))),
            )
    }
}

/// The self-updater's channel picker, while it runs.
fn update_channel(window: &mut Window, cx: &mut App) -> Option<gpui::AnyElement> {
    #[cfg(feature = "updater")]
    return crate::updater::channel_setting(window, cx);
    #[cfg(not(feature = "updater"))]
    {
        let _ = (window, cx);
        None
    }
}

/// A `bg-slate-800` panel around a `bg-slate-700` line of text.
fn panel(text: impl Into<SharedString>) -> gpui::Div {
    v_flex()
        .w_full()
        .items_center()
        .p_4()
        .bg(theme::palette().surface)
        .rounded(theme::radius_card())
        .child(
            div()
                .my_2()
                .p_4()
                .bg(theme::palette().control)
                .rounded(theme::radius_card())
                .font_weight(FontWeight::EXTRA_BOLD)
                .child(text.into()),
        )
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.settings {
            Loadable::Idle | Loadable::Loading => h_flex()
                .p_8()
                .justify_center()
                .child(theme::circular_progress("progress"))
                .into_any_element(),
            Loadable::Failed(error) => h_flex()
                .p_8()
                .justify_center()
                .text_color(theme::palette().text_muted)
                .child(error.clone())
                .into_any_element(),
            Loadable::Ready(settings) => {
                let settings = settings.clone();
                self.render_settings(&settings, window, cx)
                    .into_any_element()
            }
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
                                .bg(theme::palette().surface)
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

    #[test]
    fn runtime_names_the_locked_gpui() {
        let lock = include_str!("../../Cargo.lock");
        let version = lock
            .split("[[package]]")
            .find(|entry| {
                entry
                    .lines()
                    .any(|line| line.trim_end() == r#"name = "gpui-pre""#)
            })
            .and_then(|entry| {
                entry
                    .lines()
                    .find_map(|line| line.strip_prefix("version = "))
            })
            .expect("gpui-pre in Cargo.lock");
        assert_eq!(version.trim_matches('"'), RUNTIME.1);
    }

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
