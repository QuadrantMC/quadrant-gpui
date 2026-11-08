// No console window on Windows; `console` explains where output goes.
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod assets;
mod autostart;
mod backend;
mod color_theme;
mod config;
mod console;
mod deep_link;
mod events;
mod i18n;
mod loadable;
mod mod_catalog;
mod motion;
mod notifications;
mod oauth;
mod pages;
#[cfg(feature = "perf")]
mod perf;
mod prefs;
mod route;
#[cfg(feature = "screenshot")]
mod screenshot;
mod session;
mod snackbar;
mod taskbar;
mod text_cache;
mod theme;
mod tray;
#[cfg(feature = "updater")]
mod updater;

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowHandle, WindowOptions,
    point, px, size,
};
use gpui_component::Root;
use quadrant_host::{
    QuadrantHost, QuadrantHostOptions,
    app_paths::{self, Launch},
    single_instance::{self, Claim, Serving},
    updates,
};

use crate::{app::QuadrantApp, backend::Backend, prefs::Prefs};

/// The desktop app's Tauri identifier, whose data folder this app shares.
const APP_IDENTIFIER: &str = "dev.mrquantumoff.mcmodpackmanager";
/// This app's own identity: its installers, Start menu entry and taskbar
/// group sit beside the desktop app's instead of replacing them.
const PRODUCT_NAME: &str = "Quadrant-Native";
const BUNDLE_IDENTIFIER: &str = "dev.mrquantumoff.quadrant-native";
/// The Wayland app id. Desktops match it to `quadrant-native.desktop` for
/// the window's name and icon, so it is the binary's name, as the packages
/// name the desktop file.
const WAYLAND_APP_ID: &str = "quadrant-native";

#[derive(Debug, Default, PartialEq)]
struct Args {
    data_dir: Option<PathBuf>,
    mc_folder: Option<PathBuf>,
    mode: Mode,
    /// `--noupdater`, as the desktop app takes it: the package manager
    /// (AUR, Flatpak) owns updates.
    no_updater: bool,
    /// `--debug` or `--log`: a console on Windows, and verbose logs.
    debug: bool,
    /// `--autostart`, as the login item launches it: start in the tray.
    autostart: bool,
    /// Links to open, as the desktop app takes them: every argument that
    /// isn't a flag.
    urls: Vec<String>,
}

/// Who drives the window.
#[derive(Debug, Default, PartialEq)]
enum Mode {
    /// The user.
    #[default]
    Interactive,
    /// Screenshot mode, rendering into this folder (feature `screenshot`).
    Screenshot(PathBuf),
    /// Perf mode (feature `perf`).
    #[cfg(feature = "perf")]
    Perf(perf::Plan),
}

