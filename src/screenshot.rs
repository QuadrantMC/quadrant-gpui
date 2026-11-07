//! `--screenshot <dir>` (feature `screenshot`): renders the window's own frames
//! to PNGs and quits. It reads gpui's scene, not the screen, so other windows
//! never end up in the picture. The window stays hidden, so the user's clicks
//! can't land on it while it is driven.

use std::{path::Path, path::PathBuf, time::Duration};

use anyhow::{Context as _, Result, ensure};
use gpui::{App, AppContext as _, AsyncApp, Context, Entity, Window, WindowHandle};
use gpui_component::{Root, WindowExt as _};
use quadrant_core::{
    account::{
        id::AccountInfo,
        quadrant_sync::{ModpackOwner, SyncedModpack},
    },
    models::ModLoader,
};

use quadrant_core::{
    account::id::Notification,
    models::{Article, InstalledMod, ModSource},
};

use crate::{
    app::QuadrantApp,
    backend::Backend,
    i18n::{t, t_with},
    notifications,
    pages::{
        AccountPage, ContentType, CurrentModpackPage, InstalledContentPage, ModpacksPage,
        SearchPage,
    },
    route::Route,
    session::Session,
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
    capture_title_bar(window, dir, cx).await?;
    // Debug aid: `QUADRANT_NATIVE_SCREENSHOT_ONLY=title-bar` stops after the
    // title bar's pictures and checks.
    if std::env::var_os("QUADRANT_NATIVE_SCREENSHOT_ONLY").is_some_and(|only| only == "title-bar") {
        return Ok(());
    }
    capture(window, &dir.join("modpacks.png"), cx).await?;
    capture_prism(window, dir, cx).await?;

    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, window, cx| page.open_create_dialog(window, cx),
    )?;
    capture(window, &dir.join("create-dialog.png"), cx).await?;
    close_dialogs(window, cx).await?;

    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, window, cx| page.confirm_delete("Beta Pack".into(), window, cx),
    )?;
    capture(window, &dir.join("delete-confirm.png"), cx).await?;
    close_dialogs(window, cx).await?;

    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, window, cx| page.preview(Some("Alpha Pack"), true, window, cx),
    )?;
    capture(window, &dir.join("update-dialog.png"), cx).await?;
    close_dialogs(window, cx).await?;
    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, window, cx| page.preview(Some("Alpha Pack"), false, window, cx),
    )?;
    capture(window, &dir.join("details.png"), cx).await?;
    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, window, cx| page.preview(None, false, window, cx),
    )?;

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

    capture_mod_pages(window, dir, cx).await?;
    capture_widths(window, dir, cx).await?;
    capture_reach(window, dir, cx).await?;
    capture_install_to(window, dir, cx).await?;

    show(window, cx, Route::Settings)?;
    capture(window, &dir.join("settings.png"), cx).await?;
    // A taller window shows the whole page in one picture.
    resize(window, cx, 2300.)?;
    capture(window, &dir.join("settings-full.png"), cx).await?;

    show(window, cx, Route::InstalledContent)?;
    resize(window, cx, 720.)?;
    capture(window, &dir.join("installed-content.png"), cx).await?;
    with_page(
        window,
        cx,
        Route::InstalledContent,
        |page: &mut InstalledContentPage, _, cx| page.preview_selection(cx),
    )?;
    resize(window, cx, 1300.)?;
    capture(window, &dir.join("installed-content-open.png"), cx).await?;
    resize(window, cx, 720.)?;

    show(window, cx, Route::Account)?;
    capture(window, &dir.join("account-signed-out.png"), cx).await?;
    // Default builds have no Quadrant ID client, so this fails at once.
    with_page(
        window,
        cx,
        Route::Account,
        |page: &mut AccountPage, _, cx| page.sign_in(cx),
    )?;
    capture(window, &dir.join("account-sign-in-failed.png"), cx).await?;
    cx.update(|cx| {
        Session::global(cx).update(cx, |session, cx| {
            session.preview_signed_in(sample_account(), cx)
        })
    });
    capture(window, &dir.join("account-signed-in.png"), cx).await?;

    show(window, cx, Route::Modpacks)?;
    resize(window, cx, 1300.)?;
    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, window, cx| {
            page.preview_cloud(sample_synced(), "", None, window, cx)
        },
    )?;
    capture(window, &dir.join("modpacks-synced.png"), cx).await?;
    resize(window, cx, 720.)?;
    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, window, cx| {
            page.preview_cloud(sample_synced(), "", Some("Alpha Pack"), window, cx)
        },
    )?;
    // The page opens the dialog after the frame that sees the refusal.
    draw(window, cx)?;
    capture(window, &dir.join("sync-conflict.png"), cx).await?;
    close_dialogs(window, cx).await?;
    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, window, cx| {
            page.preview_cloud(sample_synced(), "1234567", None, window, cx)
        },
    )?;
    capture(window, &dir.join("share-code.png"), cx).await?;
    capture_motion(window, dir, cx).await?;
    capture_updater(window, dir, cx).await?;
    capture_revalidate(window, dir, cx).await
}

