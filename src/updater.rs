//! The self-updater (feature `updater`), after the desktop app's. The
//! `channel` setting, shared with the desktop app, names an app as well as a
//! release line (quadrant-host's `updates`): on a Quadrant-Native channel it
//! downloads this app's newest release; on a Quadrant channel it downloads
//! Quadrant's, to switch to it. Either is checked against its minisign
//! signature in the background, and the title bar then offers to install
//! it, which starts the installed app and quits this one.

use std::{sync::Arc, time::Duration};

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, Global, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, Subscription, Task, Window, div, rems,
};
use gpui_component::{
    IndexPath,
    button::Button,
    select::{Select, SelectEvent, SelectItem, SelectState},
    v_flex,
};
use quadrant_host::{
    app_paths,
    updates::{self, App as AppKind, Channel, Update},
};
use tokio::sync::watch;

use crate::{
    RUNNING, SingleInstance,
    assets::MdIcon,
    backend::Backend,
    config,
    i18n::{describe, t},
    snackbar::{self, SnackbarKind},
    taskbar::{self, Job},
    theme::{self, Tone},
};

const USER_AGENT: &str = concat!("QuadrantMC/quadrant-native/v", env!("CARGO_PKG_VERSION"));
/// The package name the Linux packages install under.
const PACKAGE: &str = "quadrant-native";
/// Debug builds only: an update server to ask instead of Quadrant's, for
/// testing the whole update against a local one. Plain http is allowed then.
const ENDPOINT_OVERRIDE: &str = "QUADRANT_NATIVE_UPDATE_ENDPOINT";
/// How often an idle updater looks for a release again. Closing the window
/// hides the app into the tray, where it can run for days.
const RECHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// What the title bar shows. Checking and failures show nothing, as in the
/// desktop app; they are logged.
pub enum Status {
    Idle,
    /// Downloading, `0.0..=1.0` done.
    Downloading(f32),
    Ready(Arc<Update>),
    Installing(Arc<Update>),
}

impl Status {
    /// Whether the timer should look for a release now: not while one is
    /// downloading, waiting to be installed or installing.
    fn rechecks(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// Whether a changed channel starts the check over. An install under way
    /// can't be taken back.
    fn restarts_on_channel_change(&self) -> bool {
        !matches!(self, Self::Installing(_))
    }
}

/// How this copy was installed, when it is an install that can replace
/// itself rather than a build in `target/` or a package a package manager
/// updates. On some Linux systems this asks `rpm`, so `main` calls it once.
pub fn detect() -> Option<updates::Bundle> {
    updates::detect(PACKAGE)
}

#[derive(Clone)]
struct ChannelItem(Channel);

impl SelectItem for ChannelItem {
    type Value = Channel;

    fn title(&self) -> SharedString {
        t(self.0.label_key())
    }

    fn value(&self) -> &Channel {
        &self.0
    }
}

type ChannelSelect = SelectState<Vec<ChannelItem>>;

pub struct Updater {
    check: Arc<updates::Updater>,
    status: Status,
    work: Task<()>,
    /// Asks for a release again every [`RECHECK_EVERY`].
    timer: Task<()>,
    /// The `channel` setting; `None` until read.
    channel: Option<Channel>,
    /// The settings page's picker, built on its first render.
    channel_select: Option<Entity<ChannelSelect>>,
    _channel_picked: Option<Subscription>,
}

struct GlobalUpdater(Entity<Updater>);

impl Global for GlobalUpdater {}

impl Updater {
    /// Starts checking for updates when this copy is an install that can
    /// replace itself (`bundle`, from [`detect`]), and records where it lives
    /// so Quadrant can start it. `main` skips it for `--noupdater`.
    pub fn start(bundle: Option<updates::Bundle>, cx: &mut App) {
        let Some(bundle) = bundle else {
            log::info!("Not an installed copy; the updater stays off");
            return;
        };
        let override_server = cfg!(debug_assertions)
            .then(|| std::env::var(ENDPOINT_OVERRIDE).ok())
            .flatten();
        let check = updates::Updater {
            allow_http: override_server.is_some(),
            server: override_server.unwrap_or_else(|| updates::API_BASE.to_string()),
            ..check_for(bundle)
        };
        Backend::global(cx).run_logged(cx, "record the running app", |host| async move {
            if let Some(path) = app_paths::current_executable() {
                app_paths::record(&host, RUNNING, &path)?;
            }
            Ok(())
        });
        let updater = cx.new(|cx| {
            let mut updater = Self::new(check, Status::Idle);
            updater.load_channel(cx);
            updater.check(cx);
            updater.recheck_on_a_timer(cx);
            updater
        });
        cx.set_global(GlobalUpdater(updater));
    }

