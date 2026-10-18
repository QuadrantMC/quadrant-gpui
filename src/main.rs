mod backend;
mod i18n;
mod loadable;

use std::{ffi::OsString, path::PathBuf, process::ExitCode};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{
    App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled, TitlebarOptions,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_component::{Root, Theme, ThemeMode};
use quadrant_host::{QuadrantHost, QuadrantHostOptions};

use crate::{backend::Backend, loadable::Loadable};

/// The desktop app's Tauri identifier, whose data folder this app shares.
const APP_IDENTIFIER: &str = "dev.mrquantumoff.mcmodpackmanager";

#[derive(Debug, Default, PartialEq)]
struct Paths {
    data_dir: Option<PathBuf>,
    mc_folder: Option<PathBuf>,
}

/// Flags win over the environment, which wins over the defaults.
fn parse_paths(
    args: impl IntoIterator<Item = OsString>,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<Paths> {
    let mut paths = Paths {
        data_dir: env("QUADRANT_GPUI_DATA_DIR").map(PathBuf::from),
        mc_folder: env("QUADRANT_GPUI_MC_FOLDER").map(PathBuf::from),
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let slot = match arg.to_str() {
            Some("--data-dir") => &mut paths.data_dir,
            Some("--mc-folder") => &mut paths.mc_folder,
            _ => bail!("unknown argument {}", arg.to_string_lossy()),
        };
        let value = args
            .next()
            .ok_or_else(|| anyhow!("{} needs a path", arg.to_string_lossy()))?;
        *slot = Some(PathBuf::from(value));
    }
    Ok(paths)
}

fn build_host(paths: Paths) -> Result<QuadrantHost> {
    let data_dir = match paths.data_dir {
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
    options.mc_folder = paths.mc_folder;
    let host = QuadrantHost::new(options)?;
    host.init_config()?;
    Ok(host)
}

fn start() -> Result<Backend> {
    let paths = parse_paths(std::env::args_os().skip(1), |key| std::env::var_os(key))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    Ok(Backend::new(build_host(paths)?, runtime))
}

struct Shell {
    modpacks: Loadable<usize>,
}

impl Shell {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            modpacks: Loadable::Idle,
        };
        this.load(cx);
        this
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.modpacks = Loadable::Loading;
        let task = Backend::global(cx).run(cx, |host| async move {
            Ok(host.get_modpacks(true).await?.len())
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.modpacks = Loadable::from_result(result);
                cx.notify();
            })
        })
        .detach();
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let text = match &self.modpacks {
            Loadable::Idle | Loadable::Loading => "Loading modpacks…".into(),
            Loadable::Ready(count) => format!("{count} modpack(s)").into(),
            Loadable::Failed(error) => error.clone(),
        };
        div().size_full().p_4().child(text)
    }
}

fn open_main_window(cx: &mut App) -> Result<()> {
    let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
    let options = WindowOptions {
        titlebar: Some(TitlebarOptions {
            title: Some("Quadrant".into()),
            ..Default::default()
        }),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| {
        let view = cx.new(Shell::new);
        cx.new(|cx| Root::new(view, window, cx))
    })?;
    Ok(())
}

fn main() -> ExitCode {
    let backend = match start() {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("Error: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    gpui_platform::application()
        .with_assets(gpui_kit_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            cx.set_global(backend);
            if let Err(error) = open_main_window(cx) {
                eprintln!("Error: failed to open the window: {error:#}");
                cx.quit();
                return;
            }
            cx.activate(true);
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
        let paths =
            parse_paths(args(&["--data-dir", "flag-data", "--mc-folder", "mc"]), env).unwrap();
        assert_eq!(
            paths,
            Paths {
                data_dir: Some("flag-data".into()),
                mc_folder: Some("mc".into()),
            }
        );
    }

    #[test]
    fn environment_fills_missing_flags() {
        let env = |key: &str| (key == "QUADRANT_GPUI_MC_FOLDER").then(|| OsString::from("env-mc"));
        let paths = parse_paths(args(&[]), env).unwrap();
        assert_eq!(paths.mc_folder, Some("env-mc".into()));
        assert_eq!(paths.data_dir, None);
    }

    #[test]
    fn rejects_unknown_and_valueless_flags() {
        assert!(parse_paths(args(&["--verbose"]), |_| None).is_err());
        assert!(parse_paths(args(&["--data-dir"]), |_| None).is_err());
    }
}