/// The title bar, and on Windows the check that its window controls take
/// clicks while the rest of it drags the window.
async fn capture_title_bar(
    window: WindowHandle<Root>,
    dir: &Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    let shots = dir.join("title-bar");
    std::fs::create_dir_all(&shots)?;
    capture(window, &shots.join("bar.png"), cx).await?;
    #[cfg(windows)]
    hit_test::check(window, cx).await?;

    let notifications = cx
        .update(|cx| notifications::view(cx))
        .context("no notifications")?;
    notifications.update(cx, |this, cx| {
        this.preview(
            &sample_snackbars(),
            sample_notifications(),
            sample_news(true),
            cx,
        )
    });
    capture(window, &shots.join("bell-unread.png"), cx).await?;
    // A click on the bell, as the platform hands it to gpui once the hit
    // test above has said it lands in the client area.
    let bell = window
        .update(cx, |root, _, cx| {
            app(root).map(|app| app.read(cx).title_bar_layout().buttons.first().copied())
        })??
        .context("no bell")?;
    click(window, bell.center(), cx)?;
    ensure!(
        notifications.read_with(cx, |this, _| this.is_open()),
        "the bell's click didn't open the panel"
    );
    draw(window, cx)?;
    capture_now(window, &shots.join("panel-enter-mid.png"), cx)?;
    cx.background_executor().timer(SETTLE).await;
    capture_now(window, &shots.join("panel-open.png"), cx)?;
    // A press on the backdrop closes it, as one outside a headlessui
    // popover does.
    click(window, gpui::point(gpui::px(400.), gpui::px(400.)), cx)?;
    ensure!(
        !notifications.read_with(cx, |this, _| this.is_open()),
        "a press outside didn't close the panel"
    );
    draw(window, cx)?;
    capture_now(window, &shots.join("panel-exit-mid.png"), cx)?;
    notifications.update(cx, |this, cx| {
        this.preview(&[], Vec::new(), sample_news(true), cx)
    });
    capture(window, &shots.join("bell-news.png"), cx).await?;
    notifications.update(cx, |this, cx| this.preview(&[], Vec::new(), Vec::new(), cx));

    check_mouse_navigation(window, cx)
}

/// The mouse's back and forward buttons over the page and the rail.
fn check_mouse_navigation(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
    show(window, cx, Route::Settings)?;
    draw(window, cx)?;
    let route = |cx: &mut AsyncApp| {
        window.update(cx, |root, _, cx| {
            app(root).map(|app| app.read(cx).current_route())
        })?
    };
    let size = window.update(cx, |_, window, _| window.viewport_size())?;
    let page = gpui::point(size.width * 0.5, size.height * 0.7);
    // Below the rail's buttons.
    let rail = gpui::point(gpui::px(40.), size.height - gpui::px(20.));
    for (at, place) in [(page, "the page"), (rail, "the rail")] {
        press(
            window,
            at,
            gpui::MouseButton::Navigate(gpui::NavigationDirection::Back),
            cx,
        )?;
        ensure!(
            route(cx)? == Route::Modpacks,
            "mouse back over {place} didn't go back"
        );
        draw(window, cx)?;
        press(
            window,
            at,
            gpui::MouseButton::Navigate(gpui::NavigationDirection::Forward),
            cx,
        )?;
        ensure!(
            route(cx)? == Route::Settings,
            "mouse forward over {place} didn't go forward"
        );
        draw(window, cx)?;
        println!("mouse back and forward over {place}: ok");
    }
    show(window, cx, Route::Modpacks)
}

/// A left click at `at`, through gpui's input dispatch.
fn click(
    window: WindowHandle<Root>,
    at: gpui::Point<gpui::Pixels>,
    cx: &mut AsyncApp,
) -> Result<()> {
    press(window, at, gpui::MouseButton::Left, cx)
}

/// `button` pressed and released at `at`, through gpui's input dispatch.
fn press(
    window: WindowHandle<Root>,
    at: gpui::Point<gpui::Pixels>,
    button: gpui::MouseButton,
    cx: &mut AsyncApp,
) -> Result<()> {
    cx.update_window(window.into(), |_, window, cx| {
        let modifiers = gpui::Modifiers::default();
        window.dispatch_event(
            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                position: at,
                pressed_button: None,
                modifiers,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                button,
                position: at,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                button,
                position: at,
                modifiers,
                click_count: 1,
            }),
            cx,
        );
    })
}

