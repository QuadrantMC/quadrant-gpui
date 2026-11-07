use std::{collections::HashMap, time::Duration};

use futures::{StreamExt as _, channel::mpsc::UnboundedReceiver};
use gpui::{
    AnyElement, AnyView, App, AsyncApp, Context, Entity, Global, InteractiveElement as _,
    IntoElement, MouseButton, NavigationDirection, ParentElement as _, Render, Styled as _, Window,
    WindowControlArea, WindowHandle, div, prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{Root, h_flex, v_flex};

use crate::{
    assets::MdIcon,
    backend::Backend,
    config,
    deep_link::{self, Action},
    i18n::{describe, t},
    motion, notifications, oauth,
    pages::{self, ModpacksPage, SearchPage},
    route::{History, Route},
    snackbar::{self, Snackbar, SnackbarKind},
    theme::{self, Tone},
    tray,
};

/// The config key the desktop app keeps the rail's last page under.
const LAST_PAGE: &str = "lastPageName";

/// The page a window opens on: the one last picked in the rail.
struct LastPage(Route);

impl Global for LastPage {}

/// Reads the page the rail last opened, so the window opens on it as the
/// desktop app's does. Screenshot and perf runs, and anything unreadable,
/// start on Apply. Call before opening the window.
pub fn restore_last_page(interactive: bool, cx: &mut App) {
    let route = if interactive {
        let read =
            Backend::global(cx).run(cx, |host| async move { host.get_config_value(LAST_PAGE) });
        // A read of the config file the host already holds in memory.
        cx.foreground_executor()
            .block_with_timeout(Duration::from_millis(500), read)
            .ok()
            .and_then(Result::ok)
            .flatten()
            .and_then(|name| name.as_str().and_then(Route::from_name))
    } else {
        None
    };
    let route = route.unwrap_or(Route::Modpacks);
    log::debug!("Opening on the {} page", route.name());
    cx.set_global(LastPage(route));
}

/// Saves `route` as the page to open next time, as the rail does.
fn remember_page(route: Route, cx: &mut App) {
    cx.set_global(LastPage(route));
    let task = Backend::global(cx).run(cx, move |host| async move {
        config::set(&host, LAST_PAGE, route.name().into())
    });
    cx.spawn(async move |_| {
        if let Err(error) = task.await {
            log::warn!("Couldn't save the last page: {error:#}");
        }
    })
    .detach();
}

/// Opens each batch of links `inbox` brings, from this launch, a later one
/// or the platform, after bringing the main window back as the desktop
/// app's single-instance handler does. `window` is the window `main` opened.
pub fn open_links(
    mut inbox: UnboundedReceiver<Vec<String>>,
    window: WindowHandle<Root>,
    cx: &mut App,
) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(urls) = inbox.next().await {
            cx.update(|cx| {
                log::info!("Opening {urls:?}");
                let Some(window) = tray::reveal(window, cx) else {
                    return;
                };
                let actions = deep_link::plan(&urls);
                let _ = window.update(cx, |root, window, cx| {
                    if let Ok(app) = root.view().clone().downcast::<QuadrantApp>() {
                        app.update(cx, |app, cx| app.follow(actions, window, cx));
                    }
                });
            });
        }
    })
    .detach();
}

/// The window's content: the page rail, the title bar and the open page.
pub struct QuadrantApp {
    history: History,
    /// Pages built so far. A page is built on first visit and kept, so its
    /// state survives navigating away and back.
    pages: HashMap<Route, AnyView>,
    /// Set by a press on the title bar, so the next mouse move drags the
    /// window on platforms without a native drag region.
    pressed_title_bar: bool,
    snackbar: Entity<Snackbar>,
    /// The open page's entrance, replayed whenever another page opens.
    entrance: Entrance,
    /// Where the title bar's parts were laid out in the last frame.
    #[cfg(feature = "screenshot")]
    title_bar_layout: std::rc::Rc<std::cell::RefCell<TitleBarLayout>>,
}

/// The title bar's parts as the last frame laid them out, for screenshot
/// mode's check that the window controls take clicks and the rest drags.
#[cfg(feature = "screenshot")]
#[derive(Clone, Debug, Default)]
pub struct TitleBarLayout {
    /// The title pill and the cluster on the right.
    pub bar: Vec<gpui::Bounds<gpui::Pixels>>,
    /// The window controls' rounded container.
    pub controls: Option<gpui::Bounds<gpui::Pixels>>,
    /// The buttons in it: the bell, then minimize and close.
    pub buttons: Vec<gpui::Bounds<gpui::Pixels>>,
}

/// Which of the title bar's rows `record_layout` notes.
#[derive(Clone, Copy)]
enum LayoutSlot {
    /// The title pill and the cluster on the right.
    Bar,
    /// The update button, if any, and the window controls' container.
    Cluster,
    /// The window controls.
    Controls,
}

