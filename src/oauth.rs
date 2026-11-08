//! The loopback Quadrant ID sign-in `AccountPage.tsx` runs, after the Quadrant
//! CLI's `oauth.rs`: a one-shot HTTP listener on 127.0.0.1 receives the
//! browser's redirect.

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use quadrant_host::QuadrantHost;
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt as _, AsyncWriteExt as _},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};
use url::Url;

use crate::{config, i18n::t};

const AUTHORIZE_URL: &str = "https://mrquantumoff.dev/account/oauth2/authorize";
const SCOPES: &str = "profile:read profile:write sync:read sync:write share:read share:write \
     settings:read settings:write notifications:read";
/// The redirect URIs registered for the Quadrant client.
const PORTS: std::ops::RangeInclusive<u16> = 4000..=4005;
const MAX_REQUEST_HEAD: usize = 16 * 1024;
/// How long one connection gets to send its request, so an idle connection a
/// browser opens ahead of time can't hold up the redirect behind it.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the sign-in waits for the browser before giving up.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The config key the desktop app keeps the pending sign-in's state under.
const OAUTH_STATE: &str = "oauthState";

/// How a sign-in ended without failing.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    SignedIn,
    /// The user started another sign-in, or left the page's flow.
    Cancelled,
}

/// Signs in to Quadrant ID: sends the authorize URL to `browser` for the view
/// to open, waits for the redirect, and trades its code for tokens.
pub async fn sign_in(
    host: QuadrantHost,
    browser: oneshot::Sender<Url>,
    cancel: oneshot::Receiver<()>,
) -> Result<Outcome> {
    let options = host.options();
    if options.oauth_client_id.is_empty() || options.oauth_client_secret.is_empty() {
        bail!(
            "This build has no Quadrant ID credentials, so it can't sign in. Build it with QUADRANT_OAUTH2_CLIENT_ID and QUADRANT_OAUTH2_CLIENT_SECRET set."
        );
    }
    let state = new_state()?;
    config::set(&host, OAUTH_STATE, Value::String(state.clone()))?;
    let outcome = wait_and_exchange(&host, &state, browser, cancel).await;
    forget_state(&host, &state);
    outcome
}

async fn wait_and_exchange(
    host: &QuadrantHost,
    state: &str,
    browser: oneshot::Sender<Url>,
    cancel: oneshot::Receiver<()>,
) -> Result<Outcome> {
    let (listener, port) = bind().await?;
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let _ = browser.send(authorize_url(
        &host.oauth2_client_id(),
        &redirect_uri,
        state,
    ));
    let code = tokio::select! {
        code = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_callback(listener, state)) => {
            code.map_err(|_| anyhow!("The sign-in timed out. Try again."))??
        }
        _ = cancel => return Ok(Outcome::Cancelled),
    };
    host.oauth2_login(code, redirect_uri).await?;
    Ok(Outcome::SignedIn)
}

/// A `quadrantnext://login` redirect, the desktop app's deep-link way back
/// from the browser: finishes the sign-in it belongs to, and ignores one
/// whose state no sign-in here started.
pub async fn finish_redirect(
    host: QuadrantHost,
    state: Option<String>,
    code: Option<String>,
    redirect_uri: String,
) -> Result<()> {
    let expected = host
        .get_config_value(OAUTH_STATE)?
        .and_then(|value| value.as_str().map(str::to_string));
    let (Some(state), Some(code)) = (state, code) else {
        return Ok(());
    };
    if expected.as_ref() != Some(&state) {
        log::warn!("Ignored a sign-in redirect this app didn't start");
        return Ok(());
    }
    let result = host.oauth2_login(code, redirect_uri).await;
    forget_state(&host, &state);
    result
}

