use std::future::Future;

use gpui::{App, Global, Task};
use quadrant_host::QuadrantHost;
use tokio::runtime::Runtime;

/// The Quadrant host and the tokio runtime it runs on, shared by every view.
pub struct Backend {
    host: QuadrantHost,
    runtime: Runtime,
}

impl Global for Backend {}

impl Backend {
    pub fn new(host: QuadrantHost, runtime: Runtime) -> Self {
        Self { host, runtime }
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Runs `f` on the tokio runtime and hands its result back as a gpui task.
    ///
    /// This is the only place gpui code meets tokio. Dropping the returned task
    /// stops waiting for the result but does not cancel the host call.
    pub fn run<T, Fut>(
        &self,
        cx: &App,
        f: impl FnOnce(QuadrantHost) -> Fut,
    ) -> Task<anyhow::Result<T>>
    where
        T: Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
    {
        let job = self.runtime.spawn(f(self.host.clone()));
        cx.background_executor().spawn(async move { job.await? })
    }
}
