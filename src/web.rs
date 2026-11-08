//! Opening addresses that arrive in remote data.

use gpui::App;
use url::Url;

/// Whether `url` is an address to hand the system's browser: http or https.
/// Anything else (`file:`, a UNC path, another app's scheme) would make the
/// system run whatever handles it.
fn is_web(url: &str) -> bool {
    Url::parse(url.trim()).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// Opens `url` in the browser when it is a web address. Use it for every
/// address a provider, the news feed or a shared pack supplies.
pub fn open_external(url: &str, cx: &App) {
    if is_web(url) {
        cx.open_url(url.trim());
    } else {
        log::warn!("Not opening a link that isn't http(s): {url:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::is_web;

    #[test]
    fn only_http_and_https_open() {
        assert!(is_web("https://modrinth.com/mod/sodium"));
        assert!(is_web(" http://example.com/a?b=c "));
        for url in [
            "file:///C:/Windows/System32/calc.exe",
            "\\\\host\\share\\x.exe",
            "ms-msdt:/id",
            "javascript:alert(1)",
            "quadrantnext://login?code=x",
            "mailto:me@example.com",
            "",
            "modrinth.com/mod/sodium",
        ] {
            assert!(!is_web(url), "{url}");
        }
    }
}
