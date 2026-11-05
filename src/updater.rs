//! The self-updater (feature `updater`), after the desktop app's: it asks
//! Quadrant's update server for `quadrant-native` on the chosen channel,
//! downloads the release in the background and checks its minisign
//! signature. The title bar then offers to install it, which replaces this
//! copy and starts the new one.

mod install;
mod protocol;

use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, Global, IntoElement as _,
    ParentElement as _, SharedString, Styled as _, Subscription, Task, Window, div, rems, rgb,
};
use gpui_component::{
    IndexPath,
    select::{Select, SelectEvent, SelectItem, SelectState},
    v_flex,
};
use quadrant_host::QuadrantHost;
use reqwest::StatusCode;
use tokio::sync::watch;

use crate::{
    assets::MdIcon,
    backend::Backend,
    config,
    i18n::{describe, t},
    snackbar::{self, SnackbarKind},
    theme::{self, Tone},
};
use protocol::{Bundle, Release};

use protocol::DEFAULT_CHANNEL;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const USER_AGENT: &str = concat!("QuadrantMC/quadrant-native/v", env!("CARGO_PKG_VERSION"));
/// Debug builds only: an update server to ask instead of Quadrant's, for
/// testing the whole update against a local one. Plain http is allowed then.
const ENDPOINT_OVERRIDE: &str = "QUADRANT_NATIVE_UPDATE_ENDPOINT";

/// What the title bar shows. Checking and failures show nothing, as in the
/// desktop app; they are logged.
pub enum Status {
    Idle,
    /// Downloading, `0.0..=1.0` done.
    Downloading(f32),
    Ready(Arc<Update>),
    Installing,
}

/// The channels the settings offer, as in the desktop app.
const CHANNELS: [&str; 2] = ["stable", "preview"];

#[derive(Clone)]
struct ChannelItem(&'static str);

impl SelectItem for ChannelItem {
    type Value = &'static str;

    fn title(&self) -> SharedString {
        t(self.0)
    }

    fn value(&self) -> &&'static str {
        &self.0
    }
}

type ChannelSelect = SelectState<Vec<ChannelItem>>;

/// A downloaded release whose signature checked out.
pub struct Update {
    pub version: String,
    bytes: Vec<u8>,
}

pub struct Updater {
    bundle: Bundle,
    /// The update server, without the `/api/...` path.
    server: String,
    allow_http: bool,
    status: Status,
    work: Task<()>,
    /// The `channel` setting, shared with the desktop app; `None` until read.
    channel: Option<String>,
    /// The settings page's picker, built on its first render.
    channel_select: Option<Entity<ChannelSelect>>,
    _channel_picked: Option<Subscription>,
}

struct GlobalUpdater(Entity<Updater>);

impl Global for GlobalUpdater {}

impl Updater {
    /// Starts checking for updates when this copy is an install that can
    /// replace itself. `main` skips it for `--noupdater`.
    pub fn start(cx: &mut App) {
        let Some(bundle) = install::detect() else {
            log::info!("Not an installed copy; the updater stays off");
            return;
        };
        let override_server = cfg!(debug_assertions)
            .then(|| std::env::var(ENDPOINT_OVERRIDE).ok())
            .flatten();
        let updater = cx.new(|cx| {
            let mut updater = Self {
                bundle,
                allow_http: override_server.is_some(),
                server: override_server.unwrap_or_else(|| protocol::API_BASE.to_string()),
                status: Status::Idle,
                work: Task::ready(()),
                channel: None,
                channel_select: None,
                _channel_picked: None,
            };
            updater.load_channel(cx);
            updater.check(cx);
            updater
        });
        cx.set_global(GlobalUpdater(updater));
    }

