//! The desktop app's snackbar (`App.tsx`): one message at a time, bottom left,
//! replaced by the next and hidden after a timeout or by its clear button.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, AppContext as _, Context, ElementId, Entity, Global,
    IntoElement, ParentElement as _, Render, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, px, rems,
};
use gpui_component::{Icon, h_flex};

use crate::{
    assets::MdIcon,
    theme::{self, Tone},
};

/// How long a snackbar stays up, as `useReportSuccess` and `useReportError` set it.
pub const TIMEOUT: Duration = Duration::from_millis(5000);
/// The entrance: opacity 0 to 1 while rising 24px into place.
pub const ENTER: Duration = Duration::from_millis(300);
const ENTER_RISE: f32 = 24.;

/// What a snackbar reports, which picks its colour and icon.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnackbarKind {
    /// `bg-slate-700`, as the desktop app's in-progress export snackbar.
    Neutral,
    /// `bg-emerald-600` with a check mark (`useReportSuccess`).
    Success,
    /// `bg-emerald-600` without one, as the desktop app's modpack dialog
    /// confirms a save.
    Saved,
    /// `bg-emerald-600` with a delete icon and no text, as a deleted
    /// modpack is confirmed.
    Deleted,
    /// `bg-red-700` (`useReportError`).
    Error,
}

impl SnackbarKind {
    /// The icon drawn before the message.
    pub fn icon(self) -> Option<MdIcon> {
        match self {
            Self::Success => Some(MdIcon::Check),
            Self::Deleted => Some(MdIcon::Delete),
            Self::Saved | Self::Neutral | Self::Error => None,
        }
    }
}

/// What a snackbar says.
#[derive(Clone, PartialEq, Debug)]
pub enum Message {
    Text(SharedString),
    /// The export's "Export [archive] 42%" (`App.tsx`), which carries its
    /// own icon in place of the kind's.
    Export {
        percent: u8,
    },
}

impl From<SharedString> for Message {
    fn from(text: SharedString) -> Self {
        Self::Text(text)
    }
}

impl From<&'static str> for Message {
    fn from(text: &'static str) -> Self {
        Self::Text(text.into())
    }
}

impl From<String> for Message {
    fn from(text: String) -> Self {
        Self::Text(text.into())
    }
}

/// A snackbar's icon and words, as the snackbar and the notifications
/// panel's history both draw them.
pub fn content(kind: SnackbarKind, message: &Message) -> gpui::Div {
    let row = div().flex().min_w_0();
    match message {
        Message::Text(text) => row
            .when_some(kind.icon(), |this, icon| {
                this.child(Icon::new(icon).size_5().mx_2().flex_shrink_0())
            })
            .child(div().min_w_0().child(text.clone())),
        Message::Export { percent } => row
            .items_center()
            .child(crate::i18n::t("export"))
            .child(Icon::new(MdIcon::Archive).size_6().mx_2().flex_shrink_0())
            .child(format!("{percent}%")),
    }
}

/// One snackbar to put up.
#[derive(Clone, PartialEq, Debug)]
pub struct Snack {
    pub kind: SnackbarKind,
    pub message: Message,
    pub timeout: Duration,
    /// Whether the notifications panel lists it: the desktop app's
    /// `setSnackbar`, against `setSnackbarNoState`.
    pub remember: bool,
}

#[derive(Clone, PartialEq, Debug)]
struct Shown {
    kind: SnackbarKind,
    message: Message,
}

/// The window's snackbar. Every `show` starts a new generation, so the timer
/// of a replaced snackbar cannot hide the one that replaced it.
pub struct Snackbar {
    shown: Option<Shown>,
    generation: u64,
}

impl Snackbar {
    fn new() -> Self {
        Self {
            shown: None,
            generation: 0,
        }
    }

    /// Replaces whatever is up and returns the new snackbar's generation.
    fn replace(&mut self, kind: SnackbarKind, message: Message) -> u64 {
        self.generation += 1;
        self.shown = Some(Shown { kind, message });
        self.generation
    }

    /// Hides the snackbar; returns whether one was up.
    fn hide(&mut self) -> bool {
        self.shown.take().is_some()
    }

    /// Hides the snackbar if it is still the one from `generation`.
    fn expire(&mut self, generation: u64) -> bool {
        generation == self.generation && self.hide()
    }
}

struct GlobalSnackbar(Entity<Snackbar>);

