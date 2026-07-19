//! Tiny dependency-free tween helpers for panel slide animations.
//!
//! `SlideAnim` interpolates a `Rect` from `from` to `to` over `dur`, using a
//! cubic ease-out curve. Depends only on `ratatui::layout::Rect` + `std::time`.

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

/// A rect slide from `from` to `to`, eased over `dur` starting at `start`.
#[derive(Debug, Clone, Copy)]
pub struct SlideAnim {
    pub from: Rect,
    pub to: Rect,
    pub start: Instant,
    pub dur: Duration,
}

impl SlideAnim {
    /// Construct a slide whose clock starts now.
    pub fn new(from: Rect, to: Rect, dur: Duration) -> Self {
        Self {
            from,
            to,
            start: Instant::now(),
            dur,
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
        rect_lerp(self.from, self.to, ease_out(t))
    }

    /// True once `dur` has elapsed since `start`.
    pub fn is_done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= self.dur
    }
}

/// Cubic ease-out on `t` clamped to `[0, 1]` -> `[0, 1]`: `1 - (1 - t)^3`.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    1.0 - inv * inv * inv
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
