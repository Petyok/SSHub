//! Tiny dependency-free tween helpers for panel slide animations.
//!
//! `SlideAnim` interpolates a `Rect` from `from` to `to` over `dur`, using a
//! cubic ease-out curve. Depends only on `ratatui::layout::Rect` + `std::time`.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

/// Easing curve applied by a [`SlideAnim`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    /// Front-loaded: fast then settling. Good for things flying in.
    Out,
    /// Symmetric: eases in and out. Good for A<->B morphs (zoom, tab swap).
    InOut,
}

/// A rect slide from `from` to `to`, eased over `dur` starting at `start`.
#[derive(Debug, Clone, Copy)]
pub struct SlideAnim {
    pub from: Rect,
    pub to: Rect,
    pub start: Instant,
    pub dur: Duration,
    pub easing: Easing,
}

impl SlideAnim {
    /// Construct an ease-out slide whose clock starts now.
    pub fn new(from: Rect, to: Rect, dur: Duration) -> Self {
        Self::with_easing(from, to, dur, Easing::Out)
    }

    /// Construct an ease-in-out slide (symmetric morph) whose clock starts now.
    pub fn new_in_out(from: Rect, to: Rect, dur: Duration) -> Self {
        Self::with_easing(from, to, dur, Easing::InOut)
    }

    /// Construct a slide with an explicit easing curve, clock starting now.
    pub fn with_easing(from: Rect, to: Rect, dur: Duration, easing: Easing) -> Self {
        Self {
            from,
            to,
            start: Instant::now(),
            dur,
            easing,
        }
    }

    /// Eased-lerp rect at `now`. Returns exactly `to` once `dur` has elapsed
    /// (and exactly `from` at `t == 0`).
    pub fn rect_at(&self, now: Instant) -> Rect {
        if self.dur.is_zero() {
            return self.to;
        }
        let elapsed = now.saturating_duration_since(self.start);
        if elapsed >= self.dur {
            return self.to;
        }
        let t = elapsed.as_secs_f32() / self.dur.as_secs_f32();
        let eased = match self.easing {
            Easing::Out => ease_out(t),
            Easing::InOut => ease_in_out(t),
        };
        rect_lerp(self.from, self.to, eased)
    }

    /// True once `dur` has elapsed since `start`.
    pub fn is_done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.dur
    }
}

/// Raw (un-eased) progress of a timed animation: `elapsed / dur` clamped to
/// `[0, 1]`. The shared clock helper for time-driven animations that don't use
/// a [`SlideAnim`] (e.g. a directional slide computed from an anchor).
pub fn progress(start: Instant, dur: Duration, now: Instant) -> f32 {
    if dur.is_zero() {
        return 1.0;
    }
    (now.saturating_duration_since(start).as_secs_f32() / dur.as_secs_f32()).clamp(0.0, 1.0)
}

/// Cubic ease-out on `t` clamped to `[0, 1]` -> `[0, 1]`: `1 - (1 - t)^3`.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
}

/// Cubic ease-in-out on `t` clamped to `[0, 1]` -> `[0, 1]`: slow at both ends,
/// fastest in the middle. Symmetric, for A<->B morphs.
pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        let f = -2.0 * t + 2.0;
        1.0 - f * f * f / 2.0
    }
}

/// Per-field linear interpolation of a `Rect` at (already-eased) `t`, rounded.
///
/// `t` is clamped to `[0, 1]`; each `u16` field is interpolated independently
/// and rounded to the nearest integer.
pub fn rect_lerp(from: Rect, to: Rect, t: f32) -> Rect {
    let t = t.clamp(0.0, 1.0);
    Rect {
        x: lerp_u16(from.x, to.x, t),
        y: lerp_u16(from.y, to.y, t),
        width: lerp_u16(from.width, to.width, t),
        height: lerp_u16(from.height, to.height, t),
    }
}

