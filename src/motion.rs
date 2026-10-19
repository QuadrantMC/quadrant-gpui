//! The desktop app's motion, as tokens and the curves that play them. Its
//! pages mount with `initial {y: 24, opacity: 0}` and `animate {y: 0,
//! opacity: 1}` under Motion's defaults (a spring for `y`, a 0.3 s ease for
//! `opacity`); the content container first mounts the same way over 0.1 s
//! keyframes; and opening or leaving a pack's mods is a browser view
//! transition, a 0.25 s crossfade.

use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Bounds, ContentMask, Element, ElementId,
    GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Rgba, Styled, Window, fill,
    point, px, size,
};

/// How far below its place a page starts.
pub const ENTER_OFFSET: Pixels = px(24.);

/// Motion's default spring for `y`: stiffness 500, damping 25, mass 1.
pub const ENTER_SPRING: Spring = Spring {
    stiffness: 500.,
    damping: 25.,
    mass: 1.,
};

/// Motion's default tween for `opacity`: 0.3 s, `[0.25, 0.1, 0.35, 1]`.
pub const ENTER_FADE: Duration = Duration::from_millis(300);
pub const ENTER_FADE_EASE: CubicBezier = CubicBezier(0.25, 0.1, 0.35, 1.);

/// The content container's own entrance on first mount: 0.1 s keyframes,
/// which Motion eases in and out.
pub const SHELL_ENTER: Duration = Duration::from_millis(100);
pub const SHELL_ENTER_EASE: CubicBezier = CubicBezier(0.42, 0., 0.58, 1.);

/// A page's entrance: the spring settles within half a pixel by then.
pub const ENTER: Duration = Duration::from_millis(350);

/// The browser's default view transition: a 0.25 s crossfade, `ease`.
pub const CROSSFADE: Duration = Duration::from_millis(250);
pub const CROSSFADE_EASE: CubicBezier = CubicBezier(0.25, 0.1, 0.25, 1.);

/// `Mod.tsx`'s card entrance, `initial {opacity: 0, y: 24}` under
/// `{type: "spring", stiffness: 100, duration: 0.3}`, for both values. A
/// stiffness makes Motion ignore the duration, and its default damping is 10:
/// an underdamped spring that passes its place by about a sixth of the way.
pub const CARD_SPRING: Spring = Spring {
    stiffness: 100.,
    damping: 10.,
    mass: 1.,
};

/// When Motion calls the card's spring at rest: opacity within 0.005 of 1,
/// `y` within half a pixel well before.
pub const CARD_ENTER: Duration = Duration::from_millis(1100);

/// `CircularProgress.tsx`'s spin: `rotate: 3840` over 15s, linear, repeated
/// from 0, so the ring turns 256° a second and jumps back 120° every 15s.
pub const SPIN: Duration = Duration::from_secs(15);
pub const SPIN_DEGREES: f32 = 3840.;

/// Its slide in on mount: from `x: 100` over 0.1s, linear.
pub const SPINNER_SLIDE: Duration = Duration::from_millis(100);
pub const SPINNER_SLIDE_FROM: Pixels = px(100.);

/// The spin's angle, in degrees, `progress` through one 15s pass.
pub fn spin_degrees(progress: f32) -> f32 {
    SPIN_DEGREES * progress
}

/// Search's "search further" arrow sliding away: `x: 260, opacity: 0` over
/// 0.075s, `easeIn`.
pub const FURTHER_SLIDE: Duration = Duration::from_millis(75);
pub const FURTHER_SLIDE_BY: Pixels = px(260.);

/// Motion's `easeIn`, `easeOut` and `easeInOut`.
pub const EASE_IN: CubicBezier = CubicBezier(0.42, 0., 1., 1.);
pub const EASE_OUT: CubicBezier = CubicBezier(0., 0., 0.58, 1.);
pub const EASE_IN_OUT: CubicBezier = CubicBezier(0.42, 0., 0.58, 1.);

