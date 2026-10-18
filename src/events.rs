use gpui::{Context, SharedString, Window};
use gpui_component::{WindowExt as _, notification::Notification};
use quadrant_host::HostEventEnvelope;
use tokio::sync::broadcast::error::RecvError;

use crate::backend::Backend;

/// Shows host events as notifications in `window` for as long as it lives.
pub fn forward<T: 'static>(window: &Window, cx: &Context<T>) {
    let mut events = Backend::global(cx).subscribe_events();
    cx.spawn_in(window, async move |_, cx| {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            let Some(text) = notification_text(&event) else {
                continue;
            };
            let shown = cx.update(|window, cx| {
                window.push_notification(Notification::info(text), cx);
            });
            if shown.is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Progress events arrive per chunk; a toast each would bury the screen.
fn notification_text(event: &HostEventEnvelope) -> Option<SharedString> {
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
        assert_eq!(notification_text(&event("modDownloadProgress")), None);
        assert_eq!(
            notification_text(&event("quadrantShareSubmission")).as_deref(),
            Some("quadrantShareSubmission")
        );
    }
}
