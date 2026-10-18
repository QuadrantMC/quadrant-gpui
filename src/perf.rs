//! `--perf <scenario|all>` (feature `perf`): drives the visible, maximized
//! window through fixed scenarios and records how long each frame took and
//! whether it made its refresh. Input is dispatched through gpui, never the
//! OS, and the host runs without credentials, as in screenshot mode.

mod frames;
mod stats;

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{
    App, AppContext as _, AsyncApp, Entity, ImageSource, Modifiers, MouseMoveEvent, Pixels,
    PlatformInput, ScrollDelta, ScrollWheelEvent, SharedString, Size, TouchPhase, Window,
    WindowHandle, point, px, size,
};
use gpui_component::{Root, WindowExt as _};
use serde_json::json;

use crate::{
    app::QuadrantApp,
    pages::{ModpacksPage, SearchPage},
    route::Route,
    snackbar::{self, SnackbarKind},
};

pub use frames::frame_drawn;
use stats::RunStats;

/// How long each driven scenario runs, and each page sits in `idle`.
const DURATION: Duration = Duration::from_secs(5);
/// Time for a page's first host calls to answer before it is measured.
const SETTLE: Duration = Duration::from_millis(1500);
/// Scroll speed: about 2900 px/s at 143 Hz, a brisk wheel flick.
const SCROLL_PER_FRAME: f32 = 20.;
const HOVER_PER_FRAME: f32 = 24.;
/// A search with a full page of results that have icons.
const SEARCH_QUERY: &str = "sodium";
/// Longest wait for a search to answer.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(30);
/// The frame budget never asks for less than this rate.
const MIN_RATE: f64 = 120.;

/// A fixed workload, measured `Plan::runs` times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// Every page left alone; ideally no frames at all.
    Idle,
    ScrollModpacks,
    ScrollCurrentModpack,
    ScrollInstalledContent,
    /// A live Modrinth search, scrolled while its icons load and decode.
    Search,
    ScrollSettings,
    /// One page per frame, every page already built.
    SwitchPages,
    /// Every page built afresh once a second, one per frame.
    SwitchPagesCold,
    /// A new snackbar every entrance animation.
    Snackbar,
    /// The create dialog and the delete confirmation, opened and closed.
    Dialogs,
    /// The pointer swept across the Apply page's buttons.
    Hover,
    /// The window resized every frame.
    Resize,
}

impl Scenario {
    pub const ALL: [Scenario; 12] = [
        Scenario::Idle,
        Scenario::ScrollModpacks,
        Scenario::ScrollCurrentModpack,
        Scenario::ScrollInstalledContent,
        Scenario::Search,
        Scenario::ScrollSettings,
        Scenario::SwitchPages,
        Scenario::SwitchPagesCold,
        Scenario::Snackbar,
        Scenario::Dialogs,
        Scenario::Hover,
        Scenario::Resize,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Scenario::Idle => "idle",
            Scenario::ScrollModpacks => "scroll-modpacks",
            Scenario::ScrollCurrentModpack => "scroll-current-modpack",
            Scenario::ScrollInstalledContent => "scroll-installed-content",
            Scenario::Search => "search",
            Scenario::ScrollSettings => "scroll-settings",
            Scenario::SwitchPages => "switch-pages",
            Scenario::SwitchPagesCold => "switch-pages-cold",
            Scenario::Snackbar => "snackbar",
            Scenario::Dialogs => "dialogs",
            Scenario::Hover => "hover",
            Scenario::Resize => "resize",
        }
    }

    /// The page the scenario runs on.
    fn route(self) -> Route {
        match self {
            Scenario::ScrollCurrentModpack => Route::CurrentModpack,
            Scenario::ScrollInstalledContent => Route::InstalledContent,
            Scenario::Search => Route::Search,
            Scenario::ScrollSettings => Route::Settings,
            Scenario::Idle
            | Scenario::ScrollModpacks
            | Scenario::SwitchPages
            | Scenario::SwitchPagesCold
            | Scenario::Snackbar
            | Scenario::Dialogs
            | Scenario::Hover
            | Scenario::Resize => Route::Modpacks,
        }
    }
}

/// What `--perf`, `--perf-out` and `--perf-runs` asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub scenarios: Vec<Scenario>,
    pub out: PathBuf,
    pub runs: usize,
}

