//! Feature `perf-profile`: where a run's time went, read from gpui's own
//! foreground journal. Each draw's phases, each view's self time per phase
//! (views under half a millisecond aren't journaled), the swap chain's
//! `Present`, and foreground task polls by spawn site. gpui's profiler adds
//! overhead, so frame times from a profiling build aren't comparable with the
//! harness's numbers; it explains them.

use std::collections::HashMap;

use gpui::{
    App, ViewPhase,
    profiler::journal::{ForegroundEvent, ForegroundJournalCollector, ForegroundJournalEntry},
    set_trace_enabled,
};
use serde_json::{Value, json};

use super::stats::{ms, percentile};

pub struct Profile {
    collector: ForegroundJournalCollector,
}

pub fn start(cx: &App) -> Profile {
    set_trace_enabled(true);
    Profile {
        collector: cx.foreground_journal().collector(),
    }
}

/// Totals for one kind of work.
#[derive(Default)]
struct Spent {
    samples: Vec<f64>,
}

impl Spent {
    fn add(&mut self, ms: f64) {
        self.samples.push(ms);
    }

    fn to_json(&self) -> Value {
        let total: f64 = self.samples.iter().sum();
        json!({
            "count": self.samples.len(),
            "total_ms": total,
            "mean_ms": if self.samples.is_empty() { 0. } else { total / self.samples.len() as f64 },
            "p95_ms": percentile(&self.samples, 95.),
            "max_ms": percentile(&self.samples, 100.),
        })
    }
}

impl Profile {
    pub fn finish(mut self) -> Value {
        let drained = self.collector.collect_unseen();
        let mut phases: HashMap<&str, Spent> = HashMap::new();
        let mut views: HashMap<String, Spent> = HashMap::new();
        let mut tasks: HashMap<String, Spent> = HashMap::new();
        let mut inputs: HashMap<&str, Spent> = HashMap::new();
        for entry in drained.entries {
            let ForegroundJournalEntry::Event(event) = entry else {
                continue;
            };
            match event {
                ForegroundEvent::Draw(frame) => {
                    phases
                        .entry("draw")
                        .or_default()
                        .add(ms(frame.draw_duration()));
                    let p = frame.phases;
                    phases
                        .entry("request_layout")
                        .or_default()
                        .add(ms(p.request_layout));
                    phases.entry("prepaint").or_default().add(ms(p.prepaint));
                    phases.entry("paint").or_default().add(ms(p.paint));
                }
                ForegroundEvent::Present(present) => {
                    phases
                        .entry("present")
                        .or_default()
                        .add(ms(present.present_duration()));
                }
                ForegroundEvent::View(view) => {
                    let phase = match view.phase {
                        ViewPhase::RequestLayout => "request_layout",
                        ViewPhase::Prepaint => "prepaint",
                        ViewPhase::Paint => "paint",
                    };
                    views
                        .entry(format!("{} {phase}", short(view.name)))
                        .or_default()
                        .add(ms(view.self_duration));
                }
                ForegroundEvent::TaskPoll(task) => {
                    let site = format!("{}:{}", task.location.file(), task.location.line());
                    tasks
                        .entry(site)
                        .or_default()
                        .add(ms(task.end.0 - task.start));
                }
                ForegroundEvent::Input(input) => {
                    inputs
                        .entry(input.kind)
                        .or_default()
                        .add(ms(input.end - input.start));
                }
                ForegroundEvent::Action(_) | ForegroundEvent::SmallPolls(_) => {}
            }
        }
        json!({
            "phases": ranked(phases.into_iter().map(|(k, v)| (k.to_string(), v)), usize::MAX),
            "views": ranked(views.into_iter(), 25),
            "task_polls": ranked(tasks.into_iter(), 15),
            "inputs": ranked(inputs.into_iter().map(|(k, v)| (k.to_string(), v)), usize::MAX),
            "lost_entries": drained.lost,
        })
    }
}

/// The `limit` biggest by total time, biggest first.
fn ranked(spent: impl Iterator<Item = (String, Spent)>, limit: usize) -> Value {
    let mut spent: Vec<(String, Spent)> = spent.collect();
    spent.sort_by(|a, b| {
        let total = |s: &Spent| s.samples.iter().sum::<f64>();
        total(&b.1).total_cmp(&total(&a.1))
    });
    Value::Array(
        spent
            .into_iter()
            .take(limit)
            .map(|(name, spent)| {
                let mut value = spent.to_json();
                value["name"] = json!(name);
                value
            })
            .collect(),
    )
}

/// A type name without its module path.
fn short(name: &str) -> &str {
    let name = name.split('<').next().unwrap_or(name);
    name.rsplit("::").next().unwrap_or(name)
}
