//! The Account page: Quadrant ID sign-in and the signed-in account, after
//! `AccountPage` in the desktop app.

use gpui::{
    Context, Entity, FontWeight, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, relative, rems, rgb,
};
use gpui_component::{Disableable as _, h_flex, v_flex};
use quadrant_core::account::id::AccountInfo;
use quadrant_host::QuadrantHost;
use tokio::sync::oneshot;

use crate::{
    assets::MdIcon,
    backend::Backend,
    i18n::{describe, t, t_with},
    loadable::Loadable,
    oauth::{self, Outcome},
    session::Session,
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};

const ACCOUNT_URL: &str = "https://mrquantumoff.dev/account";
const REGISTER_URL: &str = "https://mrquantumoff.dev/account/register";
const PRIVACY_URL: &str = "https://github.com/QuadrantMC/quadrant/blob/next/PRIVACY_POLICY.md";

/// A change to the sign-in. At most one runs at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    SignOut,
}

impl Op {
    pub async fn perform(self, host: QuadrantHost) -> anyhow::Result<()> {
        match self {
            Op::SignOut => host.logout(),
        }
    }
}

pub struct AccountPage {
    session: Entity<Session>,
    running: Option<Op>,
    /// Stops the sign-in waiting for the browser, when one is.
    signing_in: Option<oneshot::Sender<()>>,
    /// Set while a sign-in sets up its listener, before the browser opens.
    starting: bool,
    /// Why the last sign-in failed, shown until the next one starts.
    login_warning: Option<SharedString>,
    _session_changed: Subscription,
}

