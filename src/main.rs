mod app;
mod assets;
mod backend;
mod events;
mod i18n;
mod loadable;
mod pages;
mod route;
#[cfg(feature = "screenshot")]
mod screenshot;
mod snackbar;
mod theme;

use std::{ffi::OsString, path::PathBuf, process::ExitCode};

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
    /// Folder to render the window into instead of running interactively.
    screenshot_dir: Option<PathBuf>,
}

/// Flags win over the environment, which wins over the defaults.
fn parse_args(
    args: impl IntoIterator<Item = OsString>,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<Args> {
    let mut parsed = Args {
        data_dir: env("QUADRANT_GPUI_DATA_DIR").map(PathBuf::from),
        mc_folder: env("QUADRANT_GPUI_MC_FOLDER").map(PathBuf::from),
        screenshot_dir: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let slot = match arg.to_str() {
            Some("--data-dir") => &mut parsed.data_dir,
            Some("--mc-folder") => &mut parsed.mc_folder,
            Some("--screenshot") if cfg!(feature = "screenshot") => &mut parsed.screenshot_dir,
            _ => bail!("unknown argument {}", arg.to_string_lossy()),
        };
        let value = args
            .next()
            .ok_or_else(|| anyhow!("{} needs a path", arg.to_string_lossy()))?;
        *slot = Some(PathBuf::from(value));
    }
    Ok(parsed)
}

fn build_host(data_dir: Option<PathBuf>, mc_folder: Option<PathBuf>) -> Result<QuadrantHost> {
    let data_dir = match data_dir {
        Some(data_dir) => data_dir,
        None => dirs::data_dir()
            .ok_or_else(|| anyhow!("no data folder on this system; pass --data-dir"))?
            .join(APP_IDENTIFIER),
    };
    let mut options = QuadrantHostOptions::new(
        data_dir,
        option_env!("QUADRANT_OAUTH2_CLIENT_ID").unwrap_or_default(),
        option_env!("QUADRANT_OAUTH2_CLIENT_SECRET").unwrap_or_default(),
        option_env!("QUADRANT_API_KEY").unwrap_or_default(),
    );
    // The host persists this as the `mcFolder` setting in the data folder.
    options.mc_folder = mc_folder;
    let host = QuadrantHost::new(options)?;
    host.init_config()?;
    Ok(host)
}

fn start(args: &Args) -> Result<Backend> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    let host = build_host(args.data_dir.clone(), args.mc_folder.clone())?;
    Ok(Backend::new(host, runtime))
}

fn open_main_window(cx: &mut App) -> Result<WindowHandle<Root>> {
    let bounds = Bounds::centered(None, size(px(1150.), px(720.)), cx);
    let options = WindowOptions {
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
        ..Default::default()
    };
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
            cx.set_global(backend);
            snackbar::install(cx);
            events::forward(cx);
            let window = match open_main_window(cx) {
                Ok(window) => window,
                Err(error) => {
                    eprintln!("Error: failed to open the window: {error:#}");
                    cx.quit();
                    return;
                }
            };
            cx.activate(true);
            #[cfg(feature = "screenshot")]
            if let Some(dir) = args.screenshot_dir {
                screenshot::run(window, dir, cx);
            }
            #[cfg(not(feature = "screenshot"))]
            let _ = (window, args);
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
                screenshot_dir: None,
            }
        );
    }

    #[test]
    fn environment_fills_missing_flags() {
        let env = |key: &str| (key == "QUADRANT_GPUI_MC_FOLDER").then(|| OsString::from("env-mc"));
        let parsed = parse_args(args(&[]), env).unwrap();
        assert_eq!(parsed.mc_folder, Some("env-mc".into()));
        assert_eq!(parsed.data_dir, None);
    }

    #[test]
    fn rejects_unknown_and_valueless_flags() {
        assert!(parse_args(args(&["--verbose"]), |_| None).is_err());
        assert!(parse_args(args(&["--data-dir"]), |_| None).is_err());
    }
}
