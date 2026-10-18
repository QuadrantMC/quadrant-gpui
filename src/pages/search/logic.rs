//! The Search page's pure logic, after `searchLogic.ts` and the filter state
//! in `SearchPage.tsx`: which providers a search asks, with which arguments,
//! and how their answers merge into one list.

use chrono::DateTime;
use quadrant_core::{
    mc_mod::{GlobalSearchModsArgs, Mod, ModType, SearchCategory},
    models::{LocalModpack, ModLoader, ModSource},
};

use crate::{assets::MdIcon, i18n::t};

/// Results are paged client-side over the merged provider lists.
pub const PAGE_SIZE: usize = 25;

/// A provider the search can ask.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provider {
    CurseForge,
    Modrinth,
}

impl Provider {
    /// The providers this build can search, in request order. CurseForge
    /// needs the `curseforge` feature.
    pub fn available() -> &'static [Provider] {
        if cfg!(feature = "curseforge") {
            &[Provider::CurseForge, Provider::Modrinth]
        } else {
            &[Provider::Modrinth]
        }
    }

    pub fn source(self) -> ModSource {
        match self {
            Provider::CurseForge => ModSource::CurseForge,
            Provider::Modrinth => ModSource::Modrinth,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::CurseForge => "CurseForge",
            Provider::Modrinth => "Modrinth",
        }
    }

    /// Whether `loader` can be asked of this provider. CurseForge knows only
    /// the loaders it has an id for.
    pub fn supports(self, loader: ModLoader) -> bool {
        match self {
            Provider::CurseForge => loader.curseforge_id().is_some(),
            Provider::Modrinth => loader.modrinth_slug().is_some(),
        }
    }
}

/// What kind of content the search is for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ContentType {
    Mods,
    ResourcePacks,
    Shaders,
}

impl ContentType {
    pub const ALL: [ContentType; 3] = [
        ContentType::Mods,
        ContentType::ResourcePacks,
        ContentType::Shaders,
    ];

    pub fn mod_type(self) -> ModType {
        match self {
            ContentType::Mods => ModType::Mod,
            ContentType::ResourcePacks => ModType::ResourcePack,
            ContentType::Shaders => ModType::ShaderPack,
        }
    }

    /// The `modType` string the providers expect, as the desktop app sends it.
    fn request_name(self) -> &'static str {
        match self {
            ContentType::Mods => "Mod",
            ContentType::ResourcePacks => "ResourcePack",
            ContentType::Shaders => "ShaderPack",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            ContentType::Mods => "contentMods",
            ContentType::ResourcePacks => "contentResourcePacks",
            ContentType::Shaders => "contentShaders",
        }
    }

    pub fn icon(self) -> MdIcon {
        match self {
            ContentType::Mods => MdIcon::Extension,
            ContentType::ResourcePacks => MdIcon::GridView,
            ContentType::Shaders => MdIcon::WbSunny,
        }
    }

    /// Only mods have a loader.
    pub fn has_loader(self) -> bool {
        self == ContentType::Mods
    }
}

/// How the merged results are ordered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sort {
    Relevance,
    Downloads,
    Name,
    Updated,
}

impl Sort {
    pub const ALL: [Sort; 4] = [Sort::Relevance, Sort::Downloads, Sort::Name, Sort::Updated];

    /// The key the providers take as `sortBy`.
    pub fn value(self) -> &'static str {
        match self {
            Sort::Relevance => "relevance",
            Sort::Downloads => "downloads",
            Sort::Name => "name",
            Sort::Updated => "updated",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Sort::Relevance => "sortRelevance",
            Sort::Downloads => "sortDownloads",
            Sort::Name => "sortName",
            Sort::Updated => "sortUpdated",
        }
    }
}

/// Loader choices in the order the desktop app offers them.
pub const LOADERS: [ModLoader; 13] = [
    ModLoader::Fabric,
    ModLoader::Forge,
    ModLoader::NeoForge,
    ModLoader::Babric,
    ModLoader::BtaBabric,
    ModLoader::JavaAgent,
    ModLoader::LegacyFabric,
    ModLoader::LiteLoader,
    ModLoader::RisugamisModLoader,
    ModLoader::NilLoader,
    ModLoader::Ornithe,
    ModLoader::Quilt,
    ModLoader::Rift,
];

