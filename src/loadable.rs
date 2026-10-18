use gpui::SharedString;

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
}
