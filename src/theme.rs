//! Every visual token the app uses, after the desktop app's Tailwind palette
//! (`quadrant/.design-sync/conventions.md`). Views name a role from here and
//! never a raw color.
//!
//! The root font size is the desktop app's 16.8px, and gpui-component's `Root`
//! makes it the window's rem size, so gpui's Tailwind-named helpers (`h_10`,
//! `px_4`, `text_2xl`, ...) measure the same as the Tailwind classes they mirror.

use std::borrow::Cow;

use gpui::{
    App, Div, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Rgba, SharedString, Styled, div, prelude::FluentBuilder as _, px, rgb, rgba,
};
use gpui_component::{
    Icon, Theme, ThemeMode,
    button::{Button, ButtonCustomVariant, ButtonRounded, ButtonVariants as _},
    dialog::Dialog,
};

use crate::{assets::MdIcon, route::Route, snackbar::SnackbarKind};

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
/// red-700
const DESTRUCTIVE_HOVER: u32 = 0xb91c1c;
/// red-800
const DANGER_HOVER: u32 = 0x991b1b;
/// slate-950 at 30%, behind dialogs.
const OVERLAY: u32 = 0x0206174d;
/// slate-900 at 50%: a button on a colored surface, e.g. the snackbar's clear.
const SCRIM: u32 = 0x0f172a80;
/// slate-900 at 70%
const SCRIM_HOVER: u32 = 0x0f172ab3;

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
    /// Translucent slate-900 over a colored surface: the snackbar's clear.
    Scrim,
}

impl Tone {
    fn colors(self) -> (Rgba, Rgba) {
        let (color, hover) = match self {
            Self::Primary => (PRIMARY, PRIMARY_HOVER),
            Self::Positive => (POSITIVE, POSITIVE_HOVER),
            Self::Done => (POSITIVE_DONE, POSITIVE_DONE),
            Self::Neutral => (SURFACE, CONTROL),
            Self::Destructive => (SURFACE, DESTRUCTIVE_HOVER),
            Self::Danger => (DESTRUCTIVE_HOVER, DANGER_HOVER),
            Self::Control => (CONTROL, CONTROL_HOVER),
            Self::Scrim => return (rgba(SCRIM), rgba(SCRIM_HOVER)),
        };
        (rgb(color), rgb(hover))
    }
}

/// A fully rounded button in `color` that turns `hover` under the pointer.
fn filled(id: impl Into<ElementId>, color: Rgba, hover: Rgba, cx: &App) -> Button {
    let variant = ButtonCustomVariant::new(cx)
        .color(color.into())
        .hover(hover.into())
        .active(hover.into())
        .foreground(rgb(TEXT).into());
    Button::new(id)
        .custom(variant)
        .rounded(ButtonRounded::Size(px(999.)))
        // A custom variant paints its resting color at 20% strength; the
        // explicit background restores the solid fill, and hover still wins.
        .bg(color)
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
    filled(route.id(), rgb(base), rgb(accent), cx)
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

fn snackbar_color(kind: SnackbarKind) -> u32 {
    match kind {
        SnackbarKind::Neutral => CONTROL,
        SnackbarKind::Success => POSITIVE,
        SnackbarKind::Error => DESTRUCTIVE_HOVER,
    }
}

/// The desktop app's snackbar: `p-4 rounded-4xl font-bold text-slate-50` on
/// the kind's color.
pub fn snackbar_surface(kind: SnackbarKind) -> Div {
    div()
        .p_4()
        .rounded(radius_card())
        .bg(rgb(snackbar_color(kind)))
        .text_color(rgb(TEXT))
        .font_weight(FontWeight::BOLD)
}

// Search and mod pages

/// slate-300: unpicked option labels.
pub const TEXT_SOFT: u32 = 0xcbd5e1;
/// slate-500: hints and secondary labels.
pub const TEXT_SUBTLE: u32 = 0x64748b;
/// slate-600: disabled labels.
pub const TEXT_DISABLED: u32 = 0x475569;
/// blue-600: a picked option, the filter count.
pub const SELECTED: u32 = PRIMARY;
/// white at 10%: rules between filter sections.
pub const RULE: u32 = 0xffffff1a;
/// slate-100: a hovered quiet icon button.
pub const TEXT_BRIGHT: u32 = 0xf1f5f9;
/// red-400: a hovered "Clear all".
pub const TEXT_DANGER: u32 = 0xf87171;
/// white at 5%: a hovered filter option.
pub const OPTION_HOVER: u32 = 0xffffff0d;
/// red-900 with red-200 text: a hovered filter chip, which removes it.
pub const CHIP_REMOVE: u32 = 0x7f1d1d;
pub const CHIP_REMOVE_TEXT: u32 = 0xfecaca;
/// red-700: a failed search.
pub const ERROR_FILL: u32 = DESTRUCTIVE_HOVER;
/// slate-950: a hovered mod card.
pub const CARD_HOVER: u32 = 0x020617;
/// emerald-600: the "search further" arrow.
pub const POSITIVE_FILL: u32 = POSITIVE;
/// The providers' brand dots.
pub const CURSEFORGE_DOT: u32 = 0xf16436;
pub const MODRINTH_DOT: u32 = 0x1bd96a;
/// amber-300 on amber-900/25 with an amber-700/20 border: notes that warn.
const WARNING_TEXT: u32 = 0xfcd34d;
const WARNING_FILL: u32 = 0x78350f40;
const WARNING_BORDER: u32 = 0xb4530933;
/// sky-700 and sky-600: the Search button.
const SEARCH: u32 = 0x0369a1;
const SEARCH_HOVER: u32 = 0x0284c7;

/// The Search button: sky-700, the search icon before the label.
pub fn search_button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    filled(id, rgb(SEARCH), rgb(SEARCH_HOVER), cx)
        .h_10()
        .px_5()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(
            gpui_component::h_flex()
                .gap_2()
                .items_center()
                .text_sm()
                .child(Icon::new(MdIcon::Search).size(px(18.9)))
                .child(label.into()),
        )
}