/// `ModInstallPage`'s `expandSpring`, stiffness 320 and damping 32: the
/// details panel grows out of the card that opened the page, and the other
/// panels rise 16px as they fade in, [`HERO_REST_DELAY`] later.
pub const HERO_SPRING: Spring = Spring {
    stiffness: 320.,
    damping: 32.,
    mass: 1.,
};
/// When the expand spring is within half a pixel of rest over the longest
/// way a panel travels.
pub const HERO_EXPAND: Duration = Duration::from_millis(550);
pub const HERO_REST_DELAY: Duration = Duration::from_millis(120);
pub const HERO_REST_OFFSET: Pixels = px(16.);
/// Closing: the panel shrinks back into the card over 0.22s `easeInOut`
/// while the other panels fade out over 0.15s; then the page leaves.
pub const HERO_COLLAPSE: Duration = Duration::from_millis(220);
pub const HERO_REST_FADE: Duration = Duration::from_millis(150);

/// Search's filters sidebar folding away or out: its width over 0.28s on
/// `[0.4, 0, 0.2, 1]`, while the panel inside, fixed at its full width and
/// clipped, fades over 0.15s.
pub const SIDEBAR: Duration = Duration::from_millis(280);
pub const SIDEBAR_EASE: CubicBezier = CubicBezier(0.4, 0., 0.2, 1.);
pub const SIDEBAR_FADE: Duration = Duration::from_millis(150);

/// The sidebar's share of its full width and its panel's opacity `elapsed`
/// into `opening` or closing it; `None` once it is done.
pub fn sidebar(elapsed: Duration, opening: bool) -> Option<(f32, f32)> {
    if elapsed >= SIDEBAR {
        return None;
    }
    let width = SIDEBAR_EASE.at(elapsed, SIDEBAR);
    let fade = EASE_OUT.at(elapsed, SIDEBAR_FADE);
    Some(if opening {
        (width, fade)
    } else {
        (1. - width, 1. - fade)
    })
}

/// Where a hero transition is: the card-shaped window of the details panel,
/// as the share of the way back to the card still to go (1 at the card, 0
/// at rest), and the other panels' offset and opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeroFrame {
    pub remaining: f32,
    pub rest_offset: Pixels,
    pub rest_opacity: f32,
}

/// `elapsed` into opening, or `None` once everything is at rest.
pub fn hero_open(elapsed: Duration) -> Option<HeroFrame> {
    if elapsed >= HERO_REST_DELAY + HERO_EXPAND {
        return None;
    }
    let remaining = if elapsed >= HERO_EXPAND {
        0.
    } else {
        HERO_SPRING.remaining(elapsed)
    };
    let (rest_offset, rest_opacity) = match elapsed.checked_sub(HERO_REST_DELAY) {
        None => (HERO_REST_OFFSET, 0.),
        Some(rest) => {
            let left = HERO_SPRING.remaining(rest);
            (HERO_REST_OFFSET * left, (1. - left).clamp(0., 1.))
        }
    };
    Some(HeroFrame {
        remaining,
        rest_offset,
        rest_opacity,
    })
}

/// `elapsed` into closing; at the card from [`HERO_COLLAPSE`] on.
pub fn hero_close(elapsed: Duration) -> HeroFrame {
    HeroFrame {
        remaining: EASE_IN_OUT.at(elapsed, HERO_COLLAPSE),
        rest_offset: px(0.),
        rest_opacity: 1. - EASE_OUT.at(elapsed, HERO_REST_FADE),
    }
}

/// `ModInstallPage`'s `revealFrom`: lays `child` out in place, then paints
/// it moved `remaining` of the way to `origin` and clipped to a window of
/// its top-left corner that is `remaining` of the way to `origin`'s size,
/// so the content inside never stretches. The child draws no background of
/// its own; this paints `background` with `radius` corners behind the
/// window, as the desktop app's rounded `clip-path` does.
pub struct Reveal {
    child: AnyElement,
    origin: Bounds<Pixels>,
    remaining: f32,
    background: Rgba,
    radius: Pixels,
}

pub fn reveal(
    child: impl IntoElement,
    origin: Bounds<Pixels>,
    remaining: f32,
    background: Rgba,
    radius: Pixels,
) -> Reveal {
    Reveal {
        child: child.into_any_element(),
        origin,
        remaining,
        background,
        radius,
    }
}

