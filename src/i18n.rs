//! The desktop app's strings (`quadrant/src/locales`) and how i18next picks
//! one: the chosen language first, then English, then the key itself.

use std::sync::{
    LazyLock,
    atomic::{AtomicU8, Ordering},
};

use gpui::SharedString;
use serde_json::{Map, Value};

type Strings = LazyLock<Map<String, Value>>;

static ENGLISH: Strings =
    LazyLock::new(|| serde_json::from_str(include_str!("../locales/en.json")).unwrap_or_default());
static UKRAINIAN: Strings =
    LazyLock::new(|| serde_json::from_str(include_str!("../locales/uk.json")).unwrap_or_default());
static TURKISH: Strings =
    LazyLock::new(|| serde_json::from_str(include_str!("../locales/tr.json")).unwrap_or_default());

/// A language the desktop app ships, kept under the `locale` setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Locale {
    #[default]
    English,
    Ukrainian,
    Turkish,
}

impl Locale {
    /// In the order the settings' language picker lists them.
    pub const ALL: [Locale; 3] = [Locale::English, Locale::Ukrainian, Locale::Turkish];

    /// The `locale` setting's value.
    pub fn code(self) -> &'static str {
        match self {
            Locale::English => "en",
            Locale::Ukrainian => "uk",
            Locale::Turkish => "tr",
        }
    }

    /// The picker's label, untranslated there too.
    pub fn native_name(self) -> &'static str {
        match self {
            Locale::English => "English",
            Locale::Ukrainian => "Українська",
            Locale::Turkish => "Türkçe",
        }
    }

    /// The language a `locale` setting names. i18next falls back from a
    /// regional code (`tr-TR`) to its language and from an unknown one to
    /// English, which is also what an unset setting means.
    pub fn parse(code: Option<&str>) -> Self {
        let code = code.unwrap_or_default();
        let language = code.split(['-', '_']).next().unwrap_or_default();
        Self::ALL
            .into_iter()
            .find(|locale| locale.code() == language)
            .unwrap_or_default()
    }

    fn strings(self) -> &'static Map<String, Value> {
        match self {
            Locale::English => &ENGLISH,
            Locale::Ukrainian => &UKRAINIAN,
            Locale::Turkish => &TURKISH,
        }
    }

    /// The CLDR plural category of a whole `count`, as `Intl.PluralRules`
    /// names it.
    fn plural(self, count: usize) -> &'static str {
        match self {
            Locale::English | Locale::Turkish => {
                if count == 1 {
                    "one"
                } else {
                    "other"
                }
            }
            Locale::Ukrainian => match (count % 10, count % 100) {
                (1, rest) if rest != 11 => "one",
                (2..=4, rest) if !(12..=14).contains(&rest) => "few",
                _ => "many",
            },
        }
    }

    /// The chain i18next looks a key up in.
    fn chain(self) -> impl Iterator<Item = Locale> {
        std::iter::once(self).chain((self != Locale::English).then_some(Locale::English))
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// The language strings are shown in.
pub fn locale() -> Locale {
    #[cfg(test)]
    if let Some(locale) = tests::LOCALE.get() {
        return locale;
    }
    Locale::ALL
        .get(usize::from(CURRENT.load(Ordering::Relaxed)))
        .copied()
        .unwrap_or_default()
}

/// Shows strings in `locale` from now on. Views pick it up on their next
/// render; [`crate::prefs`] asks every window to render again.
pub fn set_locale(locale: Locale) {
    let index = Locale::ALL.iter().position(|each| *each == locale);
    CURRENT.store(index.unwrap_or_default() as u8, Ordering::Relaxed);
}

fn lookup_in(locale: Locale, key: &str) -> Option<&'static str> {
    locale.strings().get(key).and_then(Value::as_str)
}

fn lookup(key: &str) -> Option<&'static str> {
    locale().chain().find_map(|locale| lookup_in(locale, key))
}

fn fill(text: &str, values: &[(&str, &str)]) -> String {
    let mut text = text.to_string();
    for (name, value) in values {
        text = text.replace(&format!("{{{{{name}}}}}"), value);
    }
    text
}

/// The text for `key`, or the key itself when the desktop app has no such
/// string, so a typo shows up on screen instead of as a blank.
pub fn t(key: &str) -> SharedString {
    lookup(key).unwrap_or(key).to_string().into()
}

