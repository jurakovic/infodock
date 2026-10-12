//! Animations (SPEC §5.5, §9.3): values between 0 and 1 that move to a target over time with
//! an easing and are read at each frame. Pure; callers supply the clock and drive the frames.

use std::time::{Duration, Instant};

use crate::config::model::Animations;

/// The frame interval while something animates.
pub const FRAME: Duration = Duration::from_millis(16);

/// Whether the bar animates: the setting, or the OS "animation effects" for `auto`.
pub fn enabled(setting: Animations, os: bool) -> bool {
    match setting {
        Animations::Auto => os,
        Animations::On => true,
        Animations::Off => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Easing {
    In,
    Out,
}

impl Easing {
    /// Cubic: slow start (in) or slow end (out).
    fn apply(self, t: f32) -> f32 {
        match self {
            Easing::In => t * t * t,
            Easing::Out => 1.0 - (1.0 - t).powi(3),
        }
    }
}

/// A value between 0 and 1 on its way to a target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
    from: f32,
    to: f32,
    start: Instant,
    duration: Duration,
    easing: Easing,
}

impl Fade {
    /// Resting at `value`.
    pub fn at(value: f32, now: Instant) -> Self {
        Self {
            from: value,
            to: value,
            start: now,
            duration: Duration::ZERO,
            easing: Easing::Out,
        }
    }

    pub fn value(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return self.to;
        }
        let elapsed = now.saturating_duration_since(self.start).as_secs_f32();
        let t = (elapsed / self.duration.as_secs_f32()).min(1.0);
        self.from + (self.to - self.from) * self.easing.apply(t)
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    /// Still moving at `now`: another frame is needed.
    pub fn running(&self, now: Instant) -> bool {
        now < self.start + self.duration
    }

    /// Heads for `to` from wherever it is at `now`. `full` is the time for the whole range from
    /// 0 to 1, so a fade reversed halfway takes half as long; zero jumps there.
    pub fn go(&mut self, to: f32, full: Duration, easing: Easing, now: Instant) {
        if to == self.to {
            return;
        }
        let from = self.value(now);
        *self = Self {
            from,
            to,
            start: now,
            duration: full.mul_f32((to - from).abs()),
            easing,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn eases_in_and_out() {
        let t0 = Instant::now();
        let mut fade = Fade::at(0.0, t0);
        assert!(!fade.running(t0));
        fade.go(1.0, 300 * MS, Easing::In, t0);
        assert!(fade.running(t0 + 150 * MS));
        // Ease-in: an eighth of the way at half time.
        assert!((fade.value(t0 + 150 * MS) - 0.125).abs() < 1e-4);
        assert_eq!(fade.value(t0 + 300 * MS), 1.0);
        assert!(!fade.running(t0 + 300 * MS));

        fade.go(0.0, 300 * MS, Easing::Out, t0 + 400 * MS);
        // Ease-out: seven eighths of the way at half time.
        assert!((fade.value(t0 + 550 * MS) - 0.125).abs() < 1e-4);
        assert_eq!(fade.value(t0 + 700 * MS), 0.0);
    }

    #[test]
    fn reversing_halfway_takes_the_remaining_distance() {
        let t0 = Instant::now();
        let mut fade = Fade::at(0.0, t0);
        fade.go(1.0, 200 * MS, Easing::Out, t0);
        let at = t0 + 100 * MS;
        let reached = fade.value(at);
        fade.go(0.0, 300 * MS, Easing::Out, at);
        assert_eq!(fade.value(at), reached);
        assert!(fade.running(at + (300.0 * reached - 1.0) as u32 * MS));
        assert!(!fade.running(at + (300.0 * reached + 1.0) as u32 * MS));
        // Heading for the same target again changes nothing.
        let before = fade;
        fade.go(0.0, 300 * MS, Easing::In, at + 50 * MS);
        assert_eq!(fade, before);
    }

    #[test]
    fn without_animations_it_jumps() {
        let t0 = Instant::now();
        let mut fade = Fade::at(0.0, t0);
        fade.go(1.0, Duration::ZERO, Easing::In, t0);
        assert_eq!(fade.value(t0), 1.0);
        assert!(!fade.running(t0));
        assert!(enabled(Animations::Auto, true));
        assert!(!enabled(Animations::Auto, false));
        assert!(enabled(Animations::On, false));
        assert!(!enabled(Animations::Off, true));
    }
}