/// The loaders at least one of `providers` can search, or every loader when
/// none is on.
pub fn loader_options(providers: &[Provider]) -> Vec<ModLoader> {
    LOADERS
        .into_iter()
        .filter(|loader| {
            providers.is_empty() || providers.iter().any(|provider| provider.supports(*loader))
        })
        .collect()
}

/// A facet unified across providers, so one row can carry both a CurseForge
/// id and a Modrinth slug.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MergedCategory {
    pub key: String,
    pub name: String,
    pub header: String,
    pub curseforge_id: Option<String>,
    pub modrinth_id: Option<String>,
}

impl MergedCategory {
    /// The provider this facet exists on alone, if it is on only one.
    pub fn only_on(&self) -> Option<Provider> {
        match (&self.curseforge_id, &self.modrinth_id) {
            (Some(_), None) => Some(Provider::CurseForge),
            (None, Some(_)) => Some(Provider::Modrinth),
            _ => None,
        }
    }

    fn id_for(&self, provider: Provider) -> Option<&str> {
        match provider {
            Provider::CurseForge => self.curseforge_id.as_deref(),
            Provider::Modrinth => self.modrinth_id.as_deref(),
        }
    }
}

/// Known CurseForge to Modrinth category equivalences, keyed by normalized
/// name, so the providers' facets collapse into shared rows.
const CATEGORY_ALIASES: [(&str, &str); 8] = [
    ("worldgeneration", "worldgen"),
    ("adventureandrpg", "adventure"),
    ("apiandlibrary", "library"),
    ("libraryandapi", "library"),
    ("armortoolsandweapons", "equipment"),
    ("playertransport", "transportation"),
    ("utilityqol", "utility"),
    ("utilityandqol", "utility"),
];

/// A category name lowercased, stripped of punctuation and mapped through
/// the aliases.
pub fn canonical_name(name: &str) -> String {
    let compact: String = name
        .to_lowercase()
        .replace('&', "and")
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    CATEGORY_ALIASES
        .iter()
        .find(|(alias, _)| *alias == compact)
        .map_or(compact, |(_, canonical)| canonical.to_string())
}

/// Merges both providers' facets. Modrinth's go first so its shorter label
/// names a shared row.
pub fn merge_categories(
    curseforge: &[SearchCategory],
    modrinth: &[SearchCategory],
) -> Vec<MergedCategory> {
    let mut merged: Vec<MergedCategory> = Vec::new();
    let tagged = modrinth
        .iter()
        .map(|category| (Provider::Modrinth, category))
        .chain(
            curseforge
                .iter()
                .map(|category| (Provider::CurseForge, category)),
        );
    for (provider, category) in tagged {
        let key = format!("{}:{}", category.header, canonical_name(&category.name));
        let index = match merged.iter().position(|entry| entry.key == key) {
            Some(index) => index,
            None => {
                merged.push(MergedCategory {
                    key,
                    name: category.name.clone(),
                    header: category.header.clone(),
                    curseforge_id: None,
                    modrinth_id: None,
                });
                merged.len() - 1
            }
        };
        let slot = match provider {
            Provider::CurseForge => &mut merged[index].curseforge_id,
            Provider::Modrinth => &mut merged[index].modrinth_id,
        };
        *slot = Some(category.id.clone());
    }
    merged
}

/// Facet headers in the order the sidebar shows them; others go last.
const HEADER_ORDER: [&str; 4] = [
    "categories",
    "resolutions",
    "features",
    "performance impact",
];

/// A sidebar group of facets.
#[derive(Clone, PartialEq, Debug)]
pub struct Section {
    pub header: String,
    pub title: String,
    /// Drawn as wrapping pills rather than a checkbox list.
    pub pills: bool,
    pub options: Vec<MergedCategory>,
}

pub fn sections(categories: &[MergedCategory]) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for category in categories {
        match sections
            .iter_mut()
            .find(|section| section.header == category.header)
        {
            Some(section) => section.options.push(category.clone()),
            None => sections.push(Section {
                header: category.header.clone(),
                title: header_title(&category.header),
                pills: matches!(
                    category.header.as_str(),
                    "resolutions" | "performance impact"
                ),
                options: vec![category.clone()],
            }),
        }
    }
    let rank = |header: &str| {
        HEADER_ORDER
            .iter()
            .position(|known| *known == header)
            .unwrap_or(HEADER_ORDER.len())
    };
    sections.sort_by_key(|section| rank(&section.header));
    sections
}