/// An `h-8` option pill: blue-600 when picked, slate-700 otherwise.
pub fn option_pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<MdIcon>,
    picked: bool,
    cx: &App,
) -> Button {
    let tone = if picked { Tone::Primary } else { Tone::Control };
    let (color, hover) = tone.colors();
    filled(id, color, hover, cx)
        .h_8()
        .px_3()
        .font_weight(FontWeight::BOLD)
        .when(!picked, |this| this.text_color(rgb(TEXT_BADGE)))
        .child(
            gpui_component::h_flex()
                .gap_1p5()
                .items_center()
                .text_size(px(13.))
                .when_some(icon, |this, icon| this.child(Icon::new(icon).size_4()))
                .child(label.into()),
        )
}

/// A source toggle: a brand dot and the provider's name, outlined blue
/// while it is searched and dashed when something rules it out.
pub fn source_toggle(
    id: impl Into<ElementId>,
    label: &'static str,
    dot: u32,
    active: bool,
    disabled: bool,
) -> gpui::Stateful<gpui::Div> {
    let base = div()
        .id(id)
        .flex_1()
        .h(px(2.625 * FONT_SIZE))
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded_full()
        .text_size(px(13.5))
        .font_weight(FontWeight::EXTRA_BOLD);
    let styled = if disabled {
        base.bg(rgb(SURFACE))
            .border_1()
            .border_dashed()
            .border_color(rgb(CONTROL))
            .text_color(rgb(TEXT_DISABLED))
    } else if active {
        base.bg(rgb(CONTROL))
            .border_2()
            .border_color(rgb(PRIMARY))
            .cursor_pointer()
    } else {
        base.bg(rgb(SURFACE))
            .border_2()
            .border_color(rgba(0))
            .text_color(rgb(TEXT_SUBTLE))
            .cursor_pointer()
    };
    styled
        .child(div().size_2p5().flex_none().rounded_full().bg(rgb(dot)))
        .child(label)
}

/// A filter section's heading.
pub fn filter_label(label: impl Into<SharedString>) -> impl IntoElement {
    div()
        .mb_2p5()
        .text_size(px(13.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .text_color(rgb(TEXT_MUTED))
        .child(label.into())
}

/// A filter checkbox: a `rounded-[7px]` square, filled blue with a check
/// when picked.
pub fn check_box(picked: bool, disabled: bool) -> impl IntoElement {
    let border = if disabled {
        CONTROL
    } else if picked {
        PRIMARY
    } else {
        TEXT_SUBTLE
    };
    div()
        .flex_none()
        .size_5()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .border_1()
        .border_color(rgb(border))
        .when(picked && !disabled, |this| {
            this.bg(rgb(PRIMARY))
                .child(Icon::new(MdIcon::Check).size_3().text_color(rgb(TEXT)))
        })
}

/// An on/off switch: a white knob on slate-700, or on blue-600 when on.
pub fn switch(on: bool) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(2.625 * FONT_SIZE))
        .h_6()
        .p_0p5()
        .rounded_full()
        .bg(rgb(if on { PRIMARY } else { CONTROL }))
        .flex()
        .when(on, |this| this.justify_end())
        .child(div().size_5().rounded_full().bg(rgb(0xffffff)))
}

/// The amber note the desktop app uses for "already installed" and the
/// source lock.
pub fn warning_note(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .px_3()
        .py_2()
        .rounded_2xl()
        .border_1()
        .border_color(rgba(WARNING_BORDER))
        .bg(rgba(WARNING_FILL))
        .text_color(rgb(WARNING_TEXT))
        .text_size(px(11.5))
        .child(text.into())
}