/// Linearly interpolate one `u16` field and round to nearest.
fn lerp_u16(a: u16, b: u16, t: f32) -> u16 {
    let a = a as f32;
    let b = b as f32;
    (a + (b - a) * t).round().clamp(0.0, u16::MAX as f32) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn ease_out_endpoints() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
    }

    #[test]
    fn ease_out_clamps_out_of_range() {
        assert_eq!(ease_out(-1.0), 0.0);
        assert_eq!(ease_out(2.0), 1.0);
    }

    #[test]
    fn ease_out_monotonic() {
        let mut prev = ease_out(0.0);
        let mut t = 0.0;
        while t <= 1.0 {
            let cur = ease_out(t);
            assert!(cur >= prev, "ease_out not monotonic at t={t}");
            prev = cur;
            t += 0.05;
        }
    }

    #[test]
    fn ease_out_is_ahead_of_linear() {
        // Ease-out front-loads progress: it should exceed linear in the middle.
        assert!(ease_out(0.5) > 0.5);
    }

    #[test]
    fn ease_in_out_endpoints_and_symmetry() {
        assert_eq!(ease_in_out(0.0), 0.0);
        assert_eq!(ease_in_out(1.0), 1.0);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-6, "midpoint is 0.5");
        // Symmetric about (0.5, 0.5).
        assert!((ease_in_out(0.25) + ease_in_out(0.75) - 1.0).abs() < 1e-6);
        // Clamps.
        assert_eq!(ease_in_out(-1.0), 0.0);
        assert_eq!(ease_in_out(2.0), 1.0);
    }

    #[test]
    fn progress_clamps_and_endpoints() {
        let start = Instant::now();
        let dur = Duration::from_millis(200);
        assert_eq!(progress(start, dur, start), 0.0);
        assert!((progress(start, dur, start + Duration::from_millis(100)) - 0.5).abs() < 1e-3);
        assert_eq!(progress(start, dur, start + dur), 1.0);
        assert_eq!(progress(start, dur, start + Duration::from_secs(9)), 1.0);
        assert_eq!(progress(start, Duration::ZERO, start), 1.0);
    }

    #[test]
    fn in_out_slide_reaches_endpoints() {
        let from = rect(0, 0, 40, 12);
        let to = rect(60, 20, 20, 6);
        let dur = Duration::from_millis(200);
        let anim = SlideAnim::new_in_out(from, to, dur);
        assert_eq!(anim.rect_at(anim.start), from);
        assert_eq!(anim.rect_at(anim.start + dur), to);
    }

    #[test]
    fn rect_lerp_endpoints() {
        let from = rect(0, 0, 10, 4);
        let to = rect(20, 10, 30, 8);
        assert_eq!(rect_lerp(from, to, 0.0), from);
        assert_eq!(rect_lerp(from, to, 1.0), to);
    }

    #[test]
    fn rect_lerp_midpoint() {
        let from = rect(0, 0, 10, 4);
        let to = rect(20, 10, 30, 8);
        assert_eq!(rect_lerp(from, to, 0.5), rect(10, 5, 20, 6));
    }

    #[test]
    fn rect_lerp_clamps_t() {
        let from = rect(0, 0, 10, 4);
        let to = rect(20, 10, 30, 8);
        assert_eq!(rect_lerp(from, to, -1.0), from);
        assert_eq!(rect_lerp(from, to, 2.0), to);
    }

    #[test]
    fn rect_at_endpoints() {
        let from = rect(10, 10, 40, 12);
        let to = rect(60, 20, 20, 6);
        let dur = Duration::from_millis(250);
        let anim = SlideAnim::new(from, to, dur);

        // t == 0 -> exactly `from`.
        assert_eq!(anim.rect_at(anim.start), from);
        // elapsed >= dur -> exactly `to`.
        assert_eq!(anim.rect_at(anim.start + dur), to);
        assert_eq!(anim.rect_at(anim.start + dur + Duration::from_secs(1)), to);
    }

    #[test]
    fn rect_at_between_endpoints() {
        let from = rect(0, 0, 40, 12);
        let to = rect(60, 20, 20, 6);
        let dur = Duration::from_millis(200);
        let anim = SlideAnim::new(from, to, dur);
        let mid = anim.rect_at(anim.start + Duration::from_millis(100));
        // Strictly between the endpoints on the moving x axis.
        assert!(mid.x > from.x && mid.x < to.x, "x={} not between", mid.x);
    }

    #[test]
    fn rect_at_zero_duration_is_target() {
        let from = rect(0, 0, 40, 12);
        let to = rect(60, 20, 20, 6);
        let anim = SlideAnim::new(from, to, Duration::ZERO);
        assert_eq!(anim.rect_at(anim.start), to);
    }

    #[test]
    fn is_done_transitions() {
        let from = rect(0, 0, 40, 12);
        let to = rect(60, 20, 20, 6);
        let dur = Duration::from_millis(250);
        let anim = SlideAnim::new(from, to, dur);
        assert!(!anim.is_done(anim.start));
        assert!(anim.is_done(anim.start + dur));
    }
}