    /// The running updater, when there is one.
    pub fn global(cx: &App) -> Option<Entity<Updater>> {
        cx.try_global::<GlobalUpdater>()
            .map(|global| global.0.clone())
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Looks for a release on the channel in the settings and downloads it.
    /// Does nothing while a download or an install is under way.
    pub fn check(&mut self, cx: &mut Context<Self>) {
        if matches!(self.status, Status::Downloading(_) | Status::Installing) {
            return;
        }
        let (progress, mut progress_seen) = watch::channel(None::<f32>);
        let (server, bundle, allow_http) = (self.server.clone(), self.bundle, self.allow_http);
        let job = Backend::global(cx).run(cx, move |host| async move {
            let channel = read_channel(&host)?;
            let urls = protocol::endpoints(&server, &channel, bundle, CURRENT_VERSION);
            let client = reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .connect_timeout(Duration::from_secs(30))
                .build()?;
            let Some(release) = fetch_release(&client, &urls).await? else {
                log::info!("No releases on the update server");
                return Ok(None);
            };
            if !release.is_update_for(CURRENT_VERSION) {
                log::info!("Quadrant-Native {CURRENT_VERSION} is up to date");
                return Ok(None);
            }
            release.check_url(allow_http)?;
            log::info!(
                "Downloading Quadrant-Native {} from {}",
                release.version,
                release.url
            );
            let bytes = download(&client, &release.url, &progress).await?;
            protocol::verify(&bytes, &release.signature, protocol::PUBKEY)?;
            log::info!(
                "Quadrant-Native {} is downloaded and its signature checks out",
                release.version
            );
            Ok(Some(Update {
                version: release.version,
                bytes,
            }))
        });
        self.work = cx.spawn(async move |this, cx| {
            // The job owns the sender, so this ends when the job does.
            while progress_seen.changed().await.is_ok() {
                let Some(done) = *progress_seen.borrow_and_update() else {
                    continue;
                };
                let _ = this.update(cx, |this, cx| {
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
        let task = Backend::global(cx).run(cx, |host| async move { read_channel(&host) });
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
    fn set_channel(&mut self, channel: &'static str, cx: &mut Context<Self>) {
        self.channel = Some(channel.to_string());
        let task = Backend::global(cx).run(cx, move |host| async move {
            config::set(&host, "channel", channel.into())
        });
        cx.spawn(async move |this, cx| match task.await {
            Ok(()) => {
                let _ = this.update(cx, |this, cx| this.check(cx));
            }
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
        let channel = self.channel.as_deref()?;
        let row = CHANNELS.iter().position(|each| *each == channel);
        let select = cx.new(|cx| {
            let items = CHANNELS.map(ChannelItem).to_vec();
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
                    this.set_channel(channel, cx);
                }
            },
        ));
        self.channel_select = Some(select.clone());
        Some(select)
    }

    /// Installs the downloaded update and quits; the new version starts in
    /// this one's place.
    pub fn install(&mut self, cx: &mut Context<Self>) {
        let Status::Ready(update) = &self.status else {
            return;
        };
        let update = update.clone();
        let bundle = self.bundle;
        self.status = Status::Installing;
        cx.notify();
        log::info!("Installing Quadrant-Native {}", update.version);
        let job = Backend::global(cx).run(cx, {
            let update = update.clone();
            move |_| async move {
                tokio::task::spawn_blocking(move || install::apply(bundle, &update.bytes)).await?
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
            bundle: Bundle::Nsis,
            server: String::new(),
            allow_http: false,
            status,
            work: Task::ready(()),
            channel: Some(DEFAULT_CHANNEL.to_string()),
            channel_select: None,
            _channel_picked: None,
        });
        cx.set_global(GlobalUpdater(updater));
    }

    #[cfg(feature = "screenshot")]
    pub fn preview_update(version: &str) -> Arc<Update> {
        Arc::new(Update {
            version: version.to_string(),
            bytes: Vec::new(),
        })
    }
}

/// The title bar's update button, as in the desktop app: the download's
/// progress, then "Install an update" once it is ready.
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
        status @ (Status::Ready(_) | Status::Installing) => theme::pill_button(
            "app-update",
            Tone::Positive,
            t("appUpdate"),
            Some(MdIcon::InstallDesktop),
            cx,
        )
        // The label is a child element, so screen readers need the name.
        .accessibility_label(t("appUpdate"))
        .loading(matches!(status, Status::Installing))
        .on_click(move |_, _, cx| updater.update(cx, |updater, cx| updater.install(cx))),
    };
    Some(button.into_any_element())
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
                    .bg(rgb(theme::CONTROL)),
            ),
        );
    Some(field.into_any_element())
}

fn read_channel(host: &QuadrantHost) -> Result<String> {
    Ok(host
        .get_config_value("channel")?
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_CHANNEL.to_string()))
}

/// The first endpoint's answer, falling back to the next on failure, as
/// Tauri's updater does. 204 and 404 mean there is nothing to install.
async fn fetch_release(client: &reqwest::Client, urls: &[String]) -> Result<Option<Release>> {
    let mut failure = anyhow!("no update endpoints");
    for url in urls {
        log::info!("Checking for updates at {url}");
        match client
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
        {
            Ok(response)
                if matches!(
                    response.status(),
                    StatusCode::NO_CONTENT | StatusCode::NOT_FOUND
                ) =>
            {
                return Ok(None);
            }
            Ok(response) if response.status().is_success() => {
                return Release::parse(&response.bytes().await?).map(Some);
            }
            Ok(response) => failure = anyhow!("{url} answered {}", response.status()),
            Err(error) => failure = error.into(),
        }
    }
    Err(failure)
}

async fn download(
    client: &reqwest::Client,
    url: &str,
    progress: &watch::Sender<Option<f32>>,
) -> Result<Vec<u8>> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    let total = response.content_length().filter(|total| *total > 0);
    let mut bytes = Vec::with_capacity(total.unwrap_or_default() as usize);
    progress.send_replace(Some(0.));
    while let Some(chunk) = response.chunk().await? {
        bytes.extend_from_slice(&chunk);
        if let Some(total) = total {
            progress.send_replace(Some(bytes.len() as f32 / total as f32));
        }
    }
    Ok(bytes)
}
