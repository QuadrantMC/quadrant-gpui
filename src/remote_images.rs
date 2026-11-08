//! The decoded remote images views draw: mod icons and screenshots.
//!
//! Without an image cache gpui keeps every image it ever loaded, decoded to
//! full-size BGRA (a 1080p screenshot is about 8 MB), and a failed load for
//! good. [`RemoteImages`] keeps them within [`BUDGET`], letting go of the
//! least recently drawn first, and asks for a failed image again after
//! [`RETRY_AFTER`].

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, AppContext as _, Entity, Global, ImageCache, ImageCacheError, ImageCacheItem, RenderImage,
    Resource, Window,
};

/// How many bytes of decoded images to keep.
const BUDGET: usize = 192 << 20;

/// An image drawn this recently is still on screen, or about to be again:
/// letting it go would only load it again. The cache may pass its budget
/// while that many are on screen.
const IN_USE: Duration = Duration::from_secs(2);

/// How long a failed load stands before the image is asked for again, so
/// an icon that failed offline or on a 5xx comes back.
const RETRY_AFTER: Duration = Duration::from_secs(30);

struct Entry {
    item: ImageCacheItem,
    last_used: Instant,
    /// When the load finished, once it has.
    settled: Option<Instant>,
    /// The decoded size, once it loaded.
    bytes: usize,
}

pub struct RemoteImages {
    entries: HashMap<u64, Entry>,
    /// The sum of the entries' `bytes`.
    bytes: usize,
}

struct GlobalImages(Entity<RemoteImages>);

impl Global for GlobalImages {}

/// Creates the app's image cache. Call once at startup.
pub fn install(cx: &mut App) {
    let images = cx.new(|_| RemoteImages {
        entries: HashMap::new(),
        bytes: 0,
    });
    cx.set_global(GlobalImages(images));
}

/// The cache `img` elements of remote images load through, once installed.
pub fn cache(cx: &App) -> Option<Entity<RemoteImages>> {
    cx.try_global::<GlobalImages>()
        .map(|global| global.0.clone())
}

/// `img(url)` through the app's cache.
pub fn img(url: impl Into<gpui::SharedString>, cx: &App) -> gpui::Img {
    let img = gpui::img(url.into());
    match cache(cx) {
        Some(cache) => img.image_cache(&cache),
        None => img,
    }
}

/// The bytes `image` takes decoded, every frame of it.
fn decoded_size(image: &RenderImage) -> usize {
    (0..image.frame_count())
        .map(|frame| image.as_bytes(frame).map_or(0, <[u8]>::len))
        .sum()
}

/// Which images to let go of so `total` bytes fit in `budget`: the least
/// recently used first, never one used within [`IN_USE`] of `now`.
/// `images` are each image's key, decoded size and last use.
fn victims(
    images: impl IntoIterator<Item = (u64, usize, Instant)>,
    mut total: usize,
    budget: usize,
    now: Instant,
) -> Vec<u64> {
    if total <= budget {
        return Vec::new();
    }
    let mut idle: Vec<_> = images
        .into_iter()
        .filter(|&(_, bytes, last_used)| bytes > 0 && now.duration_since(last_used) >= IN_USE)
        .collect();
    idle.sort_by_key(|&(_, _, last_used)| last_used);
    let mut victims = Vec::new();
    for (key, bytes, _) in idle {
        if total <= budget {
            break;
        }
        total -= bytes;
        victims.push(key);
    }
    victims
}

impl RemoteImages {
    /// Lets go of the images [`victims`] picks, and of failures that have
    /// stood long enough to be asked for again.
    fn evict(&mut self, now: Instant, window: &mut Window, cx: &mut App) {
        let images = self
            .entries
            .iter()
            .map(|(key, entry)| (*key, entry.bytes, entry.last_used));
        for key in victims(images, self.bytes, BUDGET, now) {
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes -= entry.bytes;
                if let Some(Ok(image)) = entry.item.get() {
                    cx.drop_image(image, Some(window));
                }
            }
        }
        self.entries.retain(|_, entry| !failed_long_ago(entry, now));
    }
}

fn failed_long_ago(entry: &Entry, now: Instant) -> bool {
    matches!(entry.item.get(), Some(Err(_)))
        && entry
            .settled
            .is_some_and(|settled| now.duration_since(settled) >= RETRY_AFTER)
}

impl ImageCache for RemoteImages {
    fn load(
        &mut self,
        resource: &Resource,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let now = Instant::now();
        let key = gpui::hash(resource);
        if self
            .entries
            .get(&key)
            .is_some_and(|entry| failed_long_ago(entry, now))
        {
            self.entries.remove(&key);
        }
        let entry = self.entries.entry(key).or_insert_with(|| Entry {
            item: ImageCacheItem::new(resource, cx),
            last_used: now,
            settled: None,
            bytes: 0,
        });
        entry.last_used = now;
        let result = entry.item.use_image(window);
        if entry.settled.is_none()
            && let Some(result) = &result
        {
            entry.settled = Some(now);
            if let Ok(image) = result {
                entry.bytes = decoded_size(image);
                self.bytes += entry.bytes;
                self.evict(now, window, cx);
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_recently_drawn_go_first_until_the_rest_fit() {
        let now = Instant::now() + Duration::from_secs(60);
        let ago = |secs| now - Duration::from_secs(secs);
        let images = [(1, 40, ago(30)), (2, 40, ago(10)), (3, 40, ago(20))];
        assert_eq!(victims(images, 120, 100, now), [1]);
        assert_eq!(victims(images, 120, 50, now), [1, 3]);
        assert!(victims(images, 120, 120, now).is_empty());
    }

    #[test]
    fn images_on_screen_and_loads_in_flight_stay() {
        let now = Instant::now() + Duration::from_secs(60);
        let images = [
            (1, 40, now - Duration::from_millis(500)),
            (2, 0, now - Duration::from_secs(30)),
            (3, 40, now - Duration::from_secs(30)),
        ];
        assert_eq!(victims(images, 200, 0, now), [3]);
    }
}
