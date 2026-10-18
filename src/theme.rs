//! Every visual token the app uses, after the desktop app's Tailwind palette
//! (`quadrant/.design-sync/conventions.md`). Views name a role from here and
//! never a raw color.
//!
//! The root font size is the desktop app's 16.8px, and gpui-component's `Root`
//! makes it the window's rem size, so gpui's Tailwind-named helpers (`h_10`,
//! `px_4`, `text_2xl`, ...) measure the same as the Tailwind classes they mirror.

use std::borrow::Cow;

use gpui::{
    App, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled, div, prelude::FluentBuilder as _, px, rgb, rgba,
};
use gpui_component::{
    Icon, Theme, ThemeMode,
    button::{Button, ButtonCustomVariant, ButtonRounded, ButtonVariants as _},
    dialog::Dialog,
};

use crate::{assets::MdIcon, route::Route};

pub const FONT_FAMILY: &str = "Inter";
const FONT_SIZE: f32 = 16.8;

/// slate-900: the window background.
pub const PAGE: u32 = 0x0f172a;
/// slate-800: containers, neutral buttons, rail buttons.
pub const SURFACE: u32 = 0x1e293b;
/// slate-700: inputs, badges, title pill, round buttons, rules.
pub const CONTROL: u32 = 0x334155;
/// slate-600: hovered controls.
pub const CONTROL_HOVER: u32 = 0x475569;
/// slate-50
pub const TEXT: u32 = 0xf8fafc;
/// slate-400
pub const TEXT_MUTED: u32 = 0x94a3b8;
/// slate-200: badge labels.
pub const TEXT_BADGE: u32 = 0xe2e8f0;
const PRIMARY: u32 = 0x2563eb;
const PRIMARY_HOVER: u32 = 0x1d4ed8;
const POSITIVE: u32 = 0x059669;
const POSITIVE_HOVER: u32 = 0x047857;
/// emerald-900: an action already in effect, e.g. "Applied".
const POSITIVE_DONE: u32 = 0x064e3b;
const DESTRUCTIVE_HOVER: u32 = 0xb91c1c;
/// red-800
const DANGER_HOVER: u32 = 0x991b1b;
/// slate-950 at 30%, behind dialogs.
const OVERLAY: u32 = 0x0206174d;

/// Tailwind's `rounded-4xl`, the desktop app's card radius.
pub fn radius_card() -> gpui::Pixels {
    px(2. * FONT_SIZE)
}

/// Loads Inter and points gpui-component's theme at the palette above.
pub fn install(cx: &mut App) {
    let inter = include_bytes!("../assets/fonts/Inter.ttf");
    if let Err(error) = cx.text_system().add_fonts(vec![Cow::Borrowed(inter)]) {
        log::warn!("Falling back to the system font, Inter failed to load: {error:#}");
    }
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = FONT_FAMILY.into();
        theme.font_size = px(FONT_SIZE);
        theme.radius = px(FONT_SIZE);
        theme.radius_lg = radius_card();
        theme.shadow = false;
        let colors = &mut theme.colors;
        colors.background = rgb(PAGE).into();
        colors.foreground = rgb(TEXT).into();
        colors.muted = rgb(SURFACE).into();
        colors.muted_foreground = rgb(TEXT_MUTED).into();
        colors.border = rgb(CONTROL).into();
        colors.input = rgb(CONTROL).into();
        colors.ring = rgb(CONTROL_HOVER).into();
        colors.popover = rgb(SURFACE).into();
        colors.popover_foreground = rgb(TEXT).into();
        colors.list = rgb(SURFACE).into();
        colors.list_hover = rgb(CONTROL).into();
        colors.list_active = rgb(CONTROL).into();
        colors.accent = rgb(CONTROL).into();
        colors.accent_foreground = rgb(TEXT).into();
        colors.selection = rgb(PRIMARY).into();
        colors.overlay = rgba(OVERLAY).into();
        colors.title_bar = rgb(PAGE).into();
        colors.window_border = rgb(CONTROL).into();
        colors.scrollbar_thumb = rgb(CONTROL_HOVER).into();
    });
}

/// The desktop app's button colorways.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// blue-600: share, update.
    Primary,
    /// emerald-600: apply, create, sync.
    Positive,
    /// emerald-900: an action already in effect.
    Done,
    /// slate-800 on a card: export, details.
    Neutral,
    /// slate-800 that turns red-700 on hover: delete.
    Destructive,
    /// red-700: confirming a deletion.
    Danger,
    /// slate-700 round toolbar and window buttons.
    Control,
}