/// A page's entrance animation, after the desktop app's page `motion.div`s.
#[derive(Clone, Copy)]
struct Entrance {
    /// Counts page changes, so each one plays afresh.
    id: usize,
    /// The window's first page, which the content container's own entrance
    /// carries in too.
    first: bool,
}

impl Entrance {
    fn next(self) -> Self {
        Self {
            id: self.id + 1,
            first: false,
        }
    }
}

impl QuadrantApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        #[cfg(feature = "updater")]
        if let Some(updater) = crate::updater::Updater::global(cx) {
            cx.observe(&updater, |_, _, cx| cx.notify()).detach();
        }
        Self {
            history: History::new(
                cx.try_global::<LastPage>()
                    .map_or(Route::Modpacks, |last| last.0),
            ),
            pages: HashMap::new(),
            pressed_title_bar: false,
            snackbar: snackbar::view(cx),
            entrance: Entrance { id: 0, first: true },
            #[cfg(feature = "screenshot")]
            title_bar_layout: Default::default(),
        }
    }

    /// The open page.
    #[cfg(feature = "screenshot")]
    pub fn current_route(&self) -> Route {
        self.history.current()
    }

    /// The title bar's parts as the last frame laid them out.
    #[cfg(feature = "screenshot")]
    pub fn title_bar_layout(&self) -> TitleBarLayout {
        self.title_bar_layout.borrow().clone()
    }

    pub fn navigate(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
        if self.history.navigate(route) {
            self.entered(window, cx);
        }
    }

    /// Carries out what links ask, as `App.tsx`'s `onOpenUrl` does: a mod's
    /// install page on Search, a share code's import on Apply, the end of a
    /// Quadrant ID sign-in.
    fn follow(&mut self, actions: Vec<Action>, window: &mut Window, cx: &mut Context<Self>) {
        for action in actions {
            log::info!("Following {action:?}");
            match action {
                Action::Ignore => {}
                Action::Unsupported => {
                    snackbar::show(cx, SnackbarKind::Error, t("unsupportedDownload"))
                }
                Action::InstallMod(link) => {
                    self.navigate(Route::Search, window, cx);
                    let page = self.page(Route::Search, window, cx);
                    if let Ok(page) = page.downcast::<SearchPage>() {
                        page.update(cx, |page, cx| page.open_link(link, window, cx));
                    }
                }
                Action::ImportModpack(code) => {
                    self.navigate(Route::Modpacks, window, cx);
                    let page = self.page(Route::Modpacks, window, cx);
                    if let Ok(page) = page.downcast::<ModpacksPage>() {
                        page.update(cx, |page, cx| page.import_share(code, cx));
                    }
                }
                Action::OauthLogin {
                    state,
                    code,
                    redirect_uri,
                } => {
                    let task = Backend::global(cx).run(cx, move |host| {
                        oauth::finish_redirect(host, state, code, redirect_uri)
                    });
                    cx.spawn(async move |_, cx| {
                        if let Err(error) = task.await {
                            cx.update(|cx| {
                                snackbar::show(cx, SnackbarKind::Error, describe(error))
                            });
                        }
                    })
                    .detach();
                }
            }
        }
    }

    fn step(
        &mut self,
        direction: NavigationDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if direction == NavigationDirection::Back
            && let Some(page) = self.pages.get(&self.history.current()).cloned()
            && pages::back(&page, cx)
        {
            cx.notify();
            return;
        }
        let moved = match direction {
            NavigationDirection::Back => self.history.back(),
            NavigationDirection::Forward => self.history.forward(),
        };
        if moved.is_some() {
            self.entered(window, cx);
        }
    }

    /// Another page is open: it plays its entrance and, if it was open
    /// before, re-reads its host data.
    fn entered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.entrance = self.entrance.next();
        if let Some(page) = self.pages.get(&self.history.current()) {
            pages::shown(page, window, cx);
        }
        cx.notify();
    }

    pub fn page(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) -> AnyView {
        self.pages
            .entry(route)
            .or_insert_with(|| pages::build(route, window, cx))
            .clone()
    }

    /// Drops every built page, so each is built afresh on its next visit.
    #[cfg(feature = "perf")]
    pub fn forget_pages(&mut self) {
        self.pages.clear();
    }

    /// Notes where `div`'s children were laid out, in screenshot builds.
    #[cfg(feature = "screenshot")]
    fn record_layout(&self, div: gpui::Div, slot: LayoutSlot) -> gpui::Div {
        let layout = self.title_bar_layout.clone();
        div.on_children_prepainted(move |bounds, _, _| {
            let mut layout = layout.borrow_mut();
            match slot {
                LayoutSlot::Bar => layout.bar = bounds,
                LayoutSlot::Cluster => layout.controls = bounds.last().copied(),
                LayoutSlot::Controls => layout.buttons = bounds,
            }
        })
    }

    #[cfg(not(feature = "screenshot"))]
    fn record_layout(&self, div: gpui::Div, _: LayoutSlot) -> gpui::Div {
        div
    }

    /// The open page, entering from below as it fades in.
    fn render_page(&self, page: AnyView) -> impl IntoElement {
        motion::entering(
            div().size_full().relative().child(page),
            ("page-entrance", self.entrance.id),
            self.entrance.first,
        )
    }

    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.history.current();
        h_flex()
            .h_full()
            .child(
                v_flex()
                    .mx_2()
                    .gap_2()
                    .justify_center()
                    .children(Route::ALL.map(|route| {
                        theme::rail_button(route, route == current, cx)
                            .tooltip(route.label())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                remember_page(route, cx);
                                this.navigate(route, window, cx)
                            }))
                    })),
            )
            .child(div().w(px(4.)).h_full().bg(rgb(theme::CONTROL)))
    }

    fn render_title_bar(&self, page: &AnyView, cx: &mut Context<Self>) -> impl IntoElement {
        let title = pages::title(page, cx).unwrap_or_else(|| self.history.current().label());
        self.record_layout(h_flex(), LayoutSlot::Bar)
            .id("title-bar")
            .w_full()
            .flex_shrink_0()
            .justify_between()
            .items_center()
            .px_4()
            .py_2()
            .border_b_4()
            .border_color(rgb(theme::CONTROL))
            .window_control_area(WindowControlArea::Drag)
            .when(!cfg!(target_os = "windows"), |this| {
                this.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.pressed_title_bar = true),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.pressed_title_bar = false),
                )
                .on_mouse_move(cx.listener(|this, _, window, _| {
                    if std::mem::take(&mut this.pressed_title_bar) {
                        window.start_window_move();
                    }
                }))
            })
            .child(theme::title_pill(title))
            .child(
                self.record_layout(h_flex(), LayoutSlot::Cluster)
                    .gap_4()
                    .children(update_button(cx).map(|button| clickable(div().child(button))))
                    .child(self.render_window_controls(cx)),
            )
    }

    fn render_window_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        clickable(self.record_layout(h_flex(), LayoutSlot::Controls))
            .p_2()
            .gap_2()
            .children(notifications::view(cx))
            // macOS draws its own window buttons, as the desktop app's
            // native decorations, on by default there, do; the bell then
            // stands alone.
            .when(!cfg!(target_os = "macos"), |this| {
                this.rounded_full()
                    .bg(rgb(theme::SURFACE))
                    .child(
                        theme::window_button("minimize", Tone::Control, MdIcon::Minimize, cx)
                            .on_click(|_, window, _| window.minimize_window()),
                    )
                    .child(
                        theme::window_button("close", Tone::Control, MdIcon::Close, cx)
                            .on_click(|_, window, cx| tray::close(window, cx)),
                    )
            })
    }
}

