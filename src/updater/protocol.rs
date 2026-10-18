//! Tauri's updater protocol, as Quadrant's update server speaks it: the
//! endpoint, the answer, and the minisign check of the download. Pure, so it
//! is tested without a network.

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;

/// The update server. Quadrant-Native is its own app there, beside `quadrant`.
pub const API_BASE: &str = "https://api.usequadrant.dev";
pub const APP_SLUG: &str = "quadrant-native";
pub const DEFAULT_CHANNEL: &str = "stable";

/// The minisign public key updates must be signed with, base64 as in
/// `tauri.conf.json`. Quadrant-Native shares Quadrant's key, so one signing
/// secret serves both apps. A build can swap it (`QUADRANT_NATIVE_UPDATER_PUBKEY`)
/// to test against a throwaway key.
pub const PUBKEY: &str = match option_env!("QUADRANT_NATIVE_UPDATER_PUBKEY") {
    Some(key) => key,
    None => QUADRANT_PUBKEY,
};
const QUADRANT_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEE3MEJDMDAxMjA4NjlBQ0MKUldUTW1vWWdBY0FMcCtkS3FwaTlKSzBaMHpXaW4ramRLS1ludmVyUEZpSUY3Mmc0cm1zeGUvRGIK";

/// How this copy was installed. It picks the artifact the server answers
/// with (Tauri's `{{bundle_type}}`) and how the update is applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bundle {
    /// The NSIS setup, run passively over the install.
    Nsis,
    /// The macOS `.app`, replaced from an `.app.tar.gz`.
    App,
    AppImage,
    Deb,
    Rpm,
}

impl Bundle {
    pub fn variant(self) -> &'static str {
        match self {
            Bundle::Nsis => "nsis",
            Bundle::App => "app",
            Bundle::AppImage => "appimage",
            Bundle::Deb => "deb",
            Bundle::Rpm => "rpm",
        }
    }
}

/// Tauri's `{{target}}`.
pub fn target() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

/// Tauri's `{{arch}}`.
pub fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86" => "i686",
        "arm" => "armv7",
        other => other,
    }
}

/// The URLs to ask, in order: the chosen channel's, then stable's, as the
/// desktop app does. The server already answers a channel with stable when
/// stable is newer; the second URL only matters when the first fails.
pub fn endpoints(base: &str, channel: &str, bundle: Bundle, current_version: &str) -> Vec<String> {
    let mut channels = vec![channel];
    if channel != DEFAULT_CHANNEL {
        channels.push(DEFAULT_CHANNEL);
    }
    channels
        .into_iter()
        .map(|channel| {
            format!(
                "{}/api/any/{APP_SLUG}/updates/{channel}/{}/{}/{current_version}?variant={}",
                base.trim_end_matches('/'),
                target(),
                arch(),
                bundle.variant(),
            )
        })
        .collect()
}

/// The server's answer: the newest release for this platform and variant.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
    /// Base64 of the artifact's minisign `.sig` file.
    pub signature: String,
}

impl Release {
    pub fn parse(body: &[u8]) -> Result<Self> {
        let mut release: Release =
            serde_json::from_slice(body).context("the update server's answer isn't a release")?;
        if let Some(version) = release.version.strip_prefix('v') {
            release.version = version.to_string();
        }
        Ok(release)
    }

    /// Whether to install it. Like the desktop app, any other version counts,
    /// so switching from preview back to stable moves to stable's release.
    pub fn is_update_for(&self, current_version: &str) -> bool {
        self.version != current_version
    }

    /// The download must use TLS unless a debug build's test server serves it.
    pub fn check_url(&self, allow_http: bool) -> Result<()> {
        let url = url::Url::parse(&self.url).context("the update's download URL is invalid")?;
        match url.scheme() {
            "https" => Ok(()),
            "http" if allow_http => Ok(()),
            scheme => bail!("refusing to download an update over {scheme}"),
        }
    }
}

/// Checks `bytes` against a minisign signature, both as Tauri writes them:
/// base64 of the `.sig` file and base64 of the public key file.
pub fn verify(bytes: &[u8], signature: &str, pubkey: &str) -> Result<()> {
    let pubkey = PublicKey::decode(&decode_base64(pubkey)?).context("bad updater public key")?;
    let signature =
        Signature::decode(&decode_base64(signature)?).context("bad update signature")?;
    pubkey
        .verify(bytes, &signature, true)
        .context("the update's signature doesn't match")
}

