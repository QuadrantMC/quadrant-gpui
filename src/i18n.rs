use std::sync::LazyLock;

use gpui::SharedString;
use serde_json::{Map, Value};

/// The desktop app's English strings, vendored from `quadrant/src/locales/en.json`.
static ENGLISH: LazyLock<Map<String, Value>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../locales/en.json")).unwrap_or_default());

fn lookup(key: &str) -> Option<&'static str> {
    ENGLISH.get(key).and_then(Value::as_str)
}

/// The English text for `key`, or the key itself when the desktop app has no
/// such string, so a typo shows up on screen instead of as a blank.
pub fn t(key: &str) -> SharedString {
    lookup(key).unwrap_or(key).to_string().into()
}

/// [`t`] with i18next-style `{{name}}` placeholders filled in.
pub fn t_with(key: &str, values: &[(&str, &str)]) -> SharedString {
    let mut text = lookup(key).unwrap_or(key).to_string();
    for (name, value) in values {
        text = text.replace(&format!("{{{{{name}}}}}"), value);
    }
    text.into()
}

/// [`t_with`] for an i18next plural key: `key_one` when `count` is 1, else
/// `key_other`, with `{{count}}` filled in besides `values`.
pub fn t_count(key: &str, count: usize, values: &[(&str, &str)]) -> SharedString {
    let suffix = if count == 1 { "one" } else { "other" };
    let count = count.to_string();
    let mut all = vec![("count", count.as_str())];
    all.extend_from_slice(values);
    t_with(&format!("{key}_{suffix}"), &all)
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

    #[test]
    fn fills_placeholders() {
        assert_eq!(t_with("modCount", &[("amount", "3")]), "3 Mod(s)");
        assert_eq!(t("noSuchKey"), "noSuchKey");
    }

    #[test]
    fn picks_the_plural_form() {
        assert_eq!(
            t_count("installedContentSelected", 1, &[]),
            "1 pack selected"
        );
        assert_eq!(
            t_count("installedContentCopied", 3, &[("name", "Prism")]),
            "Copied 3 packs to Prism"
        );
    }
}