/// The provider pill on a mod card: orange for CurseForge, green otherwise.
pub fn source_pill(label: impl Into<SharedString>, curseforge: bool) -> gpui::Div {
    meta_pill(curseforge).child(label.into())
}

/// The green "Installed" pill on a mod card, its check before the label.
pub fn installed_pill(label: impl Into<SharedString>) -> gpui::Div {
    meta_pill(false)
        .child(Icon::new(MdIcon::Check).size_3())
        .child(label.into())
}

fn meta_pill(curseforge: bool) -> gpui::Div {
    let (fill, text) = if curseforge {
        (0xf9731633, 0xfb923c)
    } else {
        (0x10b98126, 0x34d399)
    };
    div()
        .flex()
        .gap_1()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(rgba(fill))
        .text_color(rgb(text))
        .text_size(px(11.))
        .font_weight(FontWeight::EXTRA_BOLD)
}

/// An `h-8` action on a mod card, the icon after the label.
pub fn card_button(
    id: impl Into<ElementId>,
    tone: Tone,
    label: impl Into<SharedString>,
    icon: MdIcon,
    cx: &App,
) -> Button {
    let (color, hover) = tone.colors();
    filled(id, color, hover, cx)
        .h_8()
        .px_5()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(
            gpui_component::h_flex()
                .gap_1p5()
                .items_center()
                .text_xs()
                .child(label.into())
                .child(Icon::new(icon).size_5()),
        )
}

/// The round "open in the web" button on a mod card.
pub fn card_icon_button(id: impl Into<ElementId>, icon: MdIcon, cx: &App) -> Button {
    filled(id, rgb(SURFACE), rgb(CONTROL), cx)
        .size(px(38.))
        .text_color(rgb(TEXT_SOFT))
        .child(Icon::new(icon).size_4())
}

/// A wide `h-10` action, the icon before the label, as on the mod install
/// panel.
pub fn wide_button(
    id: impl Into<ElementId>,
    tone: Tone,
    label: impl Into<SharedString>,
    icon: MdIcon,
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
                .text_sm()
                .child(Icon::new(icon).size_5())
                .child(label.into()),
        )
}

/// A mod's icon, round, over the desktop app's gradient letter tile, which
/// also stands in while it loads and when it has none.
pub fn mod_icon(url: &str, name: &str, size: gpui::Pixels) -> gpui::AnyElement {
    let letter: SharedString = name
        .trim()
        .chars()
        .next()
        .map_or("?".into(), |first| first.to_uppercase().to_string())
        .into();
    let hue = name
        .chars()
        .fold(0u32, |hue, char| (hue * 31 + char as u32) % 360) as f32;
    let tile = move || {
        div()
            .size(size)
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui::linear_gradient(
                135.,
                gpui::linear_color_stop(gpui::hsla(hue / 360., 0.58, 0.50, 1.), 0.),
                gpui::linear_color_stop(
                    gpui::hsla(((hue + 45.) % 360.) / 360., 0.55, 0.38, 1.),
                    1.,
                ),
            ))
            .text_2xl()
            .font_weight(FontWeight::EXTRA_BOLD)
            .child(letter.clone())
            .into_any_element()
    };
    if url.trim().is_empty() {
        return tile();
    }
    let loading = tile.clone();
    use gpui::StyledImage as _;
    gpui::img(SharedString::from(url.to_string()))
        .size(size)
        .flex_none()
        .rounded_full()
        .object_fit(gpui::ObjectFit::Cover)
        .with_loading(loading)
        .with_fallback(tile)
        .into_any_element()
}

/// The desktop app's `LinearProgress`: a blue bar labelled with its percent.
pub fn progress_bar(percent: i32) -> impl IntoElement {
    let percent = percent.clamp(0, 100);
    div().w_full().rounded_full().bg(rgb(CONTROL)).child(
        div()
            .w(gpui::relative(percent as f32 / 100.))
            .p_0p5()
            .rounded_full()
            .bg(rgb(if percent == 0 { SURFACE } else { PRIMARY }))
            .text_xs()
            .line_height(px(12.6))
            .font_weight(FontWeight::MEDIUM)
            .text_center()
            .child(format!("{percent}%")),
    )
}

/// A `rounded-[28px]` slate-800 panel, as the filters, results and install
/// panels are drawn.
pub fn panel() -> gpui::Div {
    div().rounded(px(28.)).bg(rgb(SURFACE))
}

// Content, settings and account pages

