use gpui::{App, Context, SharedString, Task};
use quadrant_host::HostEventEnvelope;
use tokio::sync::broadcast::error::RecvError;

use crate::{
    backend::Backend,
    snackbar::{self, SnackbarKind},
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
            if let Some(text) = snackbar_text(&event) {
                cx.update(|cx| snackbar::show(cx, SnackbarKind::Neutral, text));
            }
        }
    })
    .detach();
}

/// Progress events arrive per chunk; a snackbar each would flicker past unread.
fn snackbar_text(event: &HostEventEnvelope) -> Option<SharedString> {
    (!event.event.ends_with("Progress")).then(|| event.event.clone().into())
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
        assert_eq!(
            snackbar_text(&event("quadrantShareSubmission")).as_deref(),
            Some("quadrantShareSubmission")
        );
    }
}
