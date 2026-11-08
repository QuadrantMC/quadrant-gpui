//! What Modrinth and CurseForge say about installed mods (title, icon,
//! downloads, description), after the desktop app's per-mod `getMod` calls
//! in `ModpackView` and `SharedModpackView`. One app-wide cache: each mod is
//! asked about once per run, however many views show it, and a view shows
//! the manifest's own record until the answer arrives.

use std::collections::{HashMap, VecDeque};

use futures::StreamExt as _;
use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use quadrant_core::{
    mc_mod::{GetModArgs, Mod},
    models::{ModLoader, ModSource},
};
use quadrant_host::QuadrantHost;

use crate::backend::Backend;

/// Mods asked about per backend call; the page redraws once per batch.
const BATCH: usize = 24;
/// Requests in flight at once, well under the providers' rate limits.
pub const CONCURRENCY: usize = 6;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModKey {
    curseforge: bool,
    id: String,
}

impl ModKey {
    /// `None` for a source no provider answers for.
    pub fn new(source: &ModSource, id: &str) -> Option<Self> {
        let curseforge = match source {
            ModSource::CurseForge => true,
            ModSource::Modrinth => false,
            ModSource::Online => return None,
        };
        Some(Self {
            curseforge,
            id: id.to_string(),
        })
    }
}

/// Where the catalog stands on one mod it was asked about.
enum Answer {
    Pending,
    Found(Box<Mod>),
    /// The provider had no record, or couldn't be reached.
    Missing,
}

/// How far the catalog got with a set of mods, for `loadingMods`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub found: usize,
    pub pending: usize,
}

pub struct ModCatalog {
    /// Every mod asked about.
    known: HashMap<ModKey, Answer>,
    queue: VecDeque<ModKey>,
    fetching: Option<Task<()>>,
}

struct GlobalCatalog(Entity<ModCatalog>);

impl Global for GlobalCatalog {}

impl ModCatalog {
    pub fn global(cx: &mut App) -> Entity<ModCatalog> {
        if let Some(catalog) = cx.try_global::<GlobalCatalog>() {
            return catalog.0.clone();
        }
        let catalog = cx.new(|_| ModCatalog {
            known: HashMap::new(),
            queue: VecDeque::new(),
            fetching: None,
        });
        cx.set_global(GlobalCatalog(catalog.clone()));
        catalog
    }

    /// The provider's record of a mod, once it has answered.
    pub fn get(&self, source: &ModSource, id: &str) -> Option<&Mod> {
        match self.known.get(&ModKey::new(source, id)?)? {
            Answer::Found(found) => Some(found.as_ref()),
            Answer::Pending | Answer::Missing => None,
        }
    }

    /// How many of `keys` have a record and how many are still being asked
    /// about. A key never asked about counts as neither.
    pub fn tally<'a>(&self, keys: impl IntoIterator<Item = &'a ModKey>) -> Tally {
        keys.into_iter().fold(Tally::default(), |mut tally, key| {
            match self.known.get(key) {
                Some(Answer::Found(_)) => tally.found += 1,
                Some(Answer::Pending) => tally.pending += 1,
                Some(Answer::Missing) | None => {}
            }
            tally
        })
    }

    /// Asks about every mod not asked about yet. Observers are notified when
    /// records arrive and once the queue runs dry; a mod the provider doesn't
    /// know changes nothing before then.
    pub fn want(&mut self, mods: impl IntoIterator<Item = ModKey>, cx: &mut Context<Self>) {
        for key in mods {
            if !self.known.contains_key(&key) {
                self.known.insert(key.clone(), Answer::Pending);
                self.queue.push_back(key);
            }
        }
        self.pump(cx);
    }

    /// Marks `mods` as asked about without asking, so a screenshot shows them
    /// loading for as long as it likes.
    #[cfg(feature = "screenshot")]
    pub fn preview_pending(&mut self, mods: impl IntoIterator<Item = ModKey>) {
        for key in mods {
            self.known.insert(key, Answer::Pending);
        }
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        if self.fetching.is_some() || self.queue.is_empty() {
            return;
        }
        let batch: Vec<ModKey> = self.queue.drain(..self.queue.len().min(BATCH)).collect();
        let asked = batch.clone();
        let task = Backend::global(cx).run(cx, move |host| async move {
            Ok(futures::stream::iter(batch)
                .map(|key| fetch(host.clone(), key))
                .buffer_unordered(CONCURRENCY)
                .collect::<Vec<_>>()
                .await)
        });
        self.fetching = Some(cx.spawn(async move |this, cx| {
            let answers = task
                .await
                .unwrap_or_else(|_| asked.into_iter().map(|key| (key, None)).collect());
            let _ = this.update(cx, |this, cx| {
                this.fetching = None;
                let mut found = false;
                for (key, answer) in answers {
                    found |= answer.is_some();
                    this.known.insert(
                        key,
                        answer.map_or(Answer::Missing, |found| Answer::Found(Box::new(found))),
                    );
                }
                if found || this.queue.is_empty() {
                    cx.notify();
                }
                this.pump(cx);
            });
        }));
    }
}

/// The provider's record, with the flags `GetModArgs` sets left neutral;
/// each view sets its own.
async fn fetch(host: QuadrantHost, key: ModKey) -> (ModKey, Option<Mod>) {
    match provider_record(&host, &key).await {
        Ok(found) => (key, Some(found)),
        Err(error) => {
            log::debug!("no provider record for {}: {error:#}", key.id);
            (key, None)
        }
    }
}

/// What the mod's provider says about it, as the desktop app's `getMod`.
pub async fn provider_record(host: &QuadrantHost, key: &ModKey) -> anyhow::Result<Mod> {
    let args = GetModArgs {
        id: key.id.clone(),
        downloadable: false,
        show_previous_version: false,
        deletable: false,
        version_target: String::new(),
        mod_loader: ModLoader::Unknown,
        modpack: String::new(),
        selectable: false,
        select_url: None,
    };
    if key.curseforge {
        curseforge(host, args).await
    } else {
        host.get_mod_modrinth(args).await
    }
}

#[cfg(feature = "curseforge")]
async fn curseforge(host: &QuadrantHost, args: GetModArgs) -> anyhow::Result<Mod> {
    host.get_mod_curseforge(args).await
}

#[cfg(not(feature = "curseforge"))]
async fn curseforge(_: &QuadrantHost, _: GetModArgs) -> anyhow::Result<Mod> {
    anyhow::bail!("this build has no CurseForge support")
}
