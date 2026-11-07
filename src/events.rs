use std::time::Duration;

use gpui::{App, Context, SharedString, Task};
use quadrant_host::HostEventEnvelope;
use tokio::sync::broadcast::error::RecvError;

use crate::{
    backend::Backend,
    i18n::t_with,
    snackbar::{self, Message, Snack, SnackbarKind},
    taskbar::{self, Job},
};

/// Shows host events in the snackbar for as long as the app runs.
pub fn forward(cx: &mut App) {
    let mut events = Backend::global(cx).subscribe_events();
    cx.spawn(async move |cx| {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            let done = event.payload.as_f64();
            if event.event == "quadrantExportProgress"
                && let Some(done) = done
            {
                cx.update(|cx| {
                    taskbar::progress(cx, Job::Export, done);
                    snackbar::present(cx, export_snack(done));
                });
            } else if event.event == "modpackDownloadProgress"
                && let Some(done) = done
            {
                cx.update(|cx| taskbar::progress(cx, Job::ModpackDownload, done));
            } else if let Some(text) = snackbar_text(&event) {
                cx.update(|cx| snackbar::show(cx, SnackbarKind::Neutral, text));
            }
        }
    })
    .detach();
}

/// The export's snackbar, as `App.tsx` puts it up for each
/// `quadrantExportProgress`: a slate one that stays while the export runs and
/// isn't kept in the history, then an emerald one at 100%.
fn export_snack(done: f64) -> Snack {
    let percent = (done * 100.).round().clamp(0., 100.) as u8;
    let finished = done >= 1.;
    Snack {
        kind: if finished {
            SnackbarKind::Success
        } else {
            SnackbarKind::Neutral
        },
        message: Message::Export { percent },
        timeout: Duration::from_millis(if finished { 15_000 } else { 500_000 }),
        remember: finished,
    }
}

/// Puts up the snackbar an export `done` of the way through shows.
#[cfg(feature = "screenshot")]
pub fn preview_export(cx: &mut App, done: f64) {
    snackbar::present(cx, export_snack(done));
}

/// Events a view reacts to through [`listen`] instead of a snackbar.
const HANDLED_BY_VIEWS: [&str; 4] = [
    "recheckAccountToken",
    "refreshNotifications",
    "refreshSyncedModpacks",
    "configChanged",
];

/// Progress events arrive per chunk; a snackbar each would flicker past unread.
fn snackbar_text(event: &HostEventEnvelope) -> Option<SharedString> {
    let name = event.event.as_str();
    if name.ends_with("Progress") || HANDLED_BY_VIEWS.contains(&name) {
        return None;
    }
    if name == "quadrantShareSubmission" {
        let uses_left = event.payload["uses_left"].to_string();
        return Some(t_with("copiedToClipboard", &[("amount", &uses_left)]));
    }
    Some(name.to_string().into())
}

/// Calls `f` on the view for each host event named in `names`, for as long as
/// the view lives.
pub fn listen<T: 'static>(
    names: &'static [&'static str],
    cx: &mut Context<T>,
    f: impl Fn(&mut T, &HostEventEnvelope, &mut Context<T>) + 'static,
) {
    let mut events = Backend::global(cx).subscribe_events();
    cx.spawn(async move |this, cx| {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            if !names.contains(&event.event.as_str()) {
                continue;
            }
            if this.update(cx, |this, cx| f(this, &event, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Which half of a mod install a progress event reports.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModStage {
    Download,
    Install,
}

/// A `modDownloadProgress` or `modInstallProgress` event: how far, out of
/// 100, the install of `mod_id` has come.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ModProgress {
    pub mod_id: String,
    pub stage: ModStage,
    pub percent: i32,
}

impl ModProgress {
    pub fn parse(event: &HostEventEnvelope) -> Option<Self> {
        let stage = match event.event.as_str() {
            "modDownloadProgress" => ModStage::Download,
            "modInstallProgress" => ModStage::Install,
            _ => return None,
        };
        Some(Self {
            mod_id: event.payload.get("modId")?.as_str()?.to_string(),
            stage,
            percent: i32::try_from(event.payload.get("progress")?.as_i64()?).ok()?,
        })
    }
}

/// Hands every mod progress event to `on_progress` while the returned task
/// is kept, so a page sees how far its install has come.
pub fn watch_mod_progress<T: 'static>(
    cx: &mut Context<T>,
    on_progress: impl Fn(&mut T, ModProgress, &mut Context<T>) + 'static,
) -> Task<()> {
    let mut events = Backend::global(cx).subscribe_events();
    cx.spawn(async move |this, cx| {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            let Some(progress) = ModProgress::parse(&event) else {
                continue;
            };
            if this
                .update(cx, |this, cx| on_progress(this, progress, cx))
                .is_err()
            {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn event(name: &str) -> HostEventEnvelope {
        HostEventEnvelope {
            event: name.to_string(),
            payload: Value::Null,
        }
    }

    #[test]
    fn mod_progress_reads_both_stages() {
        let event = |name: &str, payload| HostEventEnvelope {
            event: name.to_string(),
            payload,
        };
        assert_eq!(
            ModProgress::parse(&event(
                "modDownloadProgress",
                serde_json::json!({ "modId": "AANobbMI", "progress": 42 })
            )),
            Some(ModProgress {
                mod_id: "AANobbMI".into(),
                stage: ModStage::Download,
                percent: 42,
            })
        );
        assert_eq!(
            ModProgress::parse(&event(
                "modInstallProgress",
                serde_json::json!({ "modId": "x", "progress": 100 })
            ))
            .map(|progress| progress.stage),
            Some(ModStage::Install)
        );
        assert_eq!(
            ModProgress::parse(&event("modpackDownloadProgress", serde_json::json!(50.0))),
            None
        );
        assert_eq!(
            ModProgress::parse(&event("modDownloadProgress", Value::Null)),
            None
        );
    }

    #[test]
    fn progress_events_stay_quiet() {
        assert_eq!(snackbar_text(&event("modDownloadProgress")), None);
        assert_eq!(snackbar_text(&event("refreshSyncedModpacks")), None);
        assert_eq!(
            snackbar_text(&event("somethingNew")).as_deref(),
            Some("somethingNew")
        );
    }

    #[test]
    fn an_export_counts_up_then_settles_in_emerald() {
        let running = export_snack(0.426);
        assert_eq!(running.message, Message::Export { percent: 43 });
        assert_eq!(running.kind, SnackbarKind::Neutral);
        assert!(!running.remember);
        assert_eq!(running.timeout, Duration::from_millis(500_000));
        let done = export_snack(1.);
        assert_eq!(done.message, Message::Export { percent: 100 });
        assert_eq!(done.kind, SnackbarKind::Success);
        assert!(done.remember);
        assert_eq!(done.timeout, Duration::from_millis(15_000));
        assert_eq!(export_snack(0.999).kind, SnackbarKind::Neutral);
    }

    #[test]
    fn a_share_says_how_many_uses_are_left() {
        let shared = HostEventEnvelope {
            event: "quadrantShareSubmission".into(),
            payload: serde_json::json!({ "code": 1234567, "uses_left": 9 }),
        };
        assert_eq!(
            snackbar_text(&shared).as_deref(),
            Some("Copied to clipboard, 9 uses left")
        );
    }
}