fn decode_base64(text: &str) -> Result<String> {
    Ok(String::from_utf8(STANDARD.decode(text.trim())?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway key pair made with `tauri signer generate` for these tests.
    const TEST_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEUxRjhGMjBFMDQzNUJDNDEKUldSQnZEVUVEdkw0NFg3Y3dOaThZK0FzNzg3L05VcFZETERJcVZwcEs5UTVmck5WM1NBdm0rSGcK";
    const FIXTURE: &[u8] = b"quadrant-native update fixture\n";
    // `tauri signer sign` of FIXTURE with that key, and with a different key.
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSQnZEVUVEdkw0NFVUTzJxU1orMnErZ1ZXUmYvL1FiMTlXNnlGR3VYMUhXcE11U2oxUk9wM1M3K09WM0hXUnVUWGpHcnpJWG9uYXlCTkV3RWV5bGVHbDBYSHN3bG1tU3djPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMzIxOTMxCWZpbGU6Zml4dHVyZS5iaW4KMkkyR2VHVmJDKzhZMkk2dlJ2VmNNenUwK3Z3Q3AyWVNkN2N3dDJnY2FhZ1pzL1FaRFY5T1ZjZC9Vdk5kTDVoMWtta0tpRExxdzFkTHlYRk9WT3VOQVE9PQo=";
    const OTHER_KEY_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVSdW9QQzNTdzlDb1N5MkpkRU5zK0dIUDF4VnZzcnJVUENwS281SzlFVk1VYWh0cUdPK0EzMDRKU09lZnJkTWtlblc2NjJ3UDMzeUV0MVRmL0lrczZZRFhDaWVhdGNTY0FFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyMzIxOTMyCWZpbGU6Zml4dHVyZS5iaW4KSkZlNmFuaTBsOFBYQldHcU51RXdONzg1Z3BvTnhybmpGRXJCUjgxUUVJNmIwOGNDZFE5WDRrVzlOVTdDWEk4c21KakRKVHpLOHU3VXZPNlhsOEwzQnc9PQo=";

    #[test]
    fn accepts_the_signed_bytes_only() {
        verify(FIXTURE, SIGNATURE, TEST_PUBKEY).unwrap();
        let mut tampered = FIXTURE.to_vec();
        tampered[0] ^= 1;
        assert!(verify(&tampered, SIGNATURE, TEST_PUBKEY).is_err());
        assert!(verify(FIXTURE, OTHER_KEY_SIGNATURE, TEST_PUBKEY).is_err());
        assert!(verify(FIXTURE, "not base64", TEST_PUBKEY).is_err());
    }

    #[test]
    fn quadrants_key_is_the_default_and_decodes() {
        if option_env!("QUADRANT_NATIVE_UPDATER_PUBKEY").is_none() {
            assert_eq!(PUBKEY, QUADRANT_PUBKEY);
        }
        PublicKey::decode(&decode_base64(QUADRANT_PUBKEY).unwrap()).unwrap();
        assert!(verify(FIXTURE, SIGNATURE, QUADRANT_PUBKEY).is_err());
    }

    #[test]
    fn asks_the_channel_then_stable() {
        let urls = endpoints(
            "https://api.test/",
            "preview",
            Bundle::Nsis,
            "26.10.0-stable",
        );
        let tail = format!("{}/{}/26.10.0-stable?variant=nsis", target(), arch());
        assert_eq!(
            urls,
            [
                format!("https://api.test/api/any/quadrant-native/updates/preview/{tail}"),
                format!("https://api.test/api/any/quadrant-native/updates/stable/{tail}"),
            ]
        );
        assert_eq!(
            endpoints(API_BASE, "stable", Bundle::AppImage, "1.0.0").len(),
            1
        );
    }

    #[test]
    fn platform_names_match_tauris() {
        assert!(["windows", "darwin", "linux"].contains(&target()));
        assert!(["x86_64", "aarch64", "i686", "armv7"].contains(&arch()));
    }

    #[test]
    fn reads_the_servers_row() {
        let body = br#"{
            "product": "quadrant-native", "version": "v26.10.1-stable",
            "pub_date": "2026-10-20T10:00:00+00:00", "arch": "x86_64",
            "signature": "c2ln", "platform": "windows",
            "url": "https://github.com/a/b/releases/download/v26.10.1-stable/setup.exe",
            "branch": "stable", "public": true, "version_id": "x",
            "platform_variation": "nsis"
        }"#;
        let release = Release::parse(body).unwrap();
        assert_eq!(release.version, "26.10.1-stable");
        assert_eq!(release.signature, "c2ln");
        assert!(release.is_update_for("26.10.0-stable"));
        assert!(!release.is_update_for("26.10.1-stable"));
        assert!(release.check_url(false).is_ok());
        assert!(Release::parse(b"No supported versions").is_err());
    }

    #[test]
    fn plain_http_downloads_need_the_test_server() {
        let release = Release {
            version: "1.0.0".into(),
            url: "http://127.0.0.1:8000/setup.exe".into(),
            signature: String::new(),
        };
        assert!(release.check_url(false).is_err());
        assert!(release.check_url(true).is_ok());
    }
}