impl AccountPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let session = Session::global(cx);
        let session_changed = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            running: None,
            signing_in: None,
            starting: false,
            login_warning: None,
            _session_changed: session_changed,
        }
    }

    /// Asks who is signed in again as the page is shown again.
    pub fn revalidate(&mut self, cx: &mut Context<Self>) {
        self.session
            .update(cx, |session, cx| session.revalidate(cx));
    }

    fn run(&mut self, op: Op, cx: &mut Context<Self>) {
        if self.running.is_some() {
            return;
        }
        self.running = Some(op);
        let task = Backend::global(cx).run(cx, move |host| op.perform(host));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running = None;
                this.login_warning = None;
                if let Err(error) = result {
                    snackbar::show(cx, SnackbarKind::Error, describe(error));
                }
                this.session.update(cx, |session, cx| session.refresh(cx));
            });
        })
        .detach();
        cx.notify();
    }

    /// Opens the browser at Quadrant ID and waits for its answer. Starting
    /// again drops the sign-in that was waiting.
    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        if self.starting {
            return;
        }
        if let Some(cancel) = self.signing_in.take() {
            let _ = cancel.send(());
        }
        self.starting = true;
        self.login_warning = None;
        let (browser, opened) = oneshot::channel();
        let (cancel, cancelled) = oneshot::channel();
        self.signing_in = Some(cancel);
        let task =
            Backend::global(cx).run(cx, move |host| oauth::sign_in(host, browser, cancelled));
        cx.spawn(async move |this, cx| {
            let url = opened.await;
            let _ = this.update(cx, |this, cx| {
                this.starting = false;
                if let Ok(url) = &url {
                    cx.open_url(url.as_str());
                }
                cx.notify();
            });
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.starting = false;
                match result {
                    Ok(Outcome::Cancelled) => return,
                    Ok(Outcome::SignedIn) => this.signing_in = None,
                    Err(error) => {
                        this.signing_in = None;
                        this.login_warning = Some(describe(error));
                        snackbar::show(cx, SnackbarKind::Error, t("accountLoginFailed"));
                    }
                }
                this.session.update(cx, |session, cx| session.refresh(cx));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_checking(&self) -> impl IntoElement {
        v_flex()
            .w(relative(0.75))
            .max_w(rems(56.))
            .items_center()
            .px_6()
            .py_10()
            .rounded(theme::radius_card())
            .bg(rgb(theme::SURFACE))
            .child(theme::circular_progress("progress"))
            .child(
                div()
                    .mt_6()
                    .text_2xl()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(t("loadingAccount")),
            )
            .child(
                div()
                    .mt_4()
                    .text_lg()
                    .text_color(rgb(theme::TEXT_MUTED))
                    .child(t("loadingAccountDetails")),
            )
    }

    fn render_signed_in(&self, info: &AccountInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let line = |label: &str, value: String| {
            div()
                .w_full()
                .my_4()
                .p_2()
                .rounded(theme::radius_card())
                .bg(rgb(theme::SURFACE))
                .flex()
                .justify_center()
                .font_weight(FontWeight::BOLD)
                .child(format!("{}: {value}", t(label)))
        };
        v_flex()
            .w_full()
            .items_center()
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(t_with("hello", &[("name", &info.name)])),
            )
            .child(
                v_flex()
                    .w(relative(0.5))
                    .child(line("email", info.email.clone()))
                    .child(line("username", info.login.clone()))
                    .child(line("syncLimit", info.quadrant_sync_limit.to_string()))
                    .child(line("shareLimit", info.quadrant_share_limit.to_string())),
            )
            .child(
                h_flex()
                    .w(relative(0.75))
                    .mt_4()
                    .gap_4()
                    .child(
                        theme::pill_button("sign-out", Tone::Destructive, t("signout"), None, cx)
                            .flex_1()
                            .loading(self.running == Some(Op::SignOut))
                            .disabled(self.running.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.run(Op::SignOut, cx))),
                    )
                    .child(
                        theme::pill_button("edit-account", Tone::Primary, t("edit"), None, cx)
                            .flex_1()
                            .on_click(|_, _, cx| cx.open_url(ACCOUNT_URL)),
                    ),
            )
    }

    fn render_signed_out(
        &self,
        warning: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w(relative(0.75))
            .child(
                div()
                    .my_4()
                    .px_4()
                    .py_2()
                    .rounded(theme::radius_card())
                    .bg(rgb(theme::SURFACE))
                    .text_2xl()
                    .font_weight(FontWeight::EXTRA_BOLD)
                    .child(t("signIn")),
            )
            .when_some(warning, |this, warning| {
                this.child(div().my_4().child(theme::warning_box(warning)))
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .child(
                        theme::pill_button(
                            "sign-in",
                            Tone::Primary,
                            t("signInWithOAuth"),
                            None,
                            cx,
                        )
                        .flex_1()
                        .loading(self.starting)
                        .disabled(self.starting)
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                    )
                    .child(
                        theme::pill_button(
                            "register",
                            Tone::Neutral,
                            t("dontHaveAccount"),
                            None,
                            cx,
                        )
                        .flex_1()
                        .on_click(|_, _, cx| cx.open_url(REGISTER_URL)),
                    ),
            )
            .child(
                div().mt_4().w_full().child(
                    theme::block_button(
                        "privacy",
                        Tone::Neutral,
                        t("acceptQuadrantIDTOS"),
                        Some(MdIcon::OpenInBrowser),
                    )
                    .text_lg()
                    .on_click(|_, _, cx| cx.open_url(PRIVACY_URL)),
                ),
            )
    }
}

impl Render for AccountPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.session.read(cx).account() {
            Loadable::Idle | Loadable::Loading => None,
            Loadable::Ready(Some(info)) => Some(Ok(info.clone())),
            Loadable::Ready(None) => Some(Err(None)),
            // An account that couldn't be read offers a fresh sign-in, as the
            // desktop app does, and says what went wrong.
            Loadable::Failed(error) => Some(Err(Some(error.clone()))),
        };
        let body = match body {
            None => self.render_checking().into_any_element(),
            Some(Ok(info)) => self.render_signed_in(&info, cx).into_any_element(),
            Some(Err(unreadable)) => {
                let warning = self.login_warning.clone().or(unreadable);
                self.render_signed_out(warning, cx).into_any_element()
            }
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .child(body)
    }
}
