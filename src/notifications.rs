//! The desktop app's notifications (`Notifications.tsx`): the bell in the
//! title bar and the panel it opens, with what the snackbar has shown, the
//! signed-in account's notifications and the Quadrant blog's news.
//!
//! The bell turns red while an account notification shown in the panel is
//! unread, else blue while a news post is new.

use std::collections::HashMap;

use gpui::{
    Animation, AnimationExt as _, App, AppContext as _, Context, Entity, FontWeight, Global,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred, div,
    prelude::FluentBuilder as _, rems,
};
use gpui_component::{h_flex, v_flex};
use quadrant_core::{
    account::id::{AccountInfo, Notification},
    models::Article,
};
use quadrant_host::QuadrantHost;
use serde_json::Value;

use crate::{
    assets::MdIcon,
    backend::Backend,
    events,
    i18n::{describe, t, t_with},
    loadable::Loadable,
    motion,
    session::Session,
    snackbar::{self, Message, SnackbarKind},
    theme::{self, Tone},
};

// The snackbar history (`snackbar.ts`).

/// How many snackbars the panel remembers (`SNACKBAR_HISTORY_LIMIT`).
pub const HISTORY_LIMIT: usize = 5;

/// A snackbar the panel lists, and how many times it came up in a row of
/// repeats.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub id: u64,
    pub kind: SnackbarKind,
    pub message: Message,
    pub count: usize,
}

/// Every snackbar shown, oldest first: a repeat of the same message and
/// kind counts up and moves to the end, and only the newest
/// [`HISTORY_LIMIT`] are kept (`appendSnackbarHistory`).
#[derive(Debug, Default)]
pub struct History {
    entries: Vec<HistoryEntry>,
    next_id: u64,
}

impl History {
    pub fn record(&mut self, kind: SnackbarKind, message: Message) {
        let repeat = self
            .entries
            .iter()
            .position(|entry| entry.kind == kind && entry.message == message);
        match repeat {
            Some(index) => {
                let mut entry = self.entries.remove(index);
                entry.count += 1;
                self.entries.push(entry);
            }
            None => {
                self.next_id += 1;
                self.entries.push(HistoryEntry {
                    id: self.next_id,
                    kind,
                    message,
                    count: 1,
                });
            }
        }
        let excess = self.entries.len().saturating_sub(HISTORY_LIMIT);
        self.entries.drain(..excess);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }
}

// Account notifications.

const MODPACK_SYNC: &str = "modpack_sync";
const INVITE_TO_SYNC: &str = "invite_to_sync";
/// What an invite's message says before the inviter's name.
const INVITED_BY: &str = "You have been invited to collaborate on a modpack by ";

/// The JSON object a notification's message usually is, or `Null`.
fn detail(notification: &Notification) -> Value {
    serde_json::from_str(&notification.message).unwrap_or(Value::Null)
}

fn field<'a>(detail: &'a Value, key: &str) -> Option<&'a str> {
    detail.get(key).and_then(Value::as_str)
}

fn kind<'a>(notification: &'a Notification, detail: &'a Value) -> Option<&'a str> {
    field(detail, "notification_type").or(notification.notification_type.as_deref())
}

fn normalize(identity: &str) -> Option<String> {
    let normalized = identity.trim().to_lowercase();
    (!normalized.is_empty()).then_some(normalized)
}

/// Who is signed in, as far as the panel knows: a pack updated by the
/// account itself is no news to it.
#[derive(Clone, Debug, PartialEq)]
pub enum Identity {
    /// The account is still being read.
    Checking,
    /// The account's name and login, lowercased; none when signed out or
    /// unreadable.
    Known(Vec<String>),
}

impl Identity {
    pub fn of(account: &Loadable<Option<AccountInfo>>) -> Self {
        match account {
            Loadable::Idle | Loadable::Loading => Self::Checking,
            Loadable::Ready(Some(info)) => Self::Known(
                [&info.name, &info.login]
                    .into_iter()
                    .filter_map(|identity| normalize(identity))
                    .collect(),
            ),
            Loadable::Ready(None) | Loadable::Failed(_) => Self::Known(Vec::new()),
        }
    }
}