/// Flags win over the environment, which wins over the defaults.
fn parse_args(
    args: impl IntoIterator<Item = OsString>,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<Args> {
    let mut data_dir = env("QUADRANT_GPUI_DATA_DIR");
    let mut mc_folder = env("QUADRANT_GPUI_MC_FOLDER");
    let mut screenshot = None;
    let [mut perf, mut perf_out, mut perf_runs, mut perf_placement] = [None, None, None, None];
    let [mut no_updater, mut debug, mut autostart] = [false; 3];
    let mut urls = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--noupdater" {
            no_updater = true;
            continue;
        }
        if arg == "--autostart" {
            autostart = true;
            continue;
        }
        if !arg.to_string_lossy().starts_with('-') {
            urls.push(arg.to_string_lossy().into_owned());
            continue;
        }
        if is_debug_flag(&arg) {
            debug = true;
            continue;
        }
        let slot = match arg.to_str() {
            Some("--data-dir") => &mut data_dir,
            Some("--mc-folder") => &mut mc_folder,
            Some("--screenshot") if cfg!(feature = "screenshot") => &mut screenshot,
            Some("--perf") if cfg!(feature = "perf") => &mut perf,
            Some("--perf-out") if cfg!(feature = "perf") => &mut perf_out,
            Some("--perf-runs") if cfg!(feature = "perf") => &mut perf_runs,
            Some("--perf-placement") if cfg!(feature = "perf") => &mut perf_placement,
            _ => bail!("unknown argument {}", arg.to_string_lossy()),
        };
        let value = args
            .next()
            .ok_or_else(|| anyhow!("{} needs a value", arg.to_string_lossy()))?;
        *slot = Some(value);
    }
    let parsed = Args {
        data_dir: data_dir.map(PathBuf::from),
        mc_folder: mc_folder.map(PathBuf::from),
        mode: match (screenshot, perf) {
            (None, None) => Mode::Interactive,
            (Some(dir), None) => Mode::Screenshot(dir.into()),
            #[cfg(feature = "perf")]
            (None, Some(scenario)) => Mode::Perf(perf::Plan::parse(
                &scenario.to_string_lossy(),
                perf_out.map(PathBuf::from),
                perf_runs.as_deref().and_then(|runs| runs.to_str()),
                perf_placement
                    .as_deref()
                    .and_then(|placement| placement.to_str()),
            )?),
            _ => bail!("--screenshot and --perf can't run together"),
        },
        no_updater,
        debug,
        autostart,
        urls,
    };
    #[cfg(not(feature = "perf"))]
    let _ = (perf_out, perf_runs, perf_placement);
    // Perf mode measures on fixtures and must never touch real data.
    #[cfg(feature = "perf")]
    if matches!(parsed.mode, Mode::Perf(_))
        && (parsed.data_dir.is_none() || parsed.mc_folder.is_none())
    {
        bail!("--perf needs --data-dir and --mc-folder, pointing at a sandbox");
    }
    // The host saves the Minecraft folder as the data folder's `mcFolder`
    // setting, so without a data folder of its own it would rewrite the
    // desktop app's.
    if parsed.mc_folder.is_some() && parsed.data_dir.is_none() {
        bail!(
            "--mc-folder (or QUADRANT_GPUI_MC_FOLDER) needs --data-dir (or QUADRANT_GPUI_DATA_DIR) too, or it would change the desktop app's Minecraft folder setting"
        );
    }
    Ok(parsed)
}

fn is_debug_flag(arg: &OsStr) -> bool {
    arg == "--debug" || arg == "--log"
}

/// The OS keyring service the Quadrant ID sign-in is kept under. The desktop
/// app's data folder shares the desktop app's sign-in; any other data folder
/// gets one of its own, so a sandbox never acts as the user's real account.
fn keyring_service(custom_data_dir: Option<&Path>) -> Option<String> {
    custom_data_dir.map(|dir| format!("{APP_IDENTIFIER}@{}", dir.display()))
}

/// The Quadrant ID client and API key baked in at build time, or none at all
/// while screenshot or perf mode drives the window: it must never sign in,
/// sync or share for real.
fn credentials(mode: &Mode) -> [&'static str; 3] {
    if *mode != Mode::Interactive {
        return ["", "", ""];
    }
    [
        option_env!("QUADRANT_OAUTH2_CLIENT_ID").unwrap_or_default(),
        option_env!("QUADRANT_OAUTH2_CLIENT_SECRET").unwrap_or_default(),
        option_env!("QUADRANT_API_KEY").unwrap_or_default(),
    ]
}

/// Whether startup reports usage, as the desktop app's setup does (the host
/// sends nothing unless data collection is on): never from a screenshot,
/// perf or test run, a sandbox's `--data-dir`, or a build without the API
/// key.
fn reports_usage(args: &Args, background: bool) -> bool {
    args.mode == Mode::Interactive
        && args.data_dir.is_none()
        && !background
        && !credentials(&args.mode)[2].is_empty()
}

/// What the desktop app's setup starts in the background (`lib.rs`): the
/// usage report and a refresh of the Minecraft version list, which the
/// pages then read from the host's cache.
fn start_background_jobs(args: &Args, cx: &mut App) {
    if args.mode != Mode::Interactive {
        return;
    }
    let backend = Backend::global(cx);
    if reports_usage(args, background_window()) {
        backend
            .run(cx, |host| async move {
                if let Err(error) = host.send_telemetry().await {
                    log::warn!("Failed to send telemetry: {error:#}");
                }
                Ok(())
            })
            .detach();
    }
    backend
        .run(cx, |host| async move {
            match host.get_versions().await {
                Ok(versions) => log::debug!("Refreshed {} Minecraft versions", versions.len()),
                Err(error) => log::error!("Failed to update Minecraft versions: {error:#}"),
            }
            Ok(())
        })
        .detach();
}