/// The window `rest` shows when `remaining` of the way to `origin`.
pub fn reveal_window(
    rest: Bounds<Pixels>,
    origin: Bounds<Pixels>,
    remaining: f32,
) -> Bounds<Pixels> {
    let shrink = |rest: Pixels, origin: Pixels| rest - (rest - origin).max(px(0.)) * remaining;
    Bounds {
        origin: point(
            rest.origin.x + (origin.origin.x - rest.origin.x) * remaining,
            rest.origin.y + (origin.origin.y - rest.origin.y) * remaining,
        ),
        size: size(
            shrink(rest.size.width, origin.size.width),
            shrink(rest.size.height, origin.size.height),
        ),
    }
}

impl IntoElement for Reveal {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Reveal {
    type RequestLayoutState = ();
    /// The window, in window coordinates.
    type PrepaintState = Bounds<Pixels>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Bounds<Pixels> {
        let shown = reveal_window(bounds, self.origin, self.remaining);
        let offset = shown.origin - bounds.origin;
        window.with_element_offset(offset, |window| {
            window.with_content_mask(Some(ContentMask { bounds: shown }), |window| {
                self.child.prepaint(window, cx)
            })
        });
        shown
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        shown: &mut Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_quad(fill(*shown, self.background).corner_radii(self.radius));
        window.with_content_mask(Some(ContentMask { bounds: *shown }), |window| {
            self.child.paint(window, cx)
        });
    }
}

/// A CSS `cubic-bezier(x1, y1, x2, y2)` timing function.
#[derive(Clone, Copy, Debug)]
pub struct CubicBezier(pub f32, pub f32, pub f32, pub f32);

impl CubicBezier {
    /// The eased progress at linear progress `x` in `0..=1`.
    pub fn ease(self, x: f32) -> f32 {
        let CubicBezier(x1, y1, x2, y2) = self;
        let x = x.clamp(0., 1.);
        let bezier = |a: f32, b: f32, t: f32| {
            let u = 1. - t;
            3. * u * u * t * a + 3. * u * t * t * b + t * t * t
        };
        // Solve x(t) = x by bisection; x(t) only grows on 0..1.
        let (mut low, mut high) = (0f32, 1f32);
        for _ in 0..24 {
            let mid = (low + high) / 2.;
            if bezier(x1, x2, mid) < x {
                low = mid;
            } else {
                high = mid;
            }
        }
        bezier(y1, y2, (low + high) / 2.)
    }

    /// The eased progress `elapsed` into an animation lasting `duration`.
    pub fn at(self, elapsed: Duration, duration: Duration) -> f32 {
        self.ease(elapsed.as_secs_f32() / duration.as_secs_f32())
    }
}

/// A damped spring from rest, as Motion's `type: "spring"` plays it.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
}

impl Spring {
    /// How much of the way from the start to the target is still left
    /// `elapsed` into the motion: 1 at the start, 0 at rest, below 0 while
    /// it overshoots.
    pub fn remaining(self, elapsed: Duration) -> f32 {
        let t = elapsed.as_secs_f32();
        let omega = (self.stiffness / self.mass).sqrt();
        let zeta = self.damping / (2. * (self.stiffness * self.mass).sqrt());
        if zeta < 1. {
            let damped = omega * (1. - zeta * zeta).sqrt();
            (-zeta * omega * t).exp()
                * ((damped * t).cos() + zeta * omega / damped * (damped * t).sin())
        } else {
            (-omega * t).exp() * (1. + omega * t)
        }
    }
}

/// Where an entering page sits and how opaque it is `elapsed` after it
/// mounted; `first` adds the content container's own first entrance.
pub fn enter(elapsed: Duration, first: bool) -> (Pixels, f32) {
    // Exactly in place at the end, not the spring's last sub-pixel.
    if elapsed >= ENTER {
        return (px(0.), 1.);
    }
    let mut offset = ENTER_OFFSET * ENTER_SPRING.remaining(elapsed);
    let mut opacity = ENTER_FADE_EASE.at(elapsed, ENTER_FADE);
    if first {
        let shell = SHELL_ENTER_EASE.at(elapsed, SHELL_ENTER);
        offset += ENTER_OFFSET * (1. - shell);
        opacity *= shell;
    }
    (offset, opacity)
}