/// Whether the panel lists `notification`: a modpack update shows only
/// while `showModpackUpdateNotifications` is on, and not when the account
/// made it itself.
pub fn is_visible(notification: &Notification, show_updates: bool, identity: &Identity) -> bool {
    let detail = detail(notification);
    if kind(notification, &detail) != Some(MODPACK_SYNC) {
        return true;
    }
    if !show_updates {
        return false;
    }
    let Some(updated_by) = field(&detail, "updated_by").and_then(normalize) else {
        return true;
    };
    match identity {
        Identity::Checking => false,
        Identity::Known(own) => !own.contains(&updated_by),
    }
}

/// What can be done about a notification from the panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// It has been read already.
    None,
    Read,
    /// An invite to a Quadrant Sync modpack, accepted or declined.
    Invite {
        modpack_id: String,
    },
}

/// Where in `notifications` the ones [`is_visible`] keeps are.
pub fn visible_indexes(
    notifications: &[Notification],
    show_updates: bool,
    identity: &Identity,
) -> Vec<usize> {
    (0..notifications.len())
        .filter(|&index| is_visible(&notifications[index], show_updates, identity))
        .collect()
}

/// A notification as the panel words it.
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub message: String,
    pub action: Action,
}

pub fn shown(notification: &Notification) -> Shown {
    let detail = detail(notification);
    let mut message = field(&detail, "simple_message")
        .unwrap_or(&notification.message)
        .to_string();
    let mut action = Action::Read;
    let invite = field(&detail, "invite_id")
        .or(notification.resource_id.as_deref())
        .filter(|id| !id.is_empty());
    if kind(notification, &detail) == Some(INVITE_TO_SYNC)
        && let Some(modpack_id) = invite
    {
        let inviter = field(&detail, "message")
            .or(field(&detail, "simple_message"))
            .unwrap_or(&notification.message)
            .split(INVITED_BY)
            .nth(1)
            .map(str::trim)
            .unwrap_or_default();
        message = t_with("invited", &[("name", inviter)]).to_string();
        action = Action::Invite {
            modpack_id: modpack_id.to_string(),
        };
    }
    if notification.read {
        action = Action::None;
    }
    Shown { message, action }
}

/// The host's notifications, newest first.
pub fn newest_first(mut notifications: Vec<Notification>) -> Vec<Notification> {
    notifications.sort_by_key(|notification| std::cmp::Reverse(notification.created_at_unix));
    notifications
}

/// The bell's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bell {
    /// red-700: an account notification is unread.
    Unread,
    /// blue-600: a news post is new.
    News,
    /// slate-700
    Quiet,
}

impl Bell {
    pub fn of(unread: usize, news: &[Article]) -> Self {
        if unread > 0 {
            Self::Unread
        } else if news.iter().any(|article| article.new) {
            Self::News
        } else {
            Self::Quiet
        }
    }

    fn tone(self) -> Tone {
        match self {
            Self::Unread => Tone::Alert,
            Self::News => Tone::Primary,
            Self::Quiet => Tone::Control,
        }
    }
}

/// An RSS summary's text, as the desktop app reads it out of the HTML.
pub fn plain_text(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let text = [
        ("&nbsp;", " "),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&#039;", "'"),
        ("&apos;", "'"),
        ("&amp;", "&"),
    ]
    .iter()
    .fold(text, |text, (entity, plain)| text.replace(entity, plain));
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// The panel.

/// Whether the panel shows. Each opening counts up, so its entrance and
/// exit play afresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Closed,
    Open(usize),
    /// Playing its exit; gone after [`motion::ENTER`].
    Closing(usize),
}

impl Panel {
    pub fn toggled(self) -> Self {
        match self {
            Self::Open(id) => Self::Closing(id),
            Self::Closed => Self::Open(1),
            Self::Closing(id) => Self::Open(id + 1),
        }
    }

    /// The panel once the exit of opening `id` has played.
    pub fn settled(self, id: usize) -> Self {
        if self == Self::Closing(id) {
            Self::Closed
        } else {
            self
        }
    }
}

/// A change to an account notification.
#[derive(Clone, Debug, PartialEq)]
enum Op {
    Read {
        notification_id: String,
    },
    Answer {
        modpack_id: String,
        notification_id: String,
        accept: bool,
    },
}

impl Op {
    fn target(&self) -> &str {
        match self {
            Op::Read { notification_id }
            | Op::Answer {
                notification_id, ..
            } => notification_id,
        }
    }