/// The data folder in use: `--data-dir`, else the desktop app's.
fn data_folder(custom: Option<&Path>) -> Result<PathBuf> {
    match custom {
        Some(dir) => Ok(std::path::absolute(dir)?),
        None => Ok(dirs::data_dir()
            .ok_or_else(|| anyhow!("no data folder on this system; pass --data-dir"))?
            .join(APP_IDENTIFIER)),
    }
}

fn build_host(
    data_dir: Option<PathBuf>,
    mc_folder: Option<PathBuf>,
    [client_id, client_secret, api_key]: [&str; 3],
) -> Result<QuadrantHost> {
    let keyring_service = keyring_service(data_dir.as_deref());
    let data_dir = match data_dir {
        Some(data_dir) => data_dir,
        None => data_folder(None)?,
    };
    let mut options = QuadrantHostOptions::new(data_dir, client_id, client_secret, api_key);
    // The host persists this as the `mcFolder` setting in the data folder.
    options.mc_folder = mc_folder;
    if let Some(service) = keyring_service {
        options.keyring_service_name = service;
    }
    let host = QuadrantHost::new(options)?;
    host.init_config()?;
    Ok(host)
}

/// This app, of the two sharing the data folder.
const RUNNING: updates::App = updates::App::QuadrantNative;

/// Whether this launch follows the `channel` setting to Quadrant when it
/// names Quadrant: an interactive run of an installed copy that updates
/// itself, on the shared data folder.
fn follows_channel(args: &Args, installed: bool) -> bool {
    installed && args.mode == Mode::Interactive && args.data_dir.is_none() && !args.no_updater
}

/// A launch that runs, rather than handing its links to a running one.
struct Running {
    backend: Backend,
    prefs: Prefs,
    themes: color_theme::Themes,
    args: Args,
    /// Where later launches reach this one; `None` in screenshot and perf
    /// runs, or when the socket failed.
    primary: Option<single_instance::Primary>,
    /// The login item for this data folder, if the platform has one.
    autostart: Option<autostart::Autostart>,
}

/// This instance's hold on the single instance socket, which the updater
/// releases before it starts the installed app.
pub struct SingleInstance(pub Serving);

impl gpui::Global for SingleInstance {}

/// `None` when Quadrant or another instance took the launch and this one is
/// done.
fn launch(raw_args: Vec<OsString>, launched: Launch) -> Result<Option<Running>> {
    let args = parse_args(raw_args.clone(), |key| std::env::var_os(key))?;
    let data_dir = data_folder(args.data_dir.as_deref())?;
    // Before the host starts, so a launch handed on leaves the config alone.
    #[cfg(feature = "updater")]
    let installed = updater::installed();
    #[cfg(not(feature = "updater"))]
    let installed = false;
    if follows_channel(&args, installed)
        && app_paths::redirect(&data_dir, RUNNING, &raw_args, launched)
    {
        return Ok(None);
    }
    let key = single_instance::key(&data_dir);
    let handoff = launched == Launch::Handoff;
    let claim = (args.mode == Mode::Interactive)
        .then(|| single_instance::claim(&data_dir, &args.urls, handoff));
    let primary = match claim {
        Some(Ok(Claim::Forwarded)) => return Ok(None),
        Some(Ok(Claim::Primary(primary))) => Ok(Some(primary)),
        Some(Err(error)) => Err(error),
        None => Ok(None),
    };
    let (backend, prefs, themes) = start(&args)?;
    // The host has set up logging by now.
    let primary = primary.unwrap_or_else(|error| {
        log::warn!("Later launches will start instances of their own: {error:#}");
        None
    });
    let mc_folder = args
        .mc_folder
        .as_deref()
        .map(std::path::absolute)
        .transpose()?;
    let sandbox =
        args.data_dir
            .is_some()
            .then_some((key.as_str(), data_dir.as_path(), mc_folder.as_deref()));
    let autostart = autostart::Autostart::new(sandbox)
        .inspect_err(|error| log::warn!("No launch at login: {error:#}"))
        .ok();
    Ok(Some(Running {
        backend,
        prefs,
        themes,
        args,
        primary,
        autostart,
    }))
}

fn start(args: &Args) -> Result<(Backend, Prefs, color_theme::Themes)> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    let host = build_host(
        args.data_dir.clone(),
        args.mc_folder.clone(),
        credentials(&args.mode),
    )?;
    log::debug!(
        "Quadrant-Native {} started with {args:?}",
        env!("CARGO_PKG_VERSION")
    );
    let prefs = Prefs::read(&host);
    let themes = color_theme::Themes::scan(&color_theme::folder(&host.options().data_dir));
    Ok((Backend::new(host, runtime), prefs, themes))
}

