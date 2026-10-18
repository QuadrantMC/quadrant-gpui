use std::{future::Future, pin::Pin, sync::Arc};

use anyhow::ensure;
use gpui::{
    App, Global, Task,
    http_client::{AsyncBody, HttpClient, Method, Request, Response, Url, http::HeaderValue},
};
use quadrant_host::{HostEventEnvelope, QuadrantHost};
use tokio::{
    runtime::{Handle, Runtime},
    sync::broadcast,
};

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

    /// Host events, e.g. download progress. `tokio::sync` channels need no
    /// tokio runtime to receive on, so gpui tasks can await this directly.
    pub fn subscribe_events(&self) -> broadcast::Receiver<HostEventEnvelope> {
        self.host.subscribe_events()
    }

    /// The client gpui's `img(url)` downloads remote images with. It runs
    /// each request on this runtime, as `run` does.
    pub fn http_client(&self) -> Arc<dyn HttpClient> {
        Arc::new(ImageFetcher {
            runtime: self.runtime.handle().clone(),
            client: reqwest::Client::new(),
            user_agent: HeaderValue::from_static(concat!(
                "QuadrantMC/",
                env!("CARGO_PKG_NAME"),
                "/v",
                env!("CARGO_PKG_VERSION")
            )),
        })
    }
}

/// Fetches mod icons and screenshots. gpui only ever GETs images, so the
/// request body is not forwarded.
struct ImageFetcher {
    runtime: Handle,
    client: reqwest::Client,
    user_agent: HeaderValue,
}

type HttpResult = anyhow::Result<Response<AsyncBody>>;

impl HttpClient for ImageFetcher {
    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> Pin<Box<dyn Future<Output = HttpResult> + Send>> {
        let method = request.method().clone();
        let fetch = self
            .client
            .get(request.uri().to_string())
            .header(reqwest::header::USER_AGENT, self.user_agent.clone());
        let job = self.runtime.spawn(async move {
            ensure!(method == Method::GET, "only GET requests are supported");
            let response = fetch.send().await?;
            let status = response.status();
            let body = response.bytes().await?;
            Ok((status, body))
        });
        Box::pin(async move {
            let (status, body) = job.await??;
            Ok(Response::builder()
                .status(status.as_u16())
                .body(AsyncBody::from(body.to_vec()))?)
        })
    }
}