    async fn perform(self, host: QuadrantHost) -> anyhow::Result<()> {
        match self {
            Op::Read { notification_id } => host.read_notification(notification_id).await,
            Op::Answer {
                modpack_id,
                notification_id,
                accept,
            } => {
                host.answer_invite(modpack_id, notification_id, accept)
                    .await
            }
        }
    }
}

/// The config key the panel follows for modpack updates.
const SHOW_UPDATES: &str = "showModpackUpdateNotifications";

/// The bell, its panel and everything they list. One for the app, so the
/// snackbar history and the notifications outlive the window.
pub struct Notifications {
    history: History,
    account: Vec<Notification>,
    /// Indexes into `account` of the ones the panel lists, kept by [`Self::sift`]:
    /// telling them apart parses each message, too slow for every frame.
    visible: Vec<usize>,
    news: Vec<Article>,
    show_updates: bool,
    error: Option<SharedString>,
    panel: Panel,
    /// Ops in flight, by the notification they change.
    running: HashMap<String, Op>,
    session: Entity<Session>,
}

struct GlobalNotifications(Entity<Notifications>);

impl Global for GlobalNotifications {}

/// Creates the app's notifications, listening from now on. Call once at
/// startup, before the window opens. `fetch_news` asks the Quadrant blog for
/// its posts.
pub fn install(fetch_news: bool, cx: &mut App) {
    let notifications = cx.new(|cx| Notifications::new(fetch_news, cx));
    cx.set_global(GlobalNotifications(notifications));
}

/// The bell, for the title bar.
pub fn view(cx: &App) -> Option<Entity<Notifications>> {
    cx.try_global::<GlobalNotifications>()
        .map(|global| global.0.clone())
}

/// Adds a snackbar to the history.
pub fn record(cx: &mut App, kind: SnackbarKind, message: Message) {
    if let Some(notifications) = view(cx) {
        notifications.update(cx, |this, cx| {
            this.history.record(kind, message);
            if this.panel != Panel::Closed {
                cx.notify();
            }
        });
    }
}

impl Notifications {
    fn new(fetch_news: bool, cx: &mut Context<Self>) -> Self {
        let session = Session::global(cx);
        cx.observe(&session, |this, session, cx| {
            // Account notifications are the signed-in account's alone.
            if matches!(session.read(cx).account(), Loadable::Ready(None)) {
                this.account.clear();
            }
            this.sift(cx);
            cx.notify();
        })
        .detach();
        events::listen(&["refreshNotifications"], cx, |this, event, cx| {
            match serde_json::from_value::<Vec<Notification>>(event.payload.clone()) {
                Ok(notifications) => this.account = newest_first(notifications),
                Err(error) => log::warn!("Unreadable notifications: {error}"),
            }
            this.sift(cx);
            cx.notify();
        });
        events::listen_bursts(&["configChanged"], cx, |this, cx| this.read_config(cx));
        let mut this = Self {
            history: History::default(),
            account: Vec::new(),
            visible: Vec::new(),
            news: Vec::new(),
            show_updates: true,
            error: None,
            panel: Panel::Closed,
            running: HashMap::new(),
            session,
        };
        this.read_config(cx);
        if fetch_news {
            this.fetch_news(cx);
        }
        this
    }

