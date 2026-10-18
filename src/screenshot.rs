//! `--screenshot <dir>` (feature `screenshot`): renders the window's own frames
//! to PNGs and quits. It reads gpui's scene, not the screen, so other windows
//! never end up in the picture and the window may be covered.

use std::{path::Path, path::PathBuf, time::Duration};

use anyhow::{Context as _, Result};
use gpui::{App, AsyncApp, Context, Window, WindowHandle};
use gpui_component::{Root, WindowExt as _};

use crate::{app::QuadrantApp, pages::ModpacksPage, route::Route};

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
    capture(window, &dir.join("modpacks.png"), cx).await?;

    with_modpacks_page(window, cx, |page, window, cx| {
        page.open_create_dialog(window, cx)
    })?;
    capture(window, &dir.join("create-dialog.png"), cx).await?;
    window.update(cx, |_, window, cx| {
        window.defer(cx, |window, cx| window.close_all_dialogs(cx))
    })?;

    with_modpacks_page(window, cx, |page, window, cx| {
        page.confirm_delete("Beta Pack".into(), window, cx)
    })?;
    capture(window, &dir.join("delete-confirm.png"), cx).await
}

fn with_modpacks_page(
    window: WindowHandle<Root>,
    cx: &mut AsyncApp,
    f: impl FnOnce(&mut ModpacksPage, &mut Window, &mut Context<ModpacksPage>) + 'static,
) -> Result<()> {
    window.update(cx, |root, window, cx| {
        let app = root
            .view()
            .clone()
            .downcast::<QuadrantApp>()
            .ok()
            .context("root is not the app")?;
        let page = app.update(cx, |app, cx| app.page(Route::Modpacks, window, cx));
        let page = page
            .downcast::<ModpacksPage>()
            .ok()
            .context("not the modpacks page")?;
        // Deferred until the root is released: dialogs read it when they open.
        window.defer(cx, move |window, cx| {
            page.update(cx, |page, cx| f(page, window, cx))
        });
        anyhow::Ok(())
    })?
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
