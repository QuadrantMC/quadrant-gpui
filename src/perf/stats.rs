//! Turns a run's recorded ticks into the numbers the harness reports. Pure, so
//! the rules for "dropped" and "median" are tested without a window.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// One frame request the platform delivered to the window: on Windows, one
/// `WM_PAINT`, which gpui's vsync thread sends every refresh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    pub start: Instant,
    /// After gpui drew, presented, or decided nothing changed.
    pub end: Instant,
    /// Whether gpui drew a frame in this tick, rather than finding the
    /// window clean.
    pub drew: bool,
}

impl Tick {
    fn cpu(&self) -> Duration {
        self.end - self.start
    }
}

/// One run of one scenario. Times are in milliseconds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunStats {
    /// Ticks that drew a frame.
    pub frames: f64,
    /// All ticks, drawn or not.
    pub ticks: f64,
    pub cpu_p50: f64,
    pub cpu_p95: f64,
    pub cpu_p99: f64,
    pub cpu_max: f64,
    /// Presents that came more than 1.5 refresh periods after the previous one.
    pub dropped: f64,
    /// Refreshes that showed no new frame. Frames that each take a little
    /// longer than a refresh are presented as soon as the last one finishes,
    /// off the refresh grid: no single interval reaches 1.5 periods, but the
    /// display still repeats a frame every few refreshes.
    pub missed_vsyncs: f64,
    pub interval_p50: f64,
    pub interval_max: f64,
}

/// Late means more than this many refresh periods since the last present.
const LATE: f64 = 1.5;

/// Summarizes `ticks` against the display's refresh `period`. Present
/// intervals are measured between consecutive drawn ticks only: a clean tick
/// between two frames means the scenario had nothing new to show, not that a
/// refresh was missed.
pub fn summarize(ticks: &[Tick], period: Duration) -> RunStats {
    let drawn: Vec<&Tick> = ticks.iter().filter(|tick| tick.drew).collect();
    let cpu: Vec<f64> = drawn.iter().map(|tick| ms(tick.cpu())).collect();
    let intervals: Vec<f64> = ticks
        .windows(2)
        .filter(|pair| pair[0].drew && pair[1].drew)
        .map(|pair| ms(pair[1].end - pair[0].end))
        .collect();
    let period = ms(period);
    let dropped = intervals
        .iter()
        .filter(|interval| **interval > LATE * period)
        .count();
    let refreshes = (intervals.iter().sum::<f64>() / period).round();
    RunStats {
        frames: drawn.len() as f64,
        ticks: ticks.len() as f64,
        cpu_p50: percentile(&cpu, 50.),
        cpu_p95: percentile(&cpu, 95.),
        cpu_p99: percentile(&cpu, 99.),
        cpu_max: percentile(&cpu, 100.),
        dropped: dropped as f64,
        missed_vsyncs: (refreshes - intervals.len() as f64).max(0.),
        interval_p50: percentile(&intervals, 50.),
        interval_max: percentile(&intervals, 100.),
    }
}

