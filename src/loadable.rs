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
}
