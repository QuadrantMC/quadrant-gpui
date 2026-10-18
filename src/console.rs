//! Where the app's output goes. On Windows the exe is a GUI-subsystem app, so
//! starting it never opens a console window. Its output still reaches a
//! parent that redirects or pipes it (cargo, scripts), since those hand it
//! their handles. `--debug` (or `--log`) attaches to the terminal it was
//! started from, or opens a console of its own, and logs verbosely. A fatal
//! error nobody would see on stderr shows a message box instead, unless
//! `QUADRANT_NATIVE_NO_DIALOG` is set, as tests and scripts do.

/// The log filter `--debug` sets, unless `RUST_LOG` says otherwise.
const DEBUG_LOG: &str = "info,quadrant_native=debug,quadrant_host=debug,quadrant_core=debug";

/// Where a fatal error is shown besides stderr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Report {
    /// Only stderr, which someone reads.
    Stderr,
    /// A message box too: nobody would see stderr.
    Dialog,
}

impl Report {
    fn choose(stderr_inherited: bool, no_dialog: bool) -> Self {
        if stderr_inherited || no_dialog {
            Report::Stderr
        } else {
            Report::Dialog
        }
    }
}

pub struct Console {
    report: Report,
}

impl Console {
    /// Run first in `main`, before anything logs or spawns a thread.
    pub fn setup(debug: bool) -> Self {
        let report = Report::choose(
            platform::stderr_inherited(),
            std::env::var_os("QUADRANT_NATIVE_NO_DIALOG").is_some(),
        );
        if debug {
            platform::open_console();
            if std::env::var_os("RUST_LOG").is_none() {
                // SAFETY: no other thread exists yet to read the environment.
                unsafe { std::env::set_var("RUST_LOG", DEBUG_LOG) };
            }
        }
        Self { report }
    }

    /// Reports an error the app can't start or run past.
    pub fn fatal(&self, message: &str) {
        self.fatal_with(message, platform::message_box);
    }

    fn fatal_with(&self, message: &str, dialog: impl FnOnce(&str)) {
        eprintln!("Error: {message}");
        if self.report == Report::Dialog {
            dialog(message);
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::io::IntoRawHandle as _;

    use windows::{
        Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{FILE_TYPE_CHAR, FILE_TYPE_UNKNOWN, GetFileType},
            System::Console::{
                ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole, CONSOLE_MODE, GetConsoleMode,
                GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
            },
            UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
        },
        core::HSTRING,
    };

    /// The standard handle, if it leads somewhere: a file, a pipe, or a
    /// console this process is attached to. A GUI app started from a
    /// terminal can hold the terminal's console handles without the console.
    fn handle(which: STD_HANDLE) -> Option<HANDLE> {
        // SAFETY: these calls only query the handle.
        unsafe {
            let handle = GetStdHandle(which)
                .ok()
                .filter(|handle| !handle.is_invalid())?;
            match GetFileType(handle) {
                FILE_TYPE_UNKNOWN => None,
                FILE_TYPE_CHAR => {
                    let mut mode = CONSOLE_MODE::default();
                    GetConsoleMode(handle, &mut mode).ok().map(|()| handle)
                }
                _ => Some(handle),
            }
        }
    }

    pub fn stderr_inherited() -> bool {
        handle(STD_ERROR_HANDLE).is_some()
    }

    /// The terminal that started the app, else a new console. Standard
    /// handles the parent redirected stay where they point.
    pub fn open_console() {
        // SAFETY: plain Win32 calls with no pointers.
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS).is_err() {
                let _ = AllocConsole();
            }
        }
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            if handle(which).is_some() {
                continue;
            }
            let Ok(console) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("CONOUT$")
            else {
                return;
            };
            // The handle lives as long as the process, as a std handle should.
            let raw = HANDLE(console.into_raw_handle());
            // SAFETY: `raw` is a valid, open console handle.
            let _ = unsafe { SetStdHandle(which, raw) };
        }
    }

    pub fn message_box(message: &str) {
        // SAFETY: both strings outlive the call; no owner window.
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(message),
                &HSTRING::from(crate::PRODUCT_NAME),
                MB_OK | MB_ICONERROR,
            );
        }
    }
}

#[cfg(not(windows))]
mod platform {
    /// Unix processes always have the stderr their parent gave them.
    pub fn stderr_inherited() -> bool {
        true
    }

    pub fn open_console() {}

    pub fn message_box(_: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_only_when_nobody_reads_stderr_and_dialogs_are_allowed() {
        assert_eq!(Report::choose(false, false), Report::Dialog);
        assert_eq!(Report::choose(true, false), Report::Stderr);
        assert_eq!(Report::choose(false, true), Report::Stderr);
        assert_eq!(Report::choose(true, true), Report::Stderr);
    }

    #[test]
    fn fatal_shows_the_dialog_only_for_dialog_reports() {
        for (report, shown) in [(Report::Dialog, true), (Report::Stderr, false)] {
            let mut seen = None;
            Console { report }.fatal_with("unknown argument --bogus", |message| {
                seen = Some(message.to_string())
            });
            assert_eq!(seen.is_some(), shown);
            if shown {
                assert_eq!(seen.as_deref(), Some("unknown argument --bogus"));
            }
        }
    }
}