fn header_title(header: &str) -> String {
    match header {
        "categories" => t("categories").to_string(),
        "resolutions" => t("resolution").to_string(),
        "features" => t("features").to_string(),
        "performance impact" => t("performanceImpact").to_string(),
        other => other
            .split(' ')
            .map(|word| {
                let mut chars = word.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect::<String>())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// An active filter, shown as a removable chip.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Chip {
    Category { key: String, name: String },
    Loader(ModLoader),
    OpenSource,
    Target(String),
}

impl Chip {
    pub fn label(&self) -> String {
        match self {
            Chip::Category { name, .. } => name.clone(),
            Chip::Loader(loader) => loader.to_string(),
            Chip::OpenSource => t("openSourceOnly").to_string(),
            Chip::Target(name) => {
                crate::i18n::t_with("targetModpackChip", &[("name", name)]).to_string()
            }
        }
    }
}

/// Everything that narrows a search besides its text.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Filters {
    pub content: ContentType,
    pub sort: Sort,
    /// The user's source toggles, before a category lock or "open source
    /// only" narrows them.
    pub curseforge: bool,
    pub modrinth: bool,
    /// `None` searches every version.
    pub version: Option<String>,
    pub loader: Option<ModLoader>,
    /// Merged category keys, in the order they were picked.
    pub selected: Vec<String>,
    pub open_source: bool,
    /// The modpack results install into.
    pub target: Option<String>,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            content: ContentType::Mods,
            sort: Sort::Relevance,
            curseforge: cfg!(feature = "curseforge"),
            modrinth: true,
            version: None,
            loader: None,
            selected: Vec::new(),
            open_source: false,
            target: None,
        }
    }
}

