//! `--screenshot <dir>` (feature `screenshot`): renders the window's own frames
//! to PNGs and quits. It reads gpui's scene, not the screen, so other windows
//! never end up in the picture and the window may be covered.

use std::{path::Path, path::PathBuf, time::Duration};

use anyhow::{Context as _, Result};
use gpui::{App, AsyncApp, Context, Window, WindowHandle};
use gpui_component::{Root, WindowExt as _};

use quadrant_core::models::{InstalledMod, ModSource};

use crate::{
    app::QuadrantApp,
    backend::Backend,
    i18n::{t, t_with},
    pages::{ContentType, CurrentModpackPage, ModpacksPage, SearchPage},
    route::Route,
    snackbar::{self, SnackbarKind},
};

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
    capture(window, &dir.join("delete-confirm.png"), cx).await?;
    window.update(cx, |_, window, cx| {
        window.defer(cx, |window, cx| window.close_all_dialogs(cx))
    })?;

    let snackbars = dir.join("snackbar");
    std::fs::create_dir_all(&snackbars)?;
    cx.update(|cx| snackbar::show(cx, SnackbarKind::Success, t("setModpackSuccess")));
    capture(window, &snackbars.join("success.png"), cx).await?;
    cx.update(|cx| {
        let reason = "The process cannot access the file because it is being used by another process. (os error 32)";
        snackbar::show(
            cx,
            SnackbarKind::Error,
            t_with("failedToDelete", &[("reason", reason)]),
        )
    });
    capture(window, &snackbars.join("error.png"), cx).await?;

    capture_mod_pages(window, dir, cx).await
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

/// Long enough for a provider search and its icons to arrive.
const NETWORK_SETTLE: Duration = Duration::from_secs(8);

/// The Search, mod install and Current modpack pages, against the live
/// Modrinth API. Nothing here installs: a download would also land in the
/// host's shared download cache, outside the sandbox.
async fn capture_mod_pages(
    window: WindowHandle<Root>,
    dir: &Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    let shots = dir.join("mods");
    std::fs::create_dir_all(&shots)?;
    let wait = |cx: &mut AsyncApp| cx.background_executor().timer(NETWORK_SETTLE);

    // The page restores its saved filters first; changes made before that
    // would be overwritten.
    with_page::<SearchPage>(window, cx, Route::Search, |_, _, _| {})?;
    wait(cx).await;
    with_page::<SearchPage>(window, cx, Route::Search, |page, window, cx| {
        page.set_content(ContentType::Shaders, window, cx)
    })?;
    wait(cx).await;
    capture(window, &shots.join("search-shaders.png"), cx).await?;

    with_page::<SearchPage>(window, cx, Route::Search, |page, window, cx| {
        page.set_content(ContentType::Mods, window, cx);
        page.set_target("Alpha Pack", window, cx);
        page.set_query("iris", window, cx);
    })?;
    wait(cx).await;
    capture(window, &shots.join("search-into-modpack.png"), cx).await?;

    with_page::<SearchPage>(window, cx, Route::Search, |page, _, cx| {
        page.set_filters_open(false, cx)
    })?;
    capture(window, &shots.join("search-filters-collapsed.png"), cx).await?;

    with_page::<SearchPage>(window, cx, Route::Search, |page, window, cx| {
        page.set_filters_open(true, cx);
        page.open_first_result(window, cx);
    })?;
    wait(cx).await;
    capture(window, &shots.join("mod-install.png"), cx).await?;

    // An old Mod Menu for the applied pack, so the update check has
    // something to offer. Registering only writes the pack's manifest.
    let register = cx.update(|cx| {
        Backend::global(cx).run(cx, |host| async move {
            let old = InstalledMod::minimal(
                "mOgUt4GM".into(),
                ModSource::Modrinth,
                "https://cdn.modrinth.com/data/mOgUt4GM/versions/xhN1IvHi/modmenu-11.0.1.jar"
                    .into(),
            );
            host.register_mod(old, "Alpha Pack".into()).await
        })
    });
    register.await?;
    with_page::<CurrentModpackPage>(window, cx, Route::CurrentModpack, |page, _, cx| {
        page.refresh(cx)
    })?;
    capture(window, &shots.join("current-modpack.png"), cx).await?;

    with_page::<CurrentModpackPage>(window, cx, Route::CurrentModpack, |page, _, cx| {
        page.toggle_updates(cx)
    })?;
    wait(cx).await;
    capture(window, &shots.join("current-modpack-updates.png"), cx).await
}

/// Opens `route` and runs `f` on its page once the root is released.
fn with_page<P: 'static>(
    window: WindowHandle<Root>,
    cx: &mut AsyncApp,
    route: Route,
    f: impl FnOnce(&mut P, &mut Window, &mut Context<P>) + 'static,
) -> Result<()> {
    window.update(cx, |root, window, cx| {
        let app = root
            .view()
            .clone()
            .downcast::<QuadrantApp>()
            .ok()
            .context("root is not the app")?;
        let page = app.update(cx, |app, cx| {
            app.navigate(route, cx);
            app.page(route, window, cx)
        });
        let page = page.downcast::<P>().ok().context("not the expected page")?;
        window.defer(cx, move |window, cx| {
            page.update(cx, |page, cx| f(page, window, cx))
        });
        anyhow::Ok(())
    })?
}
