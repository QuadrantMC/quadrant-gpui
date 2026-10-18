// No console window on Windows; `console` explains where output goes.
#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod assets;
mod backend;
mod console;
mod events;
mod i18n;
mod loadable;
mod oauth;
mod pages;
#[cfg(feature = "perf")]
mod perf;
mod route;
#[cfg(feature = "screenshot")]
mod screenshot;
mod session;
mod snackbar;
mod theme;
#[cfg(feature = "updater")]
mod updater;

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowDecorations, WindowHandle,
    WindowOptions, point, px, size,
};
use gpui_component::Root;
use quadrant_host::{QuadrantHost, QuadrantHostOptions};

use crate::{app::QuadrantApp, backend::Backend};

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
    let [mut no_updater, mut debug] = [false; 2];
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--noupdater" {
            no_updater = true;
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

fn build_host(
    data_dir: Option<PathBuf>,
    mc_folder: Option<PathBuf>,
    [client_id, client_secret, api_key]: [&str; 3],
) -> Result<QuadrantHost> {
    let keyring_service = keyring_service(data_dir.as_deref());
    let data_dir = match data_dir {
        Some(data_dir) => data_dir,
        None => dirs::data_dir()
            .ok_or_else(|| anyhow!("no data folder on this system; pass --data-dir"))?
            .join(APP_IDENTIFIER),
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

fn start(args: &Args) -> Result<Backend> {
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
    Ok(Backend::new(host, runtime))
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
fn open_main_window(mode: &Mode, cx: &mut App) -> Result<WindowHandle<Root>> {
    let hidden = matches!(mode, Mode::Screenshot(_));
    let background = background_window();
    let bounds = Bounds::centered(None, size(px(1150.), px(720.)), cx);
    #[cfg_attr(not(feature = "perf"), expect(unused_mut))]
    let mut options = WindowOptions {
        // The title bar is drawn by the app; the title still names the window
        // for the taskbar and window switchers.
        titlebar: Some(TitlebarOptions {
            title: Some(PRODUCT_NAME.into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.), px(12.))),
        }),
        window_decorations: Some(WindowDecorations::Client),
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
    let raw_args: Vec<OsString> = std::env::args_os().skip(1).collect();
    // Before parsing, so a bad argument is reported in the console too.
    let console = console::Console::setup(raw_args.iter().any(|arg| is_debug_flag(arg)));
    let started = parse_args(raw_args, |key| std::env::var_os(key))
        .and_then(|args| Ok((start(&args)?, args)));
    let (backend, args) = match started {
        Ok(started) => started,
        Err(error) => {
            console.fatal(&format!("{error:#}"));
            return ExitCode::FAILURE;
        }
    };
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            cx.set_app_identity(BUNDLE_IDENTIFIER, PRODUCT_NAME);
            gpui_component::init(cx);
            theme::install(cx);
            cx.set_http_client(backend.http_client());
            cx.set_global(backend);
            snackbar::install(cx);
            events::forward(cx);
            #[cfg(feature = "updater")]
            if args.mode == Mode::Interactive && !args.no_updater {
                updater::Updater::start(cx);
            }
            let window = match open_main_window(&args.mode, cx) {
                Ok(window) => window,
                Err(error) => {
                    console.fatal(&format!("failed to open the window: {error:#}"));
                    std::process::exit(1);
                }
            };
            if !background_window() {
                cx.activate(true);
            }
            #[cfg(not(any(feature = "screenshot", feature = "perf")))]
            let _ = window;
            match args.mode {
                Mode::Interactive => {}
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
            }
        );
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
    fn rejects_unknown_and_valueless_flags() {
        assert!(parse_args(args(&["--verbose"]), |_| None).is_err());
        assert!(parse_args(args(&["--data-dir"]), |_| None).is_err());
    }
}
