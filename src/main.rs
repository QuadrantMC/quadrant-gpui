mod app;
mod assets;
mod backend;
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

use std::{
    ffi::OsString,
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

#[derive(Debug, Default, PartialEq)]
struct Args {
    data_dir: Option<PathBuf>,
    mc_folder: Option<PathBuf>,
    mode: Mode,
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
    let [mut perf, mut perf_out, mut perf_runs] = [None, None, None];
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let slot = match arg.to_str() {
            Some("--data-dir") => &mut data_dir,
            Some("--mc-folder") => &mut mc_folder,
            Some("--screenshot") if cfg!(feature = "screenshot") => &mut screenshot,
            Some("--perf") if cfg!(feature = "perf") => &mut perf,
            Some("--perf-out") if cfg!(feature = "perf") => &mut perf_out,
            Some("--perf-runs") if cfg!(feature = "perf") => &mut perf_runs,
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
            )?),
            _ => bail!("--screenshot and --perf can't run together"),
        },
    };
    #[cfg(not(feature = "perf"))]
    let _ = (perf_out, perf_runs);
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
    Ok(Backend::new(host, runtime))
}

/// Screenshot mode's window never shows or takes focus, so the user's clicks
/// can't reach it.
fn open_main_window(mode: &Mode, cx: &mut App) -> Result<WindowHandle<Root>> {
    let hidden = matches!(mode, Mode::Screenshot(_));
    let bounds = Bounds::centered(None, size(px(1150.), px(720.)), cx);
    #[cfg_attr(not(feature = "perf"), expect(unused_mut))]
    let mut options = WindowOptions {
        // The title bar is drawn by the app; the title still names the window
        // for the taskbar and window switchers.
        titlebar: Some(TitlebarOptions {
            title: Some("Quadrant".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(12.), px(12.))),
        }),
        window_decorations: Some(WindowDecorations::Client),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(800.), px(500.))),
        show: !hidden,
        focus: !hidden,
        ..Default::default()
    };
    // Perf mode's window is maximized, to time the real swap chain at the
    // display's own resolution. gpui caps an unfocused window's animations
    // at 30 fps; the window the user works in has focus, and a run started
    // from a terminal can't count on getting it.
    #[cfg(feature = "perf")]
    if let Mode::Perf(_) = mode {
        options.window_bounds = Some(WindowBounds::Maximized(bounds));
        options.inactive_frame_interval = None;
    }
    let handle = cx.open_window(options, |window, cx| {
        let view = cx.new(QuadrantApp::new);
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    Ok(handle)
}

fn main() -> ExitCode {
    let started = parse_args(std::env::args_os().skip(1), |key| std::env::var_os(key))
        .and_then(|args| Ok((start(&args)?, args)));
    let (backend, args) = match started {
        Ok(started) => started,
        Err(error) => {
            eprintln!("Error: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    gpui_platform::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            theme::install(cx);
            cx.set_http_client(backend.http_client());
            cx.set_global(backend);
            snackbar::install(cx);
            events::forward(cx);
            let window = match open_main_window(&args.mode, cx) {
                Ok(window) => window,
                Err(error) => {
                    eprintln!("Error: failed to open the window: {error:#}");
                    cx.quit();
                    return;
                }
            };
            cx.activate(true);
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
    }

    #[test]
    fn rejects_unknown_and_valueless_flags() {
        assert!(parse_args(args(&["--verbose"]), |_| None).is_err());
        assert!(parse_args(args(&["--data-dir"]), |_| None).is_err());
    }
}