impl Filters {
    fn selected_categories<'a>(
        &'a self,
        categories: &'a [MergedCategory],
    ) -> impl Iterator<Item = &'a MergedCategory> {
        self.selected
            .iter()
            .filter_map(|key| categories.iter().find(|category| &category.key == key))
    }

    /// A category that exists on one provider alone narrows the search to it.
    pub fn lock(&self, categories: &[MergedCategory]) -> Option<Provider> {
        self.selected_categories(categories)
            .find_map(MergedCategory::only_on)
    }

    /// Whether `provider` is searched, after the lock and "open source only".
    pub fn searches(&self, provider: Provider, categories: &[MergedCategory]) -> bool {
        let lock = self.lock(categories);
        match provider {
            Provider::CurseForge => {
                self.curseforge
                    && lock != Some(Provider::Modrinth)
                    && !self.open_source
                    && Provider::available().contains(&provider)
            }
            Provider::Modrinth => self.modrinth && lock != Some(Provider::CurseForge),
        }
    }

    /// The providers searched, in request order.
    pub fn active_providers(&self, categories: &[MergedCategory]) -> Vec<Provider> {
        Provider::available()
            .iter()
            .copied()
            .filter(|provider| self.searches(*provider, categories))
            .collect()
    }

    /// Whether a source toggle is greyed out by the lock or "open source only".
    pub fn source_disabled(&self, provider: Provider, categories: &[MergedCategory]) -> bool {
        let lock = self.lock(categories);
        match provider {
            Provider::CurseForge => lock == Some(Provider::Modrinth) || self.open_source,
            Provider::Modrinth => lock == Some(Provider::CurseForge),
        }
    }

    pub fn toggle_source(&mut self, provider: Provider) {
        match provider {
            Provider::CurseForge => self.curseforge = !self.curseforge,
            Provider::Modrinth => self.modrinth = !self.modrinth,
        }
    }

    /// A single-provider facet is unusable when its provider is off, locked
    /// out, or excluded by "open source only".
    pub fn category_disabled(&self, category: &MergedCategory, all: &[MergedCategory]) -> bool {
        match category.only_on() {
            None => false,
            Some(Provider::CurseForge) => {
                !self.curseforge || self.lock(all) == Some(Provider::Modrinth) || self.open_source
            }
            Some(Provider::Modrinth) => {
                !self.modrinth || self.lock(all) == Some(Provider::CurseForge)
            }
        }
    }

    pub fn toggle_category(&mut self, key: &str) {
        match self.selected.iter().position(|selected| selected == key) {
            Some(index) => {
                self.selected.remove(index);
            }
            None => self.selected.push(key.to_string()),
        }
    }

    /// Switching content type starts over on categories and loader.
    pub fn select_content(&mut self, content: ContentType) {
        self.content = content;
        self.selected.clear();
        self.set_loader(None, None);
    }

    /// Picks the modpack results install into and adopts its version and
    /// loader. `None` clears the target.
    pub fn select_target(&mut self, modpack: Option<&LocalModpack>) {
        self.target = modpack.map(|modpack| modpack.name.clone());
        if let Some(modpack) = modpack {
            self.version = Some(modpack.version.clone());
            self.loader = Some(modpack.mod_loader);
        }
    }

    /// Diverging from the target's version would install the wrong file, so
    /// it drops the target.
    pub fn set_version(&mut self, version: Option<String>, target: Option<&LocalModpack>) {
        if self.target.is_some() && version.as_deref() != target.map(|t| t.version.as_str()) {
            self.target = None;
        }
        self.version = version;
    }

    pub fn set_loader(&mut self, loader: Option<ModLoader>, target: Option<&LocalModpack>) {
        if self.target.is_some() && loader != target.map(|t| t.mod_loader) {
            self.target = None;
        }
        self.loader = loader;
    }

    /// Turning "open source only" on drops CurseForge-only categories.
    pub fn toggle_open_source(&mut self, categories: &[MergedCategory]) {
        if !self.open_source {
            self.selected.retain(|key| {
                categories
                    .iter()
                    .find(|category| &category.key == key)
                    .is_none_or(|category| category.only_on() != Some(Provider::CurseForge))
            });
        }
        self.open_source = !self.open_source;
    }

    pub fn clear_all(&mut self) {
        self.selected.clear();
        self.loader = None;
        self.open_source = false;
        self.target = None;
    }

    pub fn remove_chip(&mut self, chip: &Chip) {
        match chip {
            Chip::Category { key, .. } => self.toggle_category(key),
            Chip::Loader(_) => self.loader = None,
            Chip::OpenSource => self.open_source = false,
            Chip::Target(_) => self.target = None,
        }
    }

    pub fn chips(&self, categories: &[MergedCategory]) -> Vec<Chip> {
        let mut chips: Vec<Chip> = self
            .selected_categories(categories)
            .map(|category| Chip::Category {
                key: category.key.clone(),
                name: category.name.clone(),
            })
            .collect();
        if let Some(loader) = self.loader.filter(|_| self.content.has_loader()) {
            chips.push(Chip::Loader(loader));
        }
        if self.open_source {
            chips.push(Chip::OpenSource);
        }
        if let Some(target) = &self.target {
            chips.push(Chip::Target(target.clone()));
        }
        chips
    }

    /// One request per searched provider, in provider order. `offsets[i]`
    /// is how many results provider `i` already returned, for "search
    /// further"; missing entries start at zero.
    ///
    /// Results are one-click installable only with a concrete target: a
    /// chosen modpack whose version and loader the filters still match.
    pub fn requests(
        &self,
        query: &str,
        categories: &[MergedCategory],
        target: Option<&LocalModpack>,
        offsets: &[usize],
    ) -> Vec<(Provider, GlobalSearchModsArgs)> {
        let loader = self.loader.filter(|_| self.content.has_loader());
        let installable = target.is_some_and(|target| {
            self.version.as_deref() == Some(target.version.as_str())
                && (!self.content.has_loader() || loader == Some(target.mod_loader))
        });
        self.active_providers(categories)
            .into_iter()
            .filter(|provider| loader.is_none_or(|loader| provider.supports(loader)))
            .enumerate()
            .map(|(index, provider)| {
                let args = GlobalSearchModsArgs {
                    source: provider.source(),
                    query: query.trim().to_lowercase(),
                    mod_type: self.content.request_name().to_string(),
                    filter_on: installable,
                    game_version: self.version.clone().unwrap_or_default(),
                    mod_loader: loader.map(|loader| loader.to_string()).unwrap_or_default(),
                    categories: self
                        .selected_categories(categories)
                        .filter_map(|category| category.id_for(provider))
                        .map(str::to_string)
                        .collect(),
                    open_source: self.open_source,
                    sort_by: self.sort.value().to_string(),
                    offset: offsets
                        .get(index)
                        .copied()
                        .unwrap_or(0)
                        .try_into()
                        .unwrap_or(u32::MAX),
                };
                (provider, args)
            })
            .collect()
    }
}