/// [`t`] with i18next-style `{{name}}` placeholders filled in.
pub fn t_with(key: &str, values: &[(&str, &str)]) -> SharedString {
    fill(lookup(key).unwrap_or(key), values).into()
}

/// [`t_with`] for an i18next plural key, with `{{count}}` filled in besides
/// `values`. Each language in the chain is asked for its own plural form
/// (`key_few` in Ukrainian, `key_other` in English), then for the bare key.
pub fn t_count(key: &str, count: usize, values: &[(&str, &str)]) -> SharedString {
    let text = locale()
        .chain()
        .find_map(|locale| {
            let zero = (count == 0).then(|| format!("{key}_zero"));
            let plural = format!("{key}_{}", locale.plural(count));
            zero.into_iter()
                .chain([plural, key.to_string()])
                .find_map(|candidate| lookup_in(locale, &candidate))
        })
        .unwrap_or(key);
    let count = count.to_string();
    let mut all = vec![("count", count.as_str())];
    all.extend_from_slice(values);
    fill(text, &all).into()
}

/// A count the way `Intl.NumberFormat(language, { notation: "compact",
/// compactDisplay: "short", maximumFractionDigits: 1 })` writes it: "999",
/// "1.5K" in English, "1,5 B" in Turkish, "1,5 тис." in Ukrainian.
pub fn compact_count(count: i64) -> String {
    // CLDR's short compact patterns; the space before a unit is a no-break
    // space.
    let (units, decimal): ([&str; 4], char) = match locale() {
        Locale::English => (["K", "M", "B", "T"], '.'),
        Locale::Turkish => (["\u{a0}B", "\u{a0}Mn", "\u{a0}Mr", "\u{a0}Tn"], ','),
        Locale::Ukrainian => (["\u{a0}тис.", "\u{a0}млн", "\u{a0}млрд", "\u{a0}трлн"], ','),
    };
    let value = count as f64;
    let scale = |unit: usize| 1000f64.powi(unit as i32);
    let mut unit = (1..=units.len())
        .rev()
        .find(|unit| value.abs() >= scale(*unit))
        .unwrap_or(0);
    let tenths = |unit: usize| (value / scale(unit) * 10.).round() / 10.;
    // 999,960 rounds to 1000K, which reads as the next unit up.
    if unit > 0 && unit < units.len() && tenths(unit).abs() >= 1000. {
        unit += 1;
    }
    let shown = if unit == 0 { value } else { tenths(unit) };
    let digits = if shown.fract() == 0. { 0 } else { 1 };
    let number = format!("{shown:.digits$}").replace('.', &decimal.to_string());
    match unit {
        0 => number,
        unit => number + units[unit - 1],
    }
}

/// The text for a failed host call, mirroring `describeError` in the
/// desktop app: classified failures arrive as i18n keys, anything else is shown
/// as it came.
pub fn describe(error: anyhow::Error) -> SharedString {
    let raw = format!("{:#}", quadrant_host::user_facing(error));
    describe_raw(raw.trim()).into()
}