    fn read_config(&mut self, cx: &mut Context<Self>) {
        let task =
            Backend::global(cx).run(
                cx,
                |host| async move { host.get_config_value(SHOW_UPDATES) },
            );
        cx.spawn(async move |this, cx| {
            let show = match task.await {
                Ok(value) => value.and_then(|value| value.as_bool()).unwrap_or(true),
                Err(error) => {
                    log::warn!("Reading {SHOW_UPDATES}: {}", describe(error));
                    return;
                }
            };
            let _ = this.update(cx, |this, cx| {
                if this.show_updates != show {
                    this.show_updates = show;
                    this.sift(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn fetch_news(&mut self, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move { host.get_news().await });
        cx.spawn(async move |this, cx| match task.await {
            Ok(news) => {
                let _ = this.update(cx, |this, cx| {
                    this.news = news;
                    cx.notify();
                });
            }
            Err(error) => log::warn!("Failed to get news: {}", describe(error)),
        })
        .detach();
    }

    /// Picks the account notifications the panel lists, after the
    /// notifications, the account or the setting changed.
    fn sift(&mut self, cx: &App) {
        let identity = Identity::of(self.session.read(cx).account());
        self.visible = visible_indexes(&self.account, self.show_updates, &identity);
    }

    fn visible(&self) -> impl Iterator<Item = &Notification> {
        self.visible.iter().map(|&index| &self.account[index])
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.panel = self.panel.toggled();
        if let Panel::Closing(id) = self.panel {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(motion::ENTER).await;
                let _ = this.update(cx, |this, cx| {
                    this.panel = this.panel.settled(id);
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if matches!(self.panel, Panel::Open(_)) {
            self.toggle(cx);
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self.panel, Panel::Open(_))
    }

    fn run(&mut self, op: Op, cx: &mut Context<Self>) {
        let target = op.target().to_string();
        if self.running.contains_key(&target) {
            return;
        }
        self.error = None;
        self.running.insert(target.clone(), op.clone());
        let task = Backend::global(cx).run(cx, move |host| op.perform(host));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.running.remove(&target);
                if let Err(error) = result {
                    log::error!("Notification action failed: {error:#}");
                    this.error = Some(describe(error));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Shows fixed contents, for screenshots: no host is asked.
    #[cfg(feature = "screenshot")]
    pub fn preview(
        &mut self,
        snackbars: &[(SnackbarKind, &str)],
        account: Vec<Notification>,
        news: Vec<Article>,
        cx: &mut Context<Self>,
    ) {
        self.history.clear();
        for (kind, message) in snackbars {
            self.history
                .record(*kind, Message::from(message.to_string()));
        }
        self.account = newest_first(account);
        self.news = news;
        self.show_updates = true;
        self.sift(cx);
        cx.notify();
    }

    fn render_history(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.history.entries();
        div()
            .flex_shrink_0()
            .border_b_2()
            .border_color(theme::palette().control)
            .when_some(entries.first(), |this, first| {
                this.child(
                    h_flex()
                        .my_2()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .child(history_entry(first).flex_1().min_w_0())
                        .child(
                            theme::panel_button(
                                "notifications-clear",
                                Tone::Control,
                                t("pureClear"),
                                MdIcon::Clear,
                                cx,
                            )
                            .flex_shrink_0()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.history.clear();
                                cx.notify();
                            })),
                        ),
                )
            })
            .children(
                entries
                    .iter()
                    .skip(1)
                    .map(|entry| div().my_2().child(history_entry(entry))),
            )
    }

    fn render_account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let cards = self
            .visible()
            .map(|notification| {
                let id = notification.notification_id.clone();
                let Shown { message, action } = shown(notification);
                let running = self.running.get(&id);
                let buttons = match action {
                    Action::None => None,
                    Action::Read => Some(
                        theme::panel_button(
                            SharedString::from(format!("notification-read-{id}")),
                            Tone::Positive,
                            t("read"),
                            MdIcon::MarkEmailRead,
                            cx,
                        )
                        .w_full()
                        .loading(matches!(running, Some(Op::Read { .. })))
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this, _, _, cx| {
                                this.run(
                                    Op::Read {
                                        notification_id: id.clone(),
                                    },
                                    cx,
                                )
                            }
                        }))
                        .into_any_element(),
                    ),
                    Action::Invite { modpack_id } => {
                        let answer = |accept: bool| Op::Answer {
                            modpack_id: modpack_id.clone(),
                            notification_id: id.clone(),
                            accept,
                        };
                        let button = |accept: bool, cx: &mut Context<Self>| {
                            let (key, tone, label, icon) = if accept {
                                (
                                    "notification-accept",
                                    Tone::Positive,
                                    "accept",
                                    MdIcon::Check,
                                )
                            } else {
                                (
                                    "notification-decline",
                                    Tone::Destructive,
                                    "decline",
                                    MdIcon::Clear,
                                )
                            };
                            let op = answer(accept);
                            // Two `w-full` buttons share the row in CSS;
                            // here each takes half.
                            theme::panel_button(
                                SharedString::from(format!("{key}-{id}")),
                                tone,
                                t(label),
                                icon,
                                cx,
                            )
                            .flex_1()
                            .loading(running == Some(&op))
                            .on_click(cx.listener(move |this, _, _, cx| this.run(op.clone(), cx)))
                        };
                        Some(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(button(true, cx))
                                .child(button(false, cx))
                                .into_any_element(),
                        )
                    }
                };
                v_flex()
                    .my_2()
                    .p_2()
                    .items_center()
                    .justify_center()
                    .text_center()
                    .rounded(theme::radius_card())
                    .bg(theme::palette().control)
                    .child(div().w_full().child(message))
                    .when_some(buttons, |this, buttons| {
                        this.child(h_flex().w_full().my_2().justify_center().child(buttons))
                    })
            })
            .collect::<Vec<_>>();
        div()
            .flex_shrink_0()
            .border_b_2()
            .border_color(theme::palette().control)
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .my_2()
                        .p_2()
                        .rounded(theme::radius_card())
                        .bg(theme::palette().destructive_hover)
                        .text_color(theme::palette().on_accent)
                        .child(error),
                )
            })
            .children(cards)
    }

    fn render_news(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .children(self.news.iter().map(|article| {
                let link = article.link.clone();
                v_flex()
                    .my_2()
                    .p_4()
                    .items_center()
                    .justify_center()
                    .text_center()
                    .rounded(theme::radius_card())
                    .bg(theme::palette().page)
                    .child(
                        div()
                            .w_full()
                            .font_weight(FontWeight::BLACK)
                            .child(article.title.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .font_weight(FontWeight::NORMAL)
                            .child(plain_text(&article.summary)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .p_2()
                            .rounded(theme::radius_card())
                            .bg(theme::palette().surface)
                            .child(
                                theme::panel_button(
                                    SharedString::from(format!("news-read-{}", article.guid)),
                                    Tone::Primary,
                                    t("read"),
                                    MdIcon::OpenInBrowser,
                                    cx,
                                )
                                .w_full()
                                .on_click(move |_, _, cx| cx.open_url(&link)),
                            ),
                    )
            }))
    }

    fn render_panel(
        &self,
        id: usize,
        entering: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rem = window.rem_size();
        let viewport = window.viewport_size();
        // `w-[35vw] min-w-72 h-[75vh]`, `mt-4` below the bell.
        let width = (viewport.width * 0.35).max(rems(18.) * rem);
        let height = viewport.height * 0.75;
        let top = rems(2. + 1.) * rem;
        let panel = div()
            .id("notifications-panel")
            .absolute()
            .right_0()
            .w(width)
            .h(height)
            .p_4()
            .rounded(theme::radius_card())
            .bg(theme::palette().surface)
            .font_weight(FontWeight::BOLD)
            .occlude()
            .child(
                v_flex()
                    .id("notifications-list")
                    .size_full()
                    .overflow_y_scroll()
                    .child(self.render_history(cx))
                    .child(self.render_account(cx))
                    .child(self.render_news(cx)),
            )
            .with_animation(
                (
                    if entering {
                        "notifications-enter"
                    } else {
                        "notifications-exit"
                    },
                    id,
                ),
                Animation::new(motion::ENTER),
                move |this, progress| {
                    let (offset, opacity) =
                        motion::popover(motion::ENTER.mul_f32(progress), entering);
                    this.top(top + offset).opacity(opacity)
                },
            );
        deferred(panel).with_priority(2)
    }
}

/// A remembered snackbar: its colour, its check mark if it had one, its
/// message, and how often it came up when that was more than once.
fn history_entry(entry: &HistoryEntry) -> gpui::Div {
    theme::snackbar_surface(entry.kind)
        .flex()
        .items_center()
        .justify_between()
        .child(snackbar::content(entry.kind, &entry.message))
        .when(entry.count > 1, |this| {
            this.child(
                div()
                    .ml_2()
                    .px_2()
                    .py_1()
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(theme::palette().scrim)
                    .text_sm()
                    .child(format!("{}x", entry.count)),
            )
        })
}

impl Render for Notifications {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let unread = self
            .visible()
            .filter(|notification| !notification.read)
            .count();
        let bell = theme::window_button(
            "notifications",
            Bell::of(unread, &self.news).tone(),
            MdIcon::Notifications,
            cx,
        )
        .when(unread > 0, |this| {
            this.tooltip(t_with(
                "unreadNotifications",
                &[("number", &unread.to_string())],
            ))
        })
        .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)));
        let viewport = window.viewport_size();
        div()
            .relative()
            .child(bell)
            // `PopoverBackdrop`: dims the window, and a press on it closes
            // the panel.
            .when(self.is_open(), |this| {
                this.child(
                    deferred(
                        anchored().position(gpui::Point::default()).child(
                            div()
                                .id("notifications-backdrop")
                                .w(viewport.width)
                                .h(viewport.height)
                                .bg(theme::palette().backdrop)
                                .occlude()
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _, _, cx| this.close(cx)),
                                ),
                        ),
                    )
                    .with_priority(1),
                )
            })
            .map(|this| match self.panel {
                Panel::Closed => this,
                Panel::Open(id) => this.child(self.render_panel(id, true, window, cx)),
                Panel::Closing(id) => this.child(self.render_panel(id, false, window, cx)),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(id: &str, message: &str, created_at_unix: i64) -> Notification {
        Notification {
            notification_id: id.into(),
            user_id: "u1".into(),
            notification_type: Some("generic".into()),
            resource_id: None,
            message: message.into(),
            created_at: String::new(),
            created_at_unix,
            read: false,
        }
    }

    fn sync_update(updated_by: &str) -> Notification {
        notification(
            "sync",
            &serde_json::json!({
                "notification_type": "modpack_sync",
                "simple_message": "Alpha Pack was updated",
                "updated_by": updated_by,
            })
            .to_string(),
            0,
        )
    }

    fn me() -> Identity {
        Identity::Known(vec!["me".into()])
    }

    // `snackbar.test.ts`

    #[test]
    fn a_new_snackbar_counts_once_with_a_new_id() {
        let mut history = History::default();
        history.record(SnackbarKind::Error, "hello".into());
        assert_eq!(history.entries().len(), 1);
        assert_eq!(history.entries()[0].count, 1);
    }

    #[test]
    fn a_repeat_counts_up_and_moves_to_the_end_keeping_its_id() {
        let mut history = History::default();
        history.record(SnackbarKind::Error, "a".into());
        history.record(SnackbarKind::Error, "b".into());
        let a = history.entries()[0].id;
        history.record(SnackbarKind::Error, "a".into());
        let messages: Vec<_> = history
            .entries()
            .iter()
            .map(|e| e.message.clone())
            .collect();
        assert_eq!(messages, [Message::from("b"), Message::from("a")]);
        assert_eq!(history.entries()[1].count, 2);
        assert_eq!(history.entries()[1].id, a);
    }

    #[test]
    fn the_same_message_in_another_kind_is_another_entry() {
        let mut history = History::default();
        history.record(SnackbarKind::Error, "a".into());
        history.record(SnackbarKind::Success, "a".into());
        assert_eq!(history.entries().len(), 2);
    }

    #[test]
    fn only_the_newest_entries_are_kept() {
        let mut history = History::default();
        for i in 0..HISTORY_LIMIT + 3 {
            history.record(SnackbarKind::Neutral, format!("m{i}").into());
        }
        assert_eq!(history.entries().len(), HISTORY_LIMIT);
        assert_eq!(
            history.entries().last().unwrap().message,
            Message::from(format!("m{}", HISTORY_LIMIT + 2))
        );
        history.clear();
        assert!(history.entries().is_empty());
    }

    // `Notifications.test.tsx` and the component's rules

    #[test]
    fn pushed_notifications_sort_newest_first() {
        let sorted = newest_first(vec![
            notification("older", "Older note", 1),
            notification("newer", "Newer note", 5),
        ]);
        assert_eq!(sorted[0].notification_id, "newer");
        assert_eq!(sorted[1].notification_id, "older");
    }

    #[test]
    fn a_plain_message_is_shown_as_it_is_with_a_read_button() {
        let shown = shown(&notification("n1", "You were mentioned", 2));
        assert_eq!(shown.message, "You were mentioned");
        assert_eq!(shown.action, Action::Read);
    }

    #[test]
    fn a_json_message_shows_its_simple_message() {
        let message = serde_json::json!({ "simple_message": "Short" }).to_string();
        assert_eq!(shown(&notification("n", &message, 0)).message, "Short");
    }

    #[test]
    fn a_read_notification_offers_nothing() {
        let mut read = notification("n1", "Done", 0);
        read.read = true;
        assert_eq!(shown(&read).action, Action::None);
    }

    #[test]
    fn an_invite_names_the_inviter_and_offers_an_answer() {
        let message = serde_json::json!({
            "notification_type": "invite_to_sync",
            "simple_message": "You have been invited to collaborate on a modpack by sam ",
            "invite_id": "pack-1",
        })
        .to_string();
        let shown = shown(&notification("n1", &message, 0));
        assert_eq!(
            shown.message,
            "You have been invited to collaborate on a Quadrant Sync modpack by sam"
        );
        assert_eq!(
            shown.action,
            Action::Invite {
                modpack_id: "pack-1".into()
            }
        );
        // The resource id stands in for a missing invite id.
        let mut legacy = notification(
            "n2",
            "You have been invited to collaborate on a modpack by kim",
            0,
        );
        legacy.notification_type = Some("invite_to_sync".into());
        legacy.resource_id = Some("pack-2".into());
        let shown = super::shown(&legacy);
        assert!(shown.message.ends_with("by kim"));
        assert_eq!(
            shown.action,
            Action::Invite {
                modpack_id: "pack-2".into()
            }
        );
    }

    #[test]
    fn modpack_updates_follow_the_setting_and_skip_the_accounts_own() {
        let plain = notification("n", "hi", 0);
        assert!(is_visible(&plain, false, &Identity::Checking));
        assert!(is_visible(&sync_update("Sam"), true, &me()));
        assert!(!is_visible(&sync_update("Sam"), false, &me()));
        assert!(!is_visible(&sync_update(" ME "), true, &me()));
        // Until the account is known, an update by someone might be its own.
        assert!(!is_visible(&sync_update("sam"), true, &Identity::Checking));
        assert!(is_visible(&sync_update(""), true, &Identity::Checking));
        assert!(is_visible(
            &sync_update("sam"),
            true,
            &Identity::Known(vec![])
        ));
    }

    #[test]
    fn the_visible_ones_are_found_by_index() {
        let account = [
            sync_update("Sam"),
            notification("plain", "Hello", 1),
            sync_update("me"),
        ];
        assert_eq!(visible_indexes(&account, true, &me()), [0, 1]);
        assert_eq!(visible_indexes(&account, false, &me()), [1]);
    }

    #[test]
    fn the_identity_is_the_accounts_name_and_login() {
        let info = AccountInfo {
            id: "1".into(),
            name: " Alex ".into(),
            email: String::new(),
            quadrant_sync_limit: 0,
            quadrant_share_limit: 0,
            login: "alex99".into(),
            notifications: Vec::new(),
        };
        assert_eq!(
            Identity::of(&Loadable::Ready(Some(info))),
            Identity::Known(vec!["alex".into(), "alex99".into()])
        );
        assert_eq!(Identity::of(&Loadable::Loading), Identity::Checking);
        assert_eq!(
            Identity::of(&Loadable::Ready(None)),
            Identity::Known(Vec::new())
        );
    }

    #[test]
    fn the_bell_is_red_for_unread_then_blue_for_news() {
        let article = |new| Article {
            title: String::new(),
            link: String::new(),
            summary: String::new(),
            date: chrono::DateTime::UNIX_EPOCH,
            guid: String::new(),
            new,
        };
        assert_eq!(Bell::of(1, &[article(true)]), Bell::Unread);
        assert_eq!(Bell::of(0, &[article(false), article(true)]), Bell::News);
        assert_eq!(Bell::of(0, &[article(false)]), Bell::Quiet);
        assert_eq!(Bell::of(0, &[]), Bell::Quiet);
    }

    #[test]
    fn summaries_lose_their_markup() {
        assert_eq!(
            plain_text("<p>New <b>sync</b> &amp; share</p>\n<p>Read&nbsp;more</p>"),
            "New sync & share Read more"
        );
    }

    #[test]
    fn the_panel_opens_closes_and_settles() {
        let open = Panel::Closed.toggled();
        assert_eq!(open, Panel::Open(1));
        let closing = open.toggled();
        assert_eq!(closing, Panel::Closing(1));
        assert_eq!(closing.settled(1), Panel::Closed);
        // Opened again while closing: the old exit's timer leaves it open.
        let reopened = closing.toggled();
        assert_eq!(reopened, Panel::Open(2));
        assert_eq!(reopened.settled(1), reopened);
    }
}
