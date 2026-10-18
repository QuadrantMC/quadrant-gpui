//! Records every frame request the platform makes of the window. On Windows,
//! gpui's vsync thread invalidates the window each refresh and the frame is
//! drawn and presented inside the resulting `WM_PAINT`, so subclassing the
//! window procedure times the whole frame: next-frame callbacks, render,
//! layout, prepaint, paint and the swap chain's `Present`.
//!
//! The same hook keeps real mouse and keyboard input away from the window,
//! which stays visible while the harness drives it, and counts what it kept.

use std::{cell::RefCell, mem, time::Instant};

use anyhow::Result;
use gpui::Window;

use super::stats::Tick;

#[derive(Default)]
struct Recorder {
    recording: bool,
    ticks: Vec<Tick>,
    /// Set by `frame_drawn` during the current tick.
    drew: bool,
    /// Frames drawn outside a `WM_PAINT`, such as on opening a window.
    off_tick_draws: u32,
    /// Real mouse and keyboard messages, which the harness never sends.
    input_messages: u32,
    /// Where the last real mouse move put the pointer. Windows repeats a move
    /// at the same spot when the window changes under a still pointer.
    last_pointer: Option<isize>,
}

thread_local! {
    static RECORDER: RefCell<Recorder> = RefCell::default();
}

/// What a recording saw besides its ticks.
#[derive(Default)]
pub struct Recording {
    pub ticks: Vec<Tick>,
    pub off_tick_draws: u32,
    pub input_messages: u32,
}

/// Marks the current tick as drawn. `QuadrantApp::render` calls it; with no
/// cached views, every drawn frame renders the app.
pub fn frame_drawn() {
    RECORDER.with_borrow_mut(|recorder| recorder.drew = true);
}

pub fn start() {
    RECORDER.with_borrow_mut(|recorder| {
        *recorder = Recorder {
            recording: true,
            ..Recorder::default()
        }
    });
}

pub fn stop() -> Recording {
    RECORDER.with_borrow_mut(|recorder| {
        let recorder = mem::take(recorder);
        Recording {
            ticks: recorder.ticks,
            off_tick_draws: recorder.off_tick_draws,
            input_messages: recorder.input_messages,
        }
    })
}

fn begin_tick() -> Instant {
    RECORDER.with_borrow_mut(|recorder| {
        if mem::take(&mut recorder.drew) && recorder.recording {
            recorder.off_tick_draws += 1;
        }
    });
    Instant::now()
}

fn end_tick(start: Instant) {
    let end = Instant::now();
    RECORDER.with_borrow_mut(|recorder| {
        let drew = mem::take(&mut recorder.drew);
        if recorder.recording {
            recorder.ticks.push(Tick { start, end, drew });
        }
    });
}

/// Counts a real input message; `pointer` is a mouse move's position.
fn count_input(pointer: Option<isize>) {
    RECORDER.with_borrow_mut(|recorder| {
        if let Some(pointer) = pointer
            && recorder.last_pointer.replace(pointer) == Some(pointer)
        {
            return;
        }
        if recorder.recording {
            recorder.input_messages += 1;
        }
    });
}

/// The display's refresh rate in Hz, as the compositor reports it.
pub fn refresh_rate() -> Result<f64> {
    platform::refresh_rate()
}

/// Starts timing `window`'s frames. Call once.
pub fn install(window: &Window) -> Result<()> {
    platform::install(window)
}

/// Takes the window out of its maximized state, so it can be resized.
pub fn restore() {
    platform::show(false);
}

pub fn maximize() {
    platform::show(true);
}

#[cfg(windows)]
mod platform {
    use std::sync::atomic::{AtomicIsize, Ordering};

    use anyhow::{Context as _, Result, anyhow, bail};
    use gpui::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo},
        UI::WindowsAndMessaging::{
            CallWindowProcW, GWLP_WNDPROC, SW_MAXIMIZE, SW_RESTORE, SetWindowLongPtrW, ShowWindow,
            WM_KEYFIRST, WM_KEYLAST, WM_MOUSEFIRST, WM_MOUSELAST, WM_MOUSEMOVE, WM_NCMOUSEMOVE,
            WM_NCXBUTTONDBLCLK, WM_PAINT, WNDPROC,
        },
    };

    /// `WM_POINTERUPDATE` through `WM_POINTERROUTEDRELEASED`.
    const WM_POINTERFIRST: u32 = 0x0245;
    const WM_POINTERLAST: u32 = 0x0253;

    /// gpui's window procedure, which ours forwards everything but input to.
    static ORIGINAL: AtomicIsize = AtomicIsize::new(0);
    static HWND_RAW: AtomicIsize = AtomicIsize::new(0);

    pub fn refresh_rate() -> Result<f64> {
        let mut info = DWM_TIMING_INFO {
            cbSize: size_of::<DWM_TIMING_INFO>() as u32,
            ..Default::default()
        };
        unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }
            .context("DwmGetCompositionTimingInfo failed")?;
        let rate = info.rateRefresh;
        if rate.uiDenominator == 0 || rate.uiNumerator == 0 {
            bail!("the compositor reported no refresh rate");
        }
        Ok(rate.uiNumerator as f64 / rate.uiDenominator as f64)
    }

    pub fn install(window: &Window) -> Result<()> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| anyhow!("no window handle: {error:?}"))?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            bail!("not a Win32 window");
        };
        let hwnd = HWND(handle.hwnd.get() as _);
        HWND_RAW.store(handle.hwnd.get(), Ordering::Relaxed);
        let original =
            unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, procedure as *const () as isize) };
        if original == 0 {
            bail!("could not subclass the window");
        }
        ORIGINAL.store(original, Ordering::Relaxed);
        Ok(())
    }

    pub fn show(maximized: bool) {
        let hwnd = HWND(HWND_RAW.load(Ordering::Relaxed) as _);
        let command = if maximized { SW_MAXIMIZE } else { SW_RESTORE };
        let _ = unsafe { ShowWindow(hwnd, command) };
    }

    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let original: WNDPROC = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Relaxed)) };
        match message {
            WM_PAINT => {
                let start = super::begin_tick();
                let result = unsafe { CallWindowProcW(original, hwnd, message, wparam, lparam) };
                super::end_tick(start);
                result
            }
            WM_MOUSEMOVE => {
                super::count_input(Some(lparam.0));
                LRESULT(0)
            }
            // Client and title bar mouse messages, pointer messages (which
            // gpui reads for touchpads and pens) and keys.
            WM_MOUSEFIRST..=WM_MOUSELAST
            | WM_NCMOUSEMOVE..=WM_NCXBUTTONDBLCLK
            | WM_POINTERFIRST..=WM_POINTERLAST
            | WM_KEYFIRST..=WM_KEYLAST => {
                super::count_input(None);
                LRESULT(0)
            }
            _ => unsafe { CallWindowProcW(original, hwnd, message, wparam, lparam) },
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use anyhow::{Result, bail};
    use gpui::Window;

    pub fn refresh_rate() -> Result<f64> {
        bail!("perf mode times frames on Windows only so far")
    }

    pub fn install(_: &Window) -> Result<()> {
        bail!("perf mode times frames on Windows only so far")
    }

    pub fn show(_: bool) {}
}