fn describe_raw(raw: &str) -> String {
    if raw.is_empty() {
        return t("errorUnknownNoDetails").to_string();
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
pub mod tests {
    use super::*;
    use quadrant_host::ErrorCode;

    thread_local! {
        /// A test's language, so tests running side by side don't see each
        /// other's.
        pub static LOCALE: std::cell::Cell<Option<Locale>> = const { std::cell::Cell::new(None) };
    }

    pub fn in_locale<R>(locale: Locale, f: impl FnOnce() -> R) -> R {
        LOCALE.set(Some(locale));
        let result = f();
        LOCALE.set(None);
        result
    }

    #[test]
    fn error_codes_become_english() {
        in_locale(Locale::English, || {
            assert_eq!(
                describe(ErrorCode::ModpackExists.into()),
                "A modpack with that name already exists. Choose a different name."
            );
        });
    }

    #[test]
    fn unclassified_errors_pass_through() {
        in_locale(Locale::English, || {
            assert_eq!(
                describe(anyhow::anyhow!("failed to parse config.json")),
                "failed to parse config.json"
            );
            assert_eq!(describe_raw(""), "Something went wrong. Please try again.");
        });
    }

    #[test]
    fn fills_placeholders() {
        in_locale(Locale::English, || {
            assert_eq!(t_with("modCount", &[("amount", "3")]), "3 Mod(s)");
            assert_eq!(t("noSuchKey"), "noSuchKey");
        });
    }

    #[test]
    fn picks_the_plural_form() {
        in_locale(Locale::English, || {
            assert_eq!(
                t_count("installedContentSelected", 1, &[]),
                "1 pack selected"
            );
            assert_eq!(
                t_count("installedContentCopied", 3, &[("name", "Prism")]),
                "Copied 3 packs to Prism"
            );
            assert_eq!(t_count("noSuchKey", 3, &[]), "noSuchKey");
        });
    }

    #[test]
    fn ukrainian_has_four_plural_forms() {
        let category = |count| Locale::Ukrainian.plural(count);
        assert_eq!(
            [1, 21, 2, 4, 22, 5, 11, 12, 14, 0, 100, 111].map(category),
            [
                "one", "one", "few", "few", "few", "many", "many", "many", "many", "many", "many",
                "many"
            ]
        );
        in_locale(Locale::Ukrainian, || {
            assert_eq!(
                t_count("installedContentSelected", 3, &[]),
                lookup_in(Locale::Ukrainian, "installedContentSelected_few")
                    .unwrap()
                    .replace("{{count}}", "3")
            );
        });
    }

    #[test]
    fn switches_language_and_falls_back_to_english() {
        in_locale(Locale::Turkish, || {
            assert_eq!(t("language"), "Dil");
            assert_eq!(t("noSuchKey"), "noSuchKey");
        });
        in_locale(Locale::Ukrainian, || {
            assert_eq!(t("language"), "Мова");
        });
        // A key only English has, as one added upstream before it's
        // translated: i18next shows the English string.
        assert!(lookup_in(Locale::Turkish, "productName").is_some());
        let only_english = ENGLISH
            .keys()
            .find(|key| lookup_in(Locale::Turkish, key).is_none());
        if let Some(key) = only_english {
            in_locale(Locale::Turkish, || {
                assert_eq!(t(key), lookup_in(Locale::English, key).unwrap())
            });
        }
    }

    #[test]
    fn every_locale_has_every_english_key() {
        for locale in Locale::ALL {
            let missing: Vec<_> = ENGLISH
                .keys()
                .filter(|key| {
                    lookup_in(locale, key).is_none()
                        && !(key.ends_with("_one") || key.ends_with("_other"))
                })
                .collect();
            assert!(missing.is_empty(), "{locale:?} lacks {missing:?}");
        }
    }

    #[test]
    fn parses_the_locale_setting_like_i18next() {
        assert_eq!(Locale::parse(None), Locale::English);
        assert_eq!(Locale::parse(Some("uk")), Locale::Ukrainian);
        assert_eq!(Locale::parse(Some("tr-TR")), Locale::Turkish);
        assert_eq!(Locale::parse(Some("de")), Locale::English);
    }

    /// Node's `Intl.NumberFormat` output for each, from the desktop app's
    /// `Mod.tsx` options.
    #[test]
    fn compact_counts_match_intl() {
        let cases: [(i64, &str, &str, &str); 8] = [
            (999, "999", "999", "999"),
            (1000, "1K", "1\u{a0}B", "1\u{a0}тис."),
            (1500, "1.5K", "1,5\u{a0}B", "1,5\u{a0}тис."),
            (90_312, "90.3K", "90,3\u{a0}B", "90,3\u{a0}тис."),
            (999_960, "1M", "1\u{a0}Mn", "1\u{a0}млн"),
            (174_200_000, "174.2M", "174,2\u{a0}Mn", "174,2\u{a0}млн"),
            (2_000_000_000, "2B", "2\u{a0}Mr", "2\u{a0}млрд"),
            (1_000_000_000_000, "1T", "1\u{a0}Tn", "1\u{a0}трлн"),
        ];
        for (count, english, turkish, ukrainian) in cases {
            for (locale, expected) in [
                (Locale::English, english),
                (Locale::Turkish, turkish),
                (Locale::Ukrainian, ukrainian),
            ] {
                assert_eq!(in_locale(locale, || compact_count(count)), expected);
            }
        }
    }
}