/// The nearest-rank percentile, or 0 for no samples.
pub fn percentile(samples: &[f64], percent: f64) -> f64 {
    if samples.is_empty() {
        return 0.;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (percent / 100. * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// The median, or 0 for no values.
pub fn median(values: impl Iterator<Item = f64>) -> f64 {
    let mut sorted: Vec<f64> = values.collect();
    if sorted.is_empty() {
        return 0.;
    }
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.
    } else {
        sorted[mid]
    }
}

/// Each statistic's median over `runs`, taken field by field.
pub fn median_of(runs: &[RunStats]) -> RunStats {
    let field = |get: fn(&RunStats) -> f64| median(runs.iter().map(get));
    RunStats {
        frames: field(|run| run.frames),
        ticks: field(|run| run.ticks),
        cpu_p50: field(|run| run.cpu_p50),
        cpu_p95: field(|run| run.cpu_p95),
        cpu_p99: field(|run| run.cpu_p99),
        cpu_max: field(|run| run.cpu_max),
        dropped: field(|run| run.dropped),
        missed_vsyncs: field(|run| run.missed_vsyncs),
        interval_p50: field(|run| run.interval_p50),
        interval_max: field(|run| run.interval_max),
    }
}

impl RunStats {
    pub fn to_json(&self) -> Value {
        json!({
            "frames": self.frames,
            "ticks": self.ticks,
            "cpu_ms": { "p50": self.cpu_p50, "p95": self.cpu_p95, "p99": self.cpu_p99, "max": self.cpu_max },
            "dropped": self.dropped,
            "missed_vsyncs": self.missed_vsyncs,
            "present_interval_ms": { "p50": self.interval_p50, "max": self.interval_max },
        })
    }

    /// The one-line summary, after the scenario's name.
    pub fn line(&self, budget_ms: f64) -> String {
        format!(
            "frames {:>5}  cpu p50 {:>6.2}  p95 {:>6.2}  p99 {:>6.2}  max {:>7.2} ms  dropped {:>4}  missed {:>4}  budget {:.2} ms",
            self.frames,
            self.cpu_p50,
            self.cpu_p95,
            self.cpu_p99,
            self.cpu_max,
            self.dropped,
            self.missed_vsyncs,
            budget_ms
        )
    }
}

pub fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ticks from `(start_ms, cpu_ms, drew)`.
    fn ticks(spec: &[(f64, f64, bool)]) -> Vec<Tick> {
        let base = Instant::now();
        let at = |ms: f64| base + Duration::from_secs_f64(ms / 1000.);
        spec.iter()
            .map(|&(start, cpu, drew)| Tick {
                start: at(start),
                end: at(start + cpu),
                drew,
            })
            .collect()
    }

    const PERIOD: Duration = Duration::from_millis(7);

    #[test]
    fn steady_frames_drop_nothing() {
        let run = summarize(
            &ticks(&[
                (0., 1., true),
                (7., 1., true),
                (14., 2., true),
                (21., 1., true),
            ]),
            PERIOD,
        );
        assert_eq!(run.frames, 4.);
        assert_eq!(run.dropped, 0.);
        assert_eq!(run.cpu_max, 2.);
        assert!((run.interval_p50 - 7.).abs() < 0.01);
    }

    #[test]
    fn a_long_frame_drops_the_refreshes_it_spans() {
        // The 12 ms frame presents 18 ms after the one before, so the refresh
        // at 14 ms repeats the first frame; the next frame catches up.
        let run = summarize(
            &ticks(&[
                (0., 1., true),
                (7., 12., true),
                (21., 1., true),
                (28., 1., true),
            ]),
            PERIOD,
        );
        assert_eq!(run.dropped, 1.);
        assert_eq!(run.missed_vsyncs, 1.);
    }

    #[test]
    fn frames_a_little_over_budget_miss_refreshes_without_dropping() {
        // 9 ms frames on a 7 ms display: four presents span 27 ms, about four
        // refreshes for three new frames.
        let run = summarize(
            &ticks(&[
                (0., 9., true),
                (9., 9., true),
                (18., 9., true),
                (27., 9., true),
            ]),
            PERIOD,
        );
        assert_eq!(run.dropped, 0.);
        assert_eq!(run.missed_vsyncs, 1.);
    }

    #[test]
    fn a_clean_tick_between_frames_is_not_a_drop() {
        let run = summarize(
            &ticks(&[(0., 1., true), (7., 0.1, false), (14., 1., true)]),
            PERIOD,
        );
        assert_eq!(run.frames, 2.);
        assert_eq!(run.ticks, 3.);
        assert_eq!(run.dropped, 0.);
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let samples: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&samples, 50.), 50.);
        assert_eq!(percentile(&samples, 99.), 99.);
        assert_eq!(percentile(&samples, 100.), 100.);
        assert_eq!(percentile(&[], 50.), 0.);
    }

    #[test]
    fn medians_are_taken_per_statistic() {
        let run = |frames, dropped| RunStats {
            frames,
            dropped,
            ..RunStats::default()
        };
        let median = median_of(&[run(700., 9.), run(710., 0.), run(705., 1.)]);
        assert_eq!(median.frames, 705.);
        assert_eq!(median.dropped, 1.);
        assert_eq!(median_of(&[run(1., 1.), run(3., 2.)]).frames, 2.);
    }
}