impl Plan {
    pub fn parse(scenario: &str, out: Option<PathBuf>, runs: Option<&str>) -> Result<Self> {
        let scenarios = match scenario {
            "all" => Scenario::ALL.to_vec(),
            name => vec![
                Scenario::ALL
                    .into_iter()
                    .find(|scenario| scenario.name() == name)
                    .ok_or_else(|| {
                        let names: Vec<_> = Scenario::ALL.map(Scenario::name).to_vec();
                        anyhow!(
                            "unknown perf scenario {name}; pick all or one of {}",
                            names.join(", ")
                        )
                    })?,
            ],
        };
        let runs = match runs {
            Some(runs) => runs
                .parse::<usize>()
                .ok()
                .filter(|runs| *runs > 0)
                .ok_or_else(|| anyhow!("--perf-runs needs a positive number, not {runs}"))?,
            None => 5,
        };
        Ok(Self {
            scenarios,
            out: out.context("--perf needs --perf-out <dir> for its results")?,
            runs,
        })
    }
}

/// The display the window is on, read once at the start.
struct Display {
    hz: f64,
    period: Duration,
    budget_ms: f64,
}

pub fn run(window: WindowHandle<Root>, plan: Plan, cx: &mut App) {
    cx.spawn(async move |cx| {
        if let Err(error) = run_plan(window, &plan, cx).await {
            eprintln!("Error: perf run failed: {error:#}");
            std::process::exit(1);
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

async fn run_plan(window: WindowHandle<Root>, plan: &Plan, cx: &mut AsyncApp) -> Result<()> {
    std::fs::create_dir_all(&plan.out)?;
    let hz = frames::refresh_rate()?;
    let display = Display {
        hz,
        period: Duration::from_secs_f64(1. / hz),
        budget_ms: 1000. / hz.max(MIN_RATE),
    };
    cx.update_window(window.into(), |_, window, _| {
        window.activate_window();
        frames::install(window)
    })??;
    wait(cx, SETTLE * 2).await;
    let (scale, viewport) = cx.update_window(window.into(), |_, window, _| {
        (window.scale_factor(), window.viewport_size())
    })?;
    let header = format!(
        "refresh {:.2} Hz, budget {:.2} ms, scale {scale}, viewport {}x{} px ({}x{} device)",
        display.hz,
        display.budget_ms,
        f32::from(viewport.width),
        f32::from(viewport.height),
        (f32::from(viewport.width) * scale).round(),
        (f32::from(viewport.height) * scale).round(),
    );
    println!("{header}");
    let mut summary = format!("{header}\n");
    let mut search_icons = Vec::new();
    for &scenario in &plan.scenarios {
        let mut runs = Vec::new();
        let mut raw = Vec::new();
        for index in 0..plan.runs {
            let measured =
                measure(scenario, window, &display, viewport, &mut search_icons, cx).await?;
            let stats = stats::summarize(&measured.recording.ticks, display.period);
            eprintln!(
                "  {} run {}: {}{}",
                scenario.name(),
                index + 1,
                stats.line(display.budget_ms),
                measured.warnings()
            );
            raw.push(measured.to_json(&stats));
            runs.push(stats);
        }
        let median = stats::median_of(&runs);
        let line = format!("{:<26}{}", scenario.name(), median.line(display.budget_ms));
        println!("{line}");
        writeln!(summary, "{line}")?;
        let report = json!({
            "scenario": scenario.name(),
            "refresh_hz": display.hz,
            "budget_ms": display.budget_ms,
            "scale_factor": scale,
            "viewport": [f32::from(viewport.width), f32::from(viewport.height)],
            "median": median.to_json(),
            "runs": raw,
        });
        write_json(&plan.out.join(format!("{}.json", scenario.name())), &report)?;
    }
    std::fs::write(plan.out.join("summary.txt"), summary)?;
    Ok(())
}

fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    std::fs::write(path, serde_json::to_string_pretty(value)?)
        .with_context(|| format!("writing {}", path.display()))
}

/// One run's recording and what else the harness noticed during it.
struct Measured {
    recording: frames::Recording,
    /// Frames the window spent unfocused. Perf mode turns off gpui's 30 fps
    /// cap for unfocused windows, so this only notes interference.
    inactive_frames: u32,
    /// `idle` only: frames drawn on each page.
    idle_frames: Vec<(Route, usize)>,
}

impl Measured {
    fn warnings(&self) -> String {
        let mut warnings = String::new();
        if self.recording.input_messages > 0 {
            let _ = write!(
                warnings,
                "  [{} real input messages: something touched the window]",
                self.recording.input_messages
            );
        }
        if self.inactive_frames > 0 {
            let _ = write!(
                warnings,
                "  [{} frames unfocused: something took focus]",
                self.inactive_frames
            );
        }
        warnings
    }

    fn to_json(&self, stats: &RunStats) -> serde_json::Value {
        let ticks = &self.recording.ticks;
        let origin = ticks.first().map(|tick| tick.start);
        let frames: Vec<_> = ticks
            .iter()
            .map(|tick| {
                let since = |at: Instant| origin.map_or(0., |origin| stats::ms(at - origin));
                json!([
                    since(tick.start),
                    stats::ms(tick.end - tick.start),
                    tick.drew
                ])
            })
            .collect();
        let idle: serde_json::Map<_, _> = self
            .idle_frames
            .iter()
            .map(|(route, count)| (format!("{route:?}"), json!(count)))
            .collect();
        json!({
            "stats": stats.to_json(),
            "off_tick_draws": self.recording.off_tick_draws,
            "input_messages": self.recording.input_messages,
            "inactive_frames": self.inactive_frames,
            "idle_frames_per_page": idle,
            "ticks": { "columns": ["start_ms", "cpu_ms", "drew"], "rows": frames },
        })
    }
}

async fn measure(
    scenario: Scenario,
    window: WindowHandle<Root>,
    display: &Display,
    viewport: Size<Pixels>,
    search_icons: &mut Vec<SharedString>,
    cx: &mut AsyncApp,
) -> Result<Measured> {
    if scenario == Scenario::Idle {
        return measure_idle(window, cx).await;
    }
    prepare(scenario, window, viewport, search_icons, cx).await?;
    frames::start();
    let inactive_frames = drive(scenario, window, viewport, display, cx).await?;
    let recording = frames::stop();
    match scenario {
        Scenario::Dialogs => update(window, cx, |window, cx| window.close_all_dialogs(cx))?,
        Scenario::Resize => {
            frames::maximize();
            wait(cx, SETTLE).await;
        }
        Scenario::Snackbar => wait(cx, snackbar::TIMEOUT).await,
        Scenario::Search => {
            *search_icons = update(window, cx, |window, cx| {
                page::<SearchPage>(Route::Search, window, cx)
                    .read(cx)
                    .icon_urls()
            })?;
        }
        _ => {}
    }
    Ok(Measured {
        recording,
        inactive_frames,
        idle_frames: Vec::new(),
    })
}

/// Every page in turn, untouched: anything drawn is a timer or a poll.
async fn measure_idle(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<Measured> {
    let mut recording = frames::Recording::default();
    let mut idle_frames = Vec::new();
    for route in Route::ALL {
        update(window, cx, |window, cx| navigate(route, window, cx))?;
        wait(cx, SETTLE).await;
        frames::start();
        wait(cx, DURATION).await;
        let page = frames::stop();
        idle_frames.push((route, page.ticks.iter().filter(|tick| tick.drew).count()));
        recording.ticks.extend(page.ticks);
        recording.off_tick_draws += page.off_tick_draws;
        recording.input_messages += page.input_messages;
    }
    Ok(Measured {
        recording,
        inactive_frames: 0,
        idle_frames,
    })
}

async fn prepare(
    scenario: Scenario,
    window: WindowHandle<Root>,
    viewport: Size<Pixels>,
    search_icons: &[SharedString],
    cx: &mut AsyncApp,
) -> Result<()> {
    update(window, cx, |window, cx| {
        window.activate_window();
        navigate(scenario.route(), window, cx);
        move_pointer(pointer(scenario, viewport), window, cx);
    })?;
    match scenario {
        Scenario::SwitchPages => {
            for route in Route::ALL {
                update(window, cx, |window, cx| navigate(route, window, cx))?;
            }
            update(window, cx, |window, cx| {
                navigate(Route::Modpacks, window, cx)
            })?;
        }
        Scenario::Resize => frames::restore(),
        Scenario::Search => return prepare_search(window, search_icons, cx).await,
        _ => {}
    }
    wait(cx, SETTLE).await;
    Ok(())
}

/// Waits for the page to boot, drops the icons the last run loaded so they
/// download and decode again, and searches. Measuring starts as the results
/// arrive, while their icons are still loading.
async fn prepare_search(
    window: WindowHandle<Root>,
    icons: &[SharedString],
    cx: &mut AsyncApp,
) -> Result<()> {
    wait_for_search(window, cx).await?;
    update(window, cx, |window, cx| {
        let search = page::<SearchPage>(Route::Search, window, cx);
        search.update(cx, |search, cx| search.set_query(SEARCH_QUERY, window, cx));
        for url in icons {
            ImageSource::from(url.clone()).remove_asset(cx);
        }
    })?;
    wait_for_search(window, cx).await
}

async fn wait_for_search(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
    let started = Instant::now();
    loop {
        let settled = update(window, cx, |window, cx| {
            page::<SearchPage>(Route::Search, window, cx)
                .read(cx)
                .results_settled()
        })?;
        if settled {
            return Ok(());
        }
        if started.elapsed() > SEARCH_TIMEOUT {
            bail!("the search did not answer within {SEARCH_TIMEOUT:?}");
        }
        wait(cx, Duration::from_millis(20)).await;
    }
}

/// Where the pointer sits while a scenario runs: over the content it scrolls.
fn pointer(scenario: Scenario, viewport: Size<Pixels>) -> gpui::Point<Pixels> {
    let (x, y) = match scenario {
        Scenario::Search => (0.65, 0.55),
        Scenario::Hover => (0., 0.),
        _ => (0.5, 0.5),
    };
    point(viewport.width * x, viewport.height * y)
}

/// Steps `scenario` once per frame for `DURATION` and returns how many frames
/// the window was unfocused. Each step runs as a next-frame callback, inside
/// the frame it changes, and refreshes the window, so every refresh draws a
/// whole frame, as it does during any real motion.
async fn drive(
    scenario: Scenario,
    window: WindowHandle<Root>,
    viewport: Size<Pixels>,
    display: &Display,
    cx: &mut AsyncApp,
) -> Result<u32> {
    let (done, finished) = tokio::sync::oneshot::channel();
    let driver = Driver {
        scenario,
        viewport,
        started: Instant::now(),
        frame: 0,
        slot: None,
        slot_frame: 0,
        inactive_frames: 0,
        done,
    };
    update(window, cx, |window, _| driver.schedule(window))?;
    let timer = cx.background_executor().timer(DURATION * 3);
    within(finished, timer).await.ok_or_else(|| {
        anyhow!(
            "{} stopped getting frames (refresh {:.0} Hz)",
            scenario.name(),
            display.hz
        )
    })
}

/// The receiver's value, or `None` if `timer` fires first.
async fn within(
    finished: tokio::sync::oneshot::Receiver<u32>,
    timer: gpui::Task<()>,
) -> Option<u32> {
    tokio::select! {
        biased;
        value = finished => value.ok(),
        () = timer => None,
    }
}

struct Driver {
    scenario: Scenario,
    viewport: Size<Pixels>,
    started: Instant,
    frame: u32,
    /// The current period of a scenario that acts every so often.
    slot: Option<u32>,
    /// Frames since `slot` began.
    slot_frame: u32,
    inactive_frames: u32,
    done: tokio::sync::oneshot::Sender<u32>,
}

impl Driver {
    fn schedule(mut self, window: &Window) {
        window.on_next_frame(move |window, cx| {
            let elapsed = self.started.elapsed();
            if elapsed >= DURATION {
                let _ = self.done.send(self.inactive_frames);
                return;
            }
            if !window.is_window_active() {
                self.inactive_frames += 1;
            }
            self.step(elapsed, window, cx);
            window.refresh();
            self.frame += 1;
            self.schedule(window);
        });
    }

    /// Whether a new `period` began this frame.
    fn enter_slot(&mut self, elapsed: Duration, period: Duration) -> bool {
        let slot = (elapsed.as_micros() / period.as_micros()) as u32;
        if self.slot == Some(slot) {
            self.slot_frame += 1;
            return false;
        }
        self.slot = Some(slot);
        self.slot_frame = 0;
        true
    }

    fn step(&mut self, elapsed: Duration, window: &mut Window, cx: &mut App) {
        let viewport = self.viewport;
        match self.scenario {
            Scenario::Idle => {}
            Scenario::ScrollModpacks
            | Scenario::ScrollCurrentModpack
            | Scenario::ScrollInstalledContent
            | Scenario::ScrollSettings
            | Scenario::Search => {
                // Down for the first half, back up for the second.
                let down = elapsed < DURATION / 2;
                let dy = if down {
                    -SCROLL_PER_FRAME
                } else {
                    SCROLL_PER_FRAME
                };
                let event = ScrollWheelEvent {
                    position: pointer(self.scenario, viewport),
                    delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                };
                window.dispatch_event(PlatformInput::ScrollWheel(event), cx);
            }
            Scenario::SwitchPages => {
                let route = Route::ALL[(self.frame as usize + 1) % Route::ALL.len()];
                navigate(route, window, cx);
            }
            Scenario::SwitchPagesCold => {
                if self.enter_slot(elapsed, Duration::from_secs(1))
                    && let Some(app) = app(window, cx)
                {
                    app.update(cx, |app, _| app.forget_pages());
                }
                if let Some(&route) = Route::ALL.get(self.slot_frame as usize) {
                    navigate(route, window, cx);
                }
            }
            Scenario::Snackbar => {
                if self.enter_slot(elapsed, snackbar::ENTER) {
                    let (kind, message) = if self.slot.unwrap_or(0).is_multiple_of(2) {
                        (SnackbarKind::Success, "Modpack applied")
                    } else {
                        (SnackbarKind::Error, "Failed to delete: the file is in use")
                    };
                    snackbar::show(cx, kind, message);
                }
            }
            Scenario::Dialogs => {
                if self.enter_slot(elapsed, Duration::from_millis(250)) {
                    let modpacks = page::<ModpacksPage>(Route::Modpacks, window, cx);
                    match self.slot.unwrap_or(0) % 4 {
                        0 => modpacks.update(cx, |page, cx| page.open_create_dialog(window, cx)),
                        2 => modpacks.update(cx, |page, cx| {
                            page.confirm_delete("Beta Pack".into(), window, cx)
                        }),
                        _ => window.close_all_dialogs(cx),
                    }
                }
            }
            Scenario::Hover => move_pointer(sweep(self.frame, viewport), window, cx),
            Scenario::Resize => window.resize(resized(elapsed)),
        }
    }
}

/// A zigzag over the window, one row of eight per sweep, so the pointer
/// crosses the rail, the toolbar and the cards' buttons.
fn sweep(frame: u32, viewport: Size<Pixels>) -> gpui::Point<Pixels> {
    let width = f32::from(viewport.width);
    let per_row = (width / HOVER_PER_FRAME).ceil() as u32;
    let row = frame / per_row;
    let along = (frame % per_row) as f32 * HOVER_PER_FRAME;
    let x = if row.is_multiple_of(2) {
        along
    } else {
        width - along
    };
    let y = viewport.height * ((row % 8) as f32 + 0.5) / 8.;
    point(px(x), y)
}

/// A triangle wave between the restored size and about twice it, two
/// seconds per cycle.
fn resized(elapsed: Duration) -> Size<Pixels> {
    let phase = (elapsed.as_secs_f32() / 2.).fract();
    let wave = 1. - (2. * phase - 1.).abs();
    size(px(1150. + 900. * wave), px(720. + 500. * wave))
}

fn move_pointer(position: gpui::Point<Pixels>, window: &mut Window, cx: &mut App) {
    let event = MouseMoveEvent {
        position,
        pressed_button: None,
        modifiers: Modifiers::default(),
    };
    window.dispatch_event(PlatformInput::MouseMove(event), cx);
}

fn app(window: &Window, cx: &App) -> Option<Entity<QuadrantApp>> {
    let root = window.root::<Root>()??;
    root.read(cx).view().clone().downcast::<QuadrantApp>().ok()
}

fn navigate(route: Route, window: &mut Window, cx: &mut App) {
    if let Some(app) = app(window, cx) {
        app.update(cx, |app, cx| app.navigate(route, cx));
    }
    window.refresh();
}

/// `route`'s page, built if it hasn't been.
fn page<P: 'static>(route: Route, window: &mut Window, cx: &mut App) -> Entity<P> {
    let app = app(window, cx).expect("the window's root is the app");
    let Ok(page) = app
        .update(cx, |app, cx| app.page(route, window, cx))
        .downcast::<P>()
    else {
        panic!("{route:?} shows another page");
    };
    page
}

/// Runs `f` on the window outside any draw, with its root free.
fn update<R>(
    window: WindowHandle<Root>,
    cx: &mut AsyncApp,
    f: impl FnOnce(&mut Window, &mut App) -> R,
) -> Result<R> {
    cx.update_window(window.into(), |_, window, cx| f(window, cx))
}

async fn wait(cx: &mut AsyncApp, duration: Duration) {
    cx.background_executor().timer(duration).await;
}
