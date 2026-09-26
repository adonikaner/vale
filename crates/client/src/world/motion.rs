//! Positions between simulation steps.
//!
//! The session thread steps the world about forty times a second and the screen
//! redraws sixty or more, so applying positions as they land makes everything in
//! the world — the player and therefore the camera included — advance in visible
//! steps.
//!
//! This is **interpolation, not extrapolation**. The drawn position lags the
//! simulation by a fixed fraction of a step; the alternative predicts ahead and
//! has to be corrected whenever the prediction is wrong, and a correction is
//! exactly the jerk this removes. `ObjectManager::advance` already extrapolates
//! the server's 500 ms broadcasts down to step rate, so this only smooths the
//! last step.
//!
//! ## Everything is stamped on the simulation's clock, and drawn behind it
//!
//! Each reading is kept with the `SessionStatus::world_ms` it was taken at, and
//! the renderer draws whatever the world looked like at
//! `world_ms + (real time since that reading) - a fixed delay`. That is the
//! whole design, and each half of it is load-bearing:
//!
//! * **stamping on the world clock** is what makes an uneven simulation draw
//!   evenly. The steps are uneven — `thread::sleep` on Windows is quantised to
//!   ~15.6 ms, so a loop asking for 25 gets 16 and 31 alternately — but a
//!   position is a function of *world* time, and world time and real time run at
//!   the same rate. Sample that function on a clock that advances smoothly and
//!   the drawn motion is smooth however lumpy the steps were.
//! * **the delay** is what keeps that sample bracketed by two readings that have
//!   actually arrived, so it is always an interpolation. Without it the newest
//!   reading is the best that can be done and the entity parks on it for the
//!   rest of the step.
//!
//! The version this replaced sized each leg by the step it had just been told
//! about and ran it from *now*, which is a subtly different thing: the window
//! describes the step that has happened where it needs to cover the one that has
//! not. Under alternating 16/31 ms steps that draws a constant 7.6 y/s run at
//! anything from 6 to 8.3, because a long step finishes its short window early
//! and parks while a short step gets 43% of the way through a long one. It reads
//! as the character juddering along the ground, which is indistinguishable from
//! a dozen other things and is why this module now has a test that states the
//! property directly.
//!
//! There is deliberately **no `speed_of`**. Differencing two interpolated
//! positions looks like the natural way to ask "is this moving?", and it cannot
//! be made steady — see [`crate::session`] and `Playback` for what replaced it.

use vale_protocol::state::movement::shortest_turn;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

/// One reading of where an entity was, stamped on the simulation's own clock.
#[derive(Clone, Copy)]
struct Sample {
    position: Vec3,
    facing: f32,
    /// `SessionStatus::world_ms` when the simulation had it here.
    at_ms: u64,
}

/// How many readings to keep per entity.
///
/// Four at ~25 ms is 100 ms of history against a delay of under 40, so the
/// bracket is always there even after a couple of slow steps. It is a fixed cost
/// of 32 bytes an entity and there is no reason to be tighter.
const HISTORY: usize = 4;

/// How far behind the simulation to draw, in steps.
///
/// One step exactly would put the sample on the newest reading, where any step
/// longer than average runs past the end and parks — the very stall this exists
/// to avoid. Half a step of headroom covers the scheduler's usual spread; the
/// price is that half-step of extra lag, which is ~12 ms and *constant*, and
/// constant lag is invisible where a stutter is not.
const DELAY_STEPS: f64 = 1.5;

/// Milliseconds, until a real step has been measured.
const NOMINAL_STEP_MS: f64 = 25.0;