/// Removes the pending sign-in's state once it has ended either way, so a
/// later redirect carrying it is ignored. A sign-in started since keeps its
/// own state.
fn forget_state(host: &QuadrantHost, state: &str) {
    let pending = host.get_config_value(OAUTH_STATE).ok().flatten();
    if pending.as_ref().and_then(Value::as_str) == Some(state)
        && let Err(error) = config::unset(host, OAUTH_STATE)
    {
        log::warn!("Couldn't forget the sign-in state: {error:#}");
    }
}

fn new_state() -> Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|error| anyhow!("no randomness available: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn authorize_url(client_id: &str, redirect_uri: &str, state: &str) -> Url {
    Url::parse_with_params(
        AUTHORIZE_URL,
        [
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("scope", SCOPES),
            ("response_type", "code"),
            ("state", state),
        ],
    )
    .expect("the authorize URL is a valid base")
}

/// Listens on the first free registered port.
async fn bind() -> Result<(TcpListener, u16)> {
    for port in PORTS {
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            return Ok((listener, port));
        }
    }
    bail!(
        "Ports {}-{} are all in use. Close whatever holds them and try again.",
        PORTS.start(),
        PORTS.end()
    )
}

#[derive(Debug, PartialEq, Eq)]
enum Callback {
    /// A request that isn't the redirect, such as the browser's favicon fetch.
    Ignored,
    Code {
        code: String,
        state: Option<String>,
    },
    Denied {
        error: String,
        state: Option<String>,
    },
}

/// Reads the redirect out of a request target like `/?code=..&state=..`.
fn parse_callback(target: &str) -> Callback {
    let Ok(url) = Url::parse("http://127.0.0.1").and_then(|base| base.join(target)) else {
        return Callback::Ignored;
    };
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    match (param("code"), param("error")) {
        (_, Some(error)) => Callback::Denied {
            error,
            state: param("state"),
        },
        (Some(code), None) => Callback::Code {
            code,
            state: param("state"),
        },
        (None, None) => Callback::Ignored,
    }
}

/// What a request means for the sign-in waiting on it.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    NotTheRedirect,
    /// A redirect for another sign-in, such as a stale tab's. It is refused
    /// and the wait goes on, so it can't cancel this sign-in.
    Foreign,
    Denied(String),
    Code(String),
}

fn judge(callback: Callback, expected_state: &str) -> Verdict {
    match callback {
        Callback::Ignored => Verdict::NotTheRedirect,
        Callback::Code { state, .. } | Callback::Denied { state, .. }
            if state.as_deref() != Some(expected_state) =>
        {
            Verdict::Foreign
        }
        Callback::Code { code, .. } => Verdict::Code(code),
        Callback::Denied { error, .. } => Verdict::Denied(error),
    }
}

/// Answers requests until the redirect for this sign-in arrives and returns
/// its code.
async fn wait_for_callback(listener: TcpListener, expected_state: &str) -> Result<String> {
    serve(listener, expected_state, READ_TIMEOUT).await
}

async fn serve(
    listener: TcpListener,
    expected_state: &str,
    read_timeout: Duration,
) -> Result<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let Ok(Some(target)) =
            tokio::time::timeout(read_timeout, read_request_target(&mut stream)).await
        else {
            continue;
        };
        match judge(parse_callback(&target), expected_state) {
            Verdict::NotTheRedirect => respond(&mut stream, "404 Not Found", "").await,
            Verdict::Foreign => {
                let page = page("This sign-in answer belongs to another sign-in. Close this tab.");
                respond(&mut stream, "400 Bad Request", &page).await;
            }
            Verdict::Denied(error) => {
                respond(&mut stream, "200 OK", &page(&t("returnToTheApp"))).await;
                bail!("The sign-in was not completed: {error}");
            }
            Verdict::Code(code) => {
                respond(&mut stream, "200 OK", &page(&t("returnToTheApp"))).await;
                return Ok(code);
            }
        }
    }
}

