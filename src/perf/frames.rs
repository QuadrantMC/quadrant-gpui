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
    /// The machine's and this process's CPU time when recording started.
    started: Option<CpuTimes>,
}

/// CPU time used so far, by every process together and by this one.
#[derive(Clone, Copy)]
struct CpuTimes {
    at: Instant,
    busy: f64,
    own: f64,
}

impl CpuTimes {
    fn now() -> Option<Self> {
        let (busy, own) = platform::cpu_seconds()?;
        Some(Self {
            at: Instant::now(),
            busy,
            own,
        })
    }

    /// Cores' worth of CPU other processes used since `self`.
    fn others_since(self) -> f64 {
        let Some(now) = Self::now() else {
            return 0.;
        };
        let wall = (now.at - self.at).as_secs_f64();
        if wall <= 0. {
            return 0.;
        }
        ((now.busy - self.busy) - (now.own - self.own)).max(0.) / wall
    }
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
    /// Cores' worth of CPU other processes used meanwhile. A build or a busy
    /// app slows the frames down; a quiet machine stays well under one.
    pub other_load: f64,
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
            started: CpuTimes::now(),
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
            other_load: recorder.started.map_or(0., CpuTimes::others_since),
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

/// Moves the window past the right edge of the virtual desktop at the
/// primary display's work-area size, the client area a maximized window gets,
/// and shows it without activating it. Call outside any gpui update: the move
/// makes gpui handle WM_SIZE.
pub fn place_offscreen() -> Result<()> {
    platform::place_offscreen()
}

/// Sets the window's client size in device pixels, without activating it.
pub fn resize(width: f32, height: f32) {
    platform::resize(width.round() as i32, height.round() as i32);
}

/// Asks Windows to run this process at full speed. Windows slows down
/// processes whose windows aren't in the foreground or on screen (EcoQoS),
/// which would measure an off-screen window, or a visible one the user has
/// clicked away from, on a slower CPU than the app gets in use.
pub fn run_at_full_speed() {
    platform::run_at_full_speed();
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
        Foundation::{FILETIME, RECT},
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo},
        System::Threading::{
            GetCurrentProcess, GetProcessTimes, GetSystemTimes,
            PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetProcessInformation,
        },
        UI::WindowsAndMessaging::{
            CallWindowProcW, GWL_EXSTYLE, GWLP_WNDPROC, GetClientRect, GetSystemMetrics,
            GetWindowLongPtrW, GetWindowRect, SM_CXVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN, SPI_GETWORKAREA, SW_MAXIMIZE, SW_RESTORE, SW_SHOWNOACTIVATE,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
            SetWindowLongPtrW, SetWindowPos, ShowWindow, SystemParametersInfoW, WM_KEYFIRST,
            WM_KEYLAST, WM_MOUSEFIRST, WM_MOUSELAST, WM_MOUSEMOVE, WM_NCMOUSEMOVE,
            WM_NCXBUTTONDBLCLK, WM_PAINT, WNDPROC, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
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

    /// Seconds of CPU every core spent busy, and this process's share.
    pub fn cpu_seconds() -> Option<(f64, f64)> {
        let seconds = |time: FILETIME| {
            ((u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)) as f64 / 1e7
        };
        let [mut idle, mut kernel, mut user] = [FILETIME::default(); 3];
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
        let [mut created, mut exited, mut own_kernel, mut own_user] = [FILETIME::default(); 4];
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut own_kernel,
                &mut own_user,
            )
        }
        .ok()?;
        // System kernel time includes idle time.
        let busy = seconds(kernel) + seconds(user) - seconds(idle);
        Some((busy, seconds(own_kernel) + seconds(own_user)))
    }

    pub fn run_at_full_speed() {
        let state = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: 0,
        };
        let _ = unsafe {
            SetProcessInformation(
                GetCurrentProcess(),
                ProcessPowerThrottling,
                &state as *const _ as *const _,
                size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            )
        };
    }

    fn hwnd() -> HWND {
        HWND(HWND_RAW.load(Ordering::Relaxed) as _)
    }

    pub fn place_offscreen() -> Result<()> {
        let hwnd = hwnd();
        let mut work = RECT::default();
        unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some(&mut work as *mut RECT as *mut _),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        }
        .context("reading the work area")?;
        // Not activatable and kept off the taskbar, so showing it takes no
        // focus and puts nothing on screen.
        unsafe {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                style | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize,
            );
        }
        let x =
            unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN) }
                + 200;
        let y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
        let flags = SWP_NOACTIVATE | SWP_NOZORDER;
        unsafe { SetWindowPos(hwnd, None, x, y, 100, 100, flags) }.context("moving the window")?;
        resize(work.right - work.left, work.bottom - work.top);
        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        Ok(())
    }

    /// Sizes the window so its client area is `width` by `height`.
    pub fn resize(width: i32, height: i32) {
        let hwnd = hwnd();
        let (mut window, mut client) = (RECT::default(), RECT::default());
        if unsafe { GetWindowRect(hwnd, &mut window).and(GetClientRect(hwnd, &mut client)) }
            .is_err()
        {
            return;
        }
        let frame_w = (window.right - window.left) - (client.right - client.left);
        let frame_h = (window.bottom - window.top) - (client.bottom - client.top);
        let flags = SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOMOVE;
        let _ = unsafe { SetWindowPos(hwnd, None, 0, 0, width + frame_w, height + frame_h, flags) };
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

    pub fn place_offscreen() -> Result<()> {
        bail!("perf mode times frames on Windows only so far")
    }

    pub fn resize(_: i32, _: i32) {}

    pub fn cpu_seconds() -> Option<(f64, f64)> {
        None
    }

    pub fn run_at_full_speed() {}
}