/// Where an entering mod card sits and how opaque it is `elapsed` after it
/// mounted. Opacity overshoots with `y` and is clamped, as a browser does.
pub fn card_enter(elapsed: Duration) -> (Pixels, f32) {
    if elapsed >= CARD_ENTER {
        return (px(0.), 1.);
    }
    let remaining = CARD_SPRING.remaining(elapsed);
    (ENTER_OFFSET * remaining, (1. - remaining).clamp(0., 1.))
}

/// The entrance of a list's mod cards: all of them together from the frame
/// the list appeared or got new results. A card drawn later, scrolled into
/// view, joins at the same point rather than starting over, as every card
/// of the desktop app's unvirtualized grids is mounted at once.
#[derive(Clone, Copy, Debug, Default)]
pub enum CardEntrance {
    /// At rest.
    #[default]
    Rest,
    /// Starts at the next frame.
    Pending,
    Since(Instant),
}

impl CardEntrance {
    /// Plays the entrance again from the next frame.
    pub fn restart(&mut self) {
        *self = CardEntrance::Pending;
    }

    /// The cards' offset and opacity this frame, `None` at rest. Asks for
    /// another frame of the current view while they move.
    pub fn frame(&mut self, window: &mut Window) -> Option<(Pixels, f32)> {
        let elapsed = match *self {
            CardEntrance::Rest => return None,
            CardEntrance::Pending => {
                *self = CardEntrance::Since(Instant::now());
                Duration::ZERO
            }
            CardEntrance::Since(started) => started.elapsed(),
        };
        if elapsed >= CARD_ENTER {
            *self = CardEntrance::Rest;
            return None;
        }
        window.request_animation_frame();
        Some(card_enter(elapsed))
    }
}

/// `card` where an entrance's [`CardEntrance::frame`] puts it.
pub fn card_at<E: Styled>(card: E, frame: Option<(Pixels, f32)>) -> E {
    match frame {
        Some((offset, opacity)) => card.relative().top(offset).opacity(opacity),
        None => card,
    }
}

/// `element` entering as a page does: from [`ENTER_OFFSET`] below and
/// transparent. A new `id` plays it again. `element` must be positioned
/// relative, so the offset moves it without moving its neighbours.
pub fn entering<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<ElementId>,
    first: bool,
) -> impl IntoElement {
    element.with_animation(id, Animation::new(ENTER), move |this, progress| {
        let (offset, opacity) = enter(ENTER.mul_f32(progress), first);
        this.top(offset).opacity(opacity)
    })
}

/// How far above its place the notifications panel starts and ends.
pub const POPOVER_OFFSET: Pixels = px(16.);

