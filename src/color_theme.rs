//! Color themes: the built-ins in `assets/themes` and the user's own JSON
//! files in `<data dir>/themes`, both read by [`parse`] into a [`Palette`].
//!
//! A file names its theme, its `mode` (`dark` or `light`), the built-in it
//! `extends` (Quadrant unless it says) and any of the palette's colors under
//! their camelCase names, as `#rrggbb` or `#rrggbbaa`; the rest come from the
//! theme it extends.

use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

use gpui::{App, Global, Rgba, SharedString};
use gpui_component::ThemeMode;
use serde_json::{Map, Value};

use crate::theme::{self, Palette};

/// A theme's id as `colorTheme` keeps it: a built-in's slug, or
/// `custom:<file stem>` for a user's file. Settings sync can bring one this
/// machine doesn't have; it resolves to Quadrant and is kept as it is.
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct ThemeId(SharedString);

impl ThemeId {
    pub const QUADRANT: &str = "quadrant";
    const CUSTOM: &str = "custom:";

    pub fn new(id: impl Into<SharedString>) -> Self {
        Self(id.into())
    }

    fn custom(stem: &str) -> Self {
        Self(format!("{}{stem}", Self::CUSTOM).into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ThemeId {
    fn default() -> Self {
        Self::new(Self::QUADRANT)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorTheme {
    pub id: ThemeId,
    pub name: SharedString,
    pub mode: ThemeMode,
    pub palette: Palette,
}

/// A user's theme file that didn't load, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct BrokenTheme {
    pub file: SharedString,
    pub error: SharedString,
}

const BUILT_IN_FILES: [(&str, &str); 9] = [
    ("quadrant", include_str!("../assets/themes/quadrant.json")),
    ("dracula", include_str!("../assets/themes/dracula.json")),
    (
        "tokyo-night",
        include_str!("../assets/themes/tokyo-night.json"),
    ),
    ("nord", include_str!("../assets/themes/nord.json")),
    (
        "catppuccin-mocha",
        include_str!("../assets/themes/catppuccin-mocha.json"),
    ),
    ("vaporwave", include_str!("../assets/themes/vaporwave.json")),
    ("light", include_str!("../assets/themes/light.json")),
    (
        "gruvbox-dark",
        include_str!("../assets/themes/gruvbox-dark.json"),
    ),
    ("ayu-dark", include_str!("../assets/themes/ayu-dark.json")),
];

/// The built-ins in the picker's order, Quadrant first. Each lists every
/// color, which a test checks, so it extends nothing.
static BUILT_IN: LazyLock<Vec<ColorTheme>> = LazyLock::new(|| {
    BUILT_IN_FILES
        .iter()
        .map(|(slug, json)| {
            parse(ThemeId::new(*slug), json, |_| None)
                .unwrap_or_else(|error| panic!("built-in theme {slug}: {error}"))
        })
        .collect()
});

pub fn built_in() -> &'static [ColorTheme] {
    &BUILT_IN
}

fn built_in_palette(slug: &str) -> Option<Palette> {
    if slug == ThemeId::QUADRANT {
        return Some(theme::QUADRANT);
    }
    built_in()
        .iter()
        .find(|theme| theme.id.as_str() == slug)
        .map(|theme| theme.palette)
}

/// Reads a theme file. `base` resolves `extends`; the built-ins pass none,
/// so they can only extend Quadrant.
pub fn parse(
    id: ThemeId,
    json: &str,
    base: impl Fn(&str) -> Option<Palette>,
) -> Result<ColorTheme, String> {
    let Value::Object(mut file) = serde_json::from_str::<Value>(json).map_err(|e| e.to_string())?
    else {
        return Err("a theme is a JSON object".into());
    };
    let name = match file.remove("name") {
        Some(Value::String(name)) if !name.trim().is_empty() => name,
        _ => return Err("\"name\" must be a non-empty string".into()),
    };
    let mode = match file.remove("mode") {
        None => ThemeMode::Dark,
        Some(Value::String(mode)) if mode == "dark" => ThemeMode::Dark,
        Some(Value::String(mode)) if mode == "light" => ThemeMode::Light,
        Some(_) => return Err("\"mode\" must be \"dark\" or \"light\"".into()),
    };
    let mut palette = match file.remove("extends") {
        None => theme::QUADRANT,
        Some(Value::String(slug)) => base(&slug)
            .or_else(|| (slug == ThemeId::QUADRANT).then_some(theme::QUADRANT))
            .ok_or_else(|| format!("\"extends\": no built-in theme is called \"{slug}\""))?,
        Some(_) => return Err("\"extends\" must be a built-in theme's id".into()),
    };
    let colors = match file.remove("colors") {
        None => Map::new(),
        Some(Value::Object(colors)) => colors,
        Some(_) => return Err("\"colors\" must be an object".into()),
    };
    if let Some(key) = file.keys().next() {
        return Err(format!("unknown key \"{key}\""));
    }
    for (key, value) in colors {
        let slot = Palette::FIELDS
            .iter()
            .find(|field| camel_case(field) == key)
            .and_then(|field| palette.field_mut(field))
            .ok_or_else(|| format!("\"colors\": unknown color \"{key}\""))?;
        *slot = value
            .as_str()
            .and_then(parse_hex)
            .ok_or_else(|| format!("\"{key}\": expected #rrggbb or #rrggbbaa, got {value}"))?;
    }
    Ok(ColorTheme {
        id,
        name: name.into(),
        mode,
        palette,
    })
}

fn parse_hex(text: &str) -> Option<Rgba> {
    let digits = text.strip_prefix('#')?;
    if !digits.chars().all(|char| char.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(digits, 16).ok()?;
    match digits.len() {
        6 => Some(gpui::rgb(value)),
        8 => Some(gpui::rgba(value)),
        _ => None,
    }
}

fn to_hex(color: Rgba) -> String {
    let byte = |channel: f32| (channel * 255.).round() as u8;
    let mut hex = format!(
        "#{:02x}{:02x}{:02x}",
        byte(color.r),
        byte(color.g),
        byte(color.b)
    );
    if color.a < 1. {
        hex.push_str(&format!("{:02x}", byte(color.a)));
    }
    hex
}

fn camel_case(snake: &str) -> String {
    let mut words = snake.split('_');
    let mut camel = words.next().unwrap_or_default().to_string();
    for word in words {
        let mut chars = word.chars();
        camel.extend(chars.next().map(|first| first.to_ascii_uppercase()));
        camel.push_str(chars.as_str());
    }
    camel
}

/// A theme file listing every color of `palette`.
pub fn to_json(name: &str, palette: &Palette) -> String {
    let colors: Map<String, Value> = Palette::FIELDS
        .iter()
        .filter_map(|field| Some((camel_case(field), to_hex(palette.field(field)?).into())))
        .collect();
    let file = serde_json::json!({ "name": name, "mode": "dark", "colors": colors });
    serde_json::to_string_pretty(&file).unwrap_or_default() + "\n"
}

pub fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join("themes")
}

/// The built-ins and the user's themes, a gpui global.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Themes {
    /// The user's files that loaded, by file name.
    pub custom: Vec<ColorTheme>,
    pub broken: Vec<BrokenTheme>,
}

impl Global for Themes {}

impl Themes {
    /// Every `*.json` in `folder`, by file name. A missing folder has none.
    pub fn scan(folder: &Path) -> Self {
        let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            })
            .collect();
        files.sort();
        let mut themes = Self::default();
        for path in files {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            let loaded = std::fs::read_to_string(&path)
                .map_err(|error| error.to_string())
                .and_then(|json| parse(ThemeId::custom(&stem), &json, built_in_palette));
            match loaded {
                Ok(theme) => themes.custom.push(theme),
                Err(error) => themes.broken.push(BrokenTheme {
                    file: file_name.into_owned().into(),
                    error: error.into(),
                }),
            }
        }
        themes
    }

