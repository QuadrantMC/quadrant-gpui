//! The settings that change how the whole window looks: the language, the
//! UI scale, round or rounded-square mod icons and the OS title bar.
//!
//! [`Prefs`] is a gpui global read from the config before the window opens.
//! Setters apply a change at once and save it in the background; a reload
//! re-reads the config after the settings page or settings sync changed it.
//! Views that keep a string or a shape across frames observe the global.

use gpui::{App, Global};
use quadrant_host::QuadrantHost;
use serde_json::Value;

use crate::{
    backend::Backend,
    config,
    i18n::{self, Locale},
    theme,
};

const LOCALE_KEY: &str = "locale";
const UI_SCALE_KEY: &str = "uiScale";
const CLIP_ICONS_KEY: &str = "clipIcons";
const NATIVE_DECORATIONS_KEY: &str = "nativeDecorations";

/// The desktop app's `uiScale.ts`: percent of the 16.8px root font size.
pub const DEFAULT_UI_SCALE: u32 = 100;
pub const MIN_UI_SCALE: u32 = 50;
pub const MAX_UI_SCALE: u32 = 200;
pub const UI_SCALE_STEP: u32 = 10;

/// `clampUiScale`: anything but a finite number is the default; numbers are
/// rounded into range.
pub fn clamp_ui_scale(scale: Option<f64>) -> u32 {
    match scale {
        Some(scale) if scale.is_finite() => scale
            .round()
            .clamp(f64::from(MIN_UI_SCALE), f64::from(MAX_UI_SCALE))
            as u32,
        _ => DEFAULT_UI_SCALE,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    pub locale: Locale,
    /// Percent, within [`MIN_UI_SCALE`]..=[`MAX_UI_SCALE`].
    pub ui_scale: u32,
    /// Round mod icons; rounded squares when off.
    pub clip_icons: bool,
    /// The OS draws the title bar and window buttons.
    pub native_decorations: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            locale: Locale::English,
            ui_scale: DEFAULT_UI_SCALE,
            clip_icons: true,
            native_decorations: false,
        }
    }
}

impl Global for Prefs {}

impl Prefs {
    /// Each key as the desktop app reads it, with its defaults.
    pub fn read(host: &QuadrantHost) -> Self {
        let value = |key: &str| host.get_config_value(key).ok().flatten();
        let defaults = Self::default();
        Self {
            locale: Locale::parse(value(LOCALE_KEY).as_ref().and_then(Value::as_str)),
            ui_scale: clamp_ui_scale(value(UI_SCALE_KEY).as_ref().and_then(Value::as_f64)),
            clip_icons: value(CLIP_ICONS_KEY)
                .and_then(|value| value.as_bool())
                .unwrap_or(defaults.clip_icons),
            native_decorations: value(NATIVE_DECORATIONS_KEY)
                .and_then(|value| value.as_bool())
                .unwrap_or(defaults.native_decorations),
        }
    }

    pub fn global(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }

