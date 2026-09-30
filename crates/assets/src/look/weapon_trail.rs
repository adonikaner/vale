//! The strip a weapon leaves behind it while a melee ability plays.
//!
//! A kit's `charProc` 8 ([`crate::tables::spell::WeaponTrail`]) arms the trail
//! on a unit. The next animation the unit starts draws one strip per weapon
//! model it carries, for the kit's length, in the kit's colour. This module
//! holds the two rules that need no renderer: where on the weapon the strip's
//! edges are ([`TrailPoints`]), and how the strip's opacity runs down
//! ([`TrailClock`]).
//!
//! ## The strip
//!
//! Every frame while the trail is being laid, the weapon's two points are
//! recorded as a pair, bottom then top, into a ring of [`RING_PAIRS`] pairs.
//! The newest pairs are drawn as one triangle strip, untextured and alpha
//! blended, with the kit's colour on every vertex. How many pairs are drawn and
//! how opaque each is are decided by the clock below.
//!
//! ## The opacity
//!
//! The strip starts at the kit's opacity `A` (out of 255). Each frame:
//!
//! ```text
//!   step  = max(1, trunc(dt_ms / 300 * A))
//!   pairs = A / step                      (integer division)
//!   pairs <= 1                            -> the trail is over
//!   pair i of the newest `pairs`, oldest first, is drawn at A - step * (i + 1)
//!   past half the kit's length:  A -= step
//! ```
//!
//! `pairs` is `300 / dt_ms` whatever `A` is, so the strip always covers the
//! last 300 ms of the swing. The oldest drawn pair is the most opaque and the
//! pair at the weapon is nearly transparent. Once past half its length the
//! whole strip loses a 300 ms share of its opacity each frame, and it ends when
//! `A` is too small to divide.

use crate::world::m2::M2Event;

/// How many pairs of points the ring holds.
pub const RING_PAIRS: usize = 64;

/// The span, in milliseconds, the opacity step is computed over; see the
/// module comment.
pub const FADE_SPAN_MS: f32 = 300.0;

/// One point on a weapon: a bone of the weapon's own skeleton and a position
/// in model space, in the file's axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailPoint {
    pub bone: u16,
    pub position: [f32; 3],
}

/// The two edges of a weapon's trail: the `$WTB` and `$WTT` events every
/// melee weapon model carries. A sword's bottom point is on the blade near
/// the guard and its top point at the tip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailPoints {
    pub bottom: TrailPoint,
    pub top: TrailPoint,
}

impl TrailPoints {
    /// Both points, or `None` for a model that lacks either. The client draws
    /// no trail on a model without both.
    pub fn of(events: &[M2Event]) -> Option<TrailPoints> {
        let point = |id: &[u8; 4]| {
            events.iter().find(|e| &e.id == id).map(|e| TrailPoint {
                // The event's bone is a 16-bit index followed by 16 bits of
                // padding.
                bone: (e.bone & 0xffff) as u16,
                position: e.position,
            })
        };
        Some(TrailPoints {
            bottom: point(b"$WTB")?,
            top: point(b"$WTT")?,
        })
    }
}

/// The clock of one trail on one weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrailClock {
    /// The strip's current opacity, out of 255.
    pub alpha: u8,
    start_ms: u64,
    duration_ms: u32,
    last_ms: u64,
}

/// What one frame of a trail does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrailFrame {
    /// Whether this frame records a new pair. False once the kit's length has
    /// passed; the strip already recorded is still drawn.
    pub lay: bool,
    /// How many of the newest pairs to draw.
    pub pairs: usize,
    /// The opacity at the start of this frame, and how much less each pair
    /// is drawn at than the one before it.
    pub alpha: u8,
    pub step: u8,
}

impl TrailFrame {
    /// The opacity of the `i`th drawn pair, oldest first.
    pub fn pair_alpha(&self, i: usize) -> u8 {
        let fall = usize::from(self.step) * (i + 1);
        usize::from(self.alpha).saturating_sub(fall) as u8
    }
}