/// emerald-800: a switched-on track.
const SWITCH_ON: u32 = 0x065f46;
/// emerald-700: a hovered switched-on track.
const SWITCH_ON_HOVER: u32 = 0x047857;
/// sky-500 at 20% and sky-300: the Cloud badge.
const CLOUD_BADGE: (u32, u32) = (0x0ea5e933, 0x7dd3fc);
/// emerald-500 at 15% and emerald-400: the Synced badge.
const SYNCED_BADGE: (u32, u32) = (0x10b98126, 0x34d399);
/// red-950 at 60%, red-400 at 40% and red-100: a failure the page keeps showing.
const WARNING: (u32, u32, u32) = (0x450a0a99, 0xf8717166, 0xfee2e2);

/// The desktop app's Headless UI switch: an `h-8 w-16` slate-700 track that
/// turns emerald-800 when on, with a `size-8` slate-300 thumb.
pub fn setting_switch(id: impl Into<ElementId>, on: bool) -> gpui::Stateful<gpui::Div> {
    let (track, hover) = if on {
        (SWITCH_ON, SWITCH_ON_HOVER)
    } else {
        (CONTROL, CONTROL_HOVER)
    };
    div()
        .id(id)
        .flex_shrink_0()
        .h_8()
        .w_16()
        .rounded_full()
        .bg(rgb(track))
        .hover(|style| style.bg(rgb(hover)))
        .cursor_pointer()
        .child(
            div()
                .size_8()
                .rounded_full()
                .bg(rgb(TEXT_SOFT))
                .when(on, |thumb| thumb.ml_8()),
        )
}

/// A `w-5 h-5` checkbox with the desktop app's emerald-600 accent.
pub fn checkbox(id: impl Into<ElementId>, checked: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .size_5()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .cursor_pointer()
        .map(|this| {
            if checked {
                this.bg(rgb(POSITIVE))
                    .child(Icon::new(MdIcon::Check).size_4().text_color(rgb(TEXT)))
            } else {
                this.bg(rgb(TEXT)).border_1().border_color(rgb(TEXT_MUTED))
            }
        })
}

/// The Cloud badge of a modpack that only exists in Quadrant Sync.
pub fn cloud_badge(label: impl Into<SharedString>) -> impl IntoElement {
    tinted_badge(label, MdIcon::Cloud, CLOUD_BADGE)
}

/// The Synced badge of a modpack with a Quadrant Sync record.
pub fn synced_badge(label: impl Into<SharedString>) -> impl IntoElement {
    tinted_badge(label, MdIcon::CloudDone, SYNCED_BADGE)
}

fn tinted_badge(
    label: impl Into<SharedString>,
    icon: MdIcon,
    (background, foreground): (u32, u32),
) -> impl IntoElement {
    gpui_component::h_flex()
        .gap_1()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(rgba(background))
        .text_color(rgb(foreground))
        .text_size(px(11.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(Icon::new(icon).size_3())
        .child(label.into())
}

/// A failure the page keeps on screen, like the Account page's sign-in error.
pub fn warning_box(text: impl Into<SharedString>) -> impl IntoElement {
    let (background, border, foreground) = WARNING;
    div()
        .w_full()
        .p_4()
        .rounded(radius_card())
        .border_1()
        .border_color(rgba(border))
        .bg(rgba(background))
        .text_color(rgb(foreground))
        .font_weight(FontWeight::BOLD)
        .child(text.into())
}

/// `pill_button` with a `text-sm` label, as on the Installed content page.
pub fn small_pill_button(
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
                .text_sm()
                .child(label.into())
                .when_some(icon, |this, icon| this.child(Icon::new(icon).size_5())),
        )
}

/// slate-700 that turns red-700 on hover: kicking a Quadrant Sync member.
pub fn kick_button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    filled(id, rgb(CONTROL), rgb(DESTRUCTIVE_HOVER), cx)
        .h_10()
        .px_4()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(label.into())
}

/// A full-width button whose label wraps, like the Account page's terms of
/// service notice. gpui-component's `Button` keeps its label on one line.
pub fn block_button(
    id: impl Into<ElementId>,
    tone: Tone,
    label: impl Into<SharedString>,
    icon: Option<MdIcon>,
) -> gpui::Stateful<Div> {
    let (color, hover) = tone.colors();
    gpui_component::h_flex()
        .id(id)
        .w_full()
        .p_2()
        .gap_2()
        .items_center()
        .rounded(radius_card())
        .bg(color)
        .hover(move |style| style.bg(hover))
        .cursor_pointer()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(div().flex_1().min_w_0().text_center().child(label.into()))
        .when_some(icon, |this, icon| {
            this.child(Icon::new(icon).flex_shrink_0().size_6())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snackbar_kinds_use_the_desktop_apps_colors() {
        // bg-slate-700, bg-emerald-600, bg-red-700
        assert_eq!(snackbar_color(SnackbarKind::Neutral), 0x334155);
        assert_eq!(snackbar_color(SnackbarKind::Success), 0x059669);
        assert_eq!(snackbar_color(SnackbarKind::Error), 0xb91c1c);
    }
}
