//! Every config write the app makes, so settings sync sees it.
//!
//! Settings sync (`quadrant_settings_sync.rs` in quadrant-core) pushes the
//! local settings when their `lastSettingsUpdated` is newer than the cloud
//! copy's and pulls them when it is older. The desktop app sets it to now on
//! every other config change (`App.tsx`, `config.onChange`); the host does
//! not, so the app has to. `clippy.toml` keeps the host's own setters out of
//! every other module.

use anyhow::Result;
use chrono::{SecondsFormat, Utc};
use quadrant_host::QuadrantHost;
use serde_json::Value;

const LAST_SETTINGS_UPDATED: &str = "lastSettingsUpdated";

pub fn set(host: &QuadrantHost, key: &str, value: Value) -> Result<()> {
    #[expect(clippy::disallowed_methods, reason = "the one writer")]
    host.set_config_value(key, value)?;
    touch(host, key)
}

pub fn unset(host: &QuadrantHost, key: &str) -> Result<()> {
    #[expect(clippy::disallowed_methods, reason = "the one writer")]
    host.remove_config_value(key)?;
    touch(host, key)
}

fn touch(host: &QuadrantHost, key: &str) -> Result<()> {
    if key == LAST_SETTINGS_UPDATED {
        return Ok(());
    }
    #[expect(clippy::disallowed_methods, reason = "the one writer")]
    host.set_config_value(LAST_SETTINGS_UPDATED, Value::String(now()))
}

/// What JavaScript's `new Date().toISOString()` writes: UTC, milliseconds,
/// a `Z`.
fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_host::QuadrantHostOptions;

    #[test]
    fn writes_bump_last_settings_updated_like_the_desktop_app() {
        let temp = tempfile::tempdir().unwrap();
        let mut options = QuadrantHostOptions::new(temp.path().join("data"), "", "", "");
        options.mc_folder = Some(temp.path().join("mc"));
        options.keyring_service_name = "quadrant-gpui-tests".into();
        let host = QuadrantHost::new(options).unwrap();
        host.init_config().unwrap();
        let stamp = || {
            host.get_config_value(LAST_SETTINGS_UPDATED)
                .unwrap()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap()
        };

        let before = stamp();
        std::thread::sleep(std::time::Duration::from_millis(5));
        set(&host, "clipIcons", false.into()).unwrap();
        let after_set = stamp();
        assert!(after_set > before, "{after_set} after {before}");
        // `2026-11-05T12:34:56.789Z`, as `toISOString` writes it.
        assert_eq!(after_set.len(), 24, "{after_set}");
        assert!(after_set.ends_with('Z'));
        assert!(chrono::DateTime::parse_from_rfc3339(&after_set).is_ok());

        std::thread::sleep(std::time::Duration::from_millis(5));
        unset(&host, "clipIcons").unwrap();
        assert!(stamp() > after_set);

        set(
            &host,
            LAST_SETTINGS_UPDATED,
            "2020-01-01T00:00:00.000Z".into(),
        )
        .unwrap();
        assert_eq!(stamp(), "2020-01-01T00:00:00.000Z");
    }
}