    pub fn global(cx: &App) -> &Self {
        static NONE: Themes = Themes {
            custom: Vec::new(),
            broken: Vec::new(),
        };
        cx.try_global::<Self>().unwrap_or(&NONE)
    }

    /// The picker's list: the built-ins, then the user's.
    pub fn all(&self) -> impl Iterator<Item = &ColorTheme> {
        built_in().iter().chain(&self.custom)
    }

    /// `id`'s theme, or Quadrant when there is none by that id here.
    pub fn resolve(&self, id: &ThemeId) -> &ColorTheme {
        self.all()
            .find(|theme| theme.id == *id)
            .unwrap_or(&built_in()[0])
    }
}

/// Makes `folder` and, while it holds no theme, `example.json`: Quadrant's
/// every color, for the user to copy.
pub fn prepare_folder(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let has_theme = std::fs::read_dir(folder)?.flatten().any(|entry| {
        entry
            .path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
    });
    if !has_theme {
        std::fs::write(
            folder.join("example.json"),
            to_json("Example", &theme::QUADRANT),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(json: &str) -> Vec<String> {
        let file: Value = serde_json::from_str(json).unwrap();
        let mut keys: Vec<String> = file["colors"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    }

    #[test]
    fn every_built_in_lists_every_color() {
        let mut expected: Vec<String> = Palette::FIELDS.iter().map(|f| camel_case(f)).collect();
        expected.sort();
        for (slug, json) in BUILT_IN_FILES {
            assert_eq!(keys(json), expected, "{slug}");
        }
        let slugs: Vec<&str> = built_in().iter().map(|theme| theme.id.as_str()).collect();
        assert_eq!(
            slugs,
            [
                "quadrant",
                "dracula",
                "tokyo-night",
                "nord",
                "catppuccin-mocha",
                "vaporwave",
                "light",
                "gruvbox-dark",
                "ayu-dark"
            ]
        );
        assert_eq!(built_in()[6].mode, ThemeMode::Light);
    }

    #[test]
    fn the_quadrant_file_is_the_default_palette() {
        assert_eq!(built_in()[0].palette, theme::QUADRANT);
        assert_eq!(built_in()[0].name, "Quadrant");
    }

    #[test]
    fn the_example_round_trips() {
        let example = parse(
            ThemeId::custom("example"),
            &to_json("Example", &theme::QUADRANT),
            |_| None,
        )
        .unwrap();
        assert_eq!(example.palette, theme::QUADRANT);
        assert_eq!(example.name, "Example");
        assert_eq!(example.id.as_str(), "custom:example");
    }

    #[test]
    fn missing_colors_come_from_the_theme_it_extends() {
        let theme = parse(
            ThemeId::custom("mine"),
            r##"{ "name": "Mine", "mode": "light", "extends": "dracula", "colors": { "page": "#000000", "scrim": "#ff000080" } }"##,
            built_in_palette,
        )
        .unwrap();
        let dracula = built_in_palette("dracula").unwrap();
        assert_eq!(theme.mode, ThemeMode::Light);
        assert_eq!(theme.palette.page, gpui::rgb(0x000000));
        assert_eq!(theme.palette.scrim, gpui::rgba(0xff000080));
        assert_eq!(theme.palette.surface, dracula.surface);

        let plain = parse(
            ThemeId::custom("plain"),
            r##"{ "name": "Plain" }"##,
            built_in_palette,
        )
        .unwrap();
        assert_eq!(plain.palette, theme::QUADRANT);
        assert_eq!(plain.mode, ThemeMode::Dark);
    }

    #[test]
    fn errors_name_the_key() {
        let error = |json: &str| parse(ThemeId::custom("bad"), json, built_in_palette).unwrap_err();
        assert!(error(r##"{ "name": "A", "colors": { "pgae": "#000000" } }"##).contains("pgae"));
        assert!(error(r##"{ "name": "A", "colors": { "page": "#00000" } }"##).contains("page"));
        assert!(error(r##"{ "name": "A", "colors": { "page": "red" } }"##).contains("page"));
        assert!(error(r##"{ "name": "A", "colors": { "page": 7 } }"##).contains("page"));
        assert!(error(r##"{ "name": "A", "colour": {} }"##).contains("colour"));
        assert!(error(r##"{ "name": "A", "extends": "solarized" }"##).contains("solarized"));
        assert!(error(r##"{ "name": "A", "mode": "dim" }"##).contains("mode"));
        assert!(error(r##"{ "colors": {} }"##).contains("name"));
        assert!(!error("{").is_empty());
    }

    #[test]
    fn scanning_loads_good_files_and_reports_broken_ones() {
        let temp = tempfile::tempdir().unwrap();
        let folder = folder(temp.path());
        assert_eq!(Themes::scan(&folder), Themes::default());

        prepare_folder(&folder).unwrap();
        std::fs::write(
            folder.join("broken.json"),
            r##"{ "name": "B", "colors": { "nope": "#000000" } }"##,
        )
        .unwrap();
        std::fs::write(folder.join("notes.txt"), "not a theme").unwrap();
        let themes = Themes::scan(&folder);
        assert_eq!(themes.custom.len(), 1);
        assert_eq!(themes.custom[0].id.as_str(), "custom:example");
        assert_eq!(themes.broken.len(), 1);
        assert_eq!(themes.broken[0].file, "broken.json");
        assert!(themes.broken[0].error.contains("nope"));

        std::fs::remove_file(folder.join("example.json")).unwrap();
        prepare_folder(&folder).unwrap();
        assert!(
            !folder.join("example.json").exists(),
            "a folder with a theme file gets no example"
        );
    }

    #[test]
    fn unknown_ids_resolve_to_quadrant() {
        let themes = Themes::default();
        assert_eq!(
            themes
                .resolve(&ThemeId::new("custom:elsewhere"))
                .id
                .as_str(),
            "quadrant"
        );
        assert_eq!(themes.resolve(&ThemeId::new("nord")).name, "Nord");
    }
}