impl Global for GlobalSnackbar {}

/// Creates the app's snackbar. Call once at startup, before `show` or `view`.
pub fn install(cx: &mut App) {
    let snackbar = cx.new(|_| Snackbar::new());
    cx.set_global(GlobalSnackbar(snackbar));
}

/// The snackbar view, for the root to draw over the page.
pub fn view(cx: &App) -> Entity<Snackbar> {
    cx.global::<GlobalSnackbar>().0.clone()
}

/// Shows `message`, replacing the current snackbar, and hides it after the
/// timeout. The notifications panel remembers it.
pub fn show(cx: &mut App, kind: SnackbarKind, message: impl Into<SharedString>) {
    present(
        cx,
        Snack {
            kind,
            message: Message::Text(message.into()),
            timeout: TIMEOUT,
            remember: true,
        },
    );
}

/// Puts `snack` up in place of the current snackbar until its timeout.
pub fn present(cx: &mut App, snack: Snack) {
    let Snack {
        kind,
        message,
        timeout,
        remember,
    } = snack;
    if remember {
        crate::notifications::record(cx, kind, message.clone());
    }
    let snackbar = view(cx);
    let generation = snackbar.update(cx, |snackbar, cx| {
        let generation = snackbar.replace(kind, message);
        cx.notify();
        generation
    });
    cx.spawn(async move |cx| {
        cx.background_executor().timer(timeout).await;
        snackbar.update(cx, |snackbar, cx| {
            if snackbar.expire(generation) {
                cx.notify();
            }
        });
    })
    .detach();
}

impl Render for Snackbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(Shown { kind, message }) = self.shown.clone() else {
            return div().absolute().into_any_element();
        };
        let rem = window.rem_size();
        // `bottom-8 left-8 max-w-[min(36rem,calc(100vw-4rem))]`
        let bottom = rems(2.) * rem;
        let max_width = (rems(36.) * rem).min(window.viewport_size().width - rems(4.) * rem);
        theme::snackbar_surface(kind)
            .absolute()
            .left_8()
            .max_w(max_width)
            .child(
                h_flex().gap_3().child(content(kind, &message)).child(
                    theme::window_button("snackbar-clear", Tone::Scrim, MdIcon::Clear, cx)
                        .flex_shrink_0()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.hide() {
                                cx.notify();
                            }
                        })),
                ),
            )
            .with_animation(
                ElementId::NamedInteger("snackbar".into(), self.generation),
                Animation::new(ENTER).with_easing(gpui::ease_out_quint()),
                move |this, delta| {
                    this.opacity(delta)
                        .bottom(bottom - px(ENTER_RISE * (1. - delta)))
                },
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_snackbar_replaces_the_current_one() {
        let mut snackbar = Snackbar::new();
        snackbar.replace(SnackbarKind::Error, Message::from("first"));
        snackbar.replace(SnackbarKind::Success, Message::from("second"));
        assert_eq!(
            snackbar.shown,
            Some(Shown {
                kind: SnackbarKind::Success,
                message: Message::from("second"),
            })
        );
    }

    #[test]
    fn a_replaced_snackbars_timer_leaves_its_successor_up() {
        let mut snackbar = Snackbar::new();
        let first = snackbar.replace(SnackbarKind::Success, Message::from("first"));
        let second = snackbar.replace(SnackbarKind::Success, Message::from("second"));
        assert!(!snackbar.expire(first));
        assert!(snackbar.shown.is_some());
        assert!(snackbar.expire(second));
        assert_eq!(snackbar.shown, None);
    }

    #[test]
    fn a_timer_after_dismissal_changes_nothing() {
        let mut snackbar = Snackbar::new();
        let generation = snackbar.replace(SnackbarKind::Neutral, Message::from("event"));
        assert!(snackbar.hide());
        assert!(!snackbar.expire(generation));
    }

    #[test]
    fn only_success_carries_a_check_mark_and_delete_its_own_icon() {
        assert_eq!(SnackbarKind::Success.icon(), Some(MdIcon::Check));
        assert_eq!(SnackbarKind::Error.icon(), None);
        assert_eq!(SnackbarKind::Saved.icon(), None);
        assert_eq!(SnackbarKind::Deleted.icon(), Some(MdIcon::Delete));
        assert_eq!(SnackbarKind::Neutral.icon(), None);
    }
}
