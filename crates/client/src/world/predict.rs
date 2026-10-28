//! The local player, drawn at the position the held keys give it this frame
//! rather than at the position of the last simulation step.
//!
//! ## Why the local player is not drawn through `Motion`
//!
//! Every other entity is drawn behind the simulation on purpose:
//! [`crate::world::motion`] stamps every reading with the session thread's own
//! clock and samples a step and a half back, so the sample always lies between
//! two readings that have arrived. That suits a creature forty yards away,
//! whose position comes from the server twice a second.
//!
//! It does not suit the entity the player is steering. Pressing W puts a
//! `Controls` on a channel; the session thread picks it up on its next tick (up
//! to ~31 ms later, ~12 ms on average); the step that results is published;
//! `poll_world` hands it to `Motion`; and `Motion` draws it 1.5 smoothed steps
//! later, another ~35 ms. The character therefore began to move 45 to 65 ms
//! after the key went down, three to six frames, and all of that time is spent
//! in machinery that smooths the movement of other entities. No packet and no
//! server is involved in that delay; it is this client waiting for itself.
//!
//! The 1.12.1 client has no such delay: it simulates its own character in the
//! frame it reads the key and tells the server afterwards. The local player is
//! the one thing a WoW client is authoritative about, and the `MSG_MOVE_*`
//! protocol and the anticheat behind it exist to report and check it.
//!
//! ## What it does
//!
//! Each frame this pass takes the last reading the session published and
//! continues it, turning by the turn keys and striding along the heading, with
//! [`vale_protocol::state::movement::turned`] and
//! [`vale_protocol::state::movement::strode`]. These are the two expressions
//! `Mover::advance` integrates, and they are free functions so that both sides
//! can call them. A prediction that integrates a slightly different expression
//! diverges every step and has to be corrected, and each correction is a
//! visible jump. Integrating the identical expression from the same base makes
//! the two agree to floating-point precision, so a reading arriving 25 ms later
//! lands on the position already drawn and nothing snaps.
//!
//! ## A reading is stamped with the time it was taken, not the time it was seen
//!
//! This stamp is what keeps the continuation even. The first version of this
//! module stamped the time the reading was seen, and the drawn character
//! juddered.
//!
//! The base is a position and a moment, and the drawn position is
//! `base + velocity * (now - moment)`. The session thread steps every ~25 ms and
//! the renderer draws every ~16 ms, so a step is noticed between 0 and one frame
//! after it was taken, by a different amount each time. If the moment is the
//! frame that noticed the reading, the drawn position advances by the interval
//! between notices while the base it restarts from advances by the interval
//! between steps. Those are two clocks of similar rate beating against each
//! other, the fault [`crate::world::motion`]'s module comment describes, and no
//! arithmetic further down can remove it. With a 25 ms step and a 60 Hz frame
//! the result is a repeating stutter between half speed and one and a half
//! times speed, about 6 cm at a run, and since the camera is framed on the same
//! position the whole view stutters with it.
//!
//! [`vale_protocol::socket::session::SessionStatus::taken`] therefore carries
//! the `Instant` at which the simulation stood where the reading says, and the
//! base holds `now - age` rather than `now`. A reading that arrives late is
//! drawn where it was at the time it names, which is the position already on
//! screen.
//!
//! Once there is a base and the character is on the ground, this pass always
//! returns a position. The first version returned `None` when no time had
//! elapsed since the base, which was true on every frame that adopted one, so
//! the local player fell back to `Motion`'s play-out delay for that frame and
//! returned to the prediction on the next. That was a ~50 ms jump each way,
//! twenty times a second: the same fault as above, an order of magnitude
//! larger.
//!
//! ## Why the jump arc is continued here too
//!
//! An arc's horizontal velocity is fixed at take-off and 1.12 gives the player
//! no control over it, so continuing the arc here removes no input latency. It
//! is continued because the two paths draw different moments: this one draws
//! `now` and `Motion` draws a step and a half back. Handing the character from
//! one to the other mid-jump puts a discontinuity at each end, ~0.28 yards at a
//! run.
//!
//! When the arc was left to `Motion`, the drawn position at take-off dropped
//! back to where it was 40 ms earlier and covered that ground again. At landing
//! the flag said the character was down while the drawn position was still 40 ms
//! of descent above the ground, so the landing animation played in mid-air and
//! the character then snapped to the ground. Neither is latency, and smoothing
//! cannot remove either: both come from joining two clocks, and this pass avoids
//! the join.
//!
//! An arc is continued on the same terms `Mover::advance` walks it: the
//! horizontal is `cos/sin * xyspeed` from the jump block, fixed, and the
//! vertical is [`vale_protocol::state::movement::fall_elevation`], the same
//! function on both ends of the socket, shared here rather than copied so that
//! the two cannot diverge. The block does not carry the arc's start, but it
//! carries `fallTime`, so the start is recovered as the current height plus
//! the distance already fallen. That makes `dt == 0` return the reading
//! exactly.
//!
//! This pass does not decide where the arc ends. A descending arc is clamped to
//! the floor under it and never drawn below it, the same test `Mover::advance`
//! makes; the landing itself is the mover's decision. The drawn character can
//! therefore touch down up to one step before the reading does, and never a
//! step after.
//!
//! ## The mouse's heading is adopted rather than continued
//!
//! The stride, the arc and the deck are quantities the reading carries, and
//! this pass advances them by `dt`. A right-drag's heading is not one of those.
//! It is a command: `camera::orbit` writes [`crate::world::camera::CameraRig`]
//! every frame from the mouse, `input::controls::send_input` aims the character
//! half a turn from it and puts that on the channel, the session thread drains
//! it at the top of its next tick, and the result comes back in a reading.
//! Between readings there is no turn key for [`turned`] to integrate, so the
//! continuation returned the reading's orientation unchanged on every frame of
//! the tick.
//!
//! At 1.12's 60 fps that is one frame in one and a half. This client draws at
//! 100 to 250 fps, where it is three to ten frames with no rotation followed by
//! one frame with the whole 25 ms of it, while the camera turns every frame
//! because the rig holds the accumulated angle. The stride steps with the
//! heading, since [`strode`] takes its direction from the same field.
//!
//! [`Predicted::facing`] holds the commanded heading. It is adopted in the frame
//! it is commanded and dropped by the reading that confirms it; see that field,
//! and [`ACK_MARGIN`] for the clock the confirmation is measured on.
//!
//! ## A passenger is continued in the deck's frame, not the world's
//!
//! This is the largest of the three corrections, because a passenger's velocity
//! is not in the movement block at all. A character standing on a moving
//! platform has a world position equal to the deck's placement composed with
//! an offset, and the two halves come from different threads at different
//! rates: the session steps the offset ~40 times a second and
//! [`crate::world::motion`] draws the deck at frame rate, a step and a half
//! behind the session's last publication.
//!
//! A continuation that knew only the keys drew the character standing still at
//! the last reading and then moved them by however far the deck had travelled
//! when the next reading arrived. On the Deeprun Tram, whose car covers 2,482
//! yards at ~17 y/s, that is 0.43 yards forty times a second, and since the
//! camera is framed on this position the whole view moves with it. The visible
//! result was models jittering back and forth fast enough to look tripled.
//!
//! [`reframe`] fixes this without new arithmetic. The offset is recovered
//! against the placement the reading was taken at
//! ([`vale_protocol::state::movement::Ferry::platform`], published with the
//! reading for this purpose) and composed again with `Motion`'s placement of
//! the same guid this frame, which is the placement the deck's own model is
//! drawn at one system later. The passenger then stays on the drawn deck
//! whatever either clock is doing.
//!
//! ## What this pass does not do
//!
//! * It does not predict the server. A resync, a knockback, a teleport and a
//!   root all arrive as ordinary readings and become the next base. The
//!   prediction is never more than one step ahead of a reading, so a
//!   correction is drawn within a step of arriving.
//! * It does not skip the world. The stride is offered to the same
//!   [`CollisionWorld`] and the same terrain the session thread walks on, in
//!   the same order (the wall shortens the stride before the height is looked
//!   up), so running into a building stops the drawn character where the
//!   simulation stops. Without that, the prediction would push a step into the
//!   wall every frame and every reading would pull it back out, a 20 cm buzz
//!   against every surface in the world.
//!
//! [`CollisionWorld`]: vale_assets::CollisionWorld

use crate::world::session::{Session, Solids};
use vale_protocol::state::movement::{
    fall_elevation, move_flags, strode, turned, wrap_angle, Controls, Ferry, MovementInfo, Platform,
    Restraint, RideCurve, Speeds, MAX_STRIDE,
};

/// The most pieces one frame's stride is walked in; the mover's own cap, for
/// the same reason. See [`MAX_STRIDE`].
const MAX_PIECES: u32 = 64;
use vale_protocol::socket::session::World;
use vale_protocol::state::update::Position;
use bevy::prelude::*;

/// The flags a keypress owns, and the only ones the prediction rewrites.
///
/// Every other flag in the block (`SWIMMING`, `JUMPING`, `ROOT`, the transport
/// bits) is set by the server or the mover and is carried through unchanged. A
/// prediction that cleared them would, a step at a time, draw a swimming
/// character walking or a rooted one moving.
const KEYED: u32 = move_flags::FORWARD
    | move_flags::BACKWARD
    | move_flags::STRAFE_LEFT
    | move_flags::STRAFE_RIGHT
    | move_flags::WALK_MODE;

/// How far ahead of its base the prediction will run before it holds.
///
/// The session thread steps forty times a second, so a base a quarter of a
/// second old means the thread has stopped: it has stalled, its socket is
/// blocked on a slow server, or the machine swapped. The character then holds
/// mid-stride, which shows that this client no longer knows where it is.
/// Without the cap it would keep striding at run speed for as long as the
/// stall lasted and then be snapped back by the same distance.
const MAX_LEAD: f32 = 0.25;

/// How long past the last mouse-look command a heading stays adopted when no
/// reading has confirmed it.
///
/// [`Predicted::facing`] is dropped by the first reading taken after the command
/// that set it. That is a comparison between two clocks: the render clock this
/// frame is on, and the session thread's `Instant` carried across as an age.
/// Both are wall clocks, so the subtraction is exact to well under a frame, but
/// the margin decides which way an error falls. Dropping the heading one reading
/// early draws the character at an orientation the session has not reached yet,
/// which is a backward jump of up to a tick's worth of turn; holding it one
/// reading late draws them at a heading the simulation is about to confirm. The
/// margin is one session tick so that a clock error costs the second of those.
const ACK_MARGIN: f32 = 0.025;

/// A reading as the session published it: everything a [`Base`] holds except
/// the moment it was taken, which [`Predicted::adopt`] derives from an age.
///
/// The fields are passed as one struct because they arrive together from one
/// `status()` call, and as separate arguments two `bool`s sat next to each
/// other in the call.
struct Reading {
    movement: MovementInfo,
    speeds: Speeds,
    airborne: bool,
    restraint: Restraint,
    riding: Option<[f32; 3]>,
    ride: Option<RideCurve>,
    ferry: Option<Ferry>,
    world_ms: u64,
}

