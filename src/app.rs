use std::collections::HashMap;

use gpui::{
    AnyElement, AnyView, App, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    NavigationDirection, ParentElement as _, Render, Styled as _, Window, WindowControlArea, div,
    prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{h_flex, v_flex};

use crate::{
    assets::MdIcon,
    motion, notifications, pages,
    route::{History, Route},
    snackbar::{self, Snackbar},
    theme::{self, Tone},
    tray,
};

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
            history: History::new(Route::Modpacks),
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