    /// Applies `self` and keeps following settings sync. Before the window
    /// opens, so it opens in the right language, scale and decorations.
    pub fn install(self, cx: &mut App) {
        self.apply(cx);
        let mut events = Backend::global(cx).subscribe_events();
        cx.spawn(async move |cx| {
            use tokio::sync::broadcast::error::RecvError;
            loop {
                match events.recv().await {
                    Ok(event) if event.event == "configChanged" => {
                        cx.update(Self::reload);
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        })
        .detach();
    }

    /// Shows `self` now, without saving it.
    pub fn apply(self, cx: &mut App) {
        let old = cx.try_global::<Self>().copied();
        if old.map(|old| old.locale) != Some(self.locale) {
            i18n::set_locale(self.locale);
        }
        if old.map(|old| old.ui_scale) != Some(self.ui_scale) {
            theme::set_ui_scale(self.ui_scale, cx);
        }
        // Only Linux can swap a window's decorations once it is open; elsewhere
        // the next window, at the next start, has them.
        #[cfg(target_os = "linux")]
        if old.is_some_and(|old| old.native_decorations != self.native_decorations) {
            for window in cx.windows() {
                let _ = window.update(cx, |_, window, _| {
                    window.request_decorations(window_decorations(self))
                });
            }
        }
        if old != Some(self) {
            cx.set_global(self);
            cx.refresh_windows();
        }
    }

    /// Re-reads the config, after something other than these setters
    /// changed it.
    pub fn reload(cx: &mut App) {
        let task = Backend::global(cx).run(cx, |host| async move { Ok(Self::read(&host)) });
        cx.spawn(async move |cx| {
            if let Ok(prefs) = task.await {
                cx.update(|cx| prefs.apply(cx));
            }
        })
        .detach();
    }

    pub fn set_locale(locale: Locale, cx: &mut App) {
        Self {
            locale,
            ..Self::global(cx)
        }
        .apply(cx);
        save(cx, LOCALE_KEY, locale.code().into());
    }

    /// `setUiScale`: clamps, applies, saves.
    pub fn set_ui_scale(scale: i64, cx: &mut App) {
        let ui_scale = clamp_ui_scale(Some(scale as f64));
        Self {
            ui_scale,
            ..Self::global(cx)
        }
        .apply(cx);
        save(cx, UI_SCALE_KEY, ui_scale.into());
    }

    /// One [`UI_SCALE_STEP`] up for a positive `direction`, down for a
    /// negative one, as the desktop app's zoom shortcuts.
    pub fn zoom(direction: i64, cx: &mut App) {
        let scale = i64::from(Self::global(cx).ui_scale);
        Self::set_ui_scale(scale + direction * i64::from(UI_SCALE_STEP), cx);
    }
}

/// What the window asks the platform for.
pub fn window_decorations(prefs: Prefs) -> gpui::WindowDecorations {
    if prefs.native_decorations {
        gpui::WindowDecorations::Server
    } else {
        gpui::WindowDecorations::Client
    }
}

/// Settings waiting to be written, in the order they changed. One write
/// runs at a time, so a burst of zoom steps can't land out of order.
#[derive(Default)]
struct Saving {
    pending: Vec<(&'static str, Value)>,
    running: bool,
}

impl Global for Saving {}

fn save(cx: &mut App, key: &'static str, value: Value) {
    let saving = cx.default_global::<Saving>();
    saving.pending.retain(|(each, _)| *each != key);
    saving.pending.push((key, value));
    if std::mem::replace(&mut saving.running, true) {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            let batch = cx.update(|cx| {
                let saving = cx.global_mut::<Saving>();
                saving.running = !saving.pending.is_empty();
                std::mem::take(&mut saving.pending)
            });
            if batch.is_empty() {
                break;
            }
            let task = cx.update(|cx| {
                Backend::global(cx).run(cx, move |host| async move {
                    batch
                        .into_iter()
                        .try_for_each(|(key, value)| config::set(&host, key, value))
                })
            });
            if let Err(error) = task.await {
                log::warn!("Failed to save a setting: {error:#}");
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_the_ui_scale_like_the_desktop_app() {
        assert_eq!(clamp_ui_scale(None), 100);
        assert_eq!(clamp_ui_scale(Some(f64::NAN)), 100);
        assert_eq!(clamp_ui_scale(Some(10.)), 50);
        assert_eq!(clamp_ui_scale(Some(400.)), 200);
        assert_eq!(clamp_ui_scale(Some(114.6)), 115);
        assert_eq!(clamp_ui_scale(Some(120.)), 120);
    }

    #[test]
    fn reads_each_key_with_its_default() {
        let temp = tempfile::tempdir().unwrap();
        let mut options =
            quadrant_host::QuadrantHostOptions::new(temp.path().join("data"), "", "", "");
        options.mc_folder = Some(temp.path().join("mc"));
        options.keyring_service_name = "quadrant-gpui-tests".into();
        let host = QuadrantHost::new(options).unwrap();
        host.init_config().unwrap();
        assert_eq!(Prefs::read(&host), Prefs::default());

        config::set(&host, LOCALE_KEY, "uk".into()).unwrap();
        config::set(&host, UI_SCALE_KEY, 130.into()).unwrap();
        config::set(&host, CLIP_ICONS_KEY, false.into()).unwrap();
        config::set(&host, NATIVE_DECORATIONS_KEY, true.into()).unwrap();
        assert_eq!(
            Prefs::read(&host),
            Prefs {
                locale: Locale::Ukrainian,
                ui_scale: 130,
                clip_icons: false,
                native_decorations: true,
            }
        );
        config::set(&host, UI_SCALE_KEY, "big".into()).unwrap();
        assert_eq!(Prefs::read(&host).ui_scale, DEFAULT_UI_SCALE);
    }
}