/// How far a reading may move an entity in one step and still be interpolated
/// to, in yards.
///
/// **A jump is not a correction and must not be drawn as one.** Everything this
/// module smooths is something that *travelled*: a spline at 60 y/s over a
/// quarter-second slice is 15 yards, and the largest correction the server makes
/// to a moving unit is metres. Two things in the world move without travelling,
/// and both are transports:
///
/// * a **same-map transport teleport** — the Grom'Gol–Undercity zeppelin jumps
///   13,700 yards at 173.4 s into its cycle, with no packet behind it;
/// * a **transport that crosses to the other continent**, whose entity this
///   client now keeps (`ObjectManager::leave_map`) because the far side never
///   states it again.
///
/// Interpolating across either draws the deck somewhere between the two places
/// for a frame and a half — and the deck is not only drawn from this reading,
/// it is *stood on*: `entities::solid` builds the hull where `Motion` says, and
/// `Standing::platform` hands that placement to the ferry. A passenger is then
/// carried to a point in the middle of the ocean. That is what
/// *"it teleported me to a random unreachable spot"* was.
///
/// 250 yards is three orders of magnitude under either jump and an order of
/// magnitude over any correction, so nothing that travels can reach it.
const TELEPORT_JUMP: f32 = 250.0;

/// How much of a new measurement goes into the smoothed step.
///
/// **Deliberately tiny, and this is the subtle part.** The delay is *subtracted*
/// from the drawn clock, so any wobble in the delay is a wobble in the drawn
/// position — the exact fault this module exists to remove, reintroduced by the
/// mechanism meant to remove it. A brisk average tracks the alternation it is
/// averaging: at 0.2 it oscillates by ±1.4 ms against a 16/31 schedule, which is
/// ±0.01 yards at a run and comes out as a 0.6 y/s error on a 7.6 y/s run — half
/// of what the whole rewrite was worth. At 0.02 the same schedule moves it by
/// 0.15 ms and it settles on the mean over about a second, which is far quicker
/// than the step rate ever really changes.
const STEP_SMOOTHING: f64 = 0.02;

/// Bounds on a measured step. The ceiling keeps one stall from putting the whole
/// world permanently behind; the floor keeps a zero-length step out of the
/// delay.
const STEP_BOUNDS: (f64, f64) = (4.0, 250.0);

/// Where the render clock stands against the simulation's.
struct Clock {
    /// The newest `world_ms` seen…
    world_ms: u64,
    /// …and the render time at which it was seen. Both clocks run at wall-clock
    /// rate, so the difference between them stays put and world time can be read
    /// off the render clock between polls.
    at: f32,
}

/// Per-entity interpolation state, keyed by GUID.
#[derive(Resource, Default)]
pub struct Motion {
    /// Newest reading last.
    tracks: HashMap<u64, Vec<Sample>>,
    clock: Option<Clock>,
    /// The simulation step, smoothed, in milliseconds. Only the *delay* is sized
    /// from this — a wrong value costs a little lag, never a stall, because the
    /// bracket is found by the stamps rather than by a predicted window.
    step_ms: f64,
}

impl Motion {
    pub fn reset(&mut self) {
        *self = Motion::default();
    }

    /// Take a new reading of the world.
    ///
    /// `entities` is `(guid, position, orientation)` in WoW coordinates, and
    /// `world_ms` is the simulation clock those positions were taken on —
    /// `SessionStatus::world_ms`, not a render timestamp. Anything absent from
    /// the reading is forgotten.
    pub fn track(
        &mut self,
        now: f32,
        world_ms: u64,
        entities: impl Iterator<Item = (u64, Vec3, f32)>,
    ) {
        match &self.clock {
            // A reconnect restarts the session thread's clock at zero, which
            // would leave every stamp in the history in the future.
            Some(clock) if world_ms < clock.world_ms => self.tracks.clear(),
            Some(clock) => {
                let step = (world_ms - clock.world_ms) as f64;
                let step = step.clamp(STEP_BOUNDS.0, STEP_BOUNDS.1);
                let previous = if self.step_ms > 0.0 { self.step_ms } else { NOMINAL_STEP_MS };
                self.step_ms = previous * (1.0 - STEP_SMOOTHING) + step * STEP_SMOOTHING;
            }
            None => self.step_ms = NOMINAL_STEP_MS,
        }
        self.clock = Some(Clock { world_ms, at: now });

        let mut seen: HashSet<u64> = HashSet::default();
        for (guid, position, facing) in entities {
            seen.insert(guid);
            let track = self.tracks.entry(guid).or_default();
            // **A jump is not a correction** — see [`TELEPORT_JUMP`]. Forgetting
            // the history leaves the new reading as the only one, and
            // `interpolate` answers the nearest reading outside a bracket, so
            // the entity is drawn where it now is rather than on the way there.
            if track
                .last()
                .is_some_and(|last| last.position.distance(position) > TELEPORT_JUMP)
            {
                track.clear();
            }
            track.push(Sample { position, facing, at_ms: world_ms });
            if track.len() > HISTORY {
                track.remove(0);
            }
        }
        self.tracks.retain(|guid, _| seen.contains(guid));
    }

