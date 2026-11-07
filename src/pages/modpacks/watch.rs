//! `Apply.tsx`'s watch: the Minecraft folder itself (the `mods` link flips
//! on apply) and the `modpacks` tree, where installs and sync dates land
//! several levels deep. Changes are gathered for 50ms, as Tauri's watch
//! debounces them, before the page hears of them.

use std::{path::Path, time::Duration};

use futures::{
    StreamExt as _,
    channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded},
};
use gpui::{AppContext as _, Context, Task};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::backend::Backend;

/// Tauri's `delayMs: 50`.
const DEBOUNCE: Duration = Duration::from_millis(50);

/// Watches the Minecraft folder for as long as the returned task is kept,
/// and calls `changed` on the view after each burst of changes.
pub fn folders<T: 'static>(
    cx: &mut Context<T>,
    changed: impl Fn(&mut T, &mut Context<T>) + 'static,
) -> Task<()> {
    let folder = Backend::global(cx).run(cx, |host| async move { host.get_minecraft_folder() });
    cx.spawn(async move |this, cx| {
        let Ok(folder) = folder.await else {
            return;
        };
        let (sender, mut changes) = unbounded();
        // A recursive watch walks the tree on Linux; keep that off the UI
        // thread.
        let watch = cx.background_spawn({
            let folder = folder.clone();
            async move { watch(&folder, sender) }
        });
        let (mut watcher, mut tree_watched) = match watch.await {
            Ok(watching) => watching,
            Err(error) => {
                log::warn!("Not watching {}: {error}", folder.display());
                return;
            }
        };
        while changes.next().await.is_some() {
            cx.background_executor().timer(DEBOUNCE).await;
            drain(&mut changes);
            // The tree may not exist until the first local pack is created.
            if !tree_watched {
                tree_watched = watch_tree(&mut watcher, &folder);
            }
            if this.update(cx, |this, cx| changed(this, cx)).is_err() {
                break;
            }
        }
    })
}

/// Watches `folder` and, when it exists, its `modpacks` tree. Reads are not
/// changes: the listing that answers a change reads every manifest.
fn watch(
    folder: &Path,
    changes: UnboundedSender<()>,
) -> notify::Result<(RecommendedWatcher, bool)> {
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| !matches!(event.kind, EventKind::Access(_))) {
            let _ = changes.unbounded_send(());
        }
    })?;
    watcher.watch(folder, RecursiveMode::NonRecursive)?;
    let tree_watched = watch_tree(&mut watcher, folder);
    Ok((watcher, tree_watched))
}

fn watch_tree(watcher: &mut RecommendedWatcher, folder: &Path) -> bool {
    watcher
        .watch(&folder.join("modpacks"), RecursiveMode::Recursive)
        .is_ok()
}

fn drain(changes: &mut UnboundedReceiver<()>) {
    while changes.try_recv().is_ok() {}
}