/// One list of results per provider, merged in `sort` order. Relevance
/// keeps each provider's own ranking by interleaving the lists.
pub fn order_results(lists: &[Vec<Mod>], sort: Sort) -> Vec<Mod> {
    if sort == Sort::Relevance {
        let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
        return (0..longest)
            .flat_map(|rank| lists.iter().filter_map(move |list| list.get(rank)))
            .cloned()
            .collect();
    }
    let mut flat: Vec<Mod> = lists.iter().flatten().cloned().collect();
    match sort {
        Sort::Name => flat.sort_by_key(|item| item.name.to_lowercase()),
        Sort::Updated => {
            flat.sort_by_key(|item| std::cmp::Reverse(date_value(&item.date_modified)))
        }
        Sort::Downloads | Sort::Relevance => {
            flat.sort_by_key(|item| std::cmp::Reverse(item.download_count))
        }
    }
    flat
}

/// An RFC 3339 timestamp in milliseconds; blank or unparsable sorts oldest.
fn date_value(value: &str) -> i64 {
    DateTime::parse_from_rfc3339(value).map_or(0, |date| date.timestamp_millis())
}

/// How many pages `count` results fill; at least one.
pub fn page_count(count: usize) -> usize {
    count.div_ceil(PAGE_SIZE).max(1)
}

/// A download count the way `Intl.NumberFormat` writes it compactly in
/// English: "999", "1.5K", "174.2M".
pub fn compact_count(count: i64) -> String {
    const UNITS: [(f64, &str); 4] = [(1., ""), (1e3, "K"), (1e6, "M"), (1e9, "B")];
    let value = count as f64;
    let mut unit = UNITS
        .iter()
        .rposition(|(scale, _)| value.abs() >= *scale)
        .unwrap_or(0);
    let tenths = |unit: usize| (value / UNITS[unit].0 * 10.).round() / 10.;
    // 999,960 rounds to 1000K, which reads as the next unit up.
    if unit > 0 && unit + 1 < UNITS.len() && tenths(unit).abs() >= 1000. {
        unit += 1;
    }
    let shown = if unit == 0 { value } else { tenths(unit) };
    let digits = if shown.fract() == 0. { 0 } else { 1 };
    format!("{shown:.digits$}{}", UNITS[unit].1)
}