    /// Where to draw an entity this frame, or `None` if it is not tracked.
    pub fn position_of(&self, guid: u64, now: f32) -> Option<Vec3> {
        self.at(guid, now).map(|(position, _)| position)
    }

    pub fn facing_of(&self, guid: u64, now: f32) -> Option<f32> {
        self.at(guid, now).map(|(_, facing)| facing)
    }

    fn at(&self, guid: u64, now: f32) -> Option<(Vec3, f32)> {
        let track = self.tracks.get(&guid)?;
        Some(interpolate(track, self.drawn_ms(now)?))
    }

    /// The world time being drawn: the newest reading's clock, carried forward
    /// by the real time since it arrived, less the play-out delay.
    fn drawn_ms(&self, now: f32) -> Option<f64> {
        let clock = self.clock.as_ref()?;
        let since = f64::from(now - clock.at) * 1000.0;
        Some(clock.world_ms as f64 + since - self.step_ms * DELAY_STEPS)
    }
}

/// The two readings bracketing `t`, interpolated. Outside the history at either
/// end, the nearest reading — which is the honest answer, not a guess.
fn interpolate(track: &[Sample], t: f64) -> (Vec3, f32) {
    let (Some(first), Some(last)) = (track.first(), track.last()) else {
        return (Vec3::ZERO, 0.0);
    };
    if t <= first.at_ms as f64 {
        return (first.position, first.facing);
    }
    if t >= last.at_ms as f64 {
        return (last.position, last.facing);
    }
    for pair in track.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if t <= b.at_ms as f64 {
            let span = (b.at_ms - a.at_ms) as f64;
            let p = if span > 0.0 {
                (((t - a.at_ms as f64) / span) as f32).clamp(0.0, 1.0)
            } else {
                1.0
            };
            return (
                a.position.lerp(b.position, p),
                a.facing + shortest_turn(a.facing, b.facing) * p,
            );
        }
    }
    (last.position, last.facing)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: u64 = 7;
    /// What the session thread aims for, in milliseconds.
    const STEP: u64 = 25;

    fn at(x: f32) -> Vec3 {
        Vec3::new(x, 0.0, 0.0)
    }

    /// Run `steps` of simulation at a constant speed, handing each one to
    /// `Motion` at the render time it would have arrived, and return what was
    /// drawn at each of those moments.
    ///
    /// `steps` is the *uneven* schedule the scheduler actually produces.
    fn walk(speed: f32, steps: &[u64]) -> Vec<(f32, f32)> {
        let mut m = Motion::default();
        let (mut world_ms, mut now, mut travelled) = (0_u64, 0.0_f32, 0.0_f32);
        m.track(now, world_ms, [(GUID, at(0.0), 0.0)].into_iter());

        let mut drawn = Vec::new();
        for step in steps {
            world_ms += step;
            now += *step as f32 / 1000.0;
            travelled += speed * (*step as f32 / 1000.0);
            m.track(now, world_ms, [(GUID, at(travelled), 0.0)].into_iter());
            drawn.push((now, m.position_of(GUID, now).expect("tracked").x));
        }
        drawn
    }

    /// **The bug this module was rewritten for.** A simulation stepping at an
    /// uneven rate — which is every simulation on Windows, where `thread::sleep`
    /// is quantised to ~15.6 ms — must still be *drawn* at an even one.
    ///
    /// The previous design sized each leg by the step it had just been told
    /// about and ran it from the moment it was told, so a long step finished its
    /// short window early and parked while a short step got part-way through a
    /// long one. A 7.6 y/s run came out anywhere between 6 and 8.3.
    ///
    /// **The bound is a distance, not a speed**, because a distance is what the
    /// eye sees: a speed differenced over the shortest interval in the schedule
    /// divides a tiny positional wobble by 16 ms and reports a frightening
    /// number for something under two millimetres. Held against the same
    /// schedule, the design this replaced put the character up to **2 cm** off
    /// where it should be, twenty-odd times a second, in alternating directions;
    /// this one is a tenth of that, and one centimetre separates them cleanly.
    #[test]
    fn an_uneven_simulation_is_still_drawn_at_an_even_speed() {
        const SPEED: f32 = 7.6; // yards a second, a run
        // 16 and 31 ms alternating: what Windows hands a loop asking for 25.
        let schedule: Vec<u64> = [16_u64, 31].iter().cycle().take(40).copied().collect();
        let drawn = walk(SPEED, &schedule);

        // Skip the first few while the history fills and the step is measured.
        for pair in drawn[8..].windows(2) {
            let (before, after) = (pair[0], pair[1]);
            let travelled = after.1 - before.1;
            let expected = SPEED * (after.0 - before.0);
            assert!(
                (travelled - expected).abs() < 0.01,
                "drew {travelled} yards where the simulation moved {expected}"
            );
        }
    }

    /// The same, on an even schedule — the case that was already fine, kept so a
    /// change that fixes the uneven one by breaking this is not mistaken for a
    /// success.
    #[test]
    fn an_even_simulation_is_drawn_at_an_even_speed() {
        const SPEED: f32 = 7.6;
        let drawn = walk(SPEED, &[STEP; 30]);
        for pair in drawn[8..].windows(2) {
            let (before, after) = (pair[0], pair[1]);
            let travelled = after.1 - before.1;
            let expected = SPEED * (after.0 - before.0);
            assert!((travelled - expected).abs() < 0.001, "drew {travelled} of {expected}");
        }
    }

    /// The drawn position must never park on the newest reading, or the frames
    /// between polls are frozen. It is the delay that guarantees this: the
    /// sampled moment stays *behind* the readings that have arrived.
    #[test]
    fn the_drawn_moment_stays_behind_the_newest_reading() {
        let mut m = Motion::default();
        for i in 0..8_u64 {
            m.track(
                i as f32 * 0.025,
                i * STEP,
                [(GUID, at(i as f32), 0.0)].into_iter(),
            );
        }
        // Right after a reading, and again just before the next is due.
        let newest = 7.0 * STEP as f64;
        for offset in [0.0_f32, 0.020] {
            let drawn = m.drawn_ms(7.0 * 0.025 + offset).expect("a clock");
            assert!(
                drawn < newest,
                "drawing {drawn} against a newest reading of {newest}"
            );
        }
    }

    /// A reading that has not been overtaken yet is still drawn on the way to
    /// it, rather than snapping.
    #[test]
    fn an_entity_interpolates_towards_its_target() {
        let mut m = Motion::default();
        m.track(0.0, 0, [(GUID, at(0.0), 0.0)].into_iter());
        m.track(0.025, STEP, [(GUID, at(10.0), 0.0)].into_iter());
        m.track(0.050, STEP * 2, [(GUID, at(20.0), 0.0)].into_iter());

        let p = m.position_of(GUID, 0.050).expect("tracked");
        assert!(p.x > 0.0 && p.x < 20.0, "not between two readings: {p:?}");
    }

    /// A correction the simulation makes — a teleport ack, a resync — is drawn
    /// as a slide between the two readings that bracket it, not as a snap.
    #[test]
    fn a_correction_is_drawn_between_the_readings_around_it() {
        let mut m = Motion::default();
        m.track(0.0, 0, [(GUID, at(0.0), 0.0)].into_iter());
        m.track(0.025, STEP, [(GUID, at(0.2), 0.0)].into_iter());
        // The server put us somewhere else.
        m.track(0.050, STEP * 2, [(GUID, at(50.0), 0.0)].into_iter());

        let mid = m.position_of(GUID, 0.050).expect("tracked");
        assert!(mid.x < 50.0, "snapped straight to the correction: {mid:?}");
        // …and it does arrive, rather than crawling for the rest of the session.
        let later = m.position_of(GUID, 0.090).expect("tracked");
        assert!((later.x - 50.0).abs() < 1e-3, "never arrived: {later:?}");
    }

    #[test]
    fn facing_crosses_the_seam_the_short_way() {
        use std::f32::consts::PI;
        // From just before 2π to just after 0 is a small step forwards, not a
        // near-full turn backwards.
        let delta = shortest_turn(0.1, PI * 2.0 - 0.1);
        assert!(delta < 0.0, "took the long way: {delta}");
        assert!((delta + 0.2).abs() < 1e-5, "{delta}");
    }

    #[test]
    fn an_entity_absent_from_a_reading_is_forgotten() {
        let mut m = Motion::default();
        m.track(0.0, 0, [(GUID, at(0.0), 0.0)].into_iter());
        m.track(0.025, STEP, std::iter::empty());
        assert!(m.position_of(GUID, 0.025).is_none());
    }

    /// Only `HISTORY` readings are kept, so a long session cannot grow a list
    /// per entity without bound.
    #[test]
    fn the_history_is_bounded() {
        let mut m = Motion::default();
        for i in 0..50_u64 {
            m.track(
                i as f32 * 0.025,
                i * STEP,
                [(GUID, at(i as f32), 0.0)].into_iter(),
            );
        }
        assert_eq!(m.tracks[&GUID].len(), HISTORY);
    }

    /// **A jump is drawn where it lands, not on the way there.** The population
    /// is the transports: a same-map teleport moves a zeppelin 13,700 yards
    /// with no packet behind it, and the deck is *stood on* as well as drawn —
    /// so a frame of interpolation across it puts a passenger in the sea.
    #[test]
    fn a_reading_that_jumps_is_not_interpolated_to() {
        let mut m = Motion::default();
        m.track(0.0, 0, [(GUID, at(0.0), 0.0)].into_iter());
        m.track(0.025, STEP, [(GUID, at(1.0), 0.0)].into_iter());
        // …and then it is somewhere else entirely.
        m.track(0.050, STEP * 2, [(GUID, at(13_700.0), 0.0)].into_iter());
        m.track(0.075, STEP * 3, [(GUID, at(13_701.0), 0.0)].into_iter());
        let p = m.position_of(GUID, 0.075).expect("tracked");
        assert!(
            p.x > 13_000.0,
            "drawn on the way across the jump, at {p:?}"
        );
    }

    /// …and an ordinary correction still slides, which is what the bound is
    /// sized against.
    #[test]
    fn a_correction_under_the_bound_is_still_interpolated() {
        let mut m = Motion::default();
        m.track(0.0, 0, [(GUID, at(0.0), 0.0)].into_iter());
        m.track(0.025, STEP, [(GUID, at(0.2), 0.0)].into_iter());
        m.track(0.050, STEP * 2, [(GUID, at(200.0), 0.0)].into_iter());
        let mid = m.position_of(GUID, 0.050).expect("tracked");
        assert!(mid.x < 200.0, "snapped a correction: {mid:?}");
    }

    /// A reconnect restarts the simulation clock at zero, and stamps from the
    /// old session would then all be in the future — freezing the world.
    #[test]
    fn a_restarted_session_clock_does_not_freeze_the_world() {
        let mut m = Motion::default();
        for i in 0..4_u64 {
            m.track(i as f32 * 0.025, 5_000 + i * STEP, [(GUID, at(i as f32), 0.0)].into_iter());
        }
        // A new session: the clock is back at zero.
        m.track(0.2, 0, [(GUID, at(99.0), 0.0)].into_iter());
        m.track(0.225, STEP, [(GUID, at(99.0), 0.0)].into_iter());
        let p = m.position_of(GUID, 0.225).expect("tracked");
        assert!((p.x - 99.0).abs() < 1e-3, "stuck in the old session at {p:?}");
    }
}