/// Takes `element` out of the title bar's drag region, so a press on it
/// reaches its buttons instead of moving the window.
///
/// The whole title bar is a `WindowControlArea::Drag`, and gpui reports the
/// window's control area from every hitbox under the pointer, not only the
/// topmost. Over a button inside the bar, Windows would hear `HTCAPTION`
/// and start a window move that swallows the click; on macOS and Wayland
/// the bar's own press handler would start a move at the first jitter.
/// Blocking the hitboxes behind `element` leaves the bar out of both.
fn clickable(element: gpui::Div) -> gpui::Div {
    element.occlude()
}

/// The self-updater's button, while it has something to show.
fn update_button(cx: &App) -> Option<AnyElement> {
    #[cfg(feature = "updater")]
    return crate::updater::title_bar_button(cx);
    #[cfg(not(feature = "updater"))]
    {
        let _ = cx;
        None
    }
}

impl Render for QuadrantApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(feature = "perf")]
        crate::perf::frame_drawn();
        let page = self.page(self.history.current(), window, cx);
        let root = h_flex()
            .id("quadrant")
            .size_full()
            .bg(rgb(theme::PAGE))
            .text_color(rgb(theme::TEXT))
            .font_family(theme::FONT_FAMILY)
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _, window, cx| this.step(NavigationDirection::Back, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, _, window, cx| {
                    this.step(NavigationDirection::Forward, window, cx)
                }),
            )
            .child(self.render_rail(cx))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .overflow_hidden()
                    .child(self.render_title_bar(&page, cx))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .child(self.render_page(page)),
                    ),
            )
            .child(self.snackbar.clone());
        #[cfg(feature = "perf")]
        let root = root.child(crate::perf::paint_marker());
        root
    }
}