impl TrailClock {
    pub fn new(alpha: u8, duration_ms: u32, now_ms: u64) -> TrailClock {
        TrailClock {
            alpha,
            start_ms: now_ms,
            duration_ms,
            last_ms: now_ms,
        }
    }

    /// Advances to `now_ms`. `None` when the trail is over.
    pub fn advance(&mut self, now_ms: u64) -> Option<TrailFrame> {
        let elapsed = now_ms.saturating_sub(self.start_ms);
        let dt = now_ms.saturating_sub(self.last_ms);
        self.last_ms = now_ms;
        let raw = (dt as f32 / FADE_SPAN_MS * f32::from(self.alpha)) as u32;
        // The client keeps the step in a byte. A frame long enough to push it
        // past 255 is clamped here rather than wrapped.
        let step = raw.clamp(1, 255) as u8;
        let pairs = usize::from(self.alpha / step);
        if pairs <= 1 {
            return None;
        }
        let frame = TrailFrame {
            lay: elapsed < u64::from(self.duration_ms),
            pairs: pairs.min(RING_PAIRS),
            alpha: self.alpha,
            step,
        };
        if elapsed > u64::from(self.duration_ms / 2) {
            self.alpha -= step;
        }
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &[u8; 4], bone: u32, position: [f32; 3]) -> M2Event {
        M2Event {
            id: *id,
            data: 0,
            bone,
            position,
            times: Vec::new(),
        }
    }

    /// `Sword_2H_Claymore_B_01.m2` as shipped: `$WTB` on bone 6 at 0.274 and
    /// `$WTT` on bone 7 at 1.414 along the blade.
    #[test]
    fn a_weapon_s_two_points_are_its_two_events() {
        let events = [
            event(b"$WTB", 6, [0.274, 0.0, 0.022]),
            event(b"$WTT", 7, [1.414, 0.0, 0.025]),
        ];
        let points = TrailPoints::of(&events).expect("both points");
        assert_eq!(points.bottom, TrailPoint { bone: 6, position: [0.274, 0.0, 0.022] });
        assert_eq!(points.top.bone, 7);
        assert_eq!(TrailPoints::of(&events[..1]), None, "one point is no trail");
    }

    /// The padding half of the bone word is not part of the index.
    #[test]
    fn the_bone_is_the_low_sixteen_bits() {
        let events = [
            event(b"$WTB", 0x0001_0005, [0.0; 3]),
            event(b"$WTT", 4, [0.0; 3]),
        ];
        assert_eq!(TrailPoints::of(&events).map(|p| p.bottom.bone), Some(5));
    }

    /// Charge's trail at 60 frames a second: opacity 100 over 1,000 ms. The
    /// step is 5, so 20 pairs are drawn; the opacity holds for the first
    /// half and then falls by the step each frame until it no longer divides.
    #[test]
    fn a_trail_covers_three_hundred_milliseconds_and_runs_down_after_half_its_length() {
        let mut clock = TrailClock::new(100, 1000, 0);
        let first = clock.advance(16).expect("running");
        assert_eq!(first, TrailFrame { lay: true, pairs: 20, alpha: 100, step: 5 });
        assert_eq!(first.pair_alpha(0), 95);
        assert_eq!(first.pair_alpha(19), 0);
        assert_eq!(clock.alpha, 100, "no fall before half the length");

        let mut now = 16;
        while now < 496 {
            now += 16;
            clock.advance(now).expect("running");
        }
        assert_eq!(clock.alpha, 100);
        now += 16;
        clock.advance(now).expect("running");
        assert!(clock.alpha < 100, "falls after half the length");

        let mut frames = 0;
        while clock.advance(now + 16).is_some() {
            now += 16;
            frames += 1;
            assert!(frames < 1000, "the trail ends");
        }
        assert!(now > 1000, "a trail outlasts its laying");
    }

    #[test]
    fn a_trail_lays_only_for_its_length() {
        let mut clock = TrailClock::new(100, 100, 0);
        assert!(clock.advance(50).expect("running").lay);
        assert!(!clock.advance(110).map_or(true, |f| f.lay));
    }
}
