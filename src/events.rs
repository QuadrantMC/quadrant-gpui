use gpui::{App, SharedString};
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
    fn progress_events_stay_quiet() {
        assert_eq!(snackbar_text(&event("modDownloadProgress")), None);
        assert_eq!(
            snackbar_text(&event("quadrantShareSubmission")).as_deref(),
            Some("quadrantShareSubmission")
        );
    }
}