/// What the snackbar showed before the panel opens: an error twice over, a
/// success and a host event.
fn sample_snackbars() -> [(SnackbarKind, &'static str); 4] {
    [
        (SnackbarKind::Error, "The modpack could not be found."),
        (SnackbarKind::Success, "The modpack has been applied"),
        (SnackbarKind::Error, "The modpack could not be found."),
        (SnackbarKind::Neutral, "Every mod is up to date."),
    ]
}

/// Account notifications as the host pushes them: a plain unread one, an
/// invite, and one already read. Nothing is asked of Quadrant ID.
fn sample_notifications() -> Vec<Notification> {
    let notification = |id: &str, kind: &str, message: String, at: i64, read: bool| Notification {
        notification_id: id.into(),
        user_id: "preview".into(),
        notification_type: Some(kind.into()),
        resource_id: None,
        message,
        created_at: String::new(),
        created_at_unix: at,
        read,
    };
    vec![
        notification(
            "plain",
            "generic",
            serde_json::json!({ "simple_message": "Gamma Pack was shared with you" }).to_string(),
            3,
            false,
        ),
        notification(
            "invite",
            "invite_to_sync",
            serde_json::json!({
                "notification_type": "invite_to_sync",
                "message": "You have been invited to collaborate on a modpack by sam",
                "invite_id": "cloud-gamma",
            })
            .to_string(),
            2,
            false,
        ),
        notification("old", "generic", "Welcome to Quadrant Sync".into(), 1, true),
    ]
}

/// A news post, new or not. The blog is not asked.
fn sample_news(new: bool) -> Vec<Article> {
    vec![Article {
        title: "Quadrant 26.10".into(),
        link: "https://blog.mrquantumoff.dev/".into(),
        summary: "<p>Faster mod search, <b>Prism Launcher</b> instances &amp; more.</p>".into(),
        date: chrono::DateTime::UNIX_EPOCH,
        guid: "preview".into(),
        new,
    }]
}

/// Asks the real window procedure what each part of the title bar is, the
/// way Windows asks it before every click: `WM_NCHITTEST`. `HTCAPTION`
/// drags the window and swallows the click; `HTCLIENT` hands the click to
/// the app. The window stays hidden; only messages to it are sent.
#[cfg(windows)]
mod hit_test {
    use anyhow::{Context as _, Result, ensure};
    use gpui::{AppContext as _, AsyncApp, Bounds, Pixels, Point, WindowHandle, point, px};
    use gpui_component::Root;
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows::Win32::{
        Foundation::{HWND, LPARAM, POINT, WPARAM},
        Graphics::Gdi::ClientToScreen,
        UI::WindowsAndMessaging::{HTCAPTION, HTCLIENT, SendMessageW, WM_NCHITTEST},
    };

    use super::{app, draw, park_pointer};
    use crate::app::TitleBarLayout;

    /// A point on the title bar, and whether pressing there should drag the
    /// window, as `data-tauri-drag-region` makes it in the desktop app.
    struct Probe {
        name: String,
        at: Point<Pixels>,
        drags: bool,
    }

    fn probes(layout: &TitleBarLayout) -> Result<Vec<Probe>> {
        let [pill, cluster] = layout.bar[..] else {
            anyhow::bail!("the title bar has {} parts, not 2", layout.bar.len());
        };
        let controls = layout.controls.context("no window controls")?;
        let middle = |bounds: Bounds<Pixels>| bounds.center();
        let mut probes = vec![
            Probe {
                name: "title pill".into(),
                at: middle(pill),
                drags: true,
            },
            Probe {
                name: "empty bar".into(),
                at: point((pill.right() + cluster.left()) / 2., middle(pill).y),
                drags: true,
            },
            Probe {
                name: "controls' padding".into(),
                at: point(controls.left() + px(4.), middle(controls).y),
                drags: false,
            },
        ];
        for (index, button) in layout.buttons.iter().enumerate() {
            let name = ["bell", "minimize", "close"]
                .get(index)
                .map_or_else(|| format!("button {index}"), |name| name.to_string());
            probes.push(Probe {
                name,
                at: middle(*button),
                drags: false,
            });
        }
        Ok(probes)
    }

    pub async fn check(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
        draw(window, cx)?;
        let (layout, hwnd, scale) = window.update(cx, |root, window, cx| {
            let layout = app(root)?.read(cx).title_bar_layout();
            let handle = window
                .window_handle()
                .map_err(|error| anyhow::anyhow!("no window handle: {error:?}"))?;
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                anyhow::bail!("not a Win32 window");
            };
            anyhow::Ok((
                layout,
                HWND(handle.hwnd.get() as *mut _),
                window.scale_factor(),
            ))
        })??;
        let mut wrong = Vec::new();
        for probe in probes(&layout)? {
            // gpui answers from the element under its last known pointer.
            cx.update_window(window.into(), |_, window, cx| {
                let event = gpui::MouseMoveEvent {
                    position: probe.at,
                    pressed_button: None,
                    modifiers: gpui::Modifiers::default(),
                };
                window.dispatch_event(gpui::PlatformInput::MouseMove(event), cx);
            })?;
            // Sent outside any gpui update, so the window procedure can
            // reach the window as it does for a real hit test.
            let mut screen = POINT {
                x: (f32::from(probe.at.x) * scale) as i32,
                y: (f32::from(probe.at.y) * scale) as i32,
            };
            unsafe { ClientToScreen(hwnd, &mut screen).ok()? };
            let lparam = (screen.x & 0xffff) | ((screen.y & 0xffff) << 16);
            let code = unsafe {
                SendMessageW(
                    hwnd,
                    WM_NCHITTEST,
                    Some(WPARAM(0)),
                    Some(LPARAM(lparam as isize)),
                )
            }
            .0 as u32;
            let expected = if probe.drags { HTCAPTION } else { HTCLIENT };
            let name = |code| match code {
                HTCAPTION => "HTCAPTION (drags)".to_string(),
                HTCLIENT => "HTCLIENT (clicks)".to_string(),
                other => format!("hit test code {other}"),
            };
            println!(
                "title bar hit test: {} at ({:.0}, {:.0}) -> {}",
                probe.name,
                f32::from(probe.at.x),
                f32::from(probe.at.y),
                name(code)
            );
            if code != expected {
                wrong.push(format!(
                    "{} is {}, not {}",
                    probe.name,
                    name(code),
                    name(expected)
                ));
            }
        }
        park_pointer(window, cx)?;
        ensure!(wrong.is_empty(), "title bar hit test: {}", wrong.join("; "));
        Ok(())
    }
}

