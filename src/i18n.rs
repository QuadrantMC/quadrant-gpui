use std::sync::LazyLock;

use gpui::SharedString;
use serde_json::{Map, Value};

/// The desktop app's English strings, vendored from `quadrant/src/locales/en.json`.
static ENGLISH: LazyLock<Map<String, Value>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../locales/en.json")).unwrap_or_default());

fn lookup(key: &str) -> Option<&'static str> {
    ENGLISH.get(key).and_then(Value::as_str)
}

/// The English text for a failed host call, mirroring `describeError` in the
/// desktop app: classified failures arrive as i18n keys, anything else is shown
/// as it came.
pub fn describe(error: anyhow::Error) -> SharedString {
    let raw = format!("{:#}", quadrant_host::user_facing(error));
    describe_raw(raw.trim()).into()
}

fn describe_raw(raw: &str) -> String {
    if raw.is_empty() {
        return lookup("errorUnknownNoDetails")
            .unwrap_or("Something went wrong.")
            .to_string();
    }
    // A key never contains whitespace, so an English sentence can't shadow one.
    if !raw.contains(char::is_whitespace)
        && let Some(text) = lookup(raw)
    {
        return text.to_string();
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadrant_host::ErrorCode;

    #[test]
    fn error_codes_become_english() {
        assert_eq!(
            describe(ErrorCode::ModpackExists.into()),
            "A modpack with that name already exists. Choose a different name."
        );
    }

    #[test]
    fn unclassified_errors_pass_through() {
        assert_eq!(
            describe(anyhow::anyhow!("failed to parse config.json")),
            "failed to parse config.json"
        );
        assert_eq!(describe_raw(""), "Something went wrong. Please try again.");
    }
}