/// Where the notifications panel sits and how opaque it is `elapsed` into
/// its entrance (`initial {y: -16, opacity: 0}` to `animate {y: 0,
/// opacity: 1}`) or its exit (back to `y: -16, opacity: 0`), under the same
/// defaults as a page.
pub fn popover(elapsed: Duration, entering: bool) -> (Pixels, f32) {
    let (left, faded) = if elapsed >= ENTER {
        (0., 1.)
    } else {
        (
            ENTER_SPRING.remaining(elapsed),
            ENTER_FADE_EASE.at(elapsed, ENTER_FADE),
        )
    };
    if entering {
        (-POPOVER_OFFSET * left, faded)
    } else {
        (-POPOVER_OFFSET * (1. - left), 1. - faded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_start_at_zero_and_end_at_one() {
        for ease in [ENTER_FADE_EASE, SHELL_ENTER_EASE, CROSSFADE_EASE] {
            assert!(ease.ease(0.).abs() < 1e-3);
            assert!((ease.ease(1.) - 1.).abs() < 1e-3);
            assert!(ease.ease(0.5) > 0.3 && ease.ease(0.5) < 0.9);
        }
    }

    #[test]
    fn the_enter_spring_overshoots_a_little_then_settles() {
        let at = |ms| ENTER_SPRING.remaining(Duration::from_millis(ms));
        assert!((at(0) - 1.).abs() < 1e-6);
        // Underdamped: it passes its place by about an eighth of the way.
        let deepest = (0..400).map(at).fold(f32::MAX, f32::min);
        assert!(deepest < -0.05 && deepest > -0.2, "{deepest}");
        assert!((ENTER_OFFSET * at(ENTER.as_millis() as u64)).abs() < px(0.5));
    }

    #[test]
    fn the_popover_drops_in_and_lifts_out() {
        let (offset, opacity) = popover(Duration::ZERO, true);
        assert_eq!(offset, -POPOVER_OFFSET);
        assert!(opacity < 1e-3);
        assert_eq!(popover(ENTER, true), (px(0.), 1.));
        let (offset, opacity) = popover(Duration::ZERO, false);
        assert!(offset.abs() < px(1e-3) && (opacity - 1.).abs() < 1e-3);
        assert_eq!(popover(ENTER, false), (-POPOVER_OFFSET, 0.));
    }

    #[test]
    fn a_card_springs_up_past_its_place_and_settles() {
        let (offset, opacity) = card_enter(Duration::ZERO);
        assert_eq!((offset, opacity), (ENTER_OFFSET, 0.));
        // Fully opaque at the spring's first pass, about 0.24s in.
        assert!(card_enter(Duration::from_millis(200)).1 < 1.);
        assert_eq!(card_enter(Duration::from_millis(260)).1, 1.);
        let deepest = (0..1100)
            .map(|ms| f32::from(card_enter(Duration::from_millis(ms)).0))
            .fold(f32::MAX, f32::min);
        assert!(deepest < -3. && deepest > -5., "{deepest}");
        let near_rest = CARD_SPRING.remaining(CARD_ENTER - Duration::from_millis(10));
        assert!(near_rest.abs() < 0.005, "{near_rest}");
        assert_eq!(card_enter(CARD_ENTER), (px(0.), 1.));
    }

    #[test]
    fn the_hero_opens_from_the_card_and_closes_into_it() {
        let card = Bounds::new(point(px(400.), px(300.)), size(px(380.), px(167.)));
        let panel = Bounds::new(point(px(120.), px(110.)), size(px(900.), px(420.)));
        let first = hero_open(Duration::ZERO).unwrap();
        assert_eq!(reveal_window(panel, card, first.remaining), card);
        assert_eq!(
            (first.rest_offset, first.rest_opacity),
            (HERO_REST_OFFSET, 0.)
        );
        // The other panels wait, then rise as they fade in.
        assert_eq!(hero_open(HERO_REST_DELAY).unwrap().rest_opacity, 0.);
        let later = hero_open(HERO_REST_DELAY + Duration::from_millis(150)).unwrap();
        assert!(later.rest_opacity > 0.5 && later.rest_offset < px(8.));
        // Within half a pixel over the longest way the panel moves.
        let near = hero_open(HERO_EXPAND - Duration::from_millis(1)).unwrap();
        let window = reveal_window(panel, card, near.remaining);
        assert!((window.size.width - panel.size.width).abs() < px(0.5));
        assert_eq!(hero_open(HERO_REST_DELAY + HERO_EXPAND), None);

        let start = hero_close(Duration::ZERO);
        assert!(start.remaining.abs() < 1e-3 && start.rest_opacity > 0.999);
        assert!(hero_close(HERO_REST_FADE).rest_opacity.abs() < 1e-3);
        let closed = reveal_window(panel, card, hero_close(HERO_COLLAPSE).remaining);
        assert!((closed.origin.x - card.origin.x).abs() < px(0.01));
        assert!((closed.size.height - card.size.height).abs() < px(0.01));
    }

    #[test]
    fn the_sidebar_folds_its_width_while_its_panel_fades_sooner() {
        let (width, opacity) = sidebar(Duration::ZERO, true).unwrap();
        assert!(width.abs() < 1e-3 && opacity.abs() < 1e-3);
        let (width, opacity) = sidebar(Duration::ZERO, false).unwrap();
        assert!(width > 0.999 && opacity > 0.999);
        let (width, opacity) = sidebar(SIDEBAR_FADE, false).unwrap();
        assert!(width > 0.1 && opacity.abs() < 1e-3, "{width} {opacity}");
        assert_eq!(sidebar(SIDEBAR, true), None);
    }

    #[test]
    fn a_page_enters_from_below_and_transparent() {
        let (offset, opacity) = enter(Duration::ZERO, true);
        assert_eq!(offset, ENTER_OFFSET * 2.);
        assert!(opacity < 1e-3);
        assert_eq!(enter(ENTER, false), (px(0.), 1.));
    }
}
