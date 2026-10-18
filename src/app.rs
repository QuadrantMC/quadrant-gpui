use std::collections::HashMap;

use gpui::{
    AnyView, Context, Entity, InteractiveElement as _, IntoElement, MouseButton,
    NavigationDirection, ParentElement as _, Render, Styled as _, Window, WindowControlArea, div,
    prelude::FluentBuilder as _, px, rgb,
};
use gpui_component::{h_flex, v_flex};

use crate::{
    assets::MdIcon,
    pages,
    route::{History, Route},
    snackbar::{self, Snackbar},
    theme::{self, Tone},
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
}

impl QuadrantApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            history: History::new(Route::Modpacks),
            pages: HashMap::new(),
            pressed_title_bar: false,
            snackbar: snackbar::view(cx),
        }
    }

    pub fn navigate(&mut self, route: Route, cx: &mut Context<Self>) {
        if self.history.navigate(route) {
            cx.notify();
        }
    }

    fn step(&mut self, direction: NavigationDirection, cx: &mut Context<Self>) {
        let moved = match direction {
            NavigationDirection::Back => self.history.back(),
            NavigationDirection::Forward => self.history.forward(),
        };
        if moved.is_some() {
            cx.notify();
        }
    }

    pub fn page(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) -> AnyView {
        self.pages
            .entry(route)
            .or_insert_with(|| pages::build(route, window, cx))
            .clone()
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
                            .on_click(cx.listener(move |this, _, _, cx| this.navigate(route, cx)))
                    })),
            )
            .child(div().w(px(4.)).h_full().bg(rgb(theme::CONTROL)))
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
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
            .child(theme::title_pill(self.history.current().label()))
            .child(
                h_flex()
                    .p_2()
                    .gap_2()
                    .rounded_full()
                    .bg(rgb(theme::SURFACE))
                    .child(theme::window_button(
                        "notifications",
                        Tone::Control,
                        MdIcon::Notifications,
                        cx,
                    ))
                    .child(
                        theme::window_button("minimize", Tone::Control, MdIcon::Minimize, cx)
                            .on_click(|_, window, _| window.minimize_window()),
                    )
                    .child(
                        theme::window_button("close", Tone::Control, MdIcon::Close, cx)
                            .on_click(|_, _, cx| cx.quit()),
                    ),
            )
    }
}

impl Render for QuadrantApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.page(self.history.current(), window, cx);
        h_flex()
            .id("quadrant")
            .size_full()
            .bg(rgb(theme::PAGE))
            .text_color(rgb(theme::TEXT))
            .font_family(theme::FONT_FAMILY)
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _, _, cx| this.step(NavigationDirection::Back, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, _, _, cx| this.step(NavigationDirection::Forward, cx)),
            )
            .child(self.render_rail(cx))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .overflow_hidden()
                    .child(self.render_title_bar(cx))
                    .child(div().flex_1().overflow_hidden().child(page)),
            )
            .child(self.snackbar.clone())
    }
}
