use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use anyhow::ensure;
use gpui::{
    App, Global, Task,
    http_client::{AsyncBody, HttpClient, Method, Request, Response, Url, http::HeaderValue},
};
use quadrant_host::{HostEventEnvelope, QuadrantHost};

use crate::remote_images::is_trusted;
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

    /// Runs a save nobody waits for. A failure (a `config.json` the desktop
    /// app holds locked, a read-only folder) is logged as `Couldn't {what}`
    /// instead of being lost with the dropped task.
    pub fn run_logged<Fut>(&self, cx: &App, what: &'static str, f: impl FnOnce(QuadrantHost) -> Fut)
    where
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let task = self.run(cx, f);
        cx.spawn(async move |_| {
            if let Err(error) = task.await {
                log::warn!("Couldn't {what}: {error:#}");
            }
        })
        .detach();
    }

    /// Host events, e.g. download progress. `tokio::sync` channels need no
    /// tokio runtime to receive on, so gpui tasks can await this directly.
    pub fn subscribe_events(&self) -> broadcast::Receiver<HostEventEnvelope> {
        self.host.subscribe_events()
    }

    /// The client gpui's `img(url)` downloads remote images with. It runs
    /// each request on this runtime, as `run` does.
    pub fn http_client(&self) -> Arc<dyn HttpClient> {
        let client = reqwest::Client::builder()
            // A CDN's redirect to another host is not followed.
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if is_trusted(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .connect_timeout(Duration::from_secs(10))
            // A stalled transfer ends soon; a slow but moving screenshot
            // gets longer.
            .read_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_else(|error| {
                log::warn!("Images load without timeouts: {error}");
                reqwest::Client::new()
            });
        Arc::new(ImageFetcher {
            runtime: self.runtime.handle().clone(),
            client,
            allowed: is_trusted,
            user_agent: HeaderValue::from_static(concat!(
                "QuadrantMC/",
                env!("CARGO_PKG_NAME"),
                "/v",
                env!("CARGO_PKG_VERSION")
            )),
        })
    }
}

/// The largest image body fetched. Icons and screenshots are well under it;
/// anything bigger is not an image worth decoding.
const MAX_IMAGE_BYTES: usize = 16 << 20;

/// Fetches mod icons and screenshots from the addresses `allowed` accepts,
/// within a timeout and [`MAX_IMAGE_BYTES`]. gpui only ever GETs images, so
/// the request body is not forwarded.
struct ImageFetcher {
    runtime: Handle,
    client: reqwest::Client,
    allowed: fn(&Url) -> bool,
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
        let url = Url::parse(&request.uri().to_string());
        let allowed = self.allowed;
        let fetch = self
            .client
            .get(request.uri().to_string())
            .header(reqwest::header::USER_AGENT, self.user_agent.clone());
        let job = self.runtime.spawn(async move {
            ensure!(method == Method::GET, "only GET requests are supported");
            ensure!(
                url.is_ok_and(|url| allowed(&url)),
                "images load only from the providers' CDNs"
            );
            let mut response = fetch.send().await?;
            let status = response.status();
            let too_big = || anyhow::anyhow!("image larger than {MAX_IMAGE_BYTES} bytes");
            let length = response.content_length().unwrap_or(0);
            if usize::try_from(length).map_or(true, |length| length > MAX_IMAGE_BYTES) {
                return Err(too_big());
            }
            let mut body = Vec::with_capacity(length as usize);
            while let Some(chunk) = response.chunk().await? {
                if body.len() + chunk.len() > MAX_IMAGE_BYTES {
                    return Err(too_big());
                }
                body.extend_from_slice(&chunk);
            }
            Ok((status, body))
        });
        Box::pin(async move {
            let (status, body) = job.await??;
            Ok(Response::builder()
                .status(status.as_u16())
                .body(AsyncBody::from(body))?)
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::AsyncReadExt as _;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    /// Serves one response per body, in turn, with its length declared.
    fn serve(runtime: &Runtime, bodies: Vec<(usize, &'static [u8])>) -> String {
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .unwrap();
        let address = listener.local_addr().unwrap();
        runtime.spawn(async move {
            for (length, body) in bodies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 1024];
                let _ = socket.read(&mut request).await;
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\n\r\n");
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body).await;
            }
        });
        format!("http://{address}/icon.png")
    }

    #[test]
    fn an_image_over_the_cap_is_refused_before_its_body() {
        let runtime = Runtime::new().unwrap();
        let url = serve(&runtime, vec![(MAX_IMAGE_BYTES + 1, b""), (3, b"png")]);
        let fetcher = ImageFetcher {
            runtime: runtime.handle().clone(),
            client: reqwest::Client::new(),
            allowed: |_| true,
            user_agent: HeaderValue::from_static("test"),
        };
        let get = || Request::get(url.as_str()).body(AsyncBody::empty()).unwrap();
        let Err(refused) = futures::executor::block_on(fetcher.send(get())) else {
            panic!("an image over the cap was fetched");
        };
        assert!(refused.to_string().contains("larger than"), "{refused}");
        let mut response = futures::executor::block_on(fetcher.send(get())).unwrap();
        let mut body = Vec::new();
        futures::executor::block_on(response.body_mut().read_to_end(&mut body)).unwrap();
        assert_eq!(body, b"png");
    }

    #[test]
    fn an_address_off_the_cdns_is_never_requested() {
        let runtime = Runtime::new().unwrap();
        let url = serve(&runtime, vec![(3, b"png")]);
        let fetcher = ImageFetcher {
            runtime: runtime.handle().clone(),
            client: reqwest::Client::new(),
            allowed: is_trusted,
            user_agent: HeaderValue::from_static("test"),
        };
        let request = Request::get(url.as_str()).body(AsyncBody::empty()).unwrap();
        let Err(refused) = futures::executor::block_on(fetcher.send(request)) else {
            panic!("an address off the CDNs was fetched");
        };
        assert!(refused.to_string().contains("CDNs"), "{refused}");
    }
}