/// Experimental features turned off and on again while the Apply page is
/// away, as Settings does: showing the page again lists the Prism Launcher
/// instances again, so the "Applied to" line leaves and comes back without
/// a refresh.
async fn capture_revalidate(
    window: WindowHandle<Root>,
    dir: &Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    let shots = dir.join("revalidate");
    std::fs::create_dir_all(&shots)?;
    with_page::<ModpacksPage>(window, cx, Route::Modpacks, |page, window, cx| {
        page.preview_cloud(Vec::new(), "", None, window, cx)
    })?;
    for (on, file) in [
        (false, "apply-experimental-off.png"),
        (true, "apply-experimental-on.png"),
    ] {
        show(window, cx, Route::Settings)?;
        cx.update(|cx| {
            Backend::global(cx).run(cx, move |host| async move {
                crate::config::set(&host, "experimentalFeatures", on.into())
            })
        })
        .await?;
        show(window, cx, Route::Modpacks)?;
        capture(window, &shots.join(file), cx).await?;
    }
    Ok(())
}

/// Frames from the page transitions: a page entering as it opens, part way
/// in, and at rest; a pack's Details crossfading in over the list and back;
/// and a mod's install page entering from Search and Search entering again
/// on the way back. The hidden window draws only when asked, so a
/// transition starts at the first frame after the change; the "mid" frame
/// is the one drawn right after it, a draw's time into the transition.
async fn capture_motion(window: WindowHandle<Root>, dir: &Path, cx: &mut AsyncApp) -> Result<()> {
    let shots = dir.join("motion");
    std::fs::create_dir_all(&shots)?;
    let rail = |route: Route| move |window, cx: &mut AsyncApp| show(window, cx, route);
    transition(window, &shots, "settings-enter", rail(Route::Settings), cx).await?;
    show(window, cx, Route::Modpacks)?;
    capture(window, &shots.join("apply-rest.png"), cx).await?;
    transition(
        window,
        &shots,
        "settings-enter-again",
        rail(Route::Settings),
        cx,
    )
    .await?;

    let details = |window, cx: &mut AsyncApp| {
        with_page(
            window,
            cx,
            Route::Modpacks,
            |page: &mut ModpacksPage, window, cx| {
                page.preview(Some("Alpha Pack"), false, window, cx)
            },
        )
    };
    let list = |window, cx: &mut AsyncApp| {
        with_page(
            window,
            cx,
            Route::Modpacks,
            |page: &mut ModpacksPage, _, cx| {
                page.back(cx);
            },
        )
    };
    show(window, cx, Route::Modpacks)?;
    capture(window, &shots.join("apply-before-details.png"), cx).await?;
    transition(window, &shots, "details-crossfade", details, cx).await?;
    transition(window, &shots, "details-back", list, cx).await?;

    let install = |window, cx: &mut AsyncApp| {
        with_page::<SearchPage>(window, cx, Route::Search, |page, window, cx| {
            page.open_first_result(window, cx)
        })
    };
    let results = |window, cx: &mut AsyncApp| {
        with_page::<SearchPage>(window, cx, Route::Search, |page, _, cx| {
            page.back(cx);
        })
    };
    show(window, cx, Route::Search)?;
    capture(window, &shots.join("search-before-install.png"), cx).await?;
    transition(window, &shots, "install-enter", install, cx).await?;
    transition(window, &shots, "install-back", results, cx).await?;

    // A result's install page growing out of its card on a double click,
    // then shrinking back into it over Search, which stays as it was.
    show(window, cx, Route::Search)?;
    capture(window, &shots.join("search-before-hero.png"), cx).await?;
    let open_card = |window, cx: &mut AsyncApp| double_click(window, cx, FIRST_RESULT);
    timeline(
        window,
        &shots,
        "hero-open",
        open_card,
        &[30, 80, 160, 250, 400, 900],
        cx,
    )
    .await?;
    timeline(
        window,
        &shots,
        "hero-close",
        results,
        &[30, 90, 160, 210, 300, 800],
        cx,
    )
    .await?;

    // The filters sidebar folding away and out again.
    let filters = |open: bool| {
        move |window, cx: &mut AsyncApp| {
            with_page::<SearchPage>(window, cx, Route::Search, move |page, _, cx| {
                page.set_filters_open(open, cx)
            })
        }
    };
    timeline(
        window,
        &shots,
        "filters-close",
        filters(false),
        &[40, 100, 180, 400],
        cx,
    )
    .await?;
    timeline(
        window,
        &shots,
        "filters-open",
        filters(true),
        &[40, 100, 180, 400],
        cx,
    )
    .await?;

    // Mod cards flying in as Current modpack is shown again.
    show(window, cx, Route::Modpacks)?;
    capture(window, &shots.join("apply-before-cards.png"), cx).await?;
    timeline(
        window,
        &shots,
        "cards-enter",
        rail(Route::CurrentModpack),
        &[40, 120, 240, 500, 1200],
        cx,
    )
    .await?;

    // The spinner sliding in, then turning.
    let searching = |window, cx: &mut AsyncApp| {
        with_page::<SearchPage>(window, cx, Route::Search, |page, _, cx| {
            page.preview_searching(cx)
        })
    };
    timeline(
        window,
        &shots,
        "progress",
        searching,
        &[30, 70, 500, 1500],
        cx,
    )
    .await
}

