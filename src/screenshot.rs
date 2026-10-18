//! `--screenshot <dir>` (feature `screenshot`): renders the window's own frames
//! to PNGs and quits. It reads gpui's scene, not the screen, so other windows
//! never end up in the picture and the window may be covered.

use std::{path::Path, path::PathBuf, time::Duration};

use anyhow::Result;
use gpui::{App, AsyncApp, WindowHandle};
use gpui_component::Root;

/// Long enough for the first host calls to answer and the page to redraw.
const SETTLE: Duration = Duration::from_secs(3);

pub fn run(window: WindowHandle<Root>, dir: PathBuf, cx: &mut App) {
    cx.spawn(async move |cx| {
        if let Err(error) = capture_all(window, &dir, cx).await {
            eprintln!("Error: screenshot failed: {error:#}");
        }
        cx.update(|cx| cx.quit());
    })
    .detach();
}

async fn capture_all(window: WindowHandle<Root>, dir: &Path, cx: &mut AsyncApp) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    capture(window, &dir.join("modpacks.png"), cx).await
}

async fn capture(window: WindowHandle<Root>, path: &Path, cx: &mut AsyncApp) -> Result<()> {
    cx.background_executor().timer(SETTLE).await;
    window.update(cx, |_, window, _| window.refresh())?;
    cx.background_executor()
        .timer(Duration::from_millis(500))
        .await;
    let image = window.update(cx, |_, window, _| window.render_to_image())??;
    image.save(path)?;
    println!("{}", path.display());
    Ok(())
}