    fn new(check: updates::Updater, status: Status) -> Self {
        Self {
            check: Arc::new(check),
            status,
            work: Task::ready(()),
            timer: Task::ready(()),
            channel: None,
            channel_select: None,
            _channel_picked: None,
        }
    }

    /// The running updater, when there is one.
    pub fn global(cx: &App) -> Option<Entity<Updater>> {
        cx.try_global::<GlobalUpdater>()
            .map(|global| global.0.clone())
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Looks for a release on the channel in the settings, of this app or
    /// of the one the channel switches to, and downloads it. Does nothing
    /// while a download or an install is under way.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        if matches!(self.status, Status::Downloading(_) | Status::Installing(_)) {
            return;
        }
        self.start_check(cx);
    }

    fn recheck_on_a_timer(&mut self, cx: &mut Context<Self>) {
        self.timer = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(RECHECK_EVERY).await;
                let rechecked = this.update(cx, |this, cx| {
                    if this.status.rechecks() {
                        this.check(cx);
                    }
                });
                if rechecked.is_err() {
                    break;
                }
            }
        });
    }

    /// Starts a check, dropping the one under way, so its release can't
    /// come back after a newer check has answered.
    fn start_check(&mut self, cx: &mut Context<Self>) {
        let (progress, mut progress_seen) = watch::channel(None::<f32>);
        let check = self.check.clone();
        let job = Backend::global(cx).run(cx, move |host| async move {
            let channel = Channel::read(&host);
            check
                .check(channel, |done| {
                    progress.send_replace(Some(done));
                })
                .await
        });
        self.work = cx.spawn(async move |this, cx| {
            // The job owns the sender, so this ends when the job does.
            while progress_seen.changed().await.is_ok() {
                let Some(done) = *progress_seen.borrow_and_update() else {
                    continue;
                };
                let _ = this.update(cx, |this, cx| {
                    taskbar::progress(cx, Job::Update, done.into());
                    let shown = match this.status {
                        Status::Downloading(shown) => (shown * 100.).round(),
                        _ => -1.,
                    };
                    this.status = Status::Downloading(done);
                    if (done * 100.).round() != shown {
                        cx.notify();
                    }
                });
            }
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                taskbar::clear(cx, Job::Update);
                this.status = match result {
                    Ok(Some(update)) => Status::Ready(Arc::new(update)),
                    Ok(None) => Status::Idle,
                    Err(error) => {
                        log::error!("Update failed: {error:#}");
                        Status::Idle
                    }
                };
                cx.notify();
            });
        });
    }

    fn load_channel(&mut self, cx: &mut Context<Self>) {
        let task = Backend::global(cx).run(cx, |host| async move { Ok(Channel::read(&host)) });
        cx.spawn(async move |this, cx| {
            if let Ok(channel) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.channel = Some(channel);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Saves the channel and checks it for a release, as the desktop app does.
    fn set_channel(&mut self, channel: Channel, cx: &mut Context<Self>) {
        self.channel = Some(channel);
        let restarts = self.status.restarts_on_channel_change();
        if restarts {
            // What was found or is downloading belongs to the old channel.
            self.work = Task::ready(());
            taskbar::clear(cx, Job::Update);
            self.status = Status::Idle;
            cx.notify();
        }
        let task = Backend::global(cx).run(cx, move |host| async move {
            config::set(&host, updates::CHANNEL_KEY, channel.as_str().into())
        });
        cx.spawn(async move |this, cx| match task.await {
            Ok(()) if restarts => {
                let _ = this.update(cx, |this, cx| this.start_check(cx));
            }
            Ok(()) => {}
            Err(error) => cx.update(|cx| snackbar::show(cx, SnackbarKind::Error, describe(error))),
        })
        .detach();
    }

    fn channel_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ChannelSelect>> {
        if let Some(select) = &self.channel_select {
            return Some(select.clone());
        }
        let channel = self.channel?;
        let row = Channel::ALL.iter().position(|each| *each == channel);
        let select = cx.new(|cx| {
            let items = Channel::ALL.map(ChannelItem).to_vec();
            SelectState::new(
                items,
                row.map(|row| IndexPath::default().row(row)),
                window,
                cx,
            )
        });
        self._channel_picked = Some(cx.subscribe(
            &select,
            |this, _, event: &SelectEvent<Vec<ChannelItem>>, cx| {
                if let SelectEvent::Confirm(Some(channel)) = event {
                    this.set_channel(*channel, cx);
                }
            },
        ));
        self.channel_select = Some(select.clone());
        Some(select)
    }

    /// Installs the downloaded release and quits; the installed app starts
    /// in this one's place.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Status::Ready(update) = &self.status else {
            return;
        };
        let update = update.clone();
        self.status = Status::Installing(update.clone());
        cx.notify();
        log::info!("Installing {} {}", update.app.name(), update.version);
        // The installed app claims the single instance socket as it starts.
        let serving = cx
            .try_global::<SingleInstance>()
            .map(|single| single.0.clone());
        let job = Backend::global(cx).run(cx, {
            let update = update.clone();
            move |_| async move {
                tokio::task::spawn_blocking(move || {
                    update.install(|| {
                        if let Some(serving) = serving {
                            serving.release();
                        }
                    })
                })
                .await?
            }
        });
        self.work = cx.spawn(async move |this, cx| match job.await {
            Ok(()) => cx.update(|cx| cx.quit()),
            Err(error) => {
                log::error!("Installing the update failed: {error:#}");
                let _ = this.update(cx, |this, cx| {
                    this.status = Status::Ready(update);
                    snackbar::show(cx, SnackbarKind::Error, describe(error));
                    cx.notify();
                });
            }
        });
    }

    /// Shows `status` without asking any server, for screenshots.
    #[cfg(feature = "screenshot")]
    pub fn preview(status: Status, cx: &mut App) {
        let updater = cx.new(|_| Self {
            channel: Some(Channel::GpuiPreview),
            ..Self::new(check_for(updates::Bundle::Nsis), status)
        });
        cx.set_global(GlobalUpdater(updater));
    }

    /// A ready release of `app`, for screenshots.
    #[cfg(feature = "screenshot")]
    pub fn preview_update(app: AppKind, version: &str) -> Arc<Update> {
        Arc::new(Update::preview(app, RUNNING, version))
    }
}

