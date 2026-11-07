//! Launch at login, as the desktop app's autostart plugin registers it: the
//! `Run` key under HKCU on Windows, a LaunchAgent on macOS and an XDG
//! autostart entry on Linux, each starting the app with `--autostart` so it
//! opens in the tray.
//!
//! The OS keeps the setting, not Quadrant's config. A `--data-dir` sandbox
//! registers under a name of its own that starts the same sandbox, so it
//! never replaces the real app's entry.

use std::path::Path;

use anyhow::{Context as _, Result};
use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use gpui::{App, Global};

/// The registered launch for this data folder.
pub struct Autostart {
    launch: AutoLaunch,
    #[cfg_attr(not(windows), expect(dead_code))]
    name: String,
}

impl Global for Autostart {}

impl Autostart {
    /// `sandbox` is the single-instance key of a `--data-dir` run, with the
    /// folders it was given.
    pub fn new(sandbox: Option<(&str, &Path, Option<&Path>)>) -> Result<Self> {
        let exe = std::env::current_exe().context("couldn't find this program")?;
        // An AppImage runs from a mount that changes every launch.
        let exe = std::env::var_os("APPIMAGE")
            .filter(|_| cfg!(target_os = "linux"))
            .map_or(exe, Into::into);
        let (name, args) = launch(sandbox);
        let mut builder = AutoLaunchBuilder::new();
        builder
            .set_app_name(&name)
            .set_app_path(&quote(&exe.to_string_lossy()))
            .set_args(&args.iter().map(|arg| quote(arg)).collect::<Vec<_>>());
        #[cfg(windows)]
        builder.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
        #[cfg(target_os = "macos")]
        builder.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
        Ok(Self {
            launch: builder.build()?,
            name,
        })
    }

    pub fn global(cx: &App) -> Option<&Self> {
        cx.try_global::<Self>()
    }

    pub fn is_enabled(&self) -> bool {
        self.launch.is_enabled().unwrap_or_else(|error| {
            log::warn!("Couldn't read the login item: {error}");
            false
        })
    }

    pub fn set(&self, on: bool) -> Result<()> {
        if on {
            self.launch.enable()?;
        } else {
            self.launch.disable()?;
            self.forget_task_manager_state();
        }
        Ok(())
    }

    /// Enabling also notes the entry as allowed under Task Manager's
    /// `StartupApproved` key, which disabling leaves behind; it goes too.
    fn forget_task_manager_state(&self) {
        #[cfg(windows)]
        {
            use windows::{
                Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyValueW},
                core::HSTRING,
            };
            // SAFETY: both strings outlive the call.
            let _ = unsafe {
                RegDeleteKeyValueW(
                    HKEY_CURRENT_USER,
                    &HSTRING::from(
                        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
                    ),
                    &HSTRING::from(&self.name),
                )
            };
        }
    }
}

/// The entry's name and the arguments it starts the app with.
fn launch(sandbox: Option<(&str, &Path, Option<&Path>)>) -> (String, Vec<String>) {
    let mut args = vec!["--autostart".to_string()];
    let Some((key, data_dir, mc_folder)) = sandbox else {
        return (crate::PRODUCT_NAME.into(), args);
    };
    args.extend(["--data-dir".into(), data_dir.display().to_string()]);
    if let Some(mc_folder) = mc_folder {
        args.extend(["--mc-folder".into(), mc_folder.display().to_string()]);
    }
    (format!("{}-{key}", crate::PRODUCT_NAME), args)
}

/// Windows' `Run` value and a desktop file's `Exec` are command lines, which
/// the library joins without quoting; a LaunchAgent lists each argument.
fn quote(arg: &str) -> String {
    if cfg!(target_os = "macos") || !arg.contains(' ') {
        arg.to_string()
    } else {
        format!("\"{arg}\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sandbox_registers_beside_the_real_app() {
        assert_eq!(
            launch(None),
            ("Quadrant-Native".into(), vec!["--autostart".into()])
        );
        let (name, args) = launch(Some((
            "abc",
            Path::new("sand box/data"),
            Some(Path::new("mc")),
        )));
        assert_eq!(name, "Quadrant-Native-abc");
        assert_eq!(
            args,
            [
                "--autostart",
                "--data-dir",
                "sand box/data",
                "--mc-folder",
                "mc"
            ]
        );
    }

    #[test]
    fn command_lines_quote_spaces() {
        assert_eq!(quote("plain"), "plain");
        if !cfg!(target_os = "macos") {
            assert_eq!(
                quote("C:\\Program Files\\q.exe"),
                "\"C:\\Program Files\\q.exe\""
            );
        }
    }

    /// Writes the real login item, under a test name, and removes it.
    #[test]
    #[ignore = "changes this user's login items"]
    fn registers_and_unregisters() {
        let autostart = Autostart::new(Some(("test", Path::new("autostart test"), None))).unwrap();
        autostart.set(true).unwrap();
        let enabled = autostart.is_enabled();
        autostart.set(false).unwrap();
        assert!(enabled);
        assert!(!autostart.is_enabled());
    }
}
