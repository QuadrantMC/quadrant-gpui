//! Progress on the app's taskbar button, as `App.tsx` sets it through Tauri's
//! `setProgressBar` for an update download, an export and a modpack download.
//!
//! Windows draws it on the taskbar button (`ITaskbarList3`); Linux sends the
//! Unity `LauncherEntry` signal that the GNOME dash-to-dock, KDE's task
//! manager and Unity read. macOS has no dock progress here: Tauri draws its own
//! view into the dock tile, which this app doesn't do (yet).

use gpui::{App, Global, WindowHandle};
use gpui_component::Root;

/// What reports progress.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Job {
    #[cfg_attr(not(feature = "updater"), expect(dead_code))]
    Update,
    Export,
    ModpackDownload,
}

/// The jobs under way and their percentages, the most recently moved last.
/// The bar shows the last one, as the desktop app's last `setProgressBar`
/// wins; a finished job hands the bar back to the one before it.
#[derive(Debug, Default)]
struct Jobs(Vec<(Job, u8)>);

impl Jobs {
    /// `done` is the fraction finished; at 100% the job is over.
    fn set(&mut self, job: Job, done: f64) {
        self.0.retain(|(running, _)| *running != job);
        let percent = (done * 100.).round().clamp(0., 100.) as u8;
        if percent < 100 {
            self.0.push((job, percent));
        }
    }

    fn clear(&mut self, job: Job) {
        self.0.retain(|(running, _)| *running != job);
    }

    fn shown(&self) -> Option<u8> {
        self.0.last().map(|(_, percent)| *percent)
    }
}

struct Taskbar {
    jobs: Jobs,
    shown: Option<u8>,
    bar: platform::Bar,
}

impl Global for Taskbar {}

/// Starts showing progress on `window`'s taskbar button. Interactive runs
/// only: without it, `progress` and `clear` do nothing.
pub fn install(window: WindowHandle<Root>, cx: &mut App) {
    let Some(bar) = platform::Bar::new(window, cx) else {
        return;
    };
    cx.set_global(Taskbar {
        jobs: Jobs::default(),
        shown: None,
        bar,
    });
}

/// `job` is `done` (0 to 1) of the way through.
pub fn progress(cx: &mut App, job: Job, done: f64) {
    change(cx, |jobs| jobs.set(job, done));
}

/// `job` stopped, finished or not.
pub fn clear(cx: &mut App, job: Job) {
    change(cx, |jobs| jobs.clear(job));
}

fn change(cx: &mut App, f: impl FnOnce(&mut Jobs)) {
    if !cx.has_global::<Taskbar>() {
        return;
    }
    let taskbar = cx.global_mut::<Taskbar>();
    f(&mut taskbar.jobs);
    let shown = taskbar.jobs.shown();
    if shown != taskbar.shown {
        taskbar.shown = shown;
        taskbar.bar.show(shown);
    }
}

#[cfg(windows)]
mod platform {
    use gpui::{App, WindowHandle};
    use gpui_component::Root;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::HWND,
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        },
        UI::Shell::{ITaskbarList3, TBPF_NOPROGRESS, TBPF_NORMAL, TaskbarList},
    };

    pub struct Bar {
        list: ITaskbarList3,
        hwnd: HWND,
    }

    impl Bar {
        pub fn new(window: WindowHandle<Root>, cx: &mut App) -> Option<Self> {
            let hwnd = window
                .update(cx, |_, window, _| {
                    match window.window_handle().ok()?.as_raw() {
                        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
                        _ => None,
                    }
                })
                .ok()
                .flatten()?;
            // gpui has COM up on this thread already; this only makes sure.
            let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            let list: ITaskbarList3 =
                match unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER) } {
                    Ok(list) => list,
                    Err(error) => {
                        log::warn!("No taskbar progress: {error}");
                        return None;
                    }
                };
            unsafe { list.HrInit() }.ok()?;
            Some(Self { list, hwnd })
        }

        pub fn show(&self, percent: Option<u8>) {
            let result = unsafe {
                match percent {
                    Some(percent) => self
                        .list
                        .SetProgressState(self.hwnd, TBPF_NORMAL)
                        .and_then(|()| self.list.SetProgressValue(self.hwnd, percent.into(), 100)),
                    None => self.list.SetProgressState(self.hwnd, TBPF_NOPROGRESS),
                }
            };
            if let Err(error) = result {
                log::debug!("Taskbar progress not shown: {error}");
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::collections::HashMap;

    use futures::{
        StreamExt as _,
        channel::mpsc::{UnboundedSender, unbounded},
    };
    use gpui::{App, WindowHandle};
    use gpui_component::Root;
    use zbus::zvariant::Value;

    /// The desktop file the launcher shows the progress on.
    const APP_URI: &str = "application://quadrant-native.desktop";
    const PATH: &str = "/dev/mrquantumoff/quadrant_native/LauncherEntry";

    /// Hands each change to one background task, which sends them in order
    /// over one session bus connection.
    pub struct Bar(UnboundedSender<Option<u8>>);

    impl Bar {
        pub fn new(_: WindowHandle<Root>, cx: &mut App) -> Option<Self> {
            let (sender, mut changes) = unbounded::<Option<u8>>();
            cx.background_executor()
                .spawn(async move {
                    let connection = match zbus::Connection::session().await {
                        Ok(connection) => connection,
                        Err(error) => {
                            log::warn!("No launcher progress: {error}");
                            return;
                        }
                    };
                    while let Some(percent) = changes.next().await {
                        let properties = HashMap::from([
                            (
                                "progress",
                                Value::from(f64::from(percent.unwrap_or(0)) / 100.),
                            ),
                            ("progress-visible", Value::from(percent.is_some())),
                        ]);
                        if let Err(error) = connection
                            .emit_signal(
                                None::<zbus::names::BusName<'_>>,
                                PATH,
                                "com.canonical.Unity.LauncherEntry",
                                "Update",
                                &(APP_URI, properties),
                            )
                            .await
                        {
                            log::debug!("Launcher progress not shown: {error}");
                        }
                    }
                })
                .detach();
            Some(Self(sender))
        }

        pub fn show(&self, percent: Option<u8>) {
            let _ = self.0.unbounded_send(percent);
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use gpui::{App, WindowHandle};
    use gpui_component::Root;

    pub struct Bar;

    impl Bar {
        pub fn new(_: WindowHandle<Root>, _: &mut App) -> Option<Self> {
            None
        }

        pub fn show(&self, _: Option<u8>) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_job_to_move_owns_the_bar() {
        let mut jobs = Jobs::default();
        assert_eq!(jobs.shown(), None);
        jobs.set(Job::Export, 0.25);
        jobs.set(Job::ModpackDownload, 0.5);
        assert_eq!(jobs.shown(), Some(50));
        jobs.set(Job::Export, 0.304);
        assert_eq!(jobs.shown(), Some(30));
        jobs.clear(Job::Export);
        assert_eq!(jobs.shown(), Some(50));
    }

    #[test]
    fn a_job_at_100_percent_is_over() {
        let mut jobs = Jobs::default();
        jobs.set(Job::ModpackDownload, 0.5);
        jobs.set(Job::ModpackDownload, 0.996);
        assert_eq!(jobs.shown(), None);
        jobs.set(Job::Export, 1.);
        assert_eq!(jobs.shown(), None);
    }
}