/// Inside the first Search result card at the motion pictures' 1150px wide
/// window with the filters open, on its description.
const FIRST_RESULT: gpui::Point<gpui::Pixels> = gpui::point(gpui::px(560.), gpui::px(355.));

/// A double click at `position`, as the platform sends one.
fn double_click(
    window: WindowHandle<Root>,
    cx: &mut AsyncApp,
    position: gpui::Point<gpui::Pixels>,
) -> Result<()> {
    cx.update_window(window.into(), |_, window, cx| {
        let modifiers = gpui::Modifiers::default();
        window.dispatch_event(
            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position,
                modifiers,
                click_count: 2,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position,
                modifiers,
                click_count: 2,
            }),
            cx,
        );
    })
}

/// Makes a change, draws its first frame, then saves frames about `at_ms`
/// milliseconds after it.
async fn timeline(
    window: WindowHandle<Root>,
    dir: &Path,
    name: &str,
    change: impl Fn(WindowHandle<Root>, &mut AsyncApp) -> Result<()>,
    at_ms: &[u64],
    cx: &mut AsyncApp,
) -> Result<()> {
    change(window, cx)?;
    draw(window, cx)?;
    let started = std::time::Instant::now();
    for &ms in at_ms {
        // Waits at least a moment, so tasks the change started (a page
        // leaving after its own animation) get to run as they would.
        let wait = Duration::from_millis(ms)
            .checked_sub(started.elapsed())
            .unwrap_or(Duration::from_millis(1));
        cx.background_executor().timer(wait).await;
        // The frames views asked for, which the hidden window never gets:
        // a grid following its new width, an animation's next step.
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx);
        })?;
        capture_now(window, &dir.join(format!("{name}-{ms}ms.png")), cx)?;
    }
    Ok(())
}

/// Makes a change, draws its first frame, saves the next one, part way
/// through the transition, then saves it at rest.
async fn transition(
    window: WindowHandle<Root>,
    dir: &Path,
    name: &str,
    change: impl Fn(WindowHandle<Root>, &mut AsyncApp) -> Result<()>,
    cx: &mut AsyncApp,
) -> Result<()> {
    change(window, cx)?;
    draw(window, cx)?;
    capture_now(window, &dir.join(format!("{name}-mid.png")), cx)?;
    cx.background_executor().timer(SETTLE).await;
    capture_now(window, &dir.join(format!("{name}-rest.png")), cx)
}

/// The Prism Launcher menu over the sandbox's two instances. Alpha Pack's
/// shows Modded linked and what applying would switch Survival to; Beta
/// Pack's shows a version and a loader switch for both. Nothing is applied.
async fn capture_prism(window: WindowHandle<Root>, dir: &Path, cx: &mut AsyncApp) -> Result<()> {
    let shots = dir.join("prism");
    std::fs::create_dir_all(&shots)?;
    for (modpack, file) in [
        ("Alpha Pack", "menu-linked.png"),
        ("Beta Pack", "menu-switch.png"),
    ] {
        with_page(
            window,
            cx,
            Route::Modpacks,
            move |page: &mut ModpacksPage, _, cx| page.preview_prism_menu(Some(modpack), cx),
        )?;
        // The popover places itself from the trigger's bounds of the frame
        // before.
        draw(window, cx)?;
        capture(window, &shots.join(file), cx).await?;
    }
    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, _, cx| page.preview_prism_menu(None, cx),
    )
}