/// The reading the prediction is running from.
struct Base {
    /// The movement block the session published, position and orientation
    /// included.
    movement: MovementInfo,
    speeds: Speeds,
    /// The render clock at the moment the reading was taken: the session
    /// thread's own `Instant`, carried across as an age and subtracted from
    /// this frame's clock, rather than the moment the renderer noticed the
    /// reading. See the module comment: stamping the moment it was noticed
    /// instead makes the continuation stutter twenty times a second.
    at: f32,
    /// Off the ground, in which case the arc is continued rather than the keys.
    airborne: bool,
    /// What the server has stopped this character doing. Carried rather than
    /// re-derived because only part of it is in the block: `ROOT` is, while a
    /// stun and death are descriptor fields the mover reads once a tick.
    restraint: Restraint,
    /// The velocity of a movement the server is driving, such as a Charge or a
    /// taxi flight. Carried for the same reason as `restraint`: it is not in
    /// the block, and a continuation that did not know about it would stride
    /// from keys the player has no control over. A full velocity rather than a
    /// speed, because the block's orientation is horizontal and a flight is
    /// not.
    riding: Option<[f32; 3]>,
    /// The same ride's path and where along it the reading was taken. When
    /// present the continuation samples the path itself, so the drawn position
    /// stays on the curve between readings and the heading turns with it.
    ride: Option<RideCurve>,
    /// The moving platform under the character's feet, and the placement the
    /// reading's position was taken against; see
    /// [`vale_protocol::state::movement::Ferry`].
    ///
    /// Carried for the same reason as `riding`, and it fixes the larger of the
    /// two faults: a passenger's velocity is not in the block. The deck's
    /// travel arrives as a correction to the position on each simulation step,
    /// so a continuation that only knows the keys draws the character standing
    /// still for a whole step and then moving by however far the platform
    /// moved: 0.43 yards forty times a second on the Deeprun Tram, with the
    /// camera framed on it. See [`reframe`].
    ferry: Option<Ferry>,
    /// The simulation clock this reading was taken on, so an unchanged reading
    /// is not adopted twice.
    world_ms: u64,
}

/// A heading the mouse has commanded and the session has not confirmed yet.
///
/// The render clock is carried beside the angle because that is what decides
/// when the override ends: the first reading taken after `at` is one that has
/// the command in it. See [`ACK_MARGIN`].
#[derive(Clone, Copy)]
struct Commanded {
    heading: f32,
    at: f32,
}

/// Where the local player is this frame, ahead of the simulation.
#[derive(Resource, Default)]
pub struct Predicted {
    base: Option<Base>,
    /// The keys held at the base's moment, or before the first entry of
    /// [`Self::changes`].
    settled: Controls,
    /// Every change of keys since the base, with the render clock it was made
    /// at, oldest first. The last entry, or [`Self::settled`] when there is
    /// none, is what is held now.
    ///
    /// The keys held now are what removes the wait for the session tick: the
    /// base's own flags are the keys the session thread had received when it
    /// took the step, which excludes anything pressed since. They are kept with
    /// their moments because the session applies each change at the moment it
    /// was made, part-way through its step (`Mover::advance_through`). A
    /// continuation that applied the keys held now to the whole time since the
    /// base would move the character as if the change had happened at the
    /// base's moment. On a strafe reversal that draws the character a step back
    /// by twice the speed times the time since the base, up to 0.4 yards at a
    /// run, in the frame the key changes.
    changes: Vec<(f32, Controls)>,
    /// The heading the mouse-look has commanded, and the render clock it was
    /// commanded at.
    ///
    /// A right-drag writes [`crate::world::camera::CameraRig::yaw`] every frame
    /// and `input::controls::send_input` aims the character half a turn from it, but the
    /// aim reached the drawn model only by way of the channel and the next
    /// reading. The character therefore turned in 25 ms steps while the camera
    /// framing it turned every frame: at 100 to 250 fps, three to ten frames
    /// carrying no rotation followed by one carrying the whole step. `strode`
    /// takes its direction from the same field, so a character running under a
    /// mouse turn travelled along those steps as well as facing along them.
    ///
    /// Adopted in the frame it is commanded, as [`Self::controls`] is, and
    /// dropped by the reading that confirms it. See [`ACK_MARGIN`].
    facing: Option<Commanded>,
    /// This frame's answer, so the placement, the camera and anything else
    /// asking all get one number rather than three samples of a moving one.
    now: Option<Position>,
}

/// What the world is to a step: something to stand on, or something to arrive
/// at.
///
/// The variant decides what the floor does, as it does in `Mover::advance`: a
/// walking character is lifted onto what is underfoot, a descending one is
/// stopped by it, and a rising one does not touch it. Treating rising and
/// falling alike would clamp a jump onto the first ledge it passed within a
/// step of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Contact {
    Ground,
    Rising,
    Falling,
    /// Neither: the server is walking a path of its own and this step is a
    /// continuation of it. The floor is not asked and the stride is not offered
    /// to the collision — see [`Predicted::step`].
    Ridden,
}

/// One frame's continuation of the base.
struct Continued {
    position: Position,
    /// The horizontal still to be offered to the world — the keys' stride on
    /// the ground, the frozen take-off velocity in the air.
    stride: [f32; 2],
    /// The height to draw at when the world has no floor data for this spot.
    ///
    /// The reading's own height, not advanced. `Mover::advance` holds an arc
    /// over a point it has no floor data for rather than walking it (see the
    /// `None` arm in [`resolve`]), so a held arc is continued at the height it
    /// is held at. Running the parabola on regardless would draw the character
    /// sinking, and every reading would pull them back up.
    held_z: f32,
    contact: Contact,
}

impl Predicted {
    /// Where to draw the local player, or `None` to fall back to
    /// [`crate::world::motion::Motion`] — no session, or no reading yet.
    pub fn of(&self) -> Option<Position> {
        self.now
    }

    /// The guid of the platform under the character's feet, or `None` when
    /// there is none, which is most of a session.
    ///
    /// Published for one reader, `camera::follow_player`, which turns the
    /// camera with a turning deck. Everything else about a passenger is handled
    /// inside this module.
    pub fn deck(&self) -> Option<u64> {
        self.base.as_ref()?.ferry.map(|ferry| ferry.guid)
    }

    /// Forget everything. A logout or a teleport across continents: the base
    /// names a place on a map that may no longer be loaded.
    pub fn reset(&mut self) {
        self.base = None;
        self.now = None;
        // The keys held now are kept; their history is about a base that is
        // gone.
        self.settled = self.controls();
        self.changes.clear();
        // Dropped with the base, for the same reason. A logout or a
        // cross-continent teleport ends with the character pointing wherever the
        // server put them. A heading commanded on the old map is one no reading
        // will ever confirm, so holding it would draw the arrival facing the
        // wrong way until the player next moved the mouse.
        self.facing = None;
    }

    /// Adopt the keys `send_input` is about to hand the session thread, held
    /// from `now` on the render clock.
    pub(crate) fn set_controls(&mut self, controls: Controls, now: f32) {
        if controls != self.controls() {
            self.changes.push((now, controls));
        }
    }

    /// The keys held now.
    fn controls(&self) -> Controls {
        self.changes.last().map_or(self.settled, |&(_, c)| c)
    }

    /// The keys held over `from..to`, as consecutive spans of `(keys,
    /// seconds)`. A change at or before `from` is in force from `from`.
    fn spans(&self, from: f32, to: f32) -> Vec<(Controls, f32)> {
        let mut spans = Vec::new();
        let (mut t, mut keys) = (from, self.settled);
        for &(at, next) in &self.changes {
            if at >= to {
                break;
            }
            if at > t {
                spans.push((keys, at - t));
                t = at;
            }
            keys = next;
        }
        spans.push((keys, (to - t).max(0.0)));
        spans
    }

    /// Adopt the heading `send_input` is about to hand the session thread, at
    /// this frame's render clock.
    ///
    /// Called every frame the mouse controls the heading, not only when it
    /// changes. `send_input` keeps a memo so that an unmoving drag does not put
    /// sixty identical commands a second on the channel, but the memo applies
    /// to the channel only. How long the override lives depends on when it was
    /// last wanted, which is every one of those frames. See [`Self::facing`].
    pub fn set_facing(&mut self, heading: f32, now: f32) {
        self.facing = Some(Commanded { heading: wrap_angle(heading), at: now });
    }

    /// Whether the character may be turned this frame: `Mover::can_turn`, over
    /// the base this pass is running from.
    ///
    /// Public because the input code needs the same answer as the step. In
    /// `input::controls::send_input`, a heading offered to a mover that will
    /// refuse it is never offered again. `true` with no base, since nothing
    /// then forbids a turn.
    pub(crate) fn can_turn(&self) -> bool {
        self.base
            .as_ref()
            .is_none_or(|b| b.riding.is_none() && can_turn(b.restraint))
    }

    /// Adopt a reading, `age` seconds after it was taken.
    ///
    /// `age` is what keeps the continuation even; see the module comment. A
    /// caller that passes zero states that the reading was taken in this frame,
    /// which is never true of a reading a background thread published.
    fn adopt(&mut self, now: f32, age: f32, reading: Reading) {
        let Reading { movement, speeds, airborne, restraint, riding, ride, ferry, world_ms } = reading;
        // A reading taken after the command carries it, so the override ends.
        // `SessionLoop::run` drains the channel at the top of a tick and
        // publishes at the bottom of it, so there is no ordering in which a
        // later reading is missing an earlier command. See [`ACK_MARGIN`] for
        // the margin on the comparison.
        if self.facing.is_some_and(|c| now - age >= c.at + ACK_MARGIN) {
            self.facing = None;
        }
        // A change made at or before the reading's moment is in it, so it
        // becomes the settled keys; the later ones stay to be applied part-way
        // through the continuation.
        let at = now - age;
        let held = self.changes.iter().take_while(|&&(t, _)| t <= at).count();
        if let Some(&(_, keys)) = self.changes[..held].last() {
            self.settled = keys;
        }
        self.changes.drain(..held);
        self.base = Some(Base {
            movement,
            speeds,
            at,
            airborne,
            restraint,
            riding,
            ride,
            ferry,
            world_ms,
        });
    }