/// Whether `modpack` already has `item`, matched by id or, across
/// providers, by slug. Keep in step with `InstalledMod::is_same_mod`.
pub fn installed_in(item: &Mod, modpack: Option<&LocalModpack>) -> Option<ModSource> {
    if item.mod_type != ModType::Mod {
        return None;
    }
    let slug = item.slug.trim().to_lowercase();
    modpack?
        .mods
        .iter()
        .find(|installed| {
            installed.id == item.id
                || (installed.source != item.source
                    && !slug.is_empty()
                    && installed.slug.trim().to_lowercase() == slug)
        })
        .map(|installed| installed.source.clone())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use quadrant_core::models::InstalledMod;

    pub fn result(name: &str, source: ModSource, downloads: i64, updated: &str) -> Mod {
        Mod {
            name: name.into(),
            id: format!("{name}-id"),
            download_count: downloads,
            version: String::new(),
            date_modified: updated.into(),
            mod_type: ModType::Mod,
            source,
            slug: name.to_lowercase(),
            thumbnail_urls: Vec::new(),
            url: format!("https://modrinth.com/mod/{}", name.to_lowercase()),
            description: format!("{name} does things."),
            license: String::new(),
            mod_icon_url: String::new(),
            downloadable: true,
            show_previous_version: false,
            new_version: None,
            deleteable: false,
            autoinstallable: false,
            selectable: false,
            modpack: None,
            select_url: None,
        }
    }

    fn category(id: &str, name: &str, header: &str, source: ModSource) -> SearchCategory {
        SearchCategory {
            id: id.into(),
            name: name.into(),
            header: header.into(),
            source,
        }
    }

    pub fn modpack(name: &str, version: &str, loader: ModLoader) -> LocalModpack {
        LocalModpack {
            name: name.into(),
            version: version.into(),
            mod_loader: loader,
            mods: Vec::new(),
            unknown_mods: false,
            is_applied: false,
            last_synced: 0,
            modpack_id: None,
        }
    }

    fn merged() -> Vec<MergedCategory> {
        merge_categories(
            &[
                category(
                    "6945",
                    "World Generation",
                    "categories",
                    ModSource::CurseForge,
                ),
                category(
                    "424",
                    "Map and Information",
                    "categories",
                    ModSource::CurseForge,
                ),
            ],
            &[
                category("worldgen", "Worldgen", "categories", ModSource::Modrinth),
                category(
                    "optimization",
                    "Optimization",
                    "categories",
                    ModSource::Modrinth,
                ),
                category("16x", "16x", "resolutions", ModSource::Modrinth),
            ],
        )
    }

    #[test]
    fn aliases_merge_the_providers_facets() {
        assert_eq!(canonical_name("Armor, Tools & Weapons"), "equipment");
        assert_eq!(canonical_name("Utility & QoL"), "utility");
        let merged = merged();
        let keys: Vec<_> = merged.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "categories:worldgen",
                "categories:optimization",
                "resolutions:16x",
                "categories:mapandinformation"
            ]
        );
        assert_eq!(merged[0].name, "Worldgen");
        assert_eq!(merged[0].curseforge_id.as_deref(), Some("6945"));
        assert_eq!(merged[0].modrinth_id.as_deref(), Some("worldgen"));
        assert_eq!(merged[3].only_on(), Some(Provider::CurseForge));
    }

    #[test]
    fn sections_follow_the_header_order() {
        let sections = sections(&merged());
        let headers: Vec<_> = sections
            .iter()
            .map(|s| (s.title.as_str(), s.pills))
            .collect();
        assert_eq!(headers, [("Categories", false), ("Resolution", true)]);
        assert_eq!(sections[0].options.len(), 3);
    }

    #[test]
    fn a_one_provider_category_locks_the_search_to_it() {
        let categories = merged();
        let mut filters = Filters {
            curseforge: true,
            ..Filters::default()
        };
        filters.toggle_category("categories:optimization");
        assert_eq!(filters.lock(&categories), Some(Provider::Modrinth));
        assert!(!filters.searches(Provider::CurseForge, &categories));
        assert!(filters.source_disabled(Provider::CurseForge, &categories));
        let mapping = &categories[3];
        assert!(filters.category_disabled(mapping, &categories));
        filters.toggle_category("categories:optimization");
        assert_eq!(filters.lock(&categories), None);
    }

    #[test]
    fn open_source_drops_curseforge_only_categories() {
        let categories = merged();
        let mut filters = Filters::default();
        filters.toggle_category("categories:mapandinformation");
        filters.toggle_category("categories:worldgen");
        filters.toggle_open_source(&categories);
        assert_eq!(filters.selected, ["categories:worldgen"]);
        assert!(!filters.searches(Provider::CurseForge, &categories));
        assert_eq!(filters.chips(&categories).len(), 2);
    }

    #[test]
    fn requests_carry_the_filters_per_provider() {
        let categories = merged();
        let mut filters = Filters {
            sort: Sort::Downloads,
            ..Filters::default()
        };
        filters.toggle_category("categories:worldgen");
        filters.set_version(Some("1.21.1".into()), None);
        filters.set_loader(Some(ModLoader::Fabric), None);
        let requests = filters.requests("  Sodium ", &categories, None, &[30]);
        let modrinth = &requests.last().unwrap().1;
        assert_eq!(requests.last().unwrap().0, Provider::Modrinth);
        assert_eq!(modrinth.source, ModSource::Modrinth);
        assert_eq!(modrinth.query, "sodium");
        assert_eq!(modrinth.mod_type, "Mod");
        assert_eq!(modrinth.game_version, "1.21.1");
        assert_eq!(modrinth.mod_loader, "Fabric");
        assert_eq!(modrinth.categories, ["worldgen"]);
        assert_eq!(modrinth.sort_by, "downloads");
        assert!(!modrinth.filter_on);
        assert_eq!(requests[0].1.offset, 30);
    }

    #[test]
    fn packs_ignore_the_loader() {
        let mut filters = Filters {
            loader: Some(ModLoader::Fabric),
            ..Filters::default()
        };
        filters.select_content(ContentType::Shaders);
        assert_eq!(filters.loader, None);
        filters.loader = Some(ModLoader::Rift);
        let requests = filters.requests("", &[], None, &[]);
        assert_eq!(requests[0].1.mod_type, "ShaderPack");
        assert_eq!(requests[0].1.mod_loader, "");
        assert!(filters.chips(&[]).is_empty());
    }

    #[test]
    fn a_matching_target_makes_results_installable() {
        let pack = modpack("Alpha", "1.21.1", ModLoader::Fabric);
        let mut filters = Filters::default();
        filters.select_target(Some(&pack));
        assert_eq!(filters.version.as_deref(), Some("1.21.1"));
        assert_eq!(filters.loader, Some(ModLoader::Fabric));
        let requests = filters.requests("", &[], Some(&pack), &[]);
        assert!(requests[0].1.filter_on);

        filters.set_loader(Some(ModLoader::Fabric), Some(&pack));
        assert_eq!(filters.target.as_deref(), Some("Alpha"));
        filters.set_version(Some("1.20.1".into()), Some(&pack));
        assert_eq!(filters.target, None);
        assert!(!filters.requests("", &[], Some(&pack), &[])[0].1.filter_on);
    }

    #[test]
    fn no_active_provider_means_no_requests() {
        let mut filters = Filters::default();
        filters.toggle_source(Provider::Modrinth);
        assert!(filters.requests("", &[], None, &[]).is_empty());
    }

    #[test]
    fn relevance_interleaves_and_other_sorts_merge() {
        let curseforge = vec![
            result("Cf1", ModSource::CurseForge, 5, "2024-01-01T00:00:00Z"),
            result("cf2", ModSource::CurseForge, 50, ""),
        ];
        let modrinth = vec![result(
            "Mr1",
            ModSource::Modrinth,
            20,
            "2025-06-01T00:00:00.5Z",
        )];
        let lists = [curseforge, modrinth];
        let names = |sort| -> Vec<String> {
            order_results(&lists, sort)
                .into_iter()
                .map(|item| item.name)
                .collect()
        };
        assert_eq!(names(Sort::Relevance), ["Cf1", "Mr1", "cf2"]);
        assert_eq!(names(Sort::Downloads), ["cf2", "Mr1", "Cf1"]);
        assert_eq!(names(Sort::Name), ["Cf1", "cf2", "Mr1"]);
        assert_eq!(names(Sort::Updated), ["Mr1", "Cf1", "cf2"]);
    }

    #[test]
    fn counts_read_like_the_desktop_app() {
        assert_eq!(compact_count(999), "999");
        assert_eq!(compact_count(1000), "1K");
        assert_eq!(compact_count(90_312), "90.3K");
        assert_eq!(compact_count(174_200_000), "174.2M");
        assert_eq!(compact_count(999_960), "1M");
        assert_eq!(compact_count(2_000_000_000), "2B");
        assert_eq!(page_count(0), 1);
        assert_eq!(page_count(26), 2);
    }

    #[test]
    fn loader_options_follow_the_providers() {
        assert_eq!(loader_options(&[Provider::CurseForge]).len(), 5);
        assert_eq!(loader_options(&[Provider::Modrinth]).len(), 13);
        assert_eq!(loader_options(&[]).len(), 13);
    }

    #[test]
    fn installed_matches_by_id_or_cross_provider_slug() {
        let mut pack = modpack("Alpha", "1.21.1", ModLoader::Fabric);
        let mut jei = InstalledMod::minimal(
            "238222".into(),
            ModSource::CurseForge,
            "https://edge.forgecdn.net/jei.jar".into(),
        );
        jei.slug = "jei".into();
        pack.mods.push(jei);
        let modrinth_jei = result("JEI", ModSource::Modrinth, 0, "");
        assert_eq!(
            installed_in(&modrinth_jei, Some(&pack)),
            Some(ModSource::CurseForge)
        );
        assert_eq!(installed_in(&modrinth_jei, None), None);
        let other = result("Sodium", ModSource::Modrinth, 0, "");
        assert_eq!(installed_in(&other, Some(&pack)), None);
    }
}