/// The "Install to" picker for resource packs, on Search and on a pack's
/// install page, against the live Modrinth API. Nothing is installed.
async fn capture_install_to(
    window: WindowHandle<Root>,
    dir: &Path,
    cx: &mut AsyncApp,
) -> Result<()> {
    let shots = dir.join("prism");
    with_page::<SearchPage>(window, cx, Route::Search, |page, window, cx| {
        page.close_details(cx);
        page.set_content(ContentType::ResourcePacks, window, cx);
        page.set_query("fresh animations", window, cx);
    })?;
    cx.background_executor().timer(NETWORK_SETTLE).await;
    capture(window, &shots.join("search-install-to.png"), cx).await?;
    with_page::<SearchPage>(window, cx, Route::Search, |page, window, cx| {
        page.open_first_result(window, cx)
    })?;
    cx.background_executor().timer(NETWORK_SETTLE).await;
    capture(window, &shots.join("mod-install-install-to.png"), cx).await?;
    with_page::<SearchPage>(window, cx, Route::Search, |page, _, cx| {
        page.close_details(cx)
    })
}

/// The title bar's update button while downloading and once ready, over the
/// settings page with its channel picker. No update server is asked.
#[cfg(feature = "updater")]
async fn capture_updater(window: WindowHandle<Root>, dir: &Path, cx: &mut AsyncApp) -> Result<()> {
    use crate::updater::{Status, Updater};
    show(window, cx, Route::Settings)?;
    cx.update(|cx| Updater::preview(Status::Downloading(0.42), cx));
    capture(window, &dir.join("update-downloading.png"), cx).await?;
    let update = Updater::preview_update("26.10.1-stable");
    cx.update(|cx| Updater::preview(Status::Ready(update), cx));
    capture(window, &dir.join("update-ready.png"), cx).await
}

#[cfg(not(feature = "updater"))]
async fn capture_updater(_: WindowHandle<Root>, _: &Path, _: &mut AsyncApp) -> Result<()> {
    Ok(())
}

/// An account for the signed-in pictures. Nothing is asked of Quadrant ID.
fn sample_account() -> AccountInfo {
    AccountInfo {
        id: "preview".into(),
        name: "Alex".into(),
        email: "alex@example.com".into(),
        quadrant_sync_limit: 10,
        quadrant_share_limit: 50,
        login: "alex".into(),
        notifications: Vec::new(),
    }
}

/// Quadrant Sync records for the signed-in pictures: one paired with the
/// sandbox's Alpha Pack by name, one only in the cloud.
fn sample_synced() -> Vec<SyncedModpack> {
    let owners = vec![
        ModpackOwner {
            username: "alex".into(),
            admin: true,
        },
        ModpackOwner {
            username: "sam".into(),
            admin: false,
        },
    ];
    let pack = |name: &str, id: &str, version: &str, loader, last_synced| {
        SyncedModpack {
        name: name.into(),
        minecraft_version: version.into(),
        mod_loader: loader,
        mods: r#"[{"id":"sodium","name":"sodium","source":"ModSource.modRinth","downloadUrl":"https://cdn.modrinth.com/data/fixture/sodium.jar"}]"#.into(),
        owners: owners.clone(),
        last_synced,
        modpack_id: id.into(),
    }
    };
    vec![
        pack(
            "Alpha Pack",
            "cloud-alpha",
            "1.21.1",
            ModLoader::Fabric,
            1_789_830_660,
        ),
        pack(
            "Gamma Pack",
            "cloud-gamma",
            "1.20.4",
            ModLoader::NeoForge,
            1_789_000_000,
        ),
    ]
}

fn resize(window: WindowHandle<Root>, cx: &mut AsyncApp, height: f32) -> Result<()> {
    resize_to(window, cx, 1150., height)
}

fn resize_to(window: WindowHandle<Root>, cx: &mut AsyncApp, width: f32, height: f32) -> Result<()> {
    window.update(cx, |_, window, _| {
        window.resize(gpui::size(gpui::px(width), gpui::px(height)))
    })
}