    /// This frame's continuation of the base: where the character is and the
    /// stride still to be offered to the world.
    ///
    /// Split from [`advance`] because it holds all of the arithmetic and none of
    /// the world queries. A test can then run a sequence of frames against a
    /// sequence of simulation steps and measure the drawn speed, which this
    /// module exists to keep even.
    ///
    /// `None` only when there is nothing to say: no reading yet.
    fn step(&self, now: f32) -> Option<Continued> {
        let base = self.base.as_ref()?;
        let dt = (now - base.at).clamp(0.0, MAX_LEAD);
        // The commanded heading replaces the reading's before anything reads
        // it, rather than being written over the answer afterwards. `strode`
        // takes its direction from this field, so overriding the orientation
        // after the stride had been computed would draw a character facing one
        // way and running along the last reading's. The session thread composes
        // them in the same order: `drain_commands` applies `Command::Face` at
        // the top of the tick and `tick_movement` strides below it.
        //
        // Gated on the two conditions `Mover::face` refuses on: a restraint (a
        // stun, death) and a ride the server is driving. `send_input` asks
        // [`Self::can_turn`] before it commands at all; this covers the
        // restraint that arrives in a reading after the command went out.
        let mut movement = base.movement;
        if let Some(commanded) = self.facing {
            if base.riding.is_none() && can_turn(base.restraint) {
                movement.position.orientation = commanded.heading;
            }
        }
        // A ride is tested before the keys and the arc, because neither applies
        // while the server is driving: `Mover::advance` returns before it reads
        // them. The mover instead walks a path this pass does not have, so the
        // continuation covers the part it can: the leg being walked, at the
        // ride's own speed, along the heading the mover has already turned to.
        // Over one step that is exact on a straight leg and off by centimetres
        // round a corner, whereas holding the position instead stalls 0.6 yards
        // per tick at a charge's 24 y/s.
        //
        // The ride is not offered to the world. The server computed the path on
        // its own navmesh and this client must not stop the character part-way
        // along it; the ride's real positions arrive with the next reading.
        //
        // The velocity covers all three axes and comes from the spline. The
        // first version walked the block's orientation at the ride's speed,
        // which is exact for a charge across flat ground and wrong for every
        // taxi flight: a bearing has no vertical component, so the drawn
        // altitude held between readings and stepped to each new one, a
        // sawtooth at the simulation's 40 Hz that the whole view followed
        // through the camera. See `Spline::velocity`.
        //
        // The path itself, when the reading carries it, rather than the
        // velocity. A straight line from the last reading leaves the curve on
        // every bend and is pulled back onto it at the next reading, and the
        // heading held between readings turns the camera in 40 Hz steps on a
        // taxi flight's sharp turns. Sampling the path at the reading's time
        // plus `dt` is what the mover will compute at its next step, so the
        // reading lands on the position already drawn.
        if let Some(ride) = &base.ride {
            let (at, heading) = ride.spline.at_secs(ride.elapsed + dt);
            let mut position = movement.position;
            position.x = at[0];
            position.y = at[1];
            position.z = at[2];
            position.orientation = wrap_angle(heading);
            return Some(Continued {
                position,
                stride: [0.0, 0.0],
                held_z: position.z,
                contact: Contact::Ridden,
            });
        }
        if let Some(velocity) = base.riding {
            let mut position = movement.position;
            position.x += velocity[0] * dt;
            position.y += velocity[1] * dt;
            position.z += velocity[2] * dt;
            return Some(Continued {
                position,
                stride: [0.0, 0.0],
                held_z: position.z,
                contact: Contact::Ridden,
            });
        }
        // A root is tested before the arc, because `Mover::set_rooted` drops the
        // arc: rooting clears both `MOVEFLAG_MASK_MOVING` and the airborne
        // state, so a reading that carries both a root and an arc was taken in
        // the step before the root arrived. Continuing its parabola would move
        // the character for a frame after the server has pinned it.
        // The keys are applied span by span, each from the moment it was
        // pressed; see [`Self::changes`].
        let spans = self.spans(base.at, base.at + dt);
        if base.airborne && !movement.has(move_flags::ROOT) {
            return Some(arc(&movement, &base.speeds, &spans, base.restraint, dt));
        }
        let (position, stride) = flat_walk(&movement, &base.speeds, &spans, base.restraint);
        Some(Continued {
            position,
            stride,
            held_z: position.z,
            contact: Contact::Ground,
        })
    }
}

/// The turn and the stride `dt` of held keys produces from one reading, with
/// nothing underfoot and nothing in the way.
///
/// Split out from [`advance`] because it holds all of the arithmetic and none
/// of the world: a test can run it against `Mover::advance` over the same
/// interval and assert that they land in the same place, which is the property
/// the prediction depends on. The caller adds the collision and the floor.
///
/// Only [`KEYED`] is rewritten. The rest of the block is set by the server or
/// the mover; a prediction that cleared `SWIMMING` would run a swimming
/// character.
///
/// Carrying `ROOT` through is not the same as obeying it. `strode` checks
/// `MOVEFLAG_MASK_MOVING`, and rewriting the keyed flags puts `FORWARD` back
/// into a block the server had cleared it from. A stunned, rooted or dead
/// character was therefore predicted forwards at run speed while
/// `Mover::advance`, which returns before its own translation, held the
/// confirmed position where the server put it: the character stayed locked in
/// place while the client drew it moving. Every one of vmangos'
/// `SetRooted(true)` callers (`HandleAuraModStun`, the root auras, and
/// `SetDeathState(JUST_DIED)` for a player) arrives as this one flag.
///
/// The rule is the mover's, unchanged, and it is two rules: a root stops the
/// translation and allows the turn, a stun stops the turn and leaves nothing to
/// translate, and death stops both. See
/// [`vale_protocol::state::movement::Restraint`], which holds both, and which is
/// why this function takes one rather than reading the block twice.
fn flat_step(
    base: &MovementInfo,
    speeds: &Speeds,
    controls: Controls,
    restraint: Restraint,
    dt: f32,
) -> (Position, [f32; 2]) {
    let mut info = *base;
    info.flags = (info.flags & !KEYED) | (controls.to_flags() & KEYED);
    if can_turn(restraint) {
        info.position.orientation = turned(base.position.orientation, speeds, controls, dt);
    }
    if !can_move(base, restraint) {
        return (info.position, [0.0, 0.0]);
    }
    (info.position, strode(&info, speeds, dt))
}

/// [`flat_step`] over consecutive spans of held keys, each span continuing
/// from where the last one left the character: the turn and the stride of a
/// key change applied at the moment it was made, as `Mover::advance_through`
/// applies it.
fn flat_walk(
    base: &MovementInfo,
    speeds: &Speeds,
    spans: &[(Controls, f32)],
    restraint: Restraint,
) -> (Position, [f32; 2]) {
    let mut info = *base;
    let mut stride = [0.0, 0.0];
    for &(keys, dt) in spans {
        let (position, [dx, dy]) = flat_step(&info, speeds, keys, restraint, dt);
        info.position = position;
        stride[0] += dx;
        stride[1] += dy;
    }
    (info.position, stride)
}

/// `Mover::can_move`, over a base rather than over the mover itself.
///
/// The rule is written twice on purpose: the mover owns its own state and the
/// prediction owns a snapshot of it, so there is no mover here to ask. The rule
/// must not drift between the two copies, which is why each is one line next
/// to its own doc.
///
/// `on_taxi` is included for the same reason as `dead`: it is the server's
/// answer about a character this pass draws ahead of the simulation. A copy
/// without it would stride the drawn body around while `Mover` held the
/// confirmed one still, which is the visible part of the fault the flag exists
/// to prevent.
fn can_move(base: &MovementInfo, restraint: Restraint) -> bool {
    !restraint.dead && !restraint.on_taxi && !base.has(move_flags::ROOT)
}

/// `Mover::can_turn`. A stun, death or a taxi flight forbids a turn; a root
/// does not.
fn can_turn(restraint: Restraint) -> bool {
    !restraint.dead && !restraint.stunned && !restraint.on_taxi
}

/// Where a walking character's feet end up given the floor under them:
/// `Mover::advance`'s vertical rule, over a height rather than over the mover.
///
/// A third one-line copy of a mover rule beside [`can_move`] and [`can_turn`],
/// for the same reason: the mover owns its own state and this owns a snapshot
/// of it, so there is no mover here to ask. The rule must not drift, and a test
/// holds this against a real `Mover` walking a real slope.
///
/// A drop deeper than `FALL_THRESHOLD` is left alone, because that is where the
/// mover takes off, and where the arc ends is the mover's decision. Anything
/// shallower is a step, a stair or a slope, and the character is placed on it,
/// going down as well as going up.
fn stood_on(z: f32, floor: f32) -> f32 {
    if floor >= z - vale_protocol::state::movement::FALL_THRESHOLD {
        floor
    } else {
        z
    }
}

/// Put a continued position back onto the platform it was standing on, at the
/// placement the platform is drawn at this frame.
///
/// This is what makes a ride smooth, and the cause it addresses is two clocks,
/// not arithmetic. A passenger's world position is the deck's placement
/// composed with an offset, and the two halves are produced by different
/// threads at different rates: the session steps the offset ~40 times a second
/// and [`crate::world::motion`] draws the deck at frame rate, a step and a half
/// behind the session's last publication. Drawing the passenger from the
/// session's own composition places them against a deck that is somewhere
/// else: the character slides back and forth across the floor by up to the
/// platform's travel in one step, and since the camera is framed on the same
/// position the whole view moves with it. On the Deeprun Tram, at 17 y/s, that
/// is 0.43 yards forty times a second.
///
/// The offset is therefore recovered against the placement the reading was
/// taken at ([`Ferry::platform`]) and composed again with the placement the
/// deck's model is drawn at. The passenger then stays on the drawn deck
/// whatever either clock is doing.
///
/// The stride is rotated by the difference between the two facings for the
/// same reason: it was computed in the world axes of the older placement. Every
/// lift and tram car translates without turning, so the rotation is zero for
/// them, but not for the boats and zeppelins. A ship's heading is `atan2` of
/// its own spline derivative, so it turns through most of a right angle while
/// manoeuvring into a dock, and a stride taken across the deck at that moment
/// is in axes that rotate under it.
fn reframe(ferry: &Ferry, now: Platform, position: Position, stride: [f32; 2]) -> (Position, [f32; 2]) {
    let was = ferry.platform();
    let position = Ferry::aboard(was, position).carry(now);
    let (sin, cos) = (now.facing - was.facing).sin_cos();
    (
        position,
        [
            stride[0] * cos - stride[1] * sin,
            stride[0] * sin + stride[1] * cos,
        ],
    )
}

/// Where an arc in flight has got to `dt` after the reading that described it.
///
/// This is the parabola `Mover::advance` walks and the server walks again to
/// check it: the horizontal velocity is the one fixed at take-off, stored in
/// the jump block, and the vertical is
/// [`vale_protocol::state::movement::fall_elevation`], shared rather than
/// copied so that the two cannot diverge.
///
/// The arc's start is recovered rather than carried. The block states where
/// the character is and how long it has been falling, not where it left the
/// ground, so the starting height is the current height plus the distance
/// already fallen. Computed that way, `dt == 0` returns the reading exactly,
/// so there is no visible join where the continuation starts. Truncating
/// `fallTime` to the millisecond changes the arc's shape by a few millimetres
/// and does not affect continuity.
///
/// The keys are not consulted, because 1.12 gives no air control and the server
/// agrees: `ExtrapolateMovement` walks the jump block and does not read the
/// movement flags while `MOVEFLAG_JUMPING` is set. Turning still applies, as it
/// does in `Mover::advance`, which turns before it branches.
fn arc(
    base: &MovementInfo,
    speeds: &Speeds,
    spans: &[(Controls, f32)],
    restraint: Restraint,
    dt: f32,
) -> Continued {
    let mut position = base.position;
    if can_turn(restraint) {
        position.orientation = spans
            .iter()
            .fold(base.position.orientation, |o, &(keys, dt)| turned(o, speeds, keys, dt));
    }

    // `fall_elevation` takes the down-positive start velocity, which is what
    // the wire carries: a jump reports `zspeed = -7.96`. `Mover::take_off`
    // writes that field from its own up-positive number, so reading it back
    // straight is reading the same value the mover integrates.
    let safe_fall = base.has(move_flags::SAFE_FALL);
    let fallen = base.fall_time as f32 / 1000.0;
    let start_z = base.position.z + fall_elevation(fallen, safe_fall, base.jump.z_speed);
    position.z = start_z - fall_elevation(fallen + dt, safe_fall, base.jump.z_speed);

    Continued {
        position,
        stride: [
            base.jump.cos_angle * base.jump.xy_speed * dt,
            base.jump.sin_angle * base.jump.xy_speed * dt,
        ],
        // The reading's own height, before the parabola moved it: what to draw
        // if the world turns out to have no floor data under the arc.
        held_z: base.position.z,
        // The same test `Mover::advance` makes, against the same previous
        // height: only a descending arc can land, or the first upward frame of
        // a jump would be clamped straight back onto the ground it left.
        contact: if position.z < base.position.z {
            Contact::Falling
        } else {
            Contact::Rising
        },
    }
}

