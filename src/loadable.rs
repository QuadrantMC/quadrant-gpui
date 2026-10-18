use gpui::{SharedString, Task};

use crate::i18n::describe;

/// One piece of page data fetched from the host.
pub enum Loadable<T> {
    Idle,
    Loading,
    Ready(T),
    Failed(SharedString),
}

impl<T> Loadable<T> {
    pub fn from_result(result: anyhow::Result<T>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(error) => Self::Failed(describe(error)),
        }
    }

    /// Marks a reload. Data already on screen stays there until the answer
    /// replaces it, so reloading never flashes the loading state over it.
    pub fn reload(&mut self) {
        if !matches!(self, Self::Ready(_)) {
            *self = Self::Loading;
        }
    }
}

/// The load in flight for one page datum. A page shown again asks for a
/// load without starting a second one beside a running one: the running
/// load is followed by one more as it lands, so the answer shown is never
/// older than the ask.
pub struct Reload {
    task: Task<()>,
    again: bool,
}

impl Default for Reload {
    fn default() -> Self {
        Self {
            task: Task::ready(()),
            again: false,
        }
    }
}

impl Reload {
    /// Keeps `task` as the load in flight, dropping an older load's answer.
    pub fn start(&mut self, task: Task<()>) {
        self.task = task;
        self.again = false;
    }

    /// Whether to start a load now. While one runs, it loads again once it
    /// lands instead.
    pub fn ask(&mut self) -> bool {
        if self.task.is_ready() {
            return true;
        }
        self.again = true;
        false
    }

    /// For a load whose answer just landed: whether it was asked for again
    /// meanwhile.
    pub fn landed(&mut self) -> bool {
        std::mem::take(&mut self.again)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reload_keeps_what_is_shown() {
        let mut ready = Loadable::Ready(3);
        ready.reload();
        assert!(matches!(ready, Loadable::Ready(3)));
        for mut other in [
            Loadable::Idle,
            Loadable::Failed("offline".into()),
            Loadable::Loading,
        ] {
            other.reload();
            assert!(matches!(other, Loadable::<i32>::Loading));
        }
    }

    #[test]
    fn an_idle_reload_loads_at_once() {
        let mut reload = Reload::default();
        assert!(reload.ask());
        assert!(!reload.landed());
        reload.start(Task::ready(()));
        assert!(reload.ask());
    }
}