/// The grids that follow the window's width, at a narrow, a middling and a
/// wide window: Search with an installed result, a modpack's Details and
/// the Current modpack page. Runs after `capture_mod_pages` registered Mod
/// Menu in Alpha Pack, so the search finds it installed.
async fn capture_widths(window: WindowHandle<Root>, dir: &Path, cx: &mut AsyncApp) -> Result<()> {
    let shots = dir.join("widths");
    std::fs::create_dir_all(&shots)?;
    let restore = window.update(cx, |_, window, _| window.viewport_size())?;
    with_page::<CurrentModpackPage>(window, cx, Route::CurrentModpack, |page, _, cx| {
        page.toggle_updates(cx)
    })?;
    with_page::<ModpacksPage>(window, cx, Route::Modpacks, |page, _, cx| page.refresh(cx))?;
    with_page::<SearchPage>(window, cx, Route::Search, |page, window, cx| {
        page.close_details(cx);
        page.refresh_modpacks(window, cx);
        page.set_content(ContentType::Mods, window, cx);
        page.set_target("Alpha Pack", window, cx);
        page.set_query("mod menu", window, cx);
    })?;
    cx.background_executor().timer(NETWORK_SETTLE).await;
    for width in [900., 1400., 2200.] {
        resize_to(window, cx, width, 1000.)?;
        with_page::<SearchPage>(window, cx, Route::Search, |page, _, cx| {
            page.close_details(cx)
        })?;
        settle_layout(window, cx).await?;
        capture(window, &shots.join(format!("search-{width}.png")), cx).await?;
        with_page::<SearchPage>(window, cx, Route::Search, |page, window, cx| {
            page.open_first_result(window, cx)
        })?;
        cx.background_executor().timer(NETWORK_SETTLE).await;
        capture(window, &shots.join(format!("mod-install-{width}.png")), cx).await?;
        with_page(
            window,
            cx,
            Route::Modpacks,
            |page: &mut ModpacksPage, window, cx| {
                page.preview(Some("Alpha Pack"), false, window, cx)
            },
        )?;
        capture(window, &shots.join(format!("details-{width}.png")), cx).await?;
        with_page(
            window,
            cx,
            Route::Modpacks,
            |page: &mut ModpacksPage, window, cx| page.preview(None, false, window, cx),
        )?;
        show(window, cx, Route::CurrentModpack)?;
        capture(
            window,
            &shots.join(format!("current-modpack-{width}.png")),
            cx,
        )
        .await?;
    }
    with_page::<SearchPage>(window, cx, Route::Search, |page, _, cx| {
        page.close_details(cx)
    })?;
    resize_to(window, cx, restore.width.into(), restore.height.into())?;

    // One card at work leaves every other card, and the card's other
    // buttons, as they were.
    with_page::<SearchPage>(window, cx, Route::Search, |page, _, cx| {
        page.preview_installing(Some(42), cx)
    })?;
    settle_layout(window, cx).await?;
    capture(window, &dir.join("search-installing.png"), cx).await?;
    with_page::<SearchPage>(window, cx, Route::Search, |page, _, cx| {
        page.preview_installing(None, cx)
    })?;
    with_page::<ModpacksPage>(window, cx, Route::Modpacks, |page, _, cx| {
        page.preview_exporting(Some("Beta Pack"), cx)
    })?;
    capture(window, &dir.join("modpacks-exporting.png"), cx).await?;
    with_page::<ModpacksPage>(window, cx, Route::Modpacks, |page, _, cx| {
        page.preview_exporting(None, cx)
    })?;

    // A pack's Details: its header scrolls away with the mods, and back
    // leaves for the list.
    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, window, cx| page.preview(Some("Alpha Pack"), false, window, cx),
    )?;
    capture(window, &dir.join("details-top.png"), cx).await?;
    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, _, cx| page.scroll_details(gpui::px(400.), cx),
    )?;
    capture(window, &dir.join("details-scrolled.png"), cx).await?;
    with_page(
        window,
        cx,
        Route::Modpacks,
        |page: &mut ModpacksPage, _, cx| {
            page.back(cx);
        },
    )?;
    capture(window, &dir.join("details-back.png"), cx).await
}

/// The applied pack's mods on Current modpack and in its Details, scrolled
/// to the end by one wheel event at a narrow, a middling and a wide window:
/// the last row is reachable however few rows were drawn before. On the
/// large sandbox the pack has 500 mods.
async fn capture_reach(window: WindowHandle<Root>, dir: &Path, cx: &mut AsyncApp) -> Result<()> {
    let shots = dir.join("reach");
    std::fs::create_dir_all(&shots)?;
    let restore = window.update(cx, |_, window, _| window.viewport_size())?;
    for width in [900., 1400., 2200.] {
        resize_to(window, cx, width, 1000.)?;
        show(window, cx, Route::CurrentModpack)?;
        settle_frames(window, cx).await?;
        capture_now(
            window,
            &shots.join(format!("current-modpack-{width}-top.png")),
            cx,
        )?;
        wheel_to_end(window, cx)?;
        settle_frames(window, cx).await?;
        capture_now(
            window,
            &shots.join(format!("current-modpack-{width}-end.png")),
            cx,
        )?;

        with_page(
            window,
            cx,
            Route::Modpacks,
            |page: &mut ModpacksPage, window, cx| {
                page.preview(Some("Alpha Pack"), false, window, cx)
            },
        )?;
        settle_frames(window, cx).await?;
        wheel_to_end(window, cx)?;
        settle_frames(window, cx).await?;
        capture_now(window, &shots.join(format!("details-{width}-end.png")), cx)?;
        // Back up past every row to the header and its Cancel button.
        wheel(window, cx, 1_000_000.)?;
        settle_frames(window, cx).await?;
        capture_now(window, &shots.join(format!("details-{width}-top.png")), cx)?;
        with_page(
            window,
            cx,
            Route::Modpacks,
            |page: &mut ModpacksPage, window, cx| page.preview(None, false, window, cx),
        )?;
    }
    resize_to(window, cx, restore.width.into(), restore.height.into())
}