/// Checks against Quadrant's update server for this copy.
fn check_for(bundle: updates::Bundle) -> updates::Updater {
    updates::Updater {
        running: RUNNING,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        bundle,
        server: updates::API_BASE.to_string(),
        allow_http: false,
        user_agent: USER_AGENT.to_string(),
        pubkey: updates::PUBKEY.to_string(),
    }
}

/// The ready button's label: an update of this app, or the app it switches
/// to.
fn install_label(update: &Update) -> SharedString {
    match update.switches_to() {
        None => t("appUpdate"),
        Some(AppKind::Quadrant) => t("switchToQuadrant"),
        Some(AppKind::QuadrantNative) => t("switchToQuadrantNative"),
    }
}

/// The title bar's update button, as in the desktop app: the download's
/// progress, then "Install an update" (or "Switch to Quadrant") once it is
/// ready.
pub fn title_bar_button(cx: &App) -> Option<AnyElement> {
    let updater = Updater::global(cx)?;
    let button = match updater.read(cx).status() {
        Status::Idle => return None,
        Status::Downloading(done) => theme::pill_button(
            "app-update",
            Tone::Control,
            format!("{:.0}%", done * 100.),
            None,
            cx,
        ),
        Status::Installing(update) => ready_button(install_label(update), cx).loading(true),
        Status::Ready(update) => ready_button(install_label(update), cx)
            .on_click(move |_, _, cx| updater.update(cx, |updater, cx| updater.install(cx))),
    };
    Some(button.into_any_element())
}

fn ready_button(label: SharedString, cx: &App) -> Button {
    theme::pill_button(
        "app-update",
        Tone::Positive,
        label.clone(),
        Some(MdIcon::InstallDesktop),
        cx,
    )
    // The label is a child element, so screen readers need the name.
    .accessibility_label(label)
}

/// The settings page's update channel field, shown while the updater runs,
/// as the desktop app shows it only when it can update itself.
pub fn channel_setting(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let updater = Updater::global(cx)?;
    let select = updater.update(cx, |updater, cx| updater.channel_select(window, cx))?;
    let field = v_flex()
        .my_4()
        .font_weight(FontWeight::BOLD)
        .child(t("updateChannel"))
        .child(
            div().my_4().w_full().max_w(rems(20.)).child(
                Select::new(&select)
                    .appearance(false)
                    .h_10()
                    .rounded_full()
                    .bg(theme::palette().control),
            ),
        );
    Some(field.into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update() -> Arc<Update> {
        Arc::new(Update::preview(AppKind::QuadrantNative, RUNNING, "26.12.0"))
    }

    #[test]
    fn the_timer_only_checks_for_an_idle_updater() {
        assert!(Status::Idle.rechecks());
        assert!(!Status::Downloading(0.5).rechecks());
        assert!(!Status::Ready(update()).rechecks());
        assert!(!Status::Installing(update()).rechecks());
    }

    #[test]
    fn a_new_channel_drops_a_download_or_a_ready_release_but_not_an_install() {
        assert!(Status::Idle.restarts_on_channel_change());
        assert!(Status::Downloading(0.4).restarts_on_channel_change());
        assert!(Status::Ready(update()).restarts_on_channel_change());
        assert!(!Status::Installing(update()).restarts_on_channel_change());
    }
}