/// Debug builds only: `QUADRANT_NATIVE_BACKGROUND` opens the window without
/// taking focus, for test runs that need a real window (the update
/// end-to-end test drives it through UI Automation) but must not take over
/// the user's keyboard.
fn background_window() -> bool {
    cfg!(debug_assertions) && std::env::var_os("QUADRANT_NATIVE_BACKGROUND").is_some()
}

/// Screenshot mode's window never shows or takes focus, so the user's clicks
/// can't reach it.
///
/// `--autostart` opens it hidden too, for the tray to show later; Wayland
/// can't hide a window, so there the tray closes it instead.
fn open_main_window(mode: &Mode, autostart: bool, cx: &mut App) -> Result<WindowHandle<Root>> {
    let hidden = matches!(mode, Mode::Screenshot(_)) || (autostart && !cfg!(target_os = "linux"));
    let background = background_window();
    let bounds = Bounds::centered(None, size(px(1150.), px(720.)), cx);
    // With `nativeDecorations` the OS draws the title bar; macOS keeps its
    // traffic lights either way. Only Linux can change this later.
    let native = Prefs::global(cx).native_decorations;
    #[cfg_attr(not(feature = "perf"), expect(unused_mut))]
    let mut options = WindowOptions {
        // Unless the OS draws it, the title bar is drawn by the app; the
        // title still names the window for the taskbar and window switchers.
        titlebar: Some(TitlebarOptions {
            title: Some(PRODUCT_NAME.into()),
            appears_transparent: !native,
            traffic_light_position: (!native).then(|| point(px(12.), px(12.))),
        }),
        window_decorations: Some(prefs::window_decorations(&Prefs::global(cx))),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(800.), px(500.))),
        show: !hidden,
        focus: !hidden && !background,
        app_id: Some(WAYLAND_APP_ID.into()),
        ..Default::default()
    };
    // Perf mode times the real swap chain at the display's own resolution:
    // maximized when visible; off screen, perf mode places the window once
    // it exists and shows it without activating it. gpui caps an unfocused
    // window's animations at 30 fps; the window the user works in has focus,
    // and a measured one can't count on it.
    #[cfg(feature = "perf")]
    if let Mode::Perf(plan) = mode {
        match plan.placement {
            perf::Placement::Visible => {
                options.window_bounds = Some(WindowBounds::Maximized(bounds))
            }
            perf::Placement::Offscreen => {
                options.show = false;
                options.focus = false;
            }
        }
        options.inactive_frame_interval = None;
    }
    let handle = cx.open_window(options, |window, cx| {
        let view = cx.new(QuadrantApp::new);
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    Ok(handle)
}

fn main() -> ExitCode {
    // Before any thread starts: it edits the environment.
    let launched = Launch::take();
    let raw_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    // Before parsing, so a bad argument is reported in the console too.
    let console = console::Console::setup(raw_args.iter().any(|arg| is_debug_flag(arg)));
    let (backend, prefs, themes, args, primary, autostart) = match launch(raw_args, launched) {
        Ok(Some(Running {
            backend,
            prefs,
            themes,
            args,
            primary,
            autostart,
        })) => (backend, prefs, themes, args, primary, autostart),
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            console.fatal(&format!("{error:#}"));
            return ExitCode::FAILURE;
        }
    };
    // Links from this launch, from later launches and from macOS's open
    // events, in the order they come.
    let (links, inbox) = futures::channel::mpsc::unbounded::<Vec<String>>();
    let app = gpui::Application::with_platform(text_cache::platform(
        gpui_platform::current_platform(false),
    ))
    .with_assets(assets::Assets);
    if args.mode == Mode::Interactive {
        let links = links.clone();
        app.on_open_urls(move |urls| {
            let _ = links.unbounded_send(urls);
        });
    }
    app.run(move |cx| {
        cx.set_app_identity(BUNDLE_IDENTIFIER, PRODUCT_NAME);
        gpui_component::init(cx);
        theme::install(cx);
        cx.set_http_client(backend.http_client());
        cx.set_global(backend);
        if let Some(autostart) = autostart {
            cx.set_global(autostart);
        }
        cx.set_global(themes);
        prefs.install(cx);
        app::bind_zoom_keys(cx);
        snackbar::install(cx);
        notifications::install(args.mode == Mode::Interactive, cx);
        events::forward(cx);
        #[cfg(feature = "updater")]
        if args.mode == Mode::Interactive && !args.no_updater {
            updater::Updater::start(cx);
        }
        start_background_jobs(&args, cx);
        app::restore_last_page(args.mode == Mode::Interactive, cx);
        let window = match open_main_window(&args.mode, args.autostart, cx) {
            Ok(window) => window,
            Err(error) => {
                console.fatal(&format!("failed to open the window: {error:#}"));
                std::process::exit(1);
            }
        };
        if !background_window() && !args.autostart {
            cx.activate(true);
        }
        match args.mode {
            Mode::Interactive => {
                taskbar::install(window, cx);
                // Test runs with a real window keep out of the user's tray.
                if !background_window() {
                    tray::install(
                        window,
                        |cx| open_main_window(&Mode::Interactive, false, cx),
                        cx,
                    );
                }
                tray::watch_close(window, cx);
                if args.autostart {
                    tray::start_hidden(window, cx);
                }
                if !args.urls.is_empty() {
                    let _ = links.unbounded_send(args.urls.clone());
                }
                if let Some(primary) = primary {
                    let serving = primary.serve(move |urls| {
                        let _ = links.unbounded_send(urls);
                    });
                    cx.set_global(SingleInstance(serving));
                }
                app::open_links(inbox, window, cx);
            }
            #[cfg(feature = "screenshot")]
            Mode::Screenshot(dir) => screenshot::run(window, dir, cx),
            #[cfg(not(feature = "screenshot"))]
            Mode::Screenshot(_) => {}
            #[cfg(feature = "perf")]
            Mode::Perf(plan) => perf::run(window, plan, cx),
        }
    });
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn flags_override_the_environment() {
        let env = |key: &str| (key == "QUADRANT_GPUI_DATA_DIR").then(|| OsString::from("env-data"));
        let parsed =
            parse_args(args(&["--data-dir", "flag-data", "--mc-folder", "mc"]), env).unwrap();
        assert_eq!(
            parsed,
            Args {
                data_dir: Some("flag-data".into()),
                mc_folder: Some("mc".into()),
                mode: Mode::Interactive,
                no_updater: false,
                debug: false,
                autostart: false,
                urls: Vec::new(),
            }
        );
    }

    #[test]
    fn links_are_the_arguments_that_arent_flags() {
        let parsed = parse_args(
            args(&[
                "--autostart",
                "modrinth://mod/sodium",
                "--data-dir",
                "data",
                "quadrantnext://modpack?code=1234567",
            ]),
            |_| None,
        )
        .unwrap();
        assert!(parsed.autostart);
        assert_eq!(parsed.data_dir, Some("data".into()));
        assert_eq!(
            parsed.urls,
            [
                "modrinth://mod/sodium",
                "quadrantnext://modpack?code=1234567"
            ]
        );
        assert!(!parse_args(args(&[]), |_| None).unwrap().autostart);
    }

    #[test]
    fn environment_fills_missing_flags() {
        let env = |key: &str| (key == "QUADRANT_GPUI_MC_FOLDER").then(|| OsString::from("env-mc"));
        let parsed = parse_args(args(&["--data-dir", "data"]), env).unwrap();
        assert_eq!(parsed.mc_folder, Some("env-mc".into()));
        assert_eq!(parsed.data_dir, Some("data".into()));
    }

    #[test]
    fn a_minecraft_folder_needs_its_own_data_folder() {
        assert!(parse_args(args(&["--mc-folder", "mc"]), |_| None).is_err());
        let env_mc = |key: &str| (key == "QUADRANT_GPUI_MC_FOLDER").then(|| OsString::from("mc"));
        assert!(parse_args(args(&[]), env_mc).is_err());
        let env_data =
            |key: &str| (key == "QUADRANT_GPUI_DATA_DIR").then(|| OsString::from("data"));
        assert!(parse_args(args(&["--mc-folder", "mc"]), env_data).is_ok());
        assert_eq!(parse_args(args(&[]), |_| None).unwrap(), Args::default());
    }

    #[test]
    fn a_custom_data_folder_keeps_its_own_sign_in() {
        assert_eq!(keyring_service(None), None);
        let sandbox = keyring_service(Some(Path::new("sandbox"))).unwrap();
        assert_ne!(sandbox, quadrant_core::account::KEYRING_SERVICE);
        assert_ne!(Some(sandbox), keyring_service(Some(Path::new("other"))));
    }

    #[test]
    fn only_a_real_interactive_run_reports_usage() {
        let real = Args::default();
        let has_key = !credentials(&Mode::Interactive)[2].is_empty();
        assert_eq!(reports_usage(&real, false), has_key);
        assert!(!reports_usage(&real, true));
        let sandbox = Args {
            data_dir: Some("sandbox".into()),
            ..Args::default()
        };
        assert!(!reports_usage(&sandbox, false));
        let screenshot = Args {
            mode: Mode::Screenshot("shots".into()),
            ..Args::default()
        };
        assert!(!reports_usage(&screenshot, false));
    }

    #[test]
    fn screenshots_run_without_credentials() {
        assert_eq!(credentials(&Mode::Screenshot("shots".into())), ["", "", ""]);
    }

    #[cfg(feature = "perf")]
    #[test]
    fn perf_runs_need_a_sandbox_and_no_credentials() {
        let perf = |extra: &[&str]| {
            let mut list = vec!["--perf", "idle", "--perf-out", "out"];
            list.extend(extra);
            parse_args(args(&list), |_| None)
        };
        assert!(perf(&[]).is_err());
        assert!(perf(&["--data-dir", "data"]).is_err());
        let parsed = perf(&["--data-dir", "data", "--mc-folder", "mc"]).unwrap();
        assert_eq!(credentials(&parsed.mode), ["", "", ""]);
        assert!(perf(&["--data-dir", "d", "--mc-folder", "m", "--perf-runs", "0"]).is_err());
        assert!(perf(&["--data-dir", "d", "--mc-folder", "m", "--perf", "nope"]).is_err());
        let all = parse_args(
            args(&[
                "--perf",
                "all",
                "--perf-out",
                "o",
                "--data-dir",
                "d",
                "--mc-folder",
                "m",
            ]),
            |_| None,
        )
        .unwrap();
        let Mode::Perf(plan) = all.mode else {
            panic!("not perf mode");
        };
        assert_eq!(plan.scenarios, perf::Scenario::ALL);
        assert_eq!(plan.runs, 5);
        assert_eq!(plan.placement, perf::Placement::Offscreen);
        assert!(
            perf(&[
                "--data-dir",
                "d",
                "--mc-folder",
                "m",
                "--perf-placement",
                "up"
            ])
            .is_err()
        );
    }

    #[test]
    fn noupdater_takes_no_value() {
        let parsed = parse_args(args(&["--noupdater", "--data-dir", "data"]), |_| None).unwrap();
        assert!(parsed.no_updater);
        assert_eq!(parsed.data_dir, Some("data".into()));
        assert!(!parse_args(args(&[]), |_| None).unwrap().no_updater);
    }

    #[test]
    fn debug_and_log_are_the_same_switch() {
        for flag in ["--debug", "--log"] {
            let parsed = parse_args(args(&[flag, "--data-dir", "data"]), |_| None).unwrap();
            assert!(parsed.debug);
            assert_eq!(parsed.data_dir, Some("data".into()));
        }
        assert!(!parse_args(args(&[]), |_| None).unwrap().debug);
    }

    #[test]
    fn only_installed_interactive_runs_on_the_shared_folder_follow_the_channel() {
        let parse = |flags: &[&str]| parse_args(args(flags), |_| None).unwrap();
        assert!(follows_channel(&parse(&["modrinth://mod/sodium"]), true));
        assert!(!follows_channel(&parse(&[]), false));
        assert!(!follows_channel(&parse(&["--noupdater"]), true));
        assert!(!follows_channel(&parse(&["--data-dir", "sandbox"]), true));
        #[cfg(feature = "screenshot")]
        assert!(!follows_channel(&parse(&["--screenshot", "out"]), true));
    }

    #[test]
    fn rejects_unknown_and_valueless_flags() {
        assert!(parse_args(args(&["--verbose"]), |_| None).is_err());
        assert!(parse_args(args(&["--data-dir"]), |_| None).is_err());
    }
}
