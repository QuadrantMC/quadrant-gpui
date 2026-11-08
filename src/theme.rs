//! Every visual token the app uses, after the desktop app's Tailwind palette
//! (`quadrant/.design-sync/conventions.md`). Views name a role from here and
//! never a raw color.
//!
//! The root font size is the desktop app's 16.8px, and gpui-component's `Root`
//! makes it the window's rem size, so gpui's Tailwind-named helpers (`h_10`,
//! `px_4`, `text_2xl`, ...) measure the same as the Tailwind classes they mirror.

use std::{
    borrow::Cow,
    sync::{
        PoisonError, RwLock,
        atomic::{AtomicU32, Ordering},
    },
};

use gpui::{
    App, Div, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Rgba, SharedString, Styled, div, prelude::FluentBuilder as _, px, rems, rgb,
};
use gpui_component::{
    Icon, Theme, ThemeMode,
    button::{Button, ButtonCustomVariant, ButtonRounded, ButtonVariants as _},
    dialog::Dialog,
};

use crate::{assets::MdIcon, motion, route::Route, snackbar::SnackbarKind};

pub const FONT_FAMILY: &str = "Inter";
/// The root font size at 100% UI scale.
pub const FONT_SIZE: f32 = 16.8;

/// The UI scale in percent, as [`set_ui_scale`] last set it.
static UI_SCALE: AtomicU32 = AtomicU32::new(100);

/// The window's rem: [`FONT_SIZE`] at the UI scale. For lengths that are
/// computed rather than styled; a style takes `rems` as it is.
pub fn rem() -> gpui::Pixels {
    px(FONT_SIZE * UI_SCALE.load(Ordering::Relaxed) as f32 / 100.)
}

/// Scales the whole UI, as the desktop app's `applyUiScale` sets the root
/// font size every rem derives from. gpui-component's `Root` makes the
/// theme's font size the window's rem size.
pub fn set_ui_scale(percent: u32, cx: &mut App) {
    UI_SCALE.store(percent, Ordering::Relaxed);
    let rem = rem();
    Theme::update(cx, |theme| {
        theme.font_size = rem;
        theme.radius = rem;
        theme.radius_lg = rem * 2.;
    });
    cx.refresh_windows();
}