/// Take a new base whenever the simulation has stepped.
///
/// Gated on the simulation's own clock: a base re-taken from an unchanged
/// reading would reset the elapsed time every frame and the prediction would
/// never advance, which looks the same as this module being absent. The
/// `world_ms` read is cheap enough to make the gate affordable sixty times a
/// second; the full status, which clones strings and vectors, is taken only
/// when it has changed.
///
/// The gate and the stamp are read from the same status, not from
/// [`crate::world::entity::WorldClock`], which `poll_world` fills one system
/// earlier. If `world_ms` and `taken` came from different reads, the base's
/// age would be wrong.
pub(super) fn rebase(time: Res<Time>, session: Res<Session>, mut predicted: ResMut<Predicted>) {
    let Some(active) = session.active.as_ref() else {
        predicted.reset();
        return;
    };
    if predicted.base.as_ref().map(|b| b.world_ms) == Some(active.live.world_ms()) {
        return;
    }
    let status = active.live.status();
    if !status.in_world {
        predicted.reset();
        return;
    }
    // How long ago the simulation stood there. Both clocks are wall clocks, so
    // this is a subtraction, not a synchronisation. Without it the base would
    // be stamped with the moment it was noticed.
    let age = status
        .taken
        .map_or(0.0, |taken| taken.elapsed().as_secs_f32());
    predicted.adopt(
        time.elapsed_secs(),
        age,
        Reading {
            movement: status.movement,
            speeds: status.speeds,
            airborne: status.airborne,
            restraint: status.restraint,
            riding: status.riding,
            ride: status.ride.clone(),
            ferry: status.ferry,
            world_ms: status.world_ms,
        },
    );
}

/// Continue the base by however long ago it was taken.
///
/// Runs after [`crate::input::controls::send_input`], so the controls it
/// integrates are this frame's rather than last frame's. This is what removes
/// the session thread's tick from the input latency.
pub(super) fn advance(
    time: Res<Time>,
    session: Res<Session>,
    solids: Res<Solids>,
    motion: Res<crate::world::motion::Motion>,
    mut predicted: ResMut<Predicted>,
) {
    predicted.now = None;
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let now = time.elapsed_secs();
    let Some(step) = predicted.step(now) else {
        return;
    };
    let mut position = step.position;
    let mut held_z = step.held_z;
    let [mut dx, mut dy] = step.stride;
    // A passenger is drawn against the deck rather than against the world; see
    // [`reframe`] for the reason. The placement is taken from `Motion` rather
    // than from the collision store because `Motion` holds the placement the
    // platform's model is drawn at this frame, so the two cannot separate on
    // screen. `None` (a platform out of view, or one `Motion` has no reading
    // of yet) leaves the session's own composition alone, which is a step
    // behind and correct.
    //
    // A platform `Motion` has no reading of also means there is no answer
    // about the floor. That is the session thread's rule (`OnMap::floor`), and
    // it is applied here too, or the two disagree for the half second it
    // matters: a boat whose route says it has crossed to the other continent
    // stops being drawn on this map, and the ground under where it was is the
    // sea bed. The mover holds its altitude there; a prediction that placed the
    // character on the floor would draw them dropping and be corrected by every
    // reading.
    let mut adrift = false;
    if let Some(ferry) = predicted.base.as_ref().and_then(|b| b.ferry) {
        let placement = motion
            .position_of(ferry.guid, now)
            .zip(motion.facing_of(ferry.guid, now));
        adrift = placement.is_none();
        if let Some((at, facing)) = placement {
            let (moved, stride) = reframe(
                &ferry,
                Platform { guid: ferry.guid, position: at.to_array(), facing },
                position,
                [dx, dy],
            );
            // The re-frame is a translation in `z` and a yaw in `xy`, so the
            // held height moves by exactly what the position's did.
            held_z += moved.z - position.z;
            position = moved;
            [dx, dy] = stride;
        }
    }
    let standing = active.standing(&solids);
    predicted.now = Some(resolve(
        &standing,
        active.map_id,
        step.contact,
        position,
        held_z,
        [dx, dy],
        adrift,
    ));
}