impl Tone {
    fn colors(self) -> (u32, u32) {
        match self {
            Self::Primary => (PRIMARY, PRIMARY_HOVER),
            Self::Positive => (POSITIVE, POSITIVE_HOVER),
            Self::Done => (POSITIVE_DONE, POSITIVE_DONE),
            Self::Neutral => (SURFACE, CONTROL),
            Self::Destructive => (SURFACE, DESTRUCTIVE_HOVER),
            Self::Danger => (DESTRUCTIVE_HOVER, DANGER_HOVER),
            Self::Control => (CONTROL, CONTROL_HOVER),
        }
    }
}

/// A fully rounded button in `color` that turns `hover` under the pointer.
fn filled(id: impl Into<ElementId>, color: u32, hover: u32, cx: &App) -> Button {
    let variant = ButtonCustomVariant::new(cx)
        .color(rgb(color).into())
        .hover(rgb(hover).into())
        .active(rgb(hover).into())
        .foreground(rgb(TEXT).into());
    Button::new(id)
        .custom(variant)
        .rounded(ButtonRounded::Size(px(999.)))
        // A custom variant paints its resting color at 20% strength; the
        // explicit background restores the solid fill, and hover still wins.
        .bg(rgb(color))
}

/// An `h-10` extrabold pill with the label first and the icon after it, as
/// every action on a modpack card is drawn.
pub fn pill_button(
    id: impl Into<ElementId>,
    tone: Tone,
    label: impl Into<SharedString>,
    icon: Option<MdIcon>,
    cx: &App,
) -> Button {
    let (color, hover) = tone.colors();
    filled(id, color, hover, cx)
        .h_10()
        .px_4()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(
            gpui_component::h_flex()
                .gap_2()
                .items_center()
                .child(label.into())
                .when_some(icon, |this, icon| this.child(Icon::new(icon).size_5())),
        )
}

/// A `w-10 h-10` round icon button, as in the modpack toolbar.
pub fn toolbar_button(id: impl Into<ElementId>, tone: Tone, icon: MdIcon, cx: &App) -> Button {
    let (color, hover) = tone.colors();
    filled(id, color, hover, cx)
        .size_10()
        .child(Icon::new(icon).size_5())
}

/// A `p-2` round icon button, as in the window controls.
pub fn window_button(id: impl Into<ElementId>, tone: Tone, icon: MdIcon, cx: &App) -> Button {
    let (color, hover) = tone.colors();
    filled(id, color, hover, cx)
        .size_8()
        .child(Icon::new(icon).size_4())
}

/// A rail button: slate-800, slate-700 when it is the open page, and the
/// page's accent on hover.
pub fn rail_button(route: Route, active: bool, cx: &App) -> Button {
    let accent = match route {
        Route::Modpacks => 0x34d399,
        Route::CurrentModpack => 0x60a5fa,
        Route::Search => 0x38bdf8,
        Route::InstalledContent => 0xa78bfa,
        Route::Account => 0xfb923c,
        Route::Settings => TEXT_MUTED,
    };
    let base = if active { CONTROL } else { SURFACE };
    filled(route.id(), base, accent, cx)
        .size_12()
        .child(Icon::new(route.icon()).size_6())
}

/// The small `rounded-full` label next to a modpack name.
pub fn badge(label: impl Into<SharedString>, icon: MdIcon) -> impl IntoElement {
    gpui_component::h_flex()
        .gap_1()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(rgb(CONTROL))
        .text_color(rgb(TEXT_BADGE))
        .text_size(px(11.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(Icon::new(icon).size_3())
        .child(label.into())
}

/// A pill showing the open page's title, top left of the title bar.
pub fn title_pill(title: impl Into<SharedString>) -> impl IntoElement {
    div()
        .id("page-title")
        .px_4()
        .py_1p5()
        .rounded_full()
        .bg(rgb(CONTROL))
        .text_lg()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(title.into())
}

/// The desktop app's `CancelButton`: a back arrow before the label.
pub fn cancel_button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    let (color, hover) = Tone::Neutral.colors();
    filled(id, color, hover, cx)
        .h_10()
        .px_4()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(
            gpui_component::h_flex()
                .gap_1()
                .items_center()
                .child(Icon::new(MdIcon::ArrowBack).size_6())
                .child(label.into()),
        )
}

/// A dialog heading: `font-black text-xl`.
pub fn dialog_title(title: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_xl()
        .font_weight(FontWeight::BLACK)
        .child(title.into())
}

/// The desktop app's dialog panel: `rounded-4xl bg-slate-800 p-8`, no border.
pub fn dialog_surface(dialog: Dialog) -> Dialog {
    dialog.bg(rgb(SURFACE)).border_0().p_8()
}