/// One color per role the views name. [`QUADRANT`] is the desktop app's
/// Tailwind slate palette; a color theme swaps the whole set at once.
macro_rules! palette {
    ($($(#[doc = $doc:literal])* $field:ident: $value:expr,)*) => {
        #[derive(Clone, Copy, PartialEq, Debug)]
        pub struct Palette {
            $($(#[doc = $doc])* pub $field: Rgba,)*
        }

        impl Palette {
            /// Every field's name, in declaration order.
            pub const FIELDS: &[&str] = &[$(stringify!($field)),*];

            pub fn field(&self, name: &str) -> Option<Rgba> {
                match name {
                    $(stringify!($field) => Some(self.$field),)*
                    _ => None,
                }
            }

            pub fn field_mut(&mut self, name: &str) -> Option<&mut Rgba> {
                match name {
                    $(stringify!($field) => Some(&mut self.$field),)*
                    _ => None,
                }
            }
        }

        /// The default theme, the desktop app's look.
        pub const QUADRANT: Palette = Palette {
            $($field: $value,)*
        };
    };
}

/// `gpui::rgb` as a `const fn`, the same arithmetic.
const fn hex(value: u32) -> Rgba {
    hexa(value << 8 | 0xff)
}

/// `gpui::rgba` as a `const fn`.
const fn hexa(value: u32) -> Rgba {
    let [r, g, b, a] = value.to_be_bytes();
    Rgba {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a: a as f32 / 255.0,
    }
}

palette! {
    /// slate-900: the window background, and `CircularProgress.tsx`'s ring.
    page: hex(0x0f172a),
    /// slate-800: containers, neutral buttons, rail buttons.
    surface: hex(0x1e293b),
    /// slate-700: inputs, badges, title pill, round buttons, rules, and
    /// `CircularProgress.tsx`'s turning top.
    control: hex(0x334155),
    /// slate-600: hovered controls.
    control_hover: hex(0x475569),
    /// slate-50
    text: hex(0xf8fafc),
    /// slate-400
    text_muted: hex(0x94a3b8),
    /// slate-200: badge labels.
    text_badge: hex(0xe2e8f0),
    /// slate-300: unpicked option labels.
    text_soft: hex(0xcbd5e1),
    /// slate-500: hints and secondary labels.
    text_subtle: hex(0x64748b),
    /// slate-600: disabled labels.
    text_disabled: hex(0x475569),
    /// slate-100: a hovered quiet icon button.
    text_bright: hex(0xf1f5f9),
    /// red-400: a hovered "Clear all".
    text_danger: hex(0xf87171),
    /// slate-50: labels on the colored fills (primary, positive, danger), which
    /// a light theme keeps light while its `text` turns dark.
    on_accent: hex(0xf8fafc),
    /// blue-600: share, update, a picked option, the filter count.
    primary: hex(0x2563eb),
    primary_hover: hex(0x1d4ed8),
    /// emerald-600: apply, create, sync, the "search further" arrow.
    positive: hex(0x059669),
    positive_hover: hex(0x047857),
    /// emerald-900: an action already in effect, e.g. "Applied".
    positive_done: hex(0x064e3b),
    /// red-700: a hovered delete, a confirmed deletion, a failed search.
    destructive_hover: hex(0xb91c1c),
    /// red-800
    danger_hover: hex(0x991b1b),
    /// red-600: the hovered bell while a notification is unread.
    alert_hover: hex(0xdc2626),
    /// slate-950 at 30%, behind dialogs.
    overlay: hexa(0x0206174d),
    /// slate-900 at 50%: a button on a colored surface, e.g. the snackbar's
    /// clear, and a repeat count on a remembered snackbar.
    scrim: hexa(0x0f172a80),
    /// slate-900 at 70%
    scrim_hover: hexa(0x0f172ab3),
    /// slate-900 at 15%: behind the notifications panel.
    backdrop: hexa(0x0f172a26),
    /// white at 10%: rules between filter sections.
    rule: hexa(0xffffff1a),
    /// white at 5%: a hovered filter option.
    option_hover: hexa(0xffffff0d),
    /// red-900 with red-200 text: a hovered filter chip, which removes it.
    chip_remove: hex(0x7f1d1d),
    chip_remove_text: hex(0xfecaca),
    /// slate-950: a hovered mod card.
    card_hover: hex(0x020617),
    /// amber-300 on amber-900/25 with an amber-700/20 border: notes that warn.
    warning_text: hex(0xfcd34d),
    warning_fill: hexa(0x78350f40),
    warning_border: hexa(0xb4530933),
    /// sky-700 and sky-600: the Search button.
    search: hex(0x0369a1),
    search_hover: hex(0x0284c7),
    /// slate-600 and emerald-400: the small spinner's ring and its top.
    spinner_ring: hex(0x475569),
    spinner_arc: hex(0x34d399),
    /// emerald-800: a switched-on track.
    switch_on: hex(0x065f46),
    /// emerald-700: a hovered switched-on track.
    switch_on_hover: hex(0x047857),
    /// sky-500 at 20% and sky-300: the Cloud badge.
    cloud_badge_fill: hexa(0x0ea5e933),
    cloud_badge_text: hex(0x7dd3fc),
    /// emerald-500 at 15% and emerald-400: the Synced badge.
    synced_badge_fill: hexa(0x10b98126),
    synced_badge_text: hex(0x34d399),
    /// red-950 at 60%, red-400 at 40% and red-100: a failure the page keeps
    /// showing.
    warning_box_fill: hexa(0x450a0a99),
    warning_box_border: hexa(0xf8717166),
    warning_box_text: hex(0xfee2e2),
    /// orange-500 at 20% and orange-400: the CurseForge pill on a mod card.
    curseforge_pill_fill: hexa(0xf9731633),
    curseforge_pill_text: hex(0xfb923c),
    /// emerald-500 at 15% and emerald-400: the Modrinth and Installed pills.
    modrinth_pill_fill: hexa(0x10b98126),
    modrinth_pill_text: hex(0x34d399),
    /// The rail's hover accents, one per page; Settings takes `text_muted`.
    rail_modpacks: hex(0x34d399),
    rail_current_modpack: hex(0x60a5fa),
    rail_search: hex(0x38bdf8),
    rail_installed_content: hex(0xa78bfa),
    rail_account: hex(0xfb923c),
}

/// The palette views draw with, as [`set_palette`] last set it.
/// A reference, not the palette itself: views read it for every color they
/// draw, and copying all of it each time would cost more than the read.
static PALETTE: RwLock<&'static Palette> = RwLock::new(&QUADRANT);

pub fn palette() -> &'static Palette {
    *PALETTE.read().unwrap_or_else(PoisonError::into_inner)
}

/// Switches every window to `palette`, with gpui-component's own widgets
/// (selects, inputs, dialogs, scrollbars) in `mode`.
pub fn set_palette(palette: Palette, mode: ThemeMode, cx: &mut App) {
    // Leaked once per switch the user makes; `Prefs` calls this only when
    // the palette changes.
    *PALETTE.write().unwrap_or_else(PoisonError::into_inner) = Box::leak(Box::new(palette));
    Theme::change(mode, None, cx);
    apply_component_theme(cx);
    cx.refresh_windows();
}

/// Tailwind's `rounded-4xl`, the desktop app's card radius.
pub fn radius_card() -> gpui::Rems {
    rems(2.)
}

/// Loads Inter and points gpui-component's theme at the palette.
pub fn install(cx: &mut App) {
    let inter = include_bytes!("../assets/fonts/Inter.ttf");
    if let Err(error) = cx.text_system().add_fonts(vec![Cow::Borrowed(inter)]) {
        log::warn!("Falling back to the system font, Inter failed to load: {error:#}");
    }
    Theme::change(ThemeMode::Dark, None, cx);
    apply_component_theme(cx);
}

/// gpui-component's fonts, shapes and colors after ours. `Theme::change`
/// loads its own registered theme, so this follows each change of mode.
fn apply_component_theme(cx: &mut App) {
    let p = palette();
    Theme::update(cx, |theme| {
        theme.font_family = FONT_FAMILY.into();
        theme.font_size = rem();
        theme.radius = rem();
        theme.radius_lg = rem() * 2.;
        theme.shadow = false;
        let colors = &mut theme.colors;
        colors.background = p.page.into();
        colors.foreground = p.text.into();
        colors.muted = p.surface.into();
        colors.muted_foreground = p.text_muted.into();
        colors.border = p.control.into();
        colors.input = p.control.into();
        colors.ring = p.control_hover.into();
        colors.popover = p.surface.into();
        colors.popover_foreground = p.text.into();
        colors.list = p.surface.into();
        colors.list_hover = p.control.into();
        colors.list_active = p.control.into();
        colors.accent = p.control.into();
        colors.accent_foreground = p.text.into();
        colors.selection = p.primary.into();
        colors.overlay = p.overlay.into();
        colors.title_bar = p.page.into();
        colors.window_border = p.control.into();
        colors.scrollbar_thumb = p.control_hover.into();
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
    /// red-700 that turns red-600 on hover: the bell with unread notifications.
    Alert,
}

impl Tone {
    /// The resting fill, the hovered fill and the label's color.
    fn colors(self) -> (Rgba, Rgba, Rgba) {
        let p = palette();
        match self {
            Self::Primary => (p.primary, p.primary_hover, p.on_accent),
            Self::Positive => (p.positive, p.positive_hover, p.on_accent),
            Self::Done => (p.positive_done, p.positive_done, p.on_accent),
            Self::Neutral => (p.surface, p.control, p.text),
            Self::Destructive => (p.surface, p.destructive_hover, p.text),
            Self::Danger => (p.destructive_hover, p.danger_hover, p.on_accent),
            Self::Control => (p.control, p.control_hover, p.text),
            Self::Alert => (p.destructive_hover, p.alert_hover, p.on_accent),
            Self::Scrim => (p.scrim, p.scrim_hover, p.on_accent),
        }
    }
}

/// A fully rounded button in `color` that turns `hover` under the pointer,
/// labelled in `label`.
fn filled(id: impl Into<ElementId>, color: Rgba, hover: Rgba, label: Rgba, cx: &App) -> Button {
    let variant = ButtonCustomVariant::new(cx)
        .color(color.into())
        .hover(hover.into())
        .active(hover.into())
        .foreground(label.into());
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
    let (color, hover, label_color) = tone.colors();
    filled(id, color, hover, label_color, cx)
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
    let (color, hover, label_color) = tone.colors();
    filled(id, color, hover, label_color, cx)
        .size_10()
        .child(Icon::new(icon).size_5())
}

/// The desktop app's `Button` around a `w-5 h-5` icon: `p-2`, round, as
/// the settings' UI scale steps are drawn.
pub fn icon_button(id: impl Into<ElementId>, tone: Tone, icon: MdIcon, cx: &App) -> Button {
    let (color, hover, label_color) = tone.colors();
    filled(id, color, hover, label_color, cx)
        .size_9()
        .child(Icon::new(icon).size_5())
}

/// A `p-2` round icon button, as in the window controls.
pub fn window_button(id: impl Into<ElementId>, tone: Tone, icon: MdIcon, cx: &App) -> Button {
    let (color, hover, label_color) = tone.colors();
    filled(id, color, hover, label_color, cx)
        .size_8()
        .child(Icon::new(icon).size_4())
}

/// A rail button: slate-800, slate-700 when it is the open page, and the
/// page's accent on hover.
pub fn rail_button(route: Route, active: bool, cx: &App) -> Button {
    let p = palette();
    let accent = match route {
        Route::Modpacks => p.rail_modpacks,
        Route::CurrentModpack => p.rail_current_modpack,
        Route::Search => p.rail_search,
        Route::InstalledContent => p.rail_installed_content,
        Route::Account => p.rail_account,
        Route::Settings => p.text_muted,
    };
    let base = if active { p.control } else { p.surface };
    filled(route.id(), base, accent, p.text, cx)
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
        .bg(palette().control)
        .text_color(palette().text_badge)
        .text_size(px(11.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(Icon::new(icon).size_3())
        .child(label.into())
}

/// The desktop app's plain `Button` in the notifications panel: `p-2`
/// extrabold, the label first and a small icon after it.
pub fn panel_button(
    id: impl Into<ElementId>,
    tone: Tone,
    label: impl Into<SharedString>,
    icon: MdIcon,
    cx: &App,
) -> Button {
    let (color, hover, label_color) = tone.colors();
    filled(id, color, hover, label_color, cx)
        .p_2()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(
            gpui_component::h_flex()
                .items_center()
                .justify_center()
                .child(label.into())
                .child(Icon::new(icon).size_4().mx_2()),
        )
}

/// A pill showing the open page's title, top left of the title bar.
pub fn title_pill(title: impl Into<SharedString>) -> impl IntoElement {
    div()
        .id("page-title")
        .px_4()
        .py_1p5()
        .rounded_full()
        .bg(palette().control)
        .text_lg()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(title.into())
}

/// The desktop app's `CancelButton`: a back arrow before the label.
pub fn cancel_button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    let (color, hover, label_color) = Tone::Neutral.colors();
    filled(id, color, hover, label_color, cx)
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
    dialog.bg(palette().surface).border_0().p_8()
}

/// A snackbar's fill and its text.
fn snackbar_colors(kind: SnackbarKind, p: &Palette) -> (Rgba, Rgba) {
    match kind {
        SnackbarKind::Neutral => (p.control, p.text),
        SnackbarKind::Success | SnackbarKind::Saved | SnackbarKind::Deleted => {
            (p.positive, p.on_accent)
        }
        SnackbarKind::Error => (p.destructive_hover, p.on_accent),
    }
}

/// The desktop app's snackbar: `p-4 rounded-4xl font-bold text-slate-50` on
/// the kind's color.
pub fn snackbar_surface(kind: SnackbarKind) -> Div {
    let (fill, text) = snackbar_colors(kind, palette());
    div()
        .p_4()
        .rounded(radius_card())
        .bg(fill)
        .text_color(text)
        .font_weight(FontWeight::BOLD)
}

// Search and mod pages

/// The providers' brand dots, the same in every theme.
pub const CURSEFORGE_DOT: u32 = 0xf16436;
pub const MODRINTH_DOT: u32 = 0x1bd96a;

/// The Search button: sky-700, the search icon before the label.
pub fn search_button(id: impl Into<ElementId>, label: impl Into<SharedString>, cx: &App) -> Button {
    let p = palette();
    filled(id, p.search, p.search_hover, p.on_accent, cx)
        .h_10()
        .px_5()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(
            gpui_component::h_flex()
                .gap_2()
                .items_center()
                .text_sm()
                .child(Icon::new(MdIcon::Search).size(rems(1.125)))
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
    let (color, hover, label_color) = tone.colors();
    filled(id, color, hover, label_color, cx)
        .h_8()
        .px_3()
        .font_weight(FontWeight::BOLD)
        .when(!picked, |this| this.text_color(palette().text_badge))
        .child(
            gpui_component::h_flex()
                .gap_1p5()
                .items_center()
                .text_size(px(13.))
                .when_some(icon, |this, icon| this.child(Icon::new(icon).size_4()))
                .child(label.into()),
        )
}

/// [`option_pill`] as a plain element, for Search's many filter options and
/// its pager, drawn on every frame of the page. A `disabled` one neither
/// lights up nor presses; give it no click.
pub fn option_chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<MdIcon>,
    picked: bool,
    disabled: bool,
) -> gpui::Stateful<Div> {
    let tone = if picked { Tone::Primary } else { Tone::Control };
    let (color, hover, label_color) = tone.colors();
    let pill = if disabled {
        resting_light(id, color, label_color)
    } else {
        filled_light(id, color, hover, label_color)
    };
    pill.h_8()
        .px_3()
        .font_weight(FontWeight::BOLD)
        .when(!picked, |this| this.text_color(palette().text_badge))
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
        .h(rems(2.625))
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded_full()
        .text_size(px(13.5))
        .font_weight(FontWeight::EXTRA_BOLD);
    let p = palette();
    let styled = if disabled {
        base.bg(p.surface)
            .border_1()
            .border_dashed()
            .border_color(p.control)
            .text_color(p.text_disabled)
    } else if active {
        base.bg(p.control)
            .border_2()
            .border_color(p.primary)
            .cursor_pointer()
    } else {
        base.bg(p.surface)
            .border_2()
            .border_color(gpui::transparent_black())
            .text_color(p.text_subtle)
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
        .text_color(palette().text_muted)
        .child(label.into())
}

/// A filter checkbox: a `rounded-[7px]` square, filled blue with a check
/// when picked.
pub fn check_box(picked: bool, disabled: bool) -> impl IntoElement {
    let p = palette();
    let border = if disabled {
        p.control
    } else if picked {
        p.primary
    } else {
        p.text_subtle
    };
    div()
        .flex_none()
        .size_5()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .border_1()
        .border_color(border)
        .when(picked && !disabled, |this| {
            this.bg(p.primary)
                .child(Icon::new(MdIcon::Check).size_3().text_color(p.on_accent))
        })
}

/// An on/off switch: a white knob on slate-700, or on blue-600 when on.
pub fn switch(on: bool) -> impl IntoElement {
    div()
        .flex_none()
        .w(rems(2.625))
        .h_6()
        .p_0p5()
        .rounded_full()
        .bg(if on {
            palette().primary
        } else {
            palette().control
        })
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
        .border_color(palette().warning_border)
        .bg(palette().warning_fill)
        .text_color(palette().warning_text)
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
    let p = palette();
    let (fill, text) = if curseforge {
        (p.curseforge_pill_fill, p.curseforge_pill_text)
    } else {
        (p.modrinth_pill_fill, p.modrinth_pill_text)
    };
    div()
        .flex()
        .gap_1()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(fill)
        .text_color(text)
        .text_size(px(11.))
        .font_weight(FontWeight::EXTRA_BOLD)
}

/// [`filled`]'s pill as a plain element: laid out, colored and pressed as
/// the `Button` is, without what a grid of mod cards drawn every frame can
/// do without: a focus handle, made again each time the card is shown, and
/// the variant's colors worked out on every frame.
fn filled_light(
    id: impl Into<ElementId>,
    color: Rgba,
    hover: Rgba,
    label: Rgba,
) -> gpui::Stateful<Div> {
    use gpui::StatefulInteractiveElement as _;
    resting_light(id, color, label)
        .hover(move |this| this.bg(hover).text_color(label))
        .active(move |this| this.bg(hover).text_color(label))
        .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
            // As a `Button`: a press neither moves focus nor selects text.
            window.prevent_default();
            gpui_component::GlobalState::suppress_text_selection(cx);
        })
}

/// [`filled_light`] as a disabled `Button` draws it: no hover, no press.
fn resting_light(id: impl Into<ElementId>, color: Rgba, label: Rgba) -> gpui::Stateful<Div> {
    use gpui::StatefulInteractiveElement as _;
    div()
        .id(id)
        .role(gpui::accesskit::Role::Button)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .cursor_default()
        .whitespace_nowrap()
        .line_height(gpui::relative(1.25))
        .rounded(px(999.))
        .bg(color)
        .text_color(label)
}

/// An `h-8` action on a mod card, the icon after the label.
pub fn card_button(
    id: impl Into<ElementId>,
    tone: Tone,
    label: impl Into<SharedString>,
    icon: MdIcon,
) -> gpui::Stateful<Div> {
    let (color, hover, label_color) = tone.colors();
    filled_light(id, color, hover, label_color)
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
    let p = palette();
    filled(id, p.surface, p.control, p.text, cx)
        .size(px(38.))
        // A fixed `w-[38px]`; the button's own rem padding would squeeze
        // the icon out of it at a larger UI scale.
        .p_0()
        .text_color(p.text_soft)
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
    let (color, hover, label_color) = tone.colors();
    filled(id, color, hover, label_color, cx)
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

/// A mod's icon over the desktop app's gradient letter tile, which also
/// stands in while it loads and when it has none: round with the
/// `clipIcons` setting on, a `rounded-2xl` square with it off.
pub fn mod_icon(
    url: &str,
    name: &str,
    size: gpui::Rems,
    round: bool,
    cx: &App,
) -> gpui::AnyElement {
    fn corners<E: Styled>(this: E, round: bool) -> E {
        if round {
            this.rounded_full()
        } else {
            this.rounded_2xl()
        }
    }
    let first = name.trim().chars().next();
    let hue = name
        .chars()
        .fold(0u32, |hue, char| (hue * 31 + char as u32) % 360) as f32;
    // Only drawn while the icon loads, when it fails or when there is none,
    // so the letter is made only then.
    let tile = move || {
        let letter: SharedString =
            first.map_or("?".into(), |first| first.to_uppercase().to_string().into());
        corners(div(), round)
            .size(size)
            .flex_none()
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
            .child(letter)
            .into_any_element()
    };
    let Some(icon) = crate::remote_images::img(url, cx) else {
        return tile();
    };
    let loading = tile;
    use gpui::StyledImage as _;
    corners(icon, round)
        .size(size)
        .flex_none()
        .object_fit(gpui::ObjectFit::Cover)
        .with_loading(loading)
        .with_fallback(tile)
        .into_any_element()
}

/// The desktop app's `LinearProgress`: a blue bar labelled with its percent.
pub fn progress_bar(percent: i32) -> impl IntoElement {
    let percent = percent.clamp(0, 100);
    let p = palette();
    let (fill, label) = if percent == 0 {
        (p.surface, p.text)
    } else {
        (p.primary, p.on_accent)
    };
    div().w_full().rounded_full().bg(p.control).child(
        div()
            .w(gpui::relative(percent as f32 / 100.))
            .p_0p5()
            .rounded_full()
            .bg(fill)
            .text_color(label)
            .text_xs()
            .line_height(rems(0.75))
            .font_weight(FontWeight::MEDIUM)
            .text_center()
            .child(format!("{percent}%")),
    )
}

/// `CircularProgress.tsx`, the desktop app's only spinner: a `border-8`
/// slate-900 ring with a slate-700 top, `p-16` inside (150.4px across),
/// turning as [`motion::SPIN`] says. It slides in from 100px right as it
/// mounts and lifts 5px, with a `shadow-2xl`, under the pointer.
pub fn circular_progress(id: impl Into<ElementId>) -> impl IntoElement {
    use gpui::{Animation, AnimationExt as _, Transformation, svg};
    let id = id.into();
    // `border-8` on each side of `p-16`: px around rems.
    let size = px(8. * 2.) + rems(2. * 4.).to_pixels(rem());
    let ring = div()
        .size(size)
        .rounded_full()
        .border_8()
        .border_color(palette().page);
    let arc = svg()
        .path(crate::assets::PROGRESS_ARC)
        .absolute()
        .top_0()
        .left_0()
        .size(size)
        .text_color(palette().control)
        .with_animation(
            (id.clone(), "spin"),
            Animation::new(motion::SPIN).repeat(),
            |arc, progress| {
                arc.with_transformation(Transformation::rotate(gpui::radians(
                    motion::spin_degrees(progress).to_radians(),
                )))
            },
        );
    div()
        .id(id.clone())
        .relative()
        .flex_none()
        .rounded_full()
        .hover(|this| this.top(px(-5.)).shadow_2xl())
        .child(ring)
        .child(arc)
        .with_animation(
            (id, "slide"),
            Animation::new(motion::SPINNER_SLIDE),
            |this, progress| this.left(motion::SPINNER_SLIDE_FROM * (1. - progress)),
        )
}

/// `SharedModpackView`'s loading spinner: a `size-6` slate-600 `border-2`
/// ring whose emerald-400 top turns under Tailwind's `animate-spin`.
pub fn spinner(id: impl Into<ElementId>) -> impl IntoElement {
    use gpui::{Animation, AnimationExt as _, Transformation, svg};
    div()
        .relative()
        .size_6()
        .flex_shrink_0()
        .rounded_full()
        .border_2()
        .border_color(palette().spinner_ring)
        .child(
            svg()
                .path(crate::assets::SPINNER_ARC)
                .absolute()
                .top(px(-2.))
                .left(px(-2.))
                .size_6()
                .text_color(palette().spinner_arc)
                .with_animation(
                    id,
                    Animation::new(motion::QUICK_SPIN).repeat(),
                    |arc, turn| {
                        arc.with_transformation(Transformation::rotate(gpui::radians(
                            turn * std::f32::consts::TAU,
                        )))
                    },
                ),
        )
}

/// `rounded-[28px]`: a panel's corners.
pub const PANEL_RADIUS: gpui::Pixels = px(28.);

/// A `rounded-[28px]` slate-800 panel, as the filters, results and install
/// panels are drawn.
pub fn panel() -> gpui::Div {
    div().rounded(PANEL_RADIUS).bg(palette().surface)
}

// Content, settings and account pages

/// The desktop app's Headless UI switch: an `h-8 w-16` slate-700 track that
/// turns emerald-800 when on, with a `size-8` slate-300 thumb.
pub fn setting_switch(id: impl Into<ElementId>, on: bool) -> gpui::Stateful<gpui::Div> {
    let p = palette();
    let (track, hover) = if on {
        (p.switch_on, p.switch_on_hover)
    } else {
        (p.control, p.control_hover)
    };
    div()
        .id(id)
        .flex_shrink_0()
        .h_8()
        .w_16()
        .rounded_full()
        .bg(track)
        .hover(move |style| style.bg(hover))
        .cursor_pointer()
        .child(
            div()
                .size_8()
                .rounded_full()
                .bg(p.text_soft)
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
            let p = palette();
            if checked {
                this.bg(p.positive)
                    .child(Icon::new(MdIcon::Check).size_4().text_color(p.on_accent))
            } else {
                // Light in every theme, as the browser's unchecked box.
                this.bg(p.on_accent).border_1().border_color(p.text_muted)
            }
        })
}

/// The Cloud badge of a modpack that only exists in Quadrant Sync.
pub fn cloud_badge(label: impl Into<SharedString>) -> impl IntoElement {
    let p = palette();
    tinted_badge(label, MdIcon::Cloud, p.cloud_badge_fill, p.cloud_badge_text)
}

/// The Synced badge of a modpack with a Quadrant Sync record.
pub fn synced_badge(label: impl Into<SharedString>) -> impl IntoElement {
    let p = palette();
    tinted_badge(
        label,
        MdIcon::CloudDone,
        p.synced_badge_fill,
        p.synced_badge_text,
    )
}

fn tinted_badge(
    label: impl Into<SharedString>,
    icon: MdIcon,
    background: Rgba,
    foreground: Rgba,
) -> impl IntoElement {
    gpui_component::h_flex()
        .gap_1()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded_full()
        .bg(background)
        .text_color(foreground)
        .text_size(px(11.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(Icon::new(icon).size_3())
        .child(label.into())
}

/// A failure the page keeps on screen, like the Account page's sign-in error.
pub fn warning_box(text: impl Into<SharedString>) -> impl IntoElement {
    let p = palette();
    div()
        .w_full()
        .p_4()
        .rounded(radius_card())
        .border_1()
        .border_color(p.warning_box_border)
        .bg(p.warning_box_fill)
        .text_color(p.warning_box_text)
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
    let (color, hover, label_color) = tone.colors();
    filled(id, color, hover, label_color, cx)
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
    let p = palette();
    filled(id, p.control, p.destructive_hover, p.text, cx)
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
    let (color, hover, label_color) = tone.colors();
    gpui_component::h_flex()
        .id(id)
        .w_full()
        .p_2()
        .gap_2()
        .items_center()
        .rounded(radius_card())
        .bg(color)
        .hover(move |style| style.bg(hover))
        .text_color(label_color)
        .cursor_pointer()
        .font_weight(FontWeight::EXTRA_BOLD)
        .child(div().flex_1().min_w_0().text_center().child(label.into()))
        .when_some(icon, |this, icon| {
            this.child(Icon::new(icon).flex_shrink_0().size_6())
        })
}

/// The floating panel of a Headless UI menu (`menuPanelClass`): a `p-2` bold
/// slate-800 panel with the card radius, at most `max-w-80` wide.
pub fn menu_panel<E: Styled>(panel: E) -> E {
    panel
        .p_2()
        .max_w(gpui::rems(20.))
        .rounded(radius_card())
        .bg(palette().surface)
        .font_weight(FontWeight::BOLD)
}

/// One entry of such a panel (`menuItemClass`): slate-700 under the pointer,
/// half transparent while disabled. The layout inside is the caller's.
pub fn menu_item(id: impl Into<ElementId>, disabled: bool) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .px_4()
        .py_2()
        .rounded(radius_card())
        .map(|this| {
            if disabled {
                this.opacity(0.5)
            } else {
                this.cursor_pointer()
                    .hover(|style| style.bg(palette().control))
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgba;

    #[test]
    fn snackbar_kinds_use_the_desktop_apps_colors() {
        let fill = |kind| snackbar_colors(kind, &QUADRANT).0;
        // bg-slate-700, bg-emerald-600, bg-red-700
        assert_eq!(fill(SnackbarKind::Neutral), rgb(0x334155));
        assert_eq!(fill(SnackbarKind::Success), rgb(0x059669));
        assert_eq!(fill(SnackbarKind::Error), rgb(0xb91c1c));
    }

    #[test]
    fn const_hex_matches_gpuis_conversion() {
        for value in [0x0f172a, 0xf8fafc, 0x000000, 0xffffff, 0x2563eb] {
            assert_eq!(hex(value), rgb(value));
        }
        for value in [0x0206174d, 0xffffff0d, 0x00000000, 0xf8717166] {
            assert_eq!(hexa(value), rgba(value));
        }
    }
}