/// Offer a frame's continuation to the world: the stride to the walls, and the
/// height to the floor the [`Contact`] says it is meeting.
///
/// Split from [`advance`] so a test can hold it against a world of its own;
/// the system passes the same [`crate::world::standing::Standing`] the session
/// thread walks on.
fn resolve(
    world: &dyn World,
    map_id: u32,
    contact: Contact,
    mut position: Position,
    held_z: f32,
    [dx, dy]: [f32; 2],
    adrift: bool,
) -> Position {
    // The same pair the session thread walks on, asked in the same order it asks
    // them: what is in the way shortens the stride, and only then is the height
    // looked up. The other way round stands the character on the floor inside
    // the building they were stopped from entering.
    //
    // On the ground the stride is walked in pieces no longer than
    // `MAX_STRIDE`, with the floor read after each, as `Mover::advance` walks
    // it. Read once at the end of a long stride, the floor misses a slope that
    // rises more than a step height before the end and the character is drawn
    // inside it; see `MAX_STRIDE`. In the air the stride is one sweep and the
    // floor is the arc's business, below.
    let walking = contact == Contact::Ground && !adrift;
    if dx != 0.0 || dy != 0.0 {
        let pieces = if walking {
            ((dx.hypot(dy) / MAX_STRIDE).ceil() as u32).clamp(1, MAX_PIECES)
        } else {
            1
        };
        let (px, py) = (dx / pieces as f32, dy / pieces as f32);
        for _ in 0..pieces {
            let from = [position.x, position.y, position.z];
            let to = [from[0] + px, from[1] + py, from[2]];
            let end = world.step(map_id, from, to);
            position.x = end[0];
            position.y = end[1];
            if !walking {
                continue;
            }
            if let Some(floor) = world.floor(map_id, position.x, position.y, position.z) {
                // `Mover::advance`'s rule, in both directions. A drop deeper
                // than `FALL_THRESHOLD` is a cliff, and the arc off it begins
                // when the mover says so, so it is left alone; anything
                // shallower is a step, a stair or a slope, and the character is
                // placed on it, going down as well as going up.
                //
                // An earlier version only lifted. Walking downhill, the mover
                // descends every tick while the prediction held the last
                // reading's height, so the drawn altitude was a 40 Hz staircase
                // while the horizontal was smooth: 9 cm of sawtooth at a run
                // down a moderate slope, on an axis nothing else in the frame
                // moves along, with the camera framed on it. Uphill was already
                // correct, so the fault appeared only going downhill.
                position.z = stood_on(position.z, floor);
            }
        }
    }
    // A held deck gives no answer about the height, as on the session thread.
    // Not folded into the match below because both of its arms would need it.
    if adrift {
        position.z = held_z;
        return position;
    }
    // What the floor does depends on the step's [`Contact`]. The ground was
    // handled piece by piece above.
    match contact {
        // Never below the ground, rising or falling. Where the arc ends is the
        // mover's decision and arrives with the next reading; this pass only
        // avoids drawing the character through the floor while it waits.
        //
        // The rising case is needed for a jump taken uphill. `Mover::advance`
        // clamps both, because a jump taken while running up a slope enters the
        // hillside a tick or two after the apex (1.34 yards under at 45
        // degrees), and the camera is framed on this position. Leaving `Rising`
        // out here would draw that fault for the 25 ms between readings, which
        // is most of the ascent.
        //
        // Clamping is not landing: the contact stays what the arc says, and
        // only the mover ends the arc.
        //
        // The floor is read from the higher of the drawn height and the
        // reading's own (`held_z`). Falling, that is the reading's, which is
        // what `Mover::advance` does with the height it started the step at:
        // the ceiling on the floor read is a step height above the feet it is
        // given, so read from the drawn height it misses a floor the arc has
        // already passed more than a step through. At terminal velocity,
        // 60 y/s, that is two frames, and the character was drawn under a
        // building's floor until the reading of the landing arrived.
        Contact::Rising | Contact::Falling => {
            let from = position.z.max(held_z);
            match world.floor(map_id, position.x, position.y, from) {
                Some(floor) => position.z = position.z.max(floor),
                // No floor data holds the arc, which is the mover's rule:
                // `Footing::floor` answers `None` when it has no data, never
                // when there is no floor, and inside a building's `MODF` box
                // that is every point the hull does not cover. The arc is held
                // at the reading's own height so the drawn character does not
                // descend through a floor the simulation is not letting them
                // through.
                None => position.z = held_z,
            }
        }
        Contact::Ground | Contact::Ridden => {}
    }
        position
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::state::movement::Mover;

    /// A position near the origin, so that `f32` precision does not decide the
    /// tests. A real Elwynn coordinate is around -9450, where an `f32` step is
    /// ~1 mm, so two integrations that agree to the last bit can still differ
    /// by 0.001, and the tolerances below would measure `f32` rather than this
    /// module. The property under test is that the expressions match; the
    /// world's precision at its far corners is a separate matter.
    fn position() -> Position {
        Position { x: 10.0, y: -10.0, z: 60.0, orientation: 1.0 }
    }

    /// A passenger is drawn on the deck's own clock, not the simulation's.
    ///
    /// The session publishes the passenger's world position composed against
    /// the placement it read the platform at; the renderer draws the platform
    /// from a different reading, a step and a half behind and moving at frame
    /// rate. Composing the two halves from different clocks makes the character
    /// slide across a floor they are standing still on, by up to the platform's
    /// travel in one step: 0.43 yards at the Deeprun Tram's 17 y/s, forty times
    /// a second, with the camera framed on it.
    ///
    /// The property is that the offset survives: wherever the deck is drawn,
    /// the passenger is drawn at the same spot on it.
    #[test]
    fn a_passenger_is_drawn_where_the_deck_is_drawn() {
        let was = Platform { guid: 7, position: [100.0, 50.0, 20.0], facing: 0.0 };
        let standing = Position { x: 103.0, y: 48.0, z: 21.5, orientation: 0.4 };
        let ferry = Ferry::aboard(was, standing);

        // The deck as the renderer has it this frame: some way further along,
        // and a whole step's travel from where the session read it.
        let drawn = Platform { position: [104.3, 50.0, 20.0], ..was };
        let (position, stride) = reframe(&ferry, drawn, standing, [0.0, 0.0]);

        assert!((position.x - (standing.x + 4.3)).abs() < 1e-3, "{position:?}");
        assert!((position.y - standing.y).abs() < 1e-3, "{position:?}");
        assert!((position.z - standing.z).abs() < 1e-3, "{position:?}");
        assert_eq!(stride, [0.0, 0.0]);

        // A deck the renderer has exactly where the session read it gives the
        // identity, so the re-frame changes nothing in a session that never
        // boards anything moving.
        let (same, _) = reframe(&ferry, was, standing, [1.0, 2.0]);
        assert!((same.x - standing.x).abs() < 1e-3 && (same.y - standing.y).abs() < 1e-3);
    }

    /// The stride is rotated with the deck. The rotation is zero on lifts and
    /// tram cars, which translate without turning, so an error in it would not
    /// show there.
    #[test]
    fn a_turning_deck_turns_the_stride_it_is_given() {
        let was = Platform { guid: 7, position: [0.0, 0.0, 0.0], facing: 0.0 };
        let standing = Position { x: 1.0, y: 0.0, z: 0.0, orientation: 0.0 };
        let ferry = Ferry::aboard(was, standing);
        let drawn = Platform { facing: std::f32::consts::FRAC_PI_2, ..was };

        let (position, stride) = reframe(&ferry, drawn, standing, [1.0, 0.0]);
        // A yard along the deck's x-axis becomes a yard along its y-axis…
        assert!(position.x.abs() < 1e-3 && (position.y - 1.0).abs() < 1e-3, "{position:?}");
        // …and so does a stride that was heading along it.
        assert!(stride[0].abs() < 1e-3 && (stride[1] - 1.0).abs() < 1e-3, "{stride:?}");
    }

    /// Predicting forward and simulating forward land in the same place. The
    /// whole module depends on this property.
    ///
    /// If they do not, every reading is a correction and the character is
    /// pulled back slightly forty times a second, which looks worse than the
    /// latency the prediction removes; a prediction that integrates a slightly
    /// different expression produces exactly that. The two share
    /// `movement::strode`, so the test checks that nothing around it has
    /// diverged: the flags composed from the keys, the speed chosen from those
    /// flags, and the heading the strafes rotate.
    #[test]
    fn the_prediction_lands_where_the_simulation_does() {
        for keys in [
            Controls { forward: true, ..Default::default() },
            Controls { forward: true, walk: true, ..Default::default() },
            Controls { backward: true, ..Default::default() },
            Controls { strafe_left: true, ..Default::default() },
            Controls { forward: true, strafe_right: true, ..Default::default() },
            Controls { backward: true, strafe_left: true, ..Default::default() },
        ] {
            let speeds = vale_protocol::state::movement::Speeds::default();
            let mut mover = Mover::new(position(), speeds);
            mover.set_controls(keys);

            // What the renderer draws 40 ms after the reading it was given…
            let (predicted, [dx, dy]) = flat_step(&mover.info, &speeds, keys, mover.restraint(), 0.040);
            let drawn = [predicted.x + dx, predicted.y + dy];

            // …and where the simulation actually gets to over the same 40 ms,
            // in the two steps it would really have taken.
            mover.advance(0.025, None);
            mover.advance(0.015, None);
            let simulated = mover.position();

            assert!(
                (drawn[0] - simulated.x).abs() < 1e-4 && (drawn[1] - simulated.y).abs() < 1e-4,
                "{keys:?}: drew {drawn:?}, simulation reached ({}, {})",
                simulated.x,
                simulated.y
            );
            // The drawn position must also have moved, or the agreement above
            // could be two zeroes. Every combination in the list is a key that
            // moves.
            let start = position();
            assert!(
                (drawn[0] - start.x).hypot(drawn[1] - start.y) > 0.05,
                "{keys:?}: drew no movement at all"
            );
        }
    }

    /// Over a run of frames against a run of simulation steps, the drawn
    /// character covers the same ground every frame.
    ///
    /// The session thread steps every 25 ms and the renderer draws every
    /// 16.7 ms, so each step is noticed a different fraction of a frame after it
    /// was taken. Stamping the base with the moment it was noticed, as the
    /// first version of this module did, makes the drawn position advance by
    /// the gap between notices while the base advances by the gap between
    /// steps. The two do not match: the character alternates between half speed
    /// and one and a half times speed several times a second, and the camera
    /// framed on it moves the whole view.
    ///
    /// The bound is a distance for the same reason as in `motion`'s test: 6 cm
    /// at a run is what the eye sees, whereas the speed it implies over one
    /// frame is a large number that says nothing about what is visible.
    #[test]
    fn the_drawn_position_advances_evenly_across_a_reading() {
        const TICK: f32 = 0.025;
        const FRAME: f32 = 1.0 / 60.0;

        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { forward: true, ..Default::default() };
        let mut mover = Mover::new(position(), speeds);
        mover.set_controls(keys);

        let mut predicted = Predicted::default();
        predicted.set_controls(keys, 0.0);

        let (mut simulated_to, mut world_ms) = (0.0_f32, 0_u64);
        let mut drawn: Vec<(f32, [f32; 2])> = Vec::new();
        for frame in 0..60 {
            let now = frame as f32 * FRAME;
            // Every step the session thread has finished by now, adopted at its
            // real age, which `rebase` computes from `SessionStatus::taken`.
            while simulated_to + TICK <= now {
                mover.advance(TICK, None);
                simulated_to += TICK;
                world_ms += 25;
                predicted.adopt(now, now - simulated_to, Reading { movement: mover.info, speeds, airborne: false, restraint: mover.restraint(), ferry: None, riding: None, ride: None, world_ms });
            }
            if let Some(step) = predicted.step(now) {
                let (p, [dx, dy]) = (step.position, step.stride);
                drawn.push((now, [p.x + dx, p.y + dy]));
            }
        }

        assert!(drawn.len() > 50, "the prediction went blank: {} frames", drawn.len());
        for pair in drawn.windows(2) {
            let ((before, from), (after, to)) = (pair[0], pair[1]);
            let travelled = (to[0] - from[0]).hypot(to[1] - from[1]);
            let expected = speeds.run() * (after - before);
            assert!(
                (travelled - expected).abs() < 0.001,
                "drew {travelled} yards over a frame the simulation moved {expected}"
            );
        }
    }

    /// A whole jump, frame by frame, against the simulation that owns it.
    ///
    /// The walk test above has the same structure and checks the horizontal;
    /// this checks the vertical. A jump is the one motion in the game where the
    /// drawn height is a parabola rather than a terrain lookup, and where the
    /// airborne state changes under the prediction twice.
    ///
    /// Two properties, one for each visible fault: the character jittering
    /// mid-air, and the end of the jump being replayed.
    ///
    /// * the drawn height goes up, then down, and never back up, so no frame
    ///   covers ground the character has already left; and
    /// * every frame's step is within a frame's worth of the arc's own speed,
    ///   so nothing is drawn twice or skipped.
    #[test]
    fn a_jump_is_drawn_as_one_arc_from_frame_to_frame() {
        const TICK: f32 = 0.025;
        const FRAME: f32 = 1.0 / 60.0;
        struct Flat;
        impl vale_protocol::state::movement::Footing for Flat {
            fn floor(&self, _x: f32, _y: f32, _z: f32) -> Option<f32> {
                Some(60.0)
            }
        }

        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { forward: true, ..Default::default() };
        let mut mover = Mover::new(position(), speeds);
        mover.set_controls(keys);
        mover.advance(TICK, Some(&Flat));

        let mut predicted = Predicted::default();
        predicted.set_controls(keys, 0.0);

        let (mut simulated_to, mut world_ms) = (0.0_f32, 0_u64);
        let mut drawn: Vec<(f32, f32)> = Vec::new();
        let mut jumped = false;
        for frame in 0..90 {
            let now = frame as f32 * FRAME;
            while simulated_to + TICK <= now {
                // Leave the ground on the fifth step, which is far enough in
                // for the base to be an ordinary stride and not the first one.
                if !jumped && simulated_to >= 4.0 * TICK {
                    assert!(mover.jump().is_some(), "the jump was refused");
                    jumped = true;
                }
                mover.advance(TICK, Some(&Flat));
                simulated_to += TICK;
                world_ms += 25;
                predicted.adopt(
                    now,
                    now - simulated_to,
                    Reading {
                        movement: mover.info,
                        speeds,
                        airborne: mover.is_airborne(),
                        restraint: mover.restraint(),
                        ferry: None,
                        riding: None,
                        ride: None,
                        world_ms,
                    },
                );
            }
            if let Some(step) = predicted.step(now) {
                // What `advance` does with the floor, which `step` deliberately
                // leaves to it: an arc is never taken below the ground and a
                // walking character is placed on it.
                drawn.push((now, step.position.z.max(60.0)));
            }
        }

        let apex = drawn
            .iter()
            .enumerate()
            .max_by(|a, b| a.1 .1.total_cmp(&b.1 .1))
            .expect("an arc")
            .0;
        assert!(drawn[apex].1 > 60.5, "the jump never left the ground: {:?}", drawn[apex]);
        assert!(
            drawn.last().expect("frames").1 <= 60.001,
            "it never came down: {:?}",
            drawn.last()
        );
        // Monotone on each side of the apex: the arc does not reset mid-air and
        // does not replay the descent.
        for pair in drawn[..=apex].windows(2) {
            assert!(
                pair[1].1 >= pair[0].1 - 1e-4,
                "the rise went backwards at {:.3}s: {} then {}",
                pair[1].0,
                pair[0].1,
                pair[1].1
            );
        }
        for pair in drawn[apex..].windows(2) {
            assert!(
                pair[1].1 <= pair[0].1 + 1e-4,
                "the fall went back up at {:.3}s: {} then {}",
                pair[1].0,
                pair[0].1,
                pair[1].1
            );
        }
        // No frame moves more than the arc's fastest step, except the take-off
        // frame. The jump leaves at 7.96 y/s under 19.29 y/s², so a 16.7 ms
        // frame is 0.133 yards at the start of the rise and less after it.
        //
        // The take-off frame is not a fault of this module: the arc begins on
        // the session thread's tick, up to 25 ms after the key went down, and
        // the first frame that sees it draws the arc where it is by then, up to
        // a tick and a frame in (0.254 yards on this run). The character holds
        // the ground for up to 25 ms after the press and then appears a quarter
        // of a yard up.
        //
        // This is measured rather than removed. Removing it would mean taking
        // off locally and reconciling, which the module comment says this pass
        // does not do ("It does not predict the server"). Every other frame of
        // the arc is within a frame's own travel.
        let mut steps: Vec<f32> = drawn
            .windows(2)
            .map(|pair| (pair[1].1 - pair[0].1).abs())
            .collect();
        let take_off = steps
            .iter()
            .position(|d| *d > 0.001)
            .expect("the arc moved at all");
        assert!(
            steps[take_off] < 0.30,
            "the take-off frame covered {} yards, which is more than a tick and a frame of arc",
            steps[take_off]
        );
        steps[take_off] = 0.0;
        for (i, travelled) in steps.iter().enumerate() {
            assert!(
                *travelled < 0.15,
                "drew {travelled} yards of height in one frame at {:.3}s",
                drawn[i + 1].0
            );
        }
    }

    /// The prediction returns a position for every frame it has a base for.
    ///
    /// The first version returned nothing when no time had elapsed since the
    /// base, which was every frame that adopted one, and `place_entities` fell
    /// back to the interpolator for that frame alone. That added and removed a
    /// step and a half of play-out delay twenty times a second, the same fault
    /// as the test above at ten times the size.
    #[test]
    fn a_reading_adopted_this_very_frame_is_still_drawn() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let mut predicted = Predicted::default();
        predicted.set_controls(Controls { forward: true, ..Default::default() }, 0.0);
        let mover = Mover::new(position(), speeds);
        predicted.adopt(4.0, 0.0, Reading { movement: mover.info, speeds, airborne: false, restraint: mover.restraint(), ferry: None, riding: None, ride: None, world_ms: 1 });
        assert!(predicted.step(4.0).is_some(), "went blank on the frame it rebased");
    }

    /// A simulation that has stopped stops the character, rather than striding
    /// on for as long as the stall lasts and being snapped back the width of it.
    #[test]
    fn a_stalled_simulation_holds_the_character_rather_than_running_away() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { forward: true, ..Default::default() };
        let mut mover = Mover::new(position(), speeds);
        mover.set_controls(keys);

        let mut predicted = Predicted::default();
        predicted.set_controls(keys, 0.0);
        predicted.adopt(0.0, 0.0, Reading { movement: mover.info, speeds, airborne: false, restraint: mover.restraint(), ferry: None, riding: None, ride: None, world_ms: 1 });

        let [dx, dy] = predicted.step(10.0).expect("a base").stride;
        let travelled = dx.hypot(dy);
        assert!(
            (travelled - speeds.run() * MAX_LEAD).abs() < 1e-3,
            "ran {travelled} yards on a ten-second-old reading"
        );
    }

    /// A ride is driven by the server, and the keys do not affect it.
    ///
    /// `Mover::advance` returns before it reads the keys while a ride runs, and
    /// this pass did not, so a player holding W through a charge was drawn
    /// striding forward at run speed on top of the ride and pulled back onto
    /// the spline forty times a second. This was one cause of a charge
    /// disturbing the character's drawn position; the other was in the packet
    /// handling.
    ///
    /// The ride is continued along its own heading at its own speed, which is
    /// none of the six in `Speeds`: a charge is capped at 24 y/s.
    #[test]
    fn a_ride_is_continued_at_the_servers_speed_and_not_the_keys() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { forward: true, ..Default::default() };
        let mut mover = Mover::new(position(), speeds);
        mover.set_controls(keys);
        // Straight along the heading the base carries, so the continuation and
        // the ride are the same line and the distance is the whole property.
        let start = position();
        let (dx, dy) = (start.orientation.cos(), start.orientation.sin());
        mover.ride(
            &vale_protocol::state::movement::MonsterMove {
                guid: 7,
                start: [start.x, start.y, start.z],
                spline_id: 41,
                path: vec![
                    [start.x, start.y, start.z],
                    [start.x + dx * 24.0, start.y + dy * 24.0, start.z],
                ],
                duration_ms: 1_000,
                facing: vale_protocol::state::movement::SplineFacing::Travel,
                flags: 0,
                transport: None,
            },
            None,
        );

        let mut predicted = Predicted::default();
        predicted.set_controls(keys, 0.0);
        predicted.adopt(0.0, 0.0, Reading { movement: mover.info, speeds, airborne: false, restraint: mover.restraint(), ferry: None, riding: mover.ride_velocity(), ride: mover.ride_curve(), world_ms: 1 });

        // The mouse does not steer a charge either: the spline sets the facing
        // (`SetFacingGUID`), so `send_input` must not offer a heading the mover
        // will refuse, or the memo that stops sixty commands a second records a
        // command that was never sent.
        assert!(!predicted.can_turn());

        let step = predicted.step(0.020).expect("a base");
        assert_eq!(step.stride, [0.0, 0.0], "the ride was offered to the collision");
        let travelled =
            (step.position.x - start.x).hypot(step.position.y - start.y);
        assert!(
            (travelled - 24.0 * 0.020).abs() < 1e-3,
            "drew {travelled} yards over 20 ms of a 24 y/s charge"
        );

        // …and it is the same line the mover walks, rather than merely the same
        // length: the two are drawn and simulated from one heading.
        mover.advance(0.020, None);
        assert!(
            (step.position.x - mover.position().x).abs() < 1e-3
                && (step.position.y - mover.position().y).abs() < 1e-3,
            "drew {:?}, the simulation reached {:?}",
            step.position,
            mover.position()
        );
    }

    /// A climbing ride is continued upwards.
    ///
    /// A taxi flight is a `MoveSplineFlag::Flying` spline that spends most of
    /// its length climbing or descending. The continuation used to be a bearing
    /// and a speed, which describe a path with no vertical component, so the
    /// drawn altitude held at the last reading's for a whole tick and then
    /// stepped to the next one, forty times a second, with the camera framed on
    /// it.
    ///
    /// Held against a real `Mover` on the same spline rather than against an
    /// expected number, because the property is that the two agree, the same
    /// standard the flat step is held to two tests up.
    #[test]
    fn a_climbing_ride_is_drawn_climbing() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let start = position();
        let mut mover = Mover::new(start, speeds);
        // Thirty yards along and twelve up over a second — a gentle taxi climb.
        mover.ride(
            &vale_protocol::state::movement::MonsterMove {
                guid: 7,
                start: [start.x, start.y, start.z],
                spline_id: 42,
                path: vec![
                    [start.x, start.y, start.z],
                    [start.x + 30.0, start.y, start.z + 12.0],
                ],
                duration_ms: 1_000,
                facing: vale_protocol::state::movement::SplineFacing::Travel,
                flags: 0,
                transport: None,
            },
            None,
        );

        let mut predicted = Predicted::default();
        predicted.adopt(0.0, 0.0, Reading { movement: mover.info, speeds, airborne: false, restraint: mover.restraint(), ferry: None, riding: mover.ride_velocity(), ride: mover.ride_curve(), world_ms: 1 });

        let step = predicted.step(0.025).expect("a base");
        assert!(
            step.position.z > start.z + 0.2,
            "drew {} of height on a climb that began at {}",
            step.position.z,
            start.z
        );
        mover.advance(0.025, None);
        assert!(
            (step.position.z - mover.position().z).abs() < 1e-3,
            "drew {} of height against the simulation's {}",
            step.position.z,
            mover.position().z
        );
        assert!(
            (step.position.x - mover.position().x).abs() < 1e-3,
            "drew {} along against the simulation's {}",
            step.position.x,
            mover.position().x
        );
    }

    /// A taxi flight round a sharp bend is drawn on its curve between steps:
    /// the next step lands on the position and heading already drawn, and the
    /// heading turns within the step rather than at its end.
    ///
    /// Continued from a velocity, the drawn point left the curve on the bend
    /// and the heading held still for a whole step, which turned the camera in
    /// 40 Hz steps.
    #[test]
    fn a_turning_flight_is_drawn_on_its_curve() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let start = position();
        let (x, y, z) = (start.x, start.y, start.z);
        let mut mover = Mover::new(start, speeds);
        mover.ride(
            &vale_protocol::state::movement::MonsterMove {
                guid: 7,
                start: [x, y, z],
                spline_id: 43,
                // A right angle twenty yards in, walked as a Catmull-Rom curve.
                path: vec![[x, y, z], [x + 20.0, y, z], [x + 20.0, y + 20.0, z], [x + 40.0, y + 20.0, z]],
                duration_ms: 2_000,
                facing: vale_protocol::state::movement::SplineFacing::Travel,
                // `MoveSplineFlag::Flying`.
                flags: 0x200,
                transport: None,
            },
            None,
        );
        // Into the bend.
        mover.advance(0.6, None);

        let mut predicted = Predicted::default();
        predicted.adopt(0.0, 0.0, Reading { movement: mover.info, speeds, airborne: false, restraint: mover.restraint(), ferry: None, riding: mover.ride_velocity(), ride: mover.ride_curve(), world_ms: 1 });
        let before = predicted.step(0.0).expect("a base").position.orientation;
        let middle = predicted.step(0.0125).expect("a base").position.orientation;
        let drawn = predicted.step(0.025).expect("a base").position;
        mover.advance(0.025, None);
        let simulated = mover.position();

        let apart = (drawn.x - simulated.x).hypot(drawn.y - simulated.y);
        assert!(apart < 1e-3, "drew {drawn:?}, the simulation reached {simulated:?}");
        let turned = vale_protocol::state::movement::shortest_turn(drawn.orientation, simulated.orientation);
        assert!(turned.abs() < 1e-3, "drew heading {}, simulated {}", drawn.orientation, simulated.orientation);
        let first = vale_protocol::state::movement::shortest_turn(before, middle);
        let second = vale_protocol::state::movement::shortest_turn(middle, drawn.orientation);
        assert!(first > 0.0 && second > 0.0, "the heading turns through the step: {before} {middle} {}", drawn.orientation);
    }

    /// The prediction follows the ground down as well as up, as
    /// `Mover::advance` does. An earlier version followed it only up.
    ///
    /// Downhill the mover descends every tick and this pass held the last
    /// reading's height, so the drawn altitude was a staircase at the
    /// simulation's 40 Hz beside a smooth horizontal. The property is the
    /// module's own: predicting forward and simulating forward land in the same
    /// place, on all three axes.
    #[test]
    fn a_character_walking_downhill_is_drawn_going_down() {
        /// A hillside falling away at 20% along +x, which is gentle enough that
        /// the mover treats every tick of it as a step rather than a cliff.
        struct Slope;
        impl vale_protocol::state::movement::Footing for Slope {
            fn floor(&self, x: f32, _y: f32, _z: f32) -> Option<f32> {
                Some(60.0 - (x - 10.0) * 0.2)
            }
            fn step(&self, _from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
                to
            }
            fn liquid(&self, _x: f32, _y: f32) -> Option<f32> {
                None
            }
        }

        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { forward: true, ..Default::default() };
        // Straight along +x, so the slope is entirely in the direction of travel.
        let downhill = Position { orientation: 0.0, ..position() };
        let mut mover = Mover::new(downhill, speeds);
        mover.set_controls(keys);

        let mut predicted = Predicted::default();
        predicted.set_controls(keys, 0.0);
        predicted.adopt(0.0, 0.0, Reading { movement: mover.info, speeds, airborne: false, restraint: mover.restraint(), ferry: None, riding: None, ride: None, world_ms: 1 });

        // What this pass would draw 25 ms on, through the same two calls
        // `advance` makes: the stride, then the floor at the end of it.
        let step = predicted.step(0.025).expect("a base");
        let mut drawn = step.position;
        drawn.x += step.stride[0];
        drawn.y += step.stride[1];
        // Named through the trait: `session::World` is in scope here and has a
        // `floor` of its own, with a map id in front of the coordinates.
        let floor = vale_protocol::state::movement::Footing::floor(&Slope, drawn.x, drawn.y, drawn.z);
        drawn.z = stood_on(drawn.z, floor.expect("a floor"));

        mover.advance(0.025, Some(&Slope));
        assert!(
            drawn.z < downhill.z - 0.01,
            "drew {} of height walking down a slope that began at {}",
            drawn.z,
            downhill.z
        );
        assert!(
            (drawn.z - mover.position().z).abs() < 1e-3,
            "drew {} of height against the simulation's {}",
            drawn.z,
            mover.position().z
        );
    }

    /// The arc is continued here rather than handed back to the interpolator,
    /// and it lands where the mover lands.
    ///
    /// The two paths draw different moments: this one draws `now` and `Motion`
    /// draws a step and a half back. Handing the character between them at
    /// take-off and again at landing costs ~0.28 yards of discontinuity each
    /// way, and at landing it plays the landing animation in mid-air followed
    /// by a snap to the ground.
    ///
    /// Held against a real `Mover` on the same parabola, since the two share
    /// `fall_elevation` and must not drift apart.
    #[test]
    fn the_arc_is_continued_where_the_mover_walks_it() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { forward: true, ..Default::default() };
        let mut mover = Mover::new(position(), speeds);
        mover.set_controls(keys);
        mover.jump();
        // A few ticks into the flight, so `fallTime` is non-zero and the arc's
        // start has to be recovered rather than being the position itself.
        for _ in 0..8 {
            mover.advance(0.025, None);
        }
        assert!(mover.is_airborne(), "the jump ended before the test began");

        let mut predicted = Predicted::default();
        predicted.set_controls(keys, 0.0);
        predicted.adopt(0.0, 0.0, Reading { movement: mover.info, speeds, airborne: true, restraint: mover.restraint(), ferry: None, riding: None, ride: None, world_ms: 1 });

        // Adopted this instant, the answer is the reading itself, so there is
        // no discontinuity at take-off or landing.
        let held = predicted.step(0.0).expect("an arc");
        assert!((held.position.z - mover.position().z).abs() < 1e-4);

        // …and 40 ms on it is where two more ticks of simulation put it.
        let step = predicted.step(0.040).expect("an arc");
        mover.advance(0.025, None);
        mover.advance(0.015, None);
        let simulated = mover.position();
        assert!(
            (step.position.x + step.stride[0] - simulated.x).abs() < 1e-3
                && (step.position.y + step.stride[1] - simulated.y).abs() < 1e-3,
            "drew ({}, {}), simulation reached ({}, {})",
            step.position.x + step.stride[0],
            step.position.y + step.stride[1],
            simulated.x,
            simulated.y
        );
        assert!(
            (step.position.z - simulated.z).abs() < 1e-3,
            "drew {} of height against the simulation's {}",
            step.position.z,
            simulated.z
        );
    }

    /// Which way the arc is moving decides what the floor does, and a rising
    /// arc must not be clamped onto a ledge it is passing.
    #[test]
    fn a_rising_arc_is_told_apart_from_a_falling_one() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let mut mover = Mover::new(position(), speeds);
        mover.jump();
        mover.advance(0.025, None);

        let mut predicted = Predicted::default();
        predicted.adopt(0.0, 0.0, Reading { movement: mover.info, speeds, airborne: true, restraint: mover.restraint(), ferry: None, riding: None, ride: None, world_ms: 1 });
        assert_eq!(predicted.step(0.020).expect("an arc").contact, Contact::Rising);

        // Past the apex — a jump at 7.96 y/s under 19.29 y/s² turns over in
        // 0.41 s — it is coming down and the floor is where it ends.
        for _ in 0..20 {
            mover.advance(0.025, None);
        }
        predicted.adopt(0.0, 0.0, Reading { movement: mover.info, speeds, airborne: true, restraint: mover.restraint(), ferry: None, riding: None, ride: None, world_ms: 2 });
        assert_eq!(predicted.step(0.020).expect("an arc").contact, Contact::Falling);
    }

    /// A turn key is integrated the same way, and the facing the camera follows
    /// comes out of the same expression the mover turns by.
    #[test]
    fn a_turn_is_predicted_at_the_movers_own_rate() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { turn_left: true, ..Default::default() };
        let mut mover = Mover::new(position(), speeds);
        mover.set_controls(keys);

        let (predicted, _) = flat_step(&mover.info, &speeds, keys, mover.restraint(), 0.040);
        mover.advance(0.025, None);
        mover.advance(0.015, None);
        assert!(
            (predicted.orientation - mover.position().orientation).abs() < 1e-4,
            "drew {} against {}",
            predicted.orientation,
            mover.position().orientation
        );
        // …and it actually turned, so the test is not passing on two zeroes.
        assert!(predicted.orientation > position().orientation);
    }

    /// The prediction rewrites the key flags and nothing else. A swimming
    /// character whose `SWIMMING` bit was cleared for a step would be moved at
    /// run speed through the water and snapped back by the next reading.
    #[test]
    fn only_the_keyed_flags_are_rewritten() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let mut base = vale_protocol::state::movement::MovementInfo {
            position: position(),
            ..Default::default()
        };
        base.flags = move_flags::SWIMMING | move_flags::FORWARD;

        let keys = Controls { backward: true, ..Default::default() };
        let (_, [dx, dy]) = flat_step(&base, &speeds, keys, Restraint::default(), 1.0);

        // Swimming survived, so the speed is the swim-back one rather than the
        // run-back one.
        let travelled = (dx * dx + dy * dy).sqrt();
        assert!(
            (travelled - speeds.swim_back()).abs() < 1e-3,
            "travelled {travelled}, expected the swim-back speed {}",
            speeds.swim_back()
        );
    }

    /// A rooted character turns and does not travel.
    ///
    /// An earlier version of this test carried `ROOT` in the base flags to show
    /// that the bit survived the rewrite, and then asserted a full second of
    /// swim-back travel: the bit survived and nothing read it.
    ///
    /// A stun, a root and a player's own death are all one
    /// `SMSG_FORCE_MOVE_ROOT` (vmangos `Unit::SetRooted`), so this one flag
    /// covers the whole fault of a character locked in place on the server
    /// while the client drew them walking. The property is the one the module
    /// exists for: the prediction and `Mover::advance` land in the same place.
    #[test]
    fn a_rooted_character_turns_and_does_not_travel() {
        use vale_protocol::state::movement::Mover;
        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { forward: true, turn_left: true, ..Default::default() };

        let mut mover = Mover::new(position(), speeds);
        mover.set_controls(keys);
        mover.set_rooted(true);
        let base = mover.info;
        let (predicted, [dx, dy]) = flat_step(&base, &speeds, keys, mover.restraint(), 0.040);

        assert_eq!([dx, dy], [0.0, 0.0], "a rooted character was predicted forwards");

        // …and it still turns, which is the other half of the mover's own rule.
        mover.advance(0.025, None);
        mover.advance(0.015, None);
        assert!(
            (predicted.orientation - mover.position().orientation).abs() < 1e-4,
            "drew {} against {}",
            predicted.orientation,
            mover.position().orientation
        );
        assert!(predicted.orientation > position().orientation, "it did not turn at all");
        // The mover did not travel either, which is what makes the two agree.
        assert!((mover.position().x - position().x).abs() < 1e-4);
        assert!((mover.position().y - position().y).abs() < 1e-4);
    }

    /// A stunned character does not turn. The stun is a separate field from
    /// the root.
    ///
    /// The two arrive together (vmangos' `HandleAuraModStun` roots as well), so
    /// the test checks that the prediction reads the stun too: with only the
    /// root obeyed, the character stands still and spins on the spot, which is
    /// what the drawn body did.
    #[test]
    fn a_stunned_character_does_not_turn_and_a_rooted_one_does() {
        use vale_protocol::state::movement::Mover;
        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { forward: true, turn_left: true, ..Default::default() };

        let mut mover = Mover::new(position(), speeds);
        mover.set_controls(keys);
        mover.set_rooted(true);
        mover.set_restraint(Restraint { stunned: true, dead: false, on_taxi: false });
        let (predicted, [dx, dy]) = flat_step(&mover.info, &speeds, keys, mover.restraint(), 0.040);
        assert_eq!([dx, dy], [0.0, 0.0]);
        assert_eq!(
            predicted.orientation,
            position().orientation,
            "a stunned character was predicted turning"
        );

        // The same block with the stun lifted and the root still on: the turn
        // comes back and the travel does not. Two fields, two rules.
        mover.set_restraint(Restraint::default());
        let (predicted, [dx, dy]) = flat_step(&mover.info, &speeds, keys, mover.restraint(), 0.040);
        assert_eq!([dx, dy], [0.0, 0.0]);
        assert!(predicted.orientation > position().orientation, "a root stops travel, not the character");
    }

    /// A dead character neither turns nor travels, and lifting its root brings
    /// neither back. A released ghost passes through that state: vmangos'
    /// `BuildPlayerRepop` calls `SetRooted(false)` on a body whose health is
    /// still being written.
    #[test]
    fn a_dead_character_neither_turns_nor_travels_even_unrooted() {
        use vale_protocol::state::movement::Mover;
        let speeds = vale_protocol::state::movement::Speeds::default();
        let keys = Controls { forward: true, turn_left: true, ..Default::default() };

        let mut mover = Mover::new(position(), speeds);
        mover.set_restraint(Restraint { stunned: false, dead: true, on_taxi: false });
        mover.set_controls(keys);
        assert!(!mover.info.is_moving(), "a corpse put a moving flag on the wire");

        let (predicted, [dx, dy]) = flat_step(&mover.info, &speeds, keys, mover.restraint(), 0.040);
        assert_eq!([dx, dy], [0.0, 0.0]);
        assert_eq!(predicted.orientation, position().orientation);
    }

    /// A mouse turn is drawn at the frame rate, not at the simulation's rate.
    ///
    /// A right-drag writes the camera rig every frame and `send_input` aims the
    /// character half a turn from it. Before [`Predicted::facing`] existed, the
    /// aim reached the drawn model only through the channel and the next
    /// reading, so at 144 fps against a 25 ms tick two frames in three drew no
    /// rotation and the third drew a whole step. The camera turned smoothly
    /// throughout, so the model visibly turned in steps inside a smoothly
    /// turning view.
    ///
    /// The bound is a ratio against the even step rather than an angle,
    /// because the unevenness is what is visible. Without the override the
    /// largest frame is `TICK / FRAME` = 3.6 times the even one and the
    /// smallest is zero.
    #[test]
    fn a_mouse_turn_is_drawn_evenly_across_a_reading() {
        use vale_protocol::state::movement::{shortest_turn, Mover};
        const TICK: f32 = 0.025;
        const FRAME: f32 = 1.0 / 144.0;
        /// A brisk but ordinary mouse-look: about 170 degrees a second.
        const RATE: f32 = 3.0;

        let speeds = vale_protocol::state::movement::Speeds::default();
        let mut mover = Mover::new(position(), speeds);
        let mut predicted = Predicted::default();

        let (mut simulated_to, mut world_ms) = (0.0_f32, 0_u64);
        let mut drawn: Vec<(f32, f32)> = Vec::new();
        for frame in 0..200 {
            let now = frame as f32 * FRAME;
            // The schedule's own order: `rebase`, then `send_input`, then
            // `advance`. A reading adopted this frame is one the session took
            // before it, carrying whatever headings had reached it by then.
            while simulated_to + TICK <= now {
                simulated_to += TICK;
                mover.face(RATE * simulated_to);
                mover.advance(TICK, None);
                world_ms += 25;
                predicted.adopt(
                    now,
                    now - simulated_to,
                    Reading {
                        movement: mover.info,
                        speeds,
                        airborne: false,
                        restraint: mover.restraint(),
                        ferry: None,
                        riding: None,
                        ride: None,
                        world_ms,
                    },
                );
            }
            predicted.set_facing(RATE * now, now);
            if let Some(step) = predicted.step(now) {
                drawn.push((now, step.position.orientation));
            }
        }

        assert!(drawn.len() > 150, "the prediction went blank: {} frames", drawn.len());
        let even = RATE * FRAME;
        for pair in drawn.windows(2) {
            let ((_, before), (at, after)) = (pair[0], pair[1]);
            let turned = shortest_turn(before, after);
            assert!(
                (turned - even).abs() < even * 0.01,
                "at {at}s a frame drew {turned} rad of turn against an even {even}"
            );
        }
    }

    /// Releasing a mouse turn does not jump the heading back.
    ///
    /// The override is dropped by the reading that confirms it, which compares
    /// two clocks, so the failure to check for is the drop happening one
    /// reading early. That draws the character at a heading the session has
    /// not reached yet, a backward jump of up to a tick's worth of turn on
    /// every release: four degrees at the rate below.
    ///
    /// The property is that the drawn heading never goes backwards, and that it
    /// ends where the last command put it.
    #[test]
    fn releasing_a_mouse_turn_does_not_jump_the_heading_back() {
        use vale_protocol::state::movement::{shortest_turn, Mover};
        const TICK: f32 = 0.025;
        const FRAME: f32 = 1.0 / 144.0;
        const RATE: f32 = 3.0;
        /// The frame the button comes up.
        const RELEASED: usize = 100;

        let speeds = vale_protocol::state::movement::Speeds::default();
        let mut mover = Mover::new(position(), speeds);
        let mut predicted = Predicted::default();

        let (mut simulated_to, mut world_ms) = (0.0_f32, 0_u64);
        let mut commanded = position().orientation;
        let mut drawn: Vec<(f32, f32)> = Vec::new();
        for frame in 0..300 {
            let now = frame as f32 * FRAME;
            while simulated_to + TICK <= now {
                simulated_to += TICK;
                // Whatever the last command to reach the thread was — which
                // stops arriving the moment the button comes up.
                mover.face(commanded);
                mover.advance(TICK, None);
                world_ms += 25;
                predicted.adopt(
                    now,
                    now - simulated_to,
                    Reading {
                        movement: mover.info,
                        speeds,
                        airborne: false,
                        restraint: mover.restraint(),
                        ferry: None,
                        riding: None,
                        ride: None,
                        world_ms,
                    },
                );
            }
            if frame < RELEASED {
                commanded = RATE * now;
                predicted.set_facing(commanded, now);
            }
            if let Some(step) = predicted.step(now) {
                drawn.push((now, step.position.orientation));
            }
        }

        for pair in drawn.windows(2) {
            let ((_, before), (at, after)) = (pair[0], pair[1]);
            let back = -shortest_turn(before, after);
            assert!(back < 1e-4, "at {at}s the drawn heading jumped back by {back} rad");
        }
        let (_, settled) = *drawn.last().expect("frames were drawn");
        assert!(
            shortest_turn(settled, commanded).abs() < 1e-4,
            "settled at {settled} rather than at the last commanded {commanded}"
        );
    }

    /// A character the server has stopped does not turn on the mouse, and the
    /// refusal has to be made here as well as at the command.
    ///
    /// `send_input` asks [`Predicted::can_turn`] before it commands at all, so
    /// the case this covers is the other order: the drag is already running and
    /// the stun arrives in a reading after it. A heading adopted before the
    /// restraint must stop being drawn the moment the restraint lands, or the
    /// character spins on the mouse while the simulation holds them still.
    #[test]
    fn a_stunned_character_does_not_take_the_commanded_heading() {
        let speeds = vale_protocol::state::movement::Speeds::default();
        let reading = |restraint| Reading {
            movement: MovementInfo { position: position(), ..Default::default() },
            speeds,
            airborne: false,
            restraint,
            ferry: None,
            riding: None,
            ride: None,
            world_ms: 25,
        };

        // Adopted on the command's own clock, so the margin has not passed and
        // the override is still standing in both halves.
        let mut free = Predicted::default();
        free.set_facing(2.5, 0.0);
        free.adopt(0.0, 0.0, reading(Restraint::default()));
        let drawn = free.step(0.0).expect("a base was adopted").position.orientation;
        assert!((drawn - 2.5).abs() < 1e-6, "an unrestrained character drew {drawn}");

        let mut stunned = Predicted::default();
        stunned.set_facing(2.5, 0.0);
        stunned.adopt(0.0, 0.0, reading(Restraint { stunned: true, dead: false, on_taxi: false }));
        let drawn = stunned.step(0.0).expect("a base was adopted").position.orientation;
        assert!(
            (drawn - position().orientation).abs() < 1e-6,
            "a stunned character turned on the mouse, to {drawn}"
        );
    }

    /// A strafe reversed and reversed again is drawn as one continuous path.
    ///
    /// The session applies a key change at the moment it was read, part-way
    /// through its step (`Mover::advance_through`), and the prediction applies
    /// it from the frame it was read. Before both did, the session applied a
    /// change from the start of the step that drained it and the prediction
    /// applied the keys held now to the whole time since its base, so the
    /// reversal frame drew the character back by twice the speed times the
    /// time since the last reading: up to 0.4 yards at a run, every reversal.
    ///
    /// The session here steps at Windows' sleep granularity, alternating
    /// 15.6 ms and 31.2 ms, against frames at 144 Hz. The property is that no
    /// frame moves the character further than a frame's travel, which a step
    /// back on a reversal or a snap on a reading would both exceed.
    #[test]
    fn a_strafe_reversal_is_drawn_without_a_step_back() {
        use vale_protocol::state::movement::Input;
        const TICKS: [f32; 2] = [0.015_625, 0.031_25];
        const FRAME: f32 = 1.0 / 144.0;
        /// Frames between reversals: about 0.3 s, a quick side-to-side.
        const HOLD: usize = 43;

        let speeds = Speeds::default();
        let start = position();
        let mut mover = Mover::new(start, speeds);
        let mut predicted = Predicted::default();
        let left = start.orientation + std::f32::consts::FRAC_PI_2;
        let (lx, ly) = (left.cos(), left.sin());
        let keys = |frame: usize| match (frame / HOLD) % 2 {
            0 => Controls { strafe_right: true, ..Default::default() },
            _ => Controls { strafe_left: true, ..Default::default() },
        };

        let (mut simulated_to, mut world_ms, mut tick) = (0.0_f32, 0_u64, 0_usize);
        let mut queued: Vec<(f32, Input)> = Vec::new();
        let mut held = Controls::default();
        let mut drawn: Vec<(usize, f32)> = Vec::new();
        for frame in 0..400 {
            let now = frame as f32 * FRAME;
            // Every step the session has finished by now. A key read before a
            // step's end is in that step, at its own offset.
            loop {
                let dt = TICKS[tick % 2];
                let end = simulated_to + dt;
                if end > now {
                    break;
                }
                let inputs: Vec<(f32, Input)> = queued
                    .iter()
                    .filter(|(at, _)| *at <= end)
                    .map(|&(at, input)| (at - simulated_to, input))
                    .collect();
                queued.retain(|(at, _)| *at > end);
                mover.advance_through(dt, &inputs, None);
                simulated_to = end;
                tick += 1;
                world_ms += 1;
                predicted.adopt(
                    now,
                    now - simulated_to,
                    Reading {
                        movement: mover.info,
                        speeds,
                        airborne: false,
                        restraint: mover.restraint(),
                        ferry: None,
                        riding: None,
                        ride: None,
                        world_ms,
                    },
                );
            }
            let c = keys(frame);
            if c != held {
                queued.push((now, Input::Controls(c)));
                held = c;
            }
            predicted.set_controls(c, now);
            if let Some(step) = predicted.step(now) {
                let p = step.position;
                let along = (p.x + step.stride[0] - start.x) * lx + (p.y + step.stride[1] - start.y) * ly;
                drawn.push((frame, along));
            }
        }

        assert!(drawn.len() > 350, "the prediction went blank: {} frames", drawn.len());
        let most = speeds.run() * FRAME * 1.001;
        for pair in drawn.windows(2) {
            let ((_, before), (frame, after)) = (pair[0], pair[1]);
            let moved = (after - before).abs();
            assert!(
                moved <= most,
                "frame {frame} moved {moved} yards against at most {most} for one frame"
            );
        }
        // …and the path does reverse: the character ends on the side the last
        // keys drove it to, rather than standing still and passing the test.
        let span = drawn.iter().map(|&(_, a)| a).fold(f32::MIN, f32::max)
            - drawn.iter().map(|&(_, a)| a).fold(f32::MAX, f32::min);
        assert!(span > 1.5, "the strafe covered only {span} yards");
    }

    /// A fall is drawn on a floor it has passed more than a step through since
    /// the last reading, as the mover lands on it.
    ///
    /// The floor is read with a ceiling a step height above the feet it is
    /// given. Read from the drawn height of an arc at terminal velocity, a
    /// building's floor a frame or two above it was out of reach, and the
    /// character was drawn under the floor until the landing's reading came.
    #[test]
    fn a_fall_is_drawn_on_the_floor_it_has_passed_through() {
        let storey = |_map: u32, _x: f32, _y: f32, z: f32| Some(if 50.0 <= z + 1.0 { 50.0 } else { 0.0 });
        let drawn = Position { x: 0.0, y: 0.0, z: 47.0, orientation: 0.0 };
        let placed = resolve(&storey, 0, Contact::Falling, drawn, 51.0, [0.0, 0.0], false);
        assert!((placed.z - 50.0).abs() < 1e-4, "drawn at {} under a floor at 50", placed.z);
        // …and an arc still above the floor is left where the parabola has it.
        let above = Position { z: 53.0, ..drawn };
        let placed = resolve(&storey, 0, Contact::Falling, above, 55.0, [0.0, 0.0], false);
        assert!((placed.z - 53.0).abs() < 1e-4, "an arc above the floor was moved to {}", placed.z);
    }

    /// A long drawn stride climbs a ramp rather than entering it, because it is
    /// walked in pieces as the mover walks it. See `MAX_STRIDE`.
    #[test]
    fn a_long_drawn_stride_climbs_a_ramp() {
        let ramp = |_map: u32, x: f32, _y: f32, z: f32| {
            let ramp = x * 1.0;
            Some(if ramp <= z + 1.0 { ramp.max(0.0) } else { 0.0 })
        };
        let start = Position { x: 0.0, y: 0.0, z: 0.0, orientation: 0.0 };
        // A mounted run over the prediction's whole lead: 14 y/s for 250 ms.
        let placed = resolve(&ramp, 0, Contact::Ground, start, 0.0, [3.5, 0.0], false);
        assert!((placed.x - 3.5).abs() < 1e-4, "{placed:?}");
        assert!((placed.z - 3.5).abs() < 1e-3, "drawn at {} inside a ramp at 3.5", placed.z);
    }
}
