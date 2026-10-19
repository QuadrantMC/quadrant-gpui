//! The desktop app's motion, as tokens and the curves that play them. Its
//! pages mount with `initial {y: 24, opacity: 0}` and `animate {y: 0,
//! opacity: 1}` under Motion's defaults (a spring for `y`, a 0.3 s ease for
//! `opacity`); the content container first mounts the same way over 0.1 s
//! keyframes; and opening or leaving a pack's mods is a browser view
//! transition, a 0.25 s crossfade.

use std::time::Duration;

use gpui::{Animation, AnimationExt as _, ElementId, IntoElement, Pixels, Styled, px};

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
    fn a_page_enters_from_below_and_transparent() {
        let (offset, opacity) = enter(Duration::ZERO, true);
        assert_eq!(offset, ENTER_OFFSET * 2.);
        assert!(opacity < 1e-3);
        assert_eq!(enter(ENTER, false), (px(0.), 1.));
    }
}