async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buffer = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).await.ok()?;
        if read == 0 || head.len() > MAX_REQUEST_HEAD {
            break;
        }
        head.extend_from_slice(&buffer[..read]);
    }
    let head = String::from_utf8_lossy(&head);
    let mut request_line = head.lines().next()?.split_whitespace();
    let _method = request_line.next()?;
    request_line.next().map(str::to_string)
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn page(message: &str) -> String {
    format!("<html><body><h1>{message}</h1></body></html>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_host::QuadrantHostOptions;

    #[test]
    fn states_are_48_hex_characters_and_differ() {
        let first = new_state().unwrap();
        assert_eq!(first.len(), 48);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(first, new_state().unwrap());
    }

    #[test]
    fn authorize_url_encodes_every_parameter() {
        let url = authorize_url("client", "http://127.0.0.1:4000", "abc");
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert_eq!(url.path(), "/account/oauth2/authorize");
        assert!(pairs.contains(&("redirect_uri".into(), "http://127.0.0.1:4000".into())));
        assert!(pairs.contains(&("scope".into(), SCOPES.into())));
        assert!(!url.as_str().contains(' '));
    }

    #[test]
    fn only_this_sign_ins_state_can_finish_or_cancel_it() {
        assert_eq!(parse_callback("/favicon.ico"), Callback::Ignored);
        assert_eq!(
            judge(parse_callback("/?code=xyz&state=abc"), "abc"),
            Verdict::Code("xyz".into())
        );
        assert_eq!(
            judge(parse_callback("/?error=access_denied&state=abc"), "abc"),
            Verdict::Denied("access_denied".into())
        );
        assert_eq!(
            judge(parse_callback("/?code=xyz&state=other"), "abc"),
            Verdict::Foreign
        );
        assert_eq!(judge(parse_callback("/?code=xyz"), "abc"), Verdict::Foreign);
    }

    async fn send(port: u16, target: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[test]
    fn loopback_skips_stray_requests_and_returns_the_code() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let waiting = tokio::spawn(wait_for_callback(listener, "abc"));
            assert!(send(port, "/favicon.ico").await.starts_with("HTTP/1.1 404"));
            let refused = send(port, "/?code=stale&state=someone-else").await;
            assert!(refused.starts_with("HTTP/1.1 400"), "{refused}");
            let page = send(port, "/?code=the-code&state=abc").await;
            assert!(page.contains("Login successful"), "{page}");
            assert_eq!(waiting.await.unwrap().unwrap(), "the-code");
        });
    }

    #[test]
    fn a_finished_sign_in_forgets_its_state_and_leaves_a_newer_ones() {
        let temp = tempfile::tempdir().unwrap();
        let mut options = QuadrantHostOptions::new(temp.path().to_path_buf(), "", "", "");
        options.keyring_service_name = "quadrant-gpui-tests".into();
        let host = QuadrantHost::new(options).unwrap();
        config::set(&host, OAUTH_STATE, Value::String("newer".into())).unwrap();
        forget_state(&host, "older");
        assert_eq!(
            host.get_config_value(OAUTH_STATE).unwrap(),
            Some(Value::String("newer".into()))
        );
        forget_state(&host, "newer");
        assert_eq!(host.get_config_value(OAUTH_STATE).unwrap(), None);
    }

    /// Default builds have no OAuth client, so signing in fails before it
    /// opens a port or a browser, with a reason the page can show.
    #[test]
    fn a_build_without_credentials_says_so() {
        let temp = tempfile::tempdir().unwrap();
        let mut options = QuadrantHostOptions::new(temp.path().to_path_buf(), "", "", "");
        options.keyring_service_name = "quadrant-gpui-tests".into();
        let host = QuadrantHost::new(options).unwrap();
        let (browser, opened) = oneshot::channel();
        let (_cancel, cancelled) = oneshot::channel();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(sign_in(host, browser, cancelled))
            .unwrap_err();
        assert!(
            error.to_string().contains("no Quadrant ID credentials"),
            "{error}"
        );
        assert!(runtime.block_on(opened).is_err(), "no browser was opened");
    }
}