/// Lets a change land and draws the frames the views ask for after it, as
/// a visible window would: a list hinting its rows at a new width, a grid
/// following it.
async fn settle_frames(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
    cx.background_executor()
        .timer(crate::motion::CARD_ENTER + Duration::from_millis(100))
        .await;
    park_pointer(window, cx)?;
    for _ in 0..3 {
        cx.update_window(window.into(), |_, window, cx| {
            window.simulate_next_frame(cx);
            window.refresh();
            window.draw(cx).clear(cx);
        })?;
    }
    Ok(())
}

/// One wheel event far past the end of the page's content.
fn wheel_to_end(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
    wheel(window, cx, -1_000_000.)
}

/// A wheel event `dy` pixels over the middle of the content.
fn wheel(window: WindowHandle<Root>, cx: &mut AsyncApp, dy: f32) -> Result<()> {
    cx.update_window(window.into(), |_, window, cx| {
        let size = window.viewport_size();
        let event = gpui::ScrollEvent {
            position: gpui::point(size.width / 2., size.height / 2.),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(dy))),
            modifiers: gpui::Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
            source: gpui::ScrollSource::Pointer,
        };
        window.dispatch_event(gpui::PlatformInput::Scroll(event), cx);
    })
}

/// Closes every dialog and draws it out of gpui-component's stack: a dialog
/// still leaving stays there and pushes the next one down. The hidden window
/// draws only when asked, so one frame starts the exit transition and one
/// after it unmounts the dialog.
async fn close_dialogs(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
    window.update(cx, |_, window, cx| {
        window.defer(cx, |window, cx| window.close_all_dialogs(cx))
    })?;
    draw(window, cx)?;
    cx.background_executor()
        .timer(Duration::from_millis(500))
        .await;
    draw(window, cx)
}

fn app(root: &Root) -> Result<Entity<QuadrantApp>> {
    root.view()
        .clone()
        .downcast::<QuadrantApp>()
        .ok()
        .context("root is not the app")
}

/// Opens `route` in the window, as a click on its rail button does.
fn show(window: WindowHandle<Root>, cx: &mut AsyncApp, route: Route) -> Result<()> {
    window.update(cx, |root, window, cx| {
        app(root)?.update(cx, |app, cx| {
            app.navigate(route, window, cx);
            // Built now rather than at the next draw, so its first load
            // runs while the capture waits.
            app.page(route, window, cx);
        });
        anyhow::Ok(())
    })?
}

/// Lets a resize land and draws one frame at the new size: search results
/// take their column count from the width the last frame measured.
async fn settle_layout(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
    cx.background_executor()
        .timer(Duration::from_millis(500))
        .await;
    park_pointer(window, cx)?;
    draw(window, cx)
}

/// Moves the pointer to the empty corner above the rail, so no tooltip
/// opens over what a picture shows.
fn park_pointer(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
    cx.update_window(window.into(), |_, window, cx| {
        let event = gpui::MouseMoveEvent {
            position: gpui::point(gpui::px(1.), gpui::px(1.)),
            pressed_button: None,
            modifiers: gpui::Modifiers::default(),
        };
        window.dispatch_event(gpui::PlatformInput::MouseMove(event), cx);
    })
}

/// Draws one frame, for work a view defers until after it renders.
fn draw(window: WindowHandle<Root>, cx: &mut AsyncApp) -> Result<()> {
    cx.update_window(window.into(), |_, window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    })
}

async fn capture(window: WindowHandle<Root>, path: &Path, cx: &mut AsyncApp) -> Result<()> {
    cx.background_executor().timer(SETTLE).await;
    // An animation starts at the first frame that shows it, and this window
    // draws only here: one frame starts it, the picture is taken at rest.
    park_pointer(window, cx)?;
    draw(window, cx)?;
    cx.background_executor()
        .timer(crate::motion::ENTER + Duration::from_millis(50))
        .await;
    capture_now(window, path, cx)
}

/// Draws and saves a frame at once, for a moment within an animation.
fn capture_now(window: WindowHandle<Root>, path: &Path, cx: &mut AsyncApp) -> Result<()> {
    // The window is hidden, so the platform never asks it for frames; each
    // picture draws one itself. `update_window` leaves the root view free for
    // the draw to render.
    let image = cx.update_window(window.into(), |_, window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
        window.render_to_image()
    })??;
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
            app.navigate(route, window, cx);
            app.page(route, window, cx)
        });
        let page = page.downcast::<P>().ok().context("not the expected page")?;
        window.defer(cx, move |window, cx| {
            page.update(cx, |page, cx| f(page, window, cx))
        });
        anyhow::Ok(())
    })?
}
