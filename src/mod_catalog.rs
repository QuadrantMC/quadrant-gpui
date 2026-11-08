//! What Modrinth and CurseForge say about installed mods (title, icon,
//! downloads, description), after the desktop app's per-mod `getMod` calls
//! in `ModpackView` and `SharedModpackView`. One app-wide cache: each mod is
//! asked about once per run, however many views show it, and a view shows
//! the manifest's own record until the answer arrives. A provider that
//! couldn't be reached is asked again, not remembered as not knowing the mod.

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use futures::StreamExt as _;
use gpui::{App, AppContext as _, Context, Entity, Global, Task};
use quadrant_core::{
    mc_mod::{GetModArgs, Mod},
    models::{ModLoader, ModSource},
};
use quadrant_host::{ErrorCode, QuadrantHost};

use crate::backend::Backend;

/// Mods asked about per backend call; the page redraws once per batch.
const BATCH: usize = 24;
/// Requests in flight at once, well under the providers' rate limits.
pub const CONCURRENCY: usize = 6;
/// How long after an unreachable provider a mod is asked about again, when a
/// view next wants it.
const RETRY_AFTER: Duration = Duration::from_secs(30);

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
    /// The provider has no record of it.
    Missing,
    /// The provider couldn't be reached (offline, rate limited, a server
    /// error) at this moment.
    Unreachable(Instant),
}

impl Answer {
    /// Whether a view that wants the mod should ask again at `now`.
    fn due(&self, now: Instant) -> bool {
        matches!(self, Self::Unreachable(at) if now.duration_since(*at) >= RETRY_AFTER)
    }
}

/// What one lookup came to.
enum Reply {
    Found(Box<Mod>),
    Missing,
    Unreachable,
}

/// Whether a failed lookup says nothing about the mod itself.
fn unreachable(error: anyhow::Error) -> bool {
    matches!(
        quadrant_host::user_facing(error).downcast_ref::<ErrorCode>(),
        Some(
            ErrorCode::Timeout
                | ErrorCode::RateLimited
                | ErrorCode::Server
                | ErrorCode::Network
                | ErrorCode::BadResponse
                | ErrorCode::Busy
        )
    )
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
            Answer::Pending | Answer::Missing | Answer::Unreachable(_) => None,
        }
    }

    /// How many of `keys` have a record and how many are still being asked
    /// about. A key never asked about counts as neither.
    pub fn tally<'a>(&self, keys: impl IntoIterator<Item = &'a ModKey>) -> Tally {
        keys.into_iter().fold(Tally::default(), |mut tally, key| {
            match self.known.get(key) {
                Some(Answer::Found(_)) => tally.found += 1,
                Some(Answer::Pending) => tally.pending += 1,
                Some(Answer::Missing | Answer::Unreachable(_)) | None => {}
            }
            tally
        })
    }

    /// Asks about every mod not asked about yet, and again about those a
    /// provider couldn't answer for a while ago. Observers are notified when
    /// records arrive and once the queue runs dry; a mod the provider doesn't
    /// know changes nothing before then.
    pub fn want(&mut self, mods: impl IntoIterator<Item = ModKey>, cx: &mut Context<Self>) {
        let now = Instant::now();
        for key in mods {
            if self.known.get(&key).is_none_or(|answer| answer.due(now)) {
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
            let answers = task.await.unwrap_or_else(|_| {
                asked
                    .into_iter()
                    .map(|key| (key, Reply::Unreachable))
                    .collect()
            });
            let _ = this.update(cx, |this, cx| {
                this.fetching = None;
                let mut found = false;
                let now = Instant::now();
                for (key, reply) in answers {
                    found |= matches!(reply, Reply::Found(_));
                    this.known.insert(
                        key,
                        match reply {
                            Reply::Found(record) => Answer::Found(record),
                            Reply::Missing => Answer::Missing,
                            Reply::Unreachable => Answer::Unreachable(now),
                        },
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
async fn fetch(host: QuadrantHost, key: ModKey) -> (ModKey, Reply) {
    match provider_record(&host, &key).await {
        Ok(found) => (key, Reply::Found(Box::new(found))),
        Err(error) => {
            log::debug!("no provider record for {}: {error:#}", key.id);
            let reply = if unreachable(error) {
                Reply::Unreachable
            } else {
                Reply::Missing
            };
            (key, reply)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_that_could_not_be_reached_says_nothing_about_the_mod() {
        for code in [
            ErrorCode::Timeout,
            ErrorCode::RateLimited,
            ErrorCode::Server,
            ErrorCode::Network,
        ] {
            assert!(unreachable(code.into()), "{code}");
        }
        assert!(!unreachable(ErrorCode::NotFound.into()));
        assert!(!unreachable(anyhow::anyhow!("Unsupported Mod Type")));
    }

    #[test]
    fn only_an_unreachable_answer_is_asked_again_and_not_at_once() {
        let at = Instant::now();
        let later = at + RETRY_AFTER;
        let unreachable = Answer::Unreachable(at);
        assert!(!unreachable.due(at));
        assert!(unreachable.due(later));
        for settled in [Answer::Pending, Answer::Missing] {
            assert!(!settled.due(later));
        }
    }
}
