//! Movement: the `MovementInfo` block, the `MSG_MOVE_*` opcodes that carry it,
//! and a local simulation that matches what vmangos's anticheat checks.
//!
//! ```text
//! curve.rs   the Catmull-Rom path a spline with the Flying flag follows
//! tests.rs   the wire round trips, teleports and the local simulation
//! ```
//!
//! Sources: `Objects/MovementInfo.{h,cpp}` (the flags and the wire layout),
//! `Handlers/MovementHandler.cpp` (what the server accepts), `Objects/Unit.cpp`
//! (`ExtrapolateMovement`, `GetSpeedForMovementInfo`) and
//! `Anticheat/MovementAnticheat/MovementAnticheat.cpp` (what the server rejects).
//!
//! ## Anticheat rules this module satisfies
//!
//! The reference server runs with `Anticheat.Enable = 1`. The rules that shape
//! the design here, all with `Reject = 1`:
//!
//! * `ctime` must be non-zero and monotonic. `CHEAT_TYPE_NULL_CLIENT_TIME`
//!   fires on zero, `CHEAT_TYPE_TIME_BACK` on any decrease. It is the client's
//!   own millisecond tick; the server only compares it against itself.
//! * `ctime` deltas must track real elapsed time. `CheckTimeDesync` subtracts
//!   the server's receive-time delta from the client's; more than a second of
//!   drift in one packet is `CHEAT_TYPE_NUM_DESYNC`.
//! * A moving unit must send a packet at least every 500 ms. `CheckSpeedHack`
//!   raises `CHEAT_TYPE_SKIPPED_HEARTBEATS` when a gap exceeds 1000 ms and the
//!   previous packet had a moving flag set.
//! * The opcode must match the flags. `CheckMoveStart`: only
//!   `MSG_MOVE_START_FORWARD` may introduce `FORWARD`, only
//!   `MSG_MOVE_START_STRAFE_LEFT` may introduce `STRAFE_LEFT`, and so on. Two
//!   flags changing at once therefore need two packets; see [`transitions`].
//! * Reported positions must be reachable. `CheckSpeedHack` re-runs the
//!   server's own extrapolation from the previous packet and allows 10% slack,
//!   so [`Mover::advance`] reproduces `Unit::ExtrapolateMovement` exactly.
//!
//! ## Why the vertical is a fall simulation and not a clamp
//!
//! [`Mover::advance`] used to pull the character towards the floor at a clamped
//! 40 yards a second. Every logged position looked plausible, but the server
//! flagged it as `CHEAT_TYPE_TELEPORT`: `CheckTeleport` compares two consecutive
//! packets that both carry no horizontal and no falling flags, and calls a z
//! difference over 2.0 yards a teleport hack. A character standing still while a
//! terrain tile loads under it produces that pair of packets. The result was a
//! 24-hour ban with `TeleportHack (Total:6)` in `Anticheat.log`, from a
//! character standing in Elwynn.
//!
//! The client the server expects falls instead: `MOVEFLAG_JUMPING` is set the
//! whole way (the flag is in the `MOVEFLAG_MASK_XZ | MOVEFLAG_JUMPING` exemption
//! list, so the check is skipped), `fallTime` counts up, and a
//! `MSG_MOVE_FALL_LAND` is sent at the bottom. Gravity is therefore the fix for
//! the teleport check, and a jump is the same arc with a non-zero initial speed.
//!
//! ## Why there are no turn flags here
//!
//! `MOVEFLAG_TURN_LEFT` / `TURN_RIGHT` make the server extrapolate along a
//! circular arc (`ExtrapolateMovement`, the `R = 1.295 * speed / pi` branch).
//! Reproducing that arc to within 10% alongside the straight-line case would be
//! a large amount of code for one keypress. Mouse-turning in the 1.12.1 client
//! does not use those flags either: it sends `MSG_MOVE_SET_FACING`. Turning here
//! only rotates the orientation locally, and the new orientation goes out on the
//! next packet, so every extrapolation the server does is a straight line. The
//! path actually walked is a curve shorter than that line, so it stays inside
//! the allowance.

use crate::bytes::{Reader, Writer};
use crate::opcodes::Opcode;
use crate::state::update::Position;

/// `enum MovementFlags` — vanilla values, which differ from TBC+.
///
/// `ONTRANSPORT` is `0x02000000` here, not the `0x200` of later expansions;
/// copying constants from a WotLK-era reference silently shifts every field
/// after it. See `Objects/MovementInfo.h`.
pub mod move_flags {
    pub const NONE: u32 = 0x0000_0000;
    pub const FORWARD: u32 = 0x0000_0001;
    pub const BACKWARD: u32 = 0x0000_0002;
    pub const STRAFE_LEFT: u32 = 0x0000_0004;
    pub const STRAFE_RIGHT: u32 = 0x0000_0008;
    pub const TURN_LEFT: u32 = 0x0000_0010;
    pub const TURN_RIGHT: u32 = 0x0000_0020;
    pub(crate) const PITCH_UP: u32 = 0x0000_0040;
    pub(crate) const PITCH_DOWN: u32 = 0x0000_0080;
    pub const WALK_MODE: u32 = 0x0000_0100;
    pub const LEVITATING: u32 = 0x0000_0400;
    pub const FIXED_Z: u32 = 0x0000_0800;
    pub const ROOT: u32 = 0x0000_1000;
    pub const JUMPING: u32 = 0x0000_2000;
    pub(crate) const FALLINGFAR: u32 = 0x0000_4000;
    pub const SWIMMING: u32 = 0x0020_0000;
    pub(crate) const SPLINE_ENABLED: u32 = 0x0040_0000;
    pub const FLYING: u32 = 0x0100_0000;
    pub(crate) const ONTRANSPORT: u32 = 0x0200_0000;
    pub(crate) const SPLINE_ELEVATION: u32 = 0x0400_0000;
    pub(crate) const WATERWALKING: u32 = 0x1000_0000;
    pub const SAFE_FALL: u32 = 0x2000_0000;
    pub(crate) const HOVER: u32 = 0x4000_0000;

    /// `MOVEFLAG_MASK_MOVING`. The anticheat's "was it moving?" test, and the
    /// trigger for the 500 ms heartbeat requirement.
    pub(crate) const MASK_MOVING: u32 = FORWARD
        | BACKWARD
        | STRAFE_LEFT
        | STRAFE_RIGHT
        | PITCH_UP
        | PITCH_DOWN
        | JUMPING
        | FALLINGFAR
        | SPLINE_ELEVATION;

    /// The bits that mean travel across the ground. This is a narrower test
    /// than [`MASK_MOVING`], and the one [`super::strode`] uses.
    ///
    /// `MASK_MOVING` is the anticheat's mask and includes the two pitch bits
    /// and the two air bits, none of which is horizontal travel: a swimmer
    /// holding the ascend key is moving by that definition but does not change
    /// map position. Keying the horizontal step on it would move a character
    /// forward at swim speed for as long as they held the key to go straight up.
    pub(crate) const MASK_TRAVEL: u32 = FORWARD | BACKWARD | STRAFE_LEFT | STRAFE_RIGHT;
}

/// Baseline run speed in yards/second (`baseMoveSpeed[MOVE_RUN]`). Only used
/// when the server has not sent the real value yet.
pub(crate) const BASE_RUN_SPEED: f32 = 7.0;

/// Downward acceleration, yards per second squared. `Movement::gravity` in
/// `Movement/spline/util.cpp`. It is the client's value; the server carries it
/// because it re-runs the client's fall to check it.
pub(crate) const GRAVITY: f32 = 19.291_105_270_385_74;

/// Upward speed a jump starts with.
///
/// The value comes from the server: `HandleMovementOpcodes` calls
/// `SetJumpInitialSpeed(7.95797334f)` when a `MSG_MOVE_JUMP` arrives, to
/// reproduce the arc the client is about to follow. A different value is not
/// rejected outright, because `CheckSpeedHack` only measures the horizontal,
/// but it puts the two simulations on different parabolas and the landing
/// position would disagree.
pub(crate) const JUMP_SPEED: f32 = 7.957_973_34;

/// Fall speed ceilings, `Movement::terminalVelocity` and
/// `terminalSavefallVelocity`. The second is Slow Fall / Levitate, which arrives
/// as `MOVEFLAG_SAFE_FALL`.
pub(crate) const TERMINAL_VELOCITY: f32 = 60.148_003;
pub(crate) const TERMINAL_SAFE_FALL_VELOCITY: f32 = 7.0;

/// How far a fall has dropped after `t` seconds, given the downward speed it
/// started with.
///
/// A direct port of `Movement::computeFallElevation(t, isSafeFall,
/// start_velocity)`, which `Unit::ExtrapolateMovement` runs to decide where a
/// jumping client should be, so both ends of the socket compute the same
/// height. Sign convention: a jump is passed `-JUMP_SPEED`, a negative downward
/// speed, and the caller subtracts the result from the arc's starting height.
///
/// `safe_fall` clamps the start velocity and nothing else. This looks like a
/// bug in the original and is copied unchanged: the linear part past terminal
/// time still runs at [`TERMINAL_VELOCITY`], so an ordinary Slow Fall (start
/// velocity zero) descends at the ordinary rate. Correcting it here would put
/// the client on a different parabola from the server that checks it.
pub fn fall_elevation(t: f32, safe_fall: bool, start_velocity: f32) -> f32 {
    let terminal = if safe_fall {
        TERMINAL_SAFE_FALL_VELOCITY
    } else {
        TERMINAL_VELOCITY
    };
    let start_velocity = start_velocity.min(terminal);
    // The time terminal velocity is reached, measured from the arc's start. It
    // is negative for a jump, which is why the branch below compares `t`
    // against it rather than testing how long the fall has lasted.
    let terminal_time = TERMINAL_VELOCITY / GRAVITY - start_velocity / GRAVITY;
    if t > terminal_time {
        TERMINAL_VELOCITY * (t - terminal_time)
            + start_velocity * terminal_time
            + GRAVITY * terminal_time * terminal_time * 0.5
    } else {
        t * (start_velocity + t * GRAVITY * 0.5)
    }
}

/// Speeds as the server sends them in a movement update: walk, run, run-back,
/// swim, swim-back, turn-rate. Order is `Object.cpp:482`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speeds(pub [f32; 6]);

impl Default for Speeds {
    fn default() -> Self {
        Speeds([2.5, BASE_RUN_SPEED, 4.5, 4.722_222, 2.5, std::f32::consts::PI])
    }
}

impl Speeds {
    pub fn walk(&self) -> f32 {
        self.0[0]
    }
    pub fn run(&self) -> f32 {
        self.0[1]
    }
    pub fn run_back(&self) -> f32 {
        self.0[2]
    }
    pub fn swim(&self) -> f32 {
        self.0[3]
    }
    pub fn swim_back(&self) -> f32 {
        self.0[4]
    }
    pub fn turn_rate(&self) -> f32 {
        self.0[5]
    }
}

/// The transport block, present only with [`move_flags::ONTRANSPORT`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TransportInfo {
    pub guid: u64,
    pub position: Position,
}

/// The current placement of a moving platform, as returned by
/// [`crate::socket::session::World::platform`].
///
/// A game object's position and facing, taken from the collision world when it
/// is asked. For a transport that is the placement the renderer last built its
/// hull at, so the platform reported here and the floor the character stands on
/// are the same placement. The carry below depends on that: a ferry computed
/// against a stale placement would slide the passenger off the deck.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Platform {
    pub guid: u64,
    pub position: [f32; 3],
    pub facing: f32,
}

/// The character's position relative to the platform under its feet: the state
/// behind `MOVEFLAG_ONTRANSPORT`.
///
/// ## Why the client owns this and the server does not
///
/// `WorldSession::HandleMoverRelocation` boards a player only when the player's
/// own movement packet carries the flag and the transport's guid: it looks the
/// guid up in the map and calls `AddPassenger`. Nothing else boards a player. A
/// client that never sets the flag is never a passenger,
/// `UpdatePassengerPositions` never moves it, and the deck moves away under it.
/// This was observed on the Deeprun Tram, whose car travels 2,482 yards.
///
/// ## Why the offset is held rather than the position
///
/// The offset is the character's place in the platform's own frame, and it
/// stays constant while the platform moves. Each step the world position is
/// recomputed from it and the platform's new placement (the carry); the
/// character then walks from there as usual, and the offset is recomputed from
/// where they ended up.
///
/// The transform is a yaw and a translation, the same arithmetic as
/// `GenericTransport::CalculatePassengerPosition` and its inverse: no pitch, no
/// roll, and the same `Rz` this client uses to place a game object's model and
/// hull. See [`Ferry::aboard`] and [`Ferry::carry`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ferry {
    /// The platform's guid, which is what the wire wants.
    pub guid: u64,
    /// Where the character is in the platform's frame, and which way they face
    /// relative to it. This is the transport block sent to the server verbatim.
    pub offset: Position,
    /// Where the platform stood when [`Self::offset`] was taken.
    at: [f32; 3],
    at_facing: f32,
}

impl Ferry {
    /// Board: convert a world position into the platform's frame.
    ///
    /// `CalculatePassengerOffset`: subtract the platform's origin, then rotate
    /// by `-facing`. The orientation is the difference of the two, so a
    /// character facing along the deck keeps facing along it as the deck turns.
    pub fn aboard(platform: Platform, world: Position) -> Ferry {
        let (sin, cos) = platform.facing.sin_cos();
        let (dx, dy) = (
            world.x - platform.position[0],
            world.y - platform.position[1],
        );
        Ferry {
            guid: platform.guid,
            offset: Position {
                x: dx * cos + dy * sin,
                y: dy * cos - dx * sin,
                z: world.z - platform.position[2],
                orientation: normalize_orientation(world.orientation - platform.facing),
            },
            at: platform.position,
            at_facing: platform.facing,
        }
    }

    /// Carry: convert the held offset back into world space using the
    /// platform's current placement.
    ///
    /// `CalculatePassengerPosition`, the exact inverse of [`Self::aboard`]. The
    /// offset does not change, the platform moves, and the character moves
    /// with it.
    pub fn carry(&self, platform: Platform) -> Position {
        let (sin, cos) = platform.facing.sin_cos();
        Position {
            x: platform.position[0] + self.offset.x * cos - self.offset.y * sin,
            y: platform.position[1] + self.offset.y * cos + self.offset.x * sin,
            z: platform.position[2] + self.offset.z,
            orientation: normalize_orientation(platform.facing + self.offset.orientation),
        }
    }

    /// Whether the platform has moved since the offset was taken. The carry is
    /// skipped when it has not, so a character standing on a stationary game
    /// object (nearly all of them) costs one comparison.
    pub fn moved(&self, platform: Platform) -> bool {
        self.at != platform.position || self.at_facing != platform.facing
    }

    /// Where the platform stood when this offset was taken.
    ///
    /// The offset is a coordinate in a frame, and this is the frame. A reader
    /// that puts the passenger back into the world against a different reading
    /// of the same platform needs both. The renderer is one: it draws the deck
    /// on its own clock and must draw whoever stands on it at the same moment.
    /// That is why this is public rather than private to the carry.
    pub fn platform(&self) -> Platform {
        Platform {
            guid: self.guid,
            position: self.at,
            facing: self.at_facing,
        }
    }
}

/// The server's `Geometry::NormalizeOrientation`. It is a modulo rather than a
/// clamp: `IsValidMapCoord` refuses an orientation outside `[0, 2pi)`, and both
/// of [`Ferry`]'s transforms add or subtract two angles that are each already
/// in range.
fn normalize_orientation(o: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let wrapped = o % tau;
    if wrapped < 0.0 {
        wrapped + tau
    } else {
        wrapped
    }
}

/// The jump block, present only with [`move_flags::JUMPING`].
///
/// Field order is `zspeed, cosAngle, sinAngle, xyspeed`: cosine comes before
/// sine, the opposite of the usual convention.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct JumpInfo {
    pub z_speed: f32,
    pub cos_angle: f32,
    pub sin_angle: f32,
    pub xy_speed: f32,
}

/// The movement block shared by every `MSG_MOVE_*` packet and by the movement
/// half of `SMSG_UPDATE_OBJECT`.
///
/// ```text
/// u32 moveFlags, u32 time, f32 x, y, z, o,
/// [ONTRANSPORT:      u64 guid + f32 x, y, z, o]
/// [SWIMMING:         f32 pitch]
/// u32 fallTime,
/// [JUMPING:          f32 zspeed, cosAngle, sinAngle, xyspeed]
/// [SPLINE_ELEVATION: f32 elevation]
/// ```
///
/// Source: `MovementInfo::Write`, `Object.cpp:144`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MovementInfo {
    pub flags: u32,
    /// The client's millisecond tick (`ctime` server-side). Must be non-zero
    /// and never decrease; see the module docs.
    pub time: u32,
    pub position: Position,
    pub transport: Option<TransportInfo>,
    /// Swim pitch, radians.
    pub pitch: f32,
    pub fall_time: u32,
    pub jump: JumpInfo,
    pub spline_elevation: f32,
}

impl MovementInfo {
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }

    /// Whether any flag is set that makes the server expect 500 ms heartbeats.
    pub fn is_moving(&self) -> bool {
        self.has(move_flags::MASK_MOVING)
    }

    pub fn write(&self, w: &mut Writer) {
        w.u32(self.flags)
            .u32(self.time)
            .f32(self.position.x)
            .f32(self.position.y)
            .f32(self.position.z)
            .f32(self.position.orientation);

        if self.has(move_flags::ONTRANSPORT) {
            let t = self.transport.unwrap_or_default();
            w.u64(t.guid)
                .f32(t.position.x)
                .f32(t.position.y)
                .f32(t.position.z)
                .f32(t.position.orientation);
        }
        if self.has(move_flags::SWIMMING) {
            w.f32(self.pitch);
        }

        w.u32(self.fall_time);

        if self.has(move_flags::JUMPING) {
            w.f32(self.jump.z_speed)
                .f32(self.jump.cos_angle)
                .f32(self.jump.sin_angle)
                .f32(self.jump.xy_speed);
        }
        if self.has(move_flags::SPLINE_ELEVATION) {
            w.f32(self.spline_elevation);
        }
    }

    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.write(&mut w);
        w.buf
    }

    /// Read a movement block, or `None` if it is truncated.
    ///
    /// Bounds-checked at every field group. The same layout appears inline
    /// inside `SMSG_UPDATE_OBJECT`, where reading one field short misaligns the
    /// reader and produces an invalid update type several blocks later.
    pub fn read(r: &mut Reader) -> Option<MovementInfo> {
        if !r.has(4 + 4 + 16) {
            return None;
        }
        let mut m = MovementInfo {
            flags: r.u32(),
            time: r.u32(),
            position: Position {
                x: r.f32(),
                y: r.f32(),
                z: r.f32(),
                orientation: r.f32(),
            },
            ..Default::default()
        };

        if m.has(move_flags::ONTRANSPORT) {
            if !r.has(8 + 16) {
                return None;
            }
            m.transport = Some(TransportInfo {
                guid: r.u64(),
                position: Position {
                    x: r.f32(),
                    y: r.f32(),
                    z: r.f32(),
                    orientation: r.f32(),
                },
            });
        }
        if m.has(move_flags::SWIMMING) {
            if !r.has(4) {
                return None;
            }
            m.pitch = r.f32();
        }

        if !r.has(4) {
            return None;
        }
        m.fall_time = r.u32();

        if m.has(move_flags::JUMPING) {
            if !r.has(16) {
                return None;
            }
            m.jump = JumpInfo {
                z_speed: r.f32(),
                cos_angle: r.f32(),
                sin_angle: r.f32(),
                xy_speed: r.f32(),
            };
        }
        if m.has(move_flags::SPLINE_ELEVATION) {
            if !r.has(4) {
                return None;
            }
            m.spline_elevation = r.f32();
        }
        Some(m)
    }

    /// The speed the server will assume for these flags, given `speeds`.
    /// Mirrors `Unit::GetSpeedForMovementInfo`.
    pub fn speed(&self, speeds: &Speeds) -> f32 {
        if self.has(move_flags::SWIMMING) {
            if self.has(move_flags::BACKWARD) {
                speeds.swim_back()
            } else {
                speeds.swim()
            }
        } else if self.has(move_flags::WALK_MODE) {
            speeds.walk()
        } else if self.is_moving() {
            if self.has(move_flags::BACKWARD) {
                speeds.run_back()
            } else {
                speeds.run()
            }
        } else {
            0.0
        }
    }

    /// Heading actually travelled: the facing rotated by the strafe keys.
    ///
    /// The forward cases are taken from `Unit::ExtrapolateMovement`. The
    /// backward diagonals deliberately differ: the server's version lets
    /// `MOVEFLAG_BACKWARD` override a strafe, so back-and-left is walked
    /// straight backwards, and pressing a strafe key while holding S has no
    /// effect. The 1.12.1 client moves a character back-and-left at 135°, the
    /// mirror of the forward-and-left 45° the same function already encodes;
    /// the server's copy models the forward pair and not the backward one.
    ///
    /// The difference does not trip the anticheat. `CheckSpeedHack`
    /// extrapolates from the last packet and accumulates `m_overspeedDistance`
    /// only when the distance travelled exceeds the distance the extrapolation
    /// allows by more than 10% (`MovementAnticheat.cpp`, `realDistance2D_sq >
    /// (allowedDX + allowedDY) * 1.1f`). A different heading over the same
    /// interval covers the same distance, so the comparison is unchanged, and
    /// vanilla is client-authoritative about position, so the server accepts
    /// the reported place either way. The renderer's prediction and the mover
    /// must integrate the same expression, and both call this function.
    pub fn heading(&self) -> f32 {
        use std::f32::consts::PI;
        let o = self.position.orientation;
        if self.has(move_flags::BACKWARD) {
            if self.has(move_flags::STRAFE_LEFT) {
                o + PI * 0.75
            } else if self.has(move_flags::STRAFE_RIGHT) {
                o - PI * 0.75
            } else {
                o + PI
            }
        } else if self.has(move_flags::STRAFE_LEFT) {
            if self.has(move_flags::FORWARD) {
                o + PI / 4.0
            } else {
                o + PI / 2.0
            }
        } else if self.has(move_flags::STRAFE_RIGHT) {
            if self.has(move_flags::FORWARD) {
                o - PI / 4.0
            } else {
                o - PI / 2.0
            }
        } else {
            o
        }
    }
}

/// Which movement keys are held. An input snapshot only; [`Mover`] turns it
/// into flags, opcodes and positions.
///
/// `turn_left` / `turn_right` do not map to `MOVEFLAG_TURN_*`; see the module
/// docs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Controls {
    pub forward: bool,
    pub backward: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub walk: bool,
    /// Swim straight up while held: `PITCHUP` in `Bindings.xml`, and the space
    /// bar in this client.
    ///
    /// Not in [`Self::to_flags`]: the flags it implies are legal only while
    /// swimming, and the 1.12.1 client applies the pitch keys only when
    /// `MOVEFLAG_SWIMMING` is set. [`Mover::pitch_flags`] applies the same
    /// condition.
    pub ascend: bool,
    /// Swim straight down while held: `PITCHDOWN`. This client binds no key to
    /// it yet; the binding exists in `Bindings.xml`, and a swimmer aims down
    /// with the mouse instead. The field exists because the 1.12.1 client
    /// treats the two keys as one opposed pair, and modelling only one of them
    /// would hide the asymmetry.
    pub descend: bool,
}

impl Controls {
    /// The movement flags these keys imply. Opposite keys cancel, as in the
    /// 1.12.1 client: sending both `FORWARD` and `BACKWARD` would make the
    /// server extrapolate backwards while the character walked forwards.
    pub fn to_flags(self) -> u32 {
        let mut f = move_flags::NONE;
        if self.forward && !self.backward {
            f |= move_flags::FORWARD;
        }
        if self.backward && !self.forward {
            f |= move_flags::BACKWARD;
        }
        if self.strafe_left && !self.strafe_right {
            f |= move_flags::STRAFE_LEFT;
        }
        if self.strafe_right && !self.strafe_left {
            f |= move_flags::STRAFE_RIGHT;
        }
        if self.walk {
            f |= move_flags::WALK_MODE;
        }
        f
    }
}

/// One packet the client owes the server: an opcode and the flags to report
/// with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveEvent {
    pub opcode: Opcode,
    /// Flags after this transition. Each event is a single flag change, so
    /// events must be applied in order.
    pub flags: u32,
}

/// Split a flag change into the sequence of packets the server will accept.
///
/// `CheckMoveStart` rejects any packet that introduces a movement flag under
/// the wrong opcode, so a change of two flags is two packets. Stops are emitted
/// before starts, as the 1.12.1 client does; this keeps a reversal on the spot
/// (`FORWARD` -> `BACKWARD`) legal.
pub(crate) fn transitions(old: u32, new: u32) -> Vec<MoveEvent> {
    use move_flags::*;
    let mut events = Vec::new();
    let mut cur = old;

    let mut push = |op: Opcode, flags: u32| {
        events.push(MoveEvent { opcode: op, flags });
    };

    // --- forward / backward ---
    let pair = FORWARD | BACKWARD;
    if new & pair != old & pair {
        if old & pair != 0 {
            cur &= !pair;
            push(Opcode::MSG_MOVE_STOP, cur);
        }
        if new & FORWARD != 0 {
            cur |= FORWARD;
            push(Opcode::MSG_MOVE_START_FORWARD, cur);
        } else if new & BACKWARD != 0 {
            cur |= BACKWARD;
            push(Opcode::MSG_MOVE_START_BACKWARD, cur);
        }
    }

    // --- strafing ---
    let strafe = STRAFE_LEFT | STRAFE_RIGHT;
    if new & strafe != old & strafe {
        if old & strafe != 0 {
            cur &= !strafe;
            push(Opcode::MSG_MOVE_STOP_STRAFE, cur);
        }
        if new & STRAFE_LEFT != 0 {
            cur |= STRAFE_LEFT;
            push(Opcode::MSG_MOVE_START_STRAFE_LEFT, cur);
        } else if new & STRAFE_RIGHT != 0 {
            cur |= STRAFE_RIGHT;
            push(Opcode::MSG_MOVE_START_STRAFE_RIGHT, cur);
        }
    }

    // --- pitching, which in 1.12 is how a swimmer goes up and down ---
    //
    // Same structure as the strafe pair, for the same reason: `CheckMoveFlags`
    // tests `MOVEFLAG_PITCH_UP` and `MOVEFLAG_PITCH_DOWN` the same way it tests
    // the four travel bits. The start opcode must carry its own flag, and no
    // other opcode may introduce it. The three opcodes are consecutive
    // (191/192/193); see `Mover::pitch_flags`.
    let pitching = PITCH_UP | PITCH_DOWN;
    if new & pitching != old & pitching {
        if old & pitching != 0 {
            cur &= !pitching;
            push(Opcode::MSG_MOVE_STOP_PITCH, cur);
        }
        if new & PITCH_UP != 0 {
            cur |= PITCH_UP;
            push(Opcode::MSG_MOVE_START_PITCH_UP, cur);
        } else if new & PITCH_DOWN != 0 {
            cur |= PITCH_DOWN;
            push(Opcode::MSG_MOVE_START_PITCH_DOWN, cur);
        }
    }

    // --- walk/run toggle ---
    // No flag-specific opcode exists for this in 1.12 that the server checks,
    // so it rides along on a heartbeat.
    if new & WALK_MODE != old & WALK_MODE {
        cur = (cur & !WALK_MODE) | (new & WALK_MODE);
        push(Opcode::MSG_MOVE_HEARTBEAT, cur);
    }

    events
}

/// An arc through the air: a jump, a step off a ledge, or a knockback.
///
/// The horizontal velocity is fixed at take-off. This is the client's rule
/// (1.12 has no air control) and the server's: `ExtrapolateMovement` computes
/// `jump.start + (cosAngle, sinAngle) * xyspeed * t` and ignores the movement
/// flags while `MOVEFLAG_JUMPING` is set. Steering in the air would be measured
/// against a straight line and accumulate `m_overspeedDistance` for as long as
/// the jump lasted.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Airborne {
    /// Where the arc began. The server sets its own copy of this from the first
    /// packet carrying the flag (`MovementInfo::Read`, `if
    /// (!jump.startClientTime)`), so a take-off is reported at once rather than
    /// on the next heartbeat.
    start: [f32; 3],
    /// Seconds since it began.
    elapsed: f32,
    /// Upward speed at t = 0. [`JUMP_SPEED`] for a jump, zero for walking off
    /// an edge, whatever the server said for a knockback.
    up: f32,
    /// Whether this began with a `MSG_MOVE_JUMP`. A knockback and a step off a
    /// ledge both end in `MSG_MOVE_FALL_LAND` too, but only a jump counts
    /// against `CHEAT_TYPE_MULTI_JUMP`, which allows exactly one before a
    /// landing.
    jumped: bool,
}

/// A server-driven move of this character: an `SMSG_MONSTER_MOVE` addressed to
/// the player's own guid, which the local simulation follows rather than
/// ignores.
///
/// Warrior Charge is the main case, and it is neither a teleport nor a
/// knockback. `Spell::OnSpellLaunch` ends in
/// `GetMotionMaster()->MoveCharge(unitTarget, …)`, which switches the player's
/// own motion master to a `ChargeMovementGenerator` and launches an ordinary
/// `MoveSpline`, the same mechanism that moves a patrolling creature. The
/// packet that arrives is the same packet every creature sends, except that it
/// names the player.
///
/// The client must acknowledge it. `MoveSplineInit::Launch` calls
/// `SetSplineDonePending(true)` for any player mover, and
/// `WorldSession::HandleMovementOpcodes` opens with
///
/// ```text
/// if (pMover->HasPendingSplineDone())
///     return;
/// ```
///
/// so from the moment a charge is cast until `CMSG_MOVE_SPLINE_DONE` arrives,
/// the server silently discards every movement packet this client sends. Only
/// two things clear the flag: that packet, and `Map::Add` (a login). Before
/// this type existed, a charge left the character's position wrong and needed
/// a relog to fix: the position was wrong because the spline was not followed,
/// and the relog was needed because the acknowledgement was never sent.
struct Ride {
    /// The path, followed the same way
    /// [`crate::state::objects::ObjectManager::advance`] follows a creature's.
    /// It is the same type, so the player and a creature on the same spline
    /// follow the same curve.
    spline: Spline,
    /// The same path as it was at the start, shared with the renderer through
    /// [`Mover::ride_curve`]. Shared rather than cloned per step because a taxi
    /// flight's path is hundreds of points and is published forty times a
    /// second.
    published: std::sync::Arc<Spline>,
    /// Seconds since the ride began. Kept beside [`Spline::elapsed_ms`] rather
    /// than replacing it because the mover steps in fractional seconds and the
    /// spline counts whole milliseconds: truncating each step instead of the
    /// total loses up to a millisecond a tick, which over a one-second charge is
    /// forty of them.
    elapsed: f32,
    /// Echoed back in `CMSG_MOVE_SPLINE_DONE`, which the server drops unless it
    /// matches `movespline->GetId()`.
    spline_id: u32,
    /// Where to look on arrival for a [`SplineFacing::Target`]. The handler
    /// resolves it, because only the object manager can find another unit. A charge
    /// carries this: `ChargeMovementGenerator::Initialize` ends in
    /// `init.SetFacingGUID(unit.GetTargetGuid())`.
    facing_at: Option<[f32; 3]>,
}

/// How far the ground may be below the feet before the character is falling
/// rather than walking down a slope.
///
/// Below this the drop is a step and the character is placed on it. A steep
/// slope still drops far less than this per 25 ms tick (a 7.6 y/s run covers
/// 19 cm), so this threshold only catches a real edge.
///
/// Shared with [`crate::state::objects::ObjectManager::advance`], which makes
/// the same stair-or-cliff decision for units it does not simulate, and acts on
/// it differently: a stair is followed, and a cliff is left for the server to
/// state.
///
/// Public because the prediction needs it too: `crates/client`'s
/// `world::predict` continues the same step this function takes, so the
/// stair-or-cliff threshold must be one shared value.
pub const FALL_THRESHOLD: f32 = 0.7;

/// The longest piece, in yards, that [`Mover::advance`] resolves against the
/// world at once.
///
/// A step is checked against the world in two parts: the wall probe sweeps the
/// horizontal stride, and the floor is then read at the end of it with a
/// ceiling one step height (`STEP_UP`, 1.0 yard) above the feet the stride
/// started from. The probe covers any length. The floor read does not: a
/// stride that rises more than a step height before its end puts the end point
/// under the surface it should be standing on, so the floor read answers the
/// surface below and the character walks into the slope. The same holds going
/// down, where a drop deeper than [`FALL_THRESHOLD`] over one stride starts a
/// fall on a slope that should have been walked.
///
/// A step is therefore cut into pieces no longer than this, horizontally or
/// vertically, and each piece is checked on its own. At 0.5 yards a surface
/// rises at most 0.87 yards over one piece before it is steep enough to be a
/// wall (60 degrees, `WALL_NORMAL_Z`), which is under the step height. The
/// same length is the body's radius (`PLAYER_RADIUS`), so a fall at terminal
/// velocity is checked every half yard of its descent as well.
///
/// At a run over the session's 25 ms tick the stride is 0.18 yards and a step
/// is one piece. A mounted run over a 250 ms stall is 3.5 yards and seven
/// pieces; a fall at terminal velocity over the same stall is thirty.
///
/// The 1.12.1 client does not cut a step into fixed pieces. It sweeps the
/// body's box over the whole displacement and stops it at the first contact,
/// so its answer does not depend on the step's length at all. Cutting the
/// step gives the same independence from speed with the probe and the floor
/// read this client has.
pub const MAX_STRIDE: f32 = 0.5;

/// The most pieces one step is cut into; see [`MAX_STRIDE`].
///
/// A step is at most 250 ms (the session clamps a stall to that), and the
/// fastest thing the mover does without a ride is fall at
/// [`TERMINAL_VELOCITY`]: 15 yards, thirty pieces. The cap is above that and
/// exists so a corrupt speed from the server cannot turn one step into an
/// unbounded loop.
const MAX_PIECES: u32 = 64;

/// The fraction of a unit's own height the water has to reach before the unit
/// swims: 0.75.
///
/// The 1.12.1 client compares one quantity against two thresholds: the depth,
/// which is the liquid surface minus the unit's feet. All four liquid types
/// count (mask 0x0f), so lava and slime are swum in too. The unit starts
/// swimming once the depth exceeds 0.75 of its own height.
///
/// vmangos states the same rule: `GetMinSwimDepth() { return
/// GetCollisionHeight() * 0.75f; }`, with the comment "client switches to swim
/// animation at this depth".
pub(crate) const SWIM_DEPTH_FRACTION: f32 = 0.75;

/// The height that [`SWIM_DEPTH_FRACTION`] is applied to. It is a fixed
/// stand-in, not this character's own height.
///
/// The height is a per-unit field (the same one the gait cascade doubles), and
/// the 1.12.1 client takes it from the model. The protocol crate cannot read a
/// model. 2.0 is what vmangos initialises `m_modelCollisionHeight` to for a
/// unit whose model states none, which makes the threshold 1.5 yards. A
/// character noticeably taller or shorter than a human therefore begins
/// swimming a few inches early or late. The rule (a fraction of the swimmer's
/// height) matches the 1.12.1 client; only the height is approximate.
pub(crate) const COLLISION_HEIGHT: f32 = 2.0;

/// How deep the water must be before the character swims; see
/// [`SWIM_DEPTH_FRACTION`] and [`COLLISION_HEIGHT`].
pub const MIN_SWIM_DEPTH: f32 = COLLISION_HEIGHT * SWIM_DEPTH_FRACTION;

/// The dead band under [`MIN_SWIM_DEPTH`] at which swimming stops: 1/36 of a
/// yard.
///
/// The 1.12.1 client stops swimming when the depth falls below
/// [`MIN_SWIM_DEPTH`] minus this value. It is about two and a half centimetres
/// and is the client's value, not a tuning choice. It keeps a swimmer floating
/// at the entry depth (where this client holds one) from switching between the
/// two states, with a packet for each switch, for as long as they stay in the
/// water.
pub(crate) const SWIM_EXIT_HYSTERESIS: f32 = 1.0 / 36.0;

/// How far off the ground [`move_flags::HOVER`] holds a unit: one yard.
///
/// Two packet families carry the flag, and the server never sends a height:
/// 1.12 has no `UNIT_FIELD_HOVERHEIGHT` (that is 3.x), and vmangos'
/// `Aura::HandleAuraHover` is one line, `GetTarget()->SetHover(apply)`. The
/// height is a constant of the 1.12.1 client, which uses it in two places.
///
/// In movement, a unit is placed on the surface below it. A hovering unit
/// instead ends at `z = surface + 1.0` when the surface is more than a yard
/// below, and is not moved at all when it is less than a yard below. A
/// hovering unit is lowered onto its yard of air and never lifted onto it.
///
/// In selection, a hovering unit's selection box extends 1.0 lower, so the box
/// a player clicks still reaches the ground under it.
pub(crate) const HOVER_HEIGHT: f32 = 1.0;

/// The height a unit with these movement flags stands at, given the floor under
/// it and the liquid surface over that. `None` where there is no floor data.
///
/// Two flags change the answer:
///
/// * [`move_flags::WATERWALKING`] puts the unit on the liquid surface rather
///   than the floor under it. The floor still wins where it is the higher of
///   the two, as on a bridge over a lake: a water-walker on the bridge stands
///   on the bridge.
/// * [`move_flags::HOVER`] adds [`HOVER_HEIGHT`] to whichever surface was
///   chosen. The two compose: the 1.12.1 client applies the hover offset to
///   the surface the water-walking choice produced.
///
/// Both callers use this one function so they cannot differ:
/// [`Mover::advance`] moves the local player and
/// `ObjectManager::stand_on_the_ground` places every other unit. A unit drawn
/// on the lake bed while the server has it on the surface is wrong in either
/// path.
///
/// The caller decides whether to fetch the liquid. It is passed in rather than
/// queried here so that the observer path can skip [`Footing::liquid`] for
/// the large majority of units, which carry neither flag.
pub(crate) fn standing_surface(flags: u32, floor: Option<f32>, liquid: Option<f32>) -> Option<f32> {
    let surface = match liquid {
        Some(water)
            if flags & move_flags::WATERWALKING != 0
                && floor.is_none_or(|ground| water >= ground) =>
        {
            water
        }
        _ => floor?,
    };
    Some(match flags & move_flags::HOVER {
        0 => surface,
        _ => surface + HOVER_HEIGHT,
    })
}

/// The facing `dt` seconds of held turn keys produces.
///
/// A free function so that two callers can share it. The renderer predicts the
/// local player forward from the last reading it was given, so that a keypress
/// moves the character in the frame it was pressed rather than a round trip
/// later. A prediction that integrates a different expression from the
/// simulation has to be corrected every step, which causes the judder the
/// prediction exists to remove. There is one copy of the arithmetic and both
/// callers use it.
pub fn turned(orientation: f32, speeds: &Speeds, controls: Controls, dt: f32) -> f32 {
    let mut o = orientation;
    if controls.turn_left {
        o += speeds.turn_rate() * dt;
    }
    if controls.turn_right {
        o -= speeds.turn_rate() * dt;
    }
    wrap_angle(o)
}

/// How far the drawn body is turned from the direction a unit is aiming, in
/// radians, given the keys it is holding.
///
/// This body turn is how 1.12 shows a strafe; there is no strafe animation.
/// No ground gait moves sideways: `RunLeft`/`RunRight` are rows in
/// `AnimationData.dbc` that no model carries, and the Shuffles are the
/// turn-in-place foot shuffle, so a strafing character plays Run. It looks
/// like running sideways because the rendered root is turned into the slide
/// while the aim (the camera, the server's orientation, the target the
/// character faces) stays fixed: ±90° for a pure strafe, ±45° when forward or
/// backward is held too. The renderer eases the model onto it and twists the
/// spine and head back; see `vale_assets::world::m2::BodyTwist`.
///
/// Yaw is left-positive, so a strafe left is `+`, and the sign is mirrored
/// while backpedalling: a back-and-left diagonal faces the body forward-right
/// and backpedals along the line of travel, so the legs never cross.
///
/// Zero when not strafing, and zero when both strafe keys are held, which is
/// also when [`Controls::to_flags`] has already cancelled them.
///
/// It is here rather than in the renderer for the same reason [`heading`] is:
/// it is a pure reading of the movement flags, it decides nothing about meshes
/// or materials, and the renderer and the CLI both want the same copy.
pub fn strafe_body_offset(flags: u32) -> f32 {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
    let left = flags & move_flags::STRAFE_LEFT != 0;
    let right = flags & move_flags::STRAFE_RIGHT != 0;
    if left == right {
        return 0.0;
    }
    let diagonal = flags & (move_flags::FORWARD | move_flags::BACKWARD) != 0;
    let magnitude = if diagonal { FRAC_PI_4 } else { FRAC_PI_2 };
    let back = flags & move_flags::BACKWARD != 0;
    if left != back {
        magnitude
    } else {
        -magnitude
    }
}

/// The horizontal stride `dt` seconds of a movement block produces on flat
/// ground, before anything in the way has been consulted.
///
/// Zero when the block says the unit is not moving, so it is safe to call
/// unconditionally. The airborne case is not handled here: in the air the
/// velocity is the one fixed at take-off, held in the jump block and handled by
/// the mover. For the same reason the renderer's prediction stops when the
/// character leaves the ground (see [`turned`]).
pub fn strode(info: &MovementInfo, speeds: &Speeds, dt: f32) -> [f32; 2] {
    // `MASK_TRAVEL`, not `is_moving`; see the mask's note. A swimmer holding
    // the ascend key carries `MOVEFLAG_PITCH_UP`, which is in the anticheat's
    // moving mask but is not horizontal travel. Testing that mask would make
    // the key to go straight up also swim the character forwards.
    if !info.has(move_flags::MASK_TRAVEL) {
        return [0.0, 0.0];
    }
    let speed = info.speed(speeds);
    let heading = info.heading();
    // A swimmer's stroke is only partly horizontal. The body is pitched
    // ([`MovementInfo::pitch`], the field the wire carries only under
    // `MOVEFLAG_SWIMMING`), so a stroke along the body covers `cos(pitch)` of
    // its length horizontally and the rest in [`climbed`]. Diving straight
    // down at `pitch = -pi/2` therefore does not change map position.
    let flat = if info.has(move_flags::SWIMMING) {
        info.pitch.cos().max(0.0)
    } else {
        1.0
    };
    [
        heading.cos() * speed * dt * flat,
        heading.sin() * speed * dt * flat,
    ]
}

/// The vertical part of the same stroke as [`strode`], which only a swimmer
/// has.
///
/// Zero on the ground and in the air: a fall is a parabola the mover handles,
/// and a walk has no vertical component, so the ground path reads
/// [`Footing::floor`] instead. In water there is no gravity, and only the
/// swimmer's pitch and the pitch keys change the altitude.
///
/// A strafe has no vertical component, and neither does turning: the pitch
/// belongs to the body, and `FORWARD`/`BACKWARD` travel along it.
/// Backpedalling mirrors it, for the same reason [`MovementInfo::heading`]
/// mirrors the horizontal: a character pitched up and swimming backwards goes
/// down.
///
/// A free function beside [`strode`] because the two are one velocity split
/// into its components; a caller that integrated one without the other would
/// move a diver horizontally at full speed.
pub(crate) fn climbed(info: &MovementInfo, speeds: &Speeds, dt: f32) -> f32 {
    if !info.has(move_flags::SWIMMING) {
        return 0.0;
    }
    let speed = info.speed(speeds);
    // The ascend key is a full stroke on its own. `MOVEFLAG_PITCH_UP` is a
    // movement bit (it is in `MOVEFLAG_MASK_MOVING`), so the server measures
    // it as travel at swim speed, and that is the rate used here. Held alone
    // it moves straight up: [`strode`] gives no horizontal without a travel
    // key.
    //
    // What the 1.12.1 client does with both a travel key and the ascend key
    // held is not established. This code adds them: swimming forward with the
    // ascend key covers ground and rises at the same time, faster through the
    // water than either alone. The sum is not clamped, because a clamp would
    // be a second invented rule on top of an unmeasured one.
    let pitched = if info.has(move_flags::FORWARD) {
        info.pitch.sin() * speed
    } else if info.has(move_flags::BACKWARD) {
        -info.pitch.sin() * speed
    } else {
        0.0
    };
    let keyed = if info.has(move_flags::PITCH_UP) {
        speed
    } else if info.has(move_flags::PITCH_DOWN) {
        -speed
    } else {
        0.0
    };
    (pitched + keyed) * dt
}

/// What the server has taken away from this character, as distinct from the
/// movement flags, which say what it is doing.
///
/// Travel and turning are restricted separately because the 1.12.1 client
/// restricts them separately, and that is why a root and a stun play
/// differently: a rooted character still turns on the spot and still swings at
/// what it is facing, while a stunned one cannot act at all. The client decides
/// this from two different fields (see
/// [`crate::state::objects::Entity::is_stunned`], which states the rule):
/// `MOVEFLAG_ROOT` in the movement block for travel, `UNIT_FLAG_STUNNED` in
/// `UNIT_FIELD_FLAGS` for turning, with health applying to both.
///
/// Neither field here is a movement-block value. `ROOT` arrives as its own
/// packet and lands in [`MovementInfo::flags`]; these two are descriptor fields on the
/// player's own entity, so they are read out of the object manager once a tick
/// and handed to the mover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Restraint {
    /// `UNIT_FIELD_FLAGS & UNIT_FLAG_STUNNED` (0x40000): no turning.
    pub stunned: bool,
    /// `UNIT_FIELD_HEALTH == 0`. The client tests health and the stand state
    /// separately and both fail a corpse; one boolean
    /// covers them because vmangos writes the health and the stand state
    /// together in `KillPlayer`.
    pub dead: bool,
    /// `UNIT_FIELD_FLAGS & UNIT_FLAG_TAXI_FLIGHT`: the server is flying this
    /// character, and neither the keys nor the mouse move it until it lands.
    ///
    /// A ride already blocks both ([`Mover::can_move`] tests
    /// `ride.is_none()`), so during an ordinary flight this adds nothing. It
    /// covers the case where the two disagree: the flag persists across a
    /// session and the ride does not. A character who logs out mid-flight logs
    /// back in with the flag set. If the resumed spline is missed, ignoring
    /// the flag leaves the character walking on the ground while the server
    /// holds them on the flight and discards every packet they send.
    /// `SessionLoop::enter_world` previously behaved that way.
    ///
    /// The flag has one meaning: vmangos sets and clears it only in
    /// `FlightPathMovementGenerator`'s `Initialize` and `Finalize`, always
    /// beside `UNIT_FLAG_REMOVE_CLIENT_CONTROL`, whose comment is "disable
    /// player movement".
    pub on_taxi: bool,
}

/// A movement key change or a jump, given to [`Mover::advance_through`] with
/// the moment it happened.
///
/// The 1.12.1 client queues movement input with a timestamp and advances the
/// character to that moment under the old flags before it applies the input.
/// A key change therefore takes effect where the character was when the key
/// went down, not where it is at the end of the step that read it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    /// The movement keys held from this moment on.
    Controls(Controls),
    /// Leave the ground; see [`Mover::jump`].
    Jump,
}

/// A packet the mover owes the server, with the movement block as it stood at
/// the moment the packet became owed.
///
/// The block is kept because one step can owe several packets: a key change
/// part-way through it, a landing, the start of a swim. Each must report the
/// position at its own moment. A block read after the whole step has run
/// would report the end of the step for all of them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Owed {
    pub opcode: Opcode,
    pub info: MovementInfo,
}

/// The client's copy of where it is and where it is going.
///
/// The server is authoritative but only sends corrections; between corrections
/// the client simulates itself and reports the result. An incorrect simulation
/// does not just drift; it trips the speed hack check.
pub struct Mover {
    pub info: MovementInfo,
    pub speeds: Speeds,
    controls: Controls,
    /// Orientation last sent, so a facing change can be detected.
    sent_orientation: f32,
    /// The arc, while there is one. `None` means standing on something.
    air: Option<Airborne>,
    /// The server-driven ride, while there is one; see [`Ride`]. It takes
    /// precedence over the keys, the arc and the ground.
    ride: Option<Ride>,
    /// The spline id a finished ride owes the server, until the loop sends it.
    ///
    /// Held rather than sent because a handler under the world lock must not
    /// write to the socket, and because `CMSG_MOVE_SPLINE_DONE` is the one
    /// outbound movement packet that is not a bare [`MovementInfo`]; see
    /// [`spline_done_body`].
    finished_ride: Option<u32>,
    /// What the server has stopped this character doing; see [`Restraint`].
    restraint: Restraint,
    /// The moving platform this character is standing on, or `None` for most
    /// of a session. See [`Ferry`].
    ferry: Option<Ferry>,
    /// Which platform the server was last told about, so a boarding and a
    /// step ashore are each reported when they happen rather than at the next
    /// heartbeat. See [`Self::ferry_changed`].
    sent_ferry: Option<u64>,
    /// A transport teleport has restated the offset, and the world position
    /// must be recomputed against the deck's next placement; see
    /// [`Self::transfer_aboard`]. While it is set the offset is frozen:
    /// neither the carry nor the stow may recompute it from a world position,
    /// because every world position between the teleport and the first
    /// placement on the new map refers to the old map.
    ferry_recompose: bool,
}

impl Mover {
    pub fn new(position: Position, speeds: Speeds) -> Mover {
        Mover {
            info: MovementInfo {
                position,
                ..Default::default()
            },
            speeds,
            controls: Controls::default(),
            sent_orientation: position.orientation,
            air: None,
            ride: None,
            finished_ride: None,
            restraint: Restraint::default(),
            ferry: None,
            sent_ferry: None,
            ferry_recompose: false,
        }
    }

    /// The platform this character is riding, if any.
    pub fn ferry(&self) -> Option<Ferry> {
        self.ferry
    }

    /// Board or carry, once per tick and before the step.
    ///
    /// `under` is the moving object directly underfoot, if any. The caller
    /// filters out objects that do not move, because this crate does not know
    /// which game objects are transports and the object manager does
    /// (`UPDATEFLAG_TRANSPORT`, `Entity::transport_phase_ms`).
    ///
    /// All three cases are handled here rather than at the call site, because
    /// their ordering matters:
    ///
    /// * Still on the same platform: the character is moved by however far the
    ///   platform moved, before their own step is taken, so they walk from
    ///   where the deck put them. This is the carry.
    /// * A different platform, or a first boarding: the offset is taken from
    ///   where they already are. Nothing moves.
    /// * Nothing underfoot: the flag and the block are removed. The position is
    ///   left unchanged: stepping off a moving deck leaves the character where
    ///   they were standing, and recomputing anything here would teleport a
    ///   character who walked down the gangway.
    ///
    /// This function must be paired with [`Self::stow_platform`]: the offset it
    /// leaves describes where the character stood before their stride, so
    /// `stow_platform` must run after the step to record where they ended up.
    /// That function's doc gives the reason.
    ///
    /// The carry applies a delta, not a stored place. The offset is recomputed
    /// from wherever the character is when this runs, so a heading the mouse
    /// set, a position the server corrected and a knockback the server sent
    /// (all three applied before `tick_movement` begins) are kept while aboard.
    /// Reading the recorded offset instead would restore the state of a tick
    /// ago over all three.
    ///
    /// Returns whether the world position changed, which the caller uses to
    /// decide whether to tell the server sooner than the heartbeat.
    pub(crate) fn carry_platform(&mut self, under: Option<Platform>) -> bool {
        let Some(platform) = under else {
            // A transfer in progress is not a step ashore. The server kept the
            // deck across a far teleport (`TELE_TO_NOT_LEAVE_TRANSPORT`), and
            // the world has not placed it on the new map yet. Dropping the
            // ferry here would send a packet without the flag, the server
            // would call `RemovePassenger`, and the character would be left
            // wherever the loading screen caught them.
            if self.ferry_recompose {
                return false;
            }
            let was = self.ferry.take().is_some();
            if was {
                self.info.flags &= !move_flags::ONTRANSPORT;
                self.info.transport = None;
            }
            return false;
        };
        let carried = match self.ferry {
            // A transport teleport's first placement on the new map: the
            // position is composed from the offset directly, not carried as a
            // delta. The recorded placement describes the map that was left,
            // and between the teleport and now the position was resynced, first
            // to `SMSG_NEW_WORLD`'s body and then to the new map's create
            // block. A delta between those two frames would add thousands of
            // yards of error to the offset. Both ends still agree on the offset
            // (`HandleMoveWorldportAck` relocates a passenger with
            // `UpdatePassengerPosition`, from the offset, rather than to the
            // coordinates it sent), so the position is rebuilt from the offset
            // alone.
            Some(ferry) if ferry.guid == platform.guid && self.ferry_recompose => {
                // Against the held record there is nothing to compose yet: the
                // placement passed in is the stale one the hold repeats, and
                // the offset stays frozen until a new placement arrives.
                if !ferry.moved(platform) {
                    return false;
                }
                let before = self.info.position;
                self.info.position = ferry.carry(platform);
                self.ferry_recompose = false;
                if let Some(air) = self.air.as_mut() {
                    air.start[0] += self.info.position.x - before.x;
                    air.start[1] += self.info.position.y - before.y;
                    air.start[2] += self.info.position.z - before.z;
                }
                true
            }
            // Same deck, and it has moved: this is the ride.
            Some(ferry) if ferry.guid == platform.guid && ferry.moved(platform) => {
                let before = self.info.position;
                // The offset is recomputed from the character's current
                // position, not read from the record. Between the last tick's
                // `stow_platform` and this line, three things can write the
                // position or the heading: `Mover::face` from the mouse,
                // `Mover::resync` from a server correction, and a knockback.
                // All three are applied before `tick_movement` runs. Carrying
                // the stored offset would overwrite them with the state of a
                // tick ago, the same fault as the frozen stride described on
                // `stow_platform`, applied to a different quantity.
                //
                // This showed with mouse-look: `face` writes an absolute world
                // heading and the carry restored the old one, so a passenger
                // could turn with A and D (which run inside `advance`, after
                // this) but not with the mouse. Recomputed this way the carry
                // is a delta: the character is moved and turned by however much
                // the deck was, and nothing else changes. When nothing wrote,
                // the result equals the stored offset, so the ride is
                // unchanged.
                let ferry = Ferry::aboard(ferry.platform(), before);
                self.info.position = ferry.carry(platform);
                // An arc in progress is carried too, by translating its start.
                // `advance` computes the vertical from `Airborne::start` and
                // overwrites whatever the carry above put in `z`, so a jump
                // taken on a rising lift would otherwise be measured against
                // the world: the parabola would come down where the deck used
                // to be, and the floor query would read that as falling through
                // it. Moving the start keeps the arc in the platform's frame,
                // the frame the character jumped in.
                //
                // All three axes are moved, though only `z` is read: the
                // horizontal is fixed at take-off in the jump block, and the
                // carry above has already moved `x`/`y`. Moving all three costs
                // two additions and keeps the record a single point in one
                // frame, rather than mixing axes from two frames.
                if let Some(air) = self.air.as_mut() {
                    air.start[0] += self.info.position.x - before.x;
                    air.start[1] += self.info.position.y - before.y;
                    air.start[2] += self.info.position.z - before.z;
                }
                true
            }
            // Same deck, standing still: nothing to do but keep the block current.
            Some(ferry) if ferry.guid == platform.guid => false,
            // A first boarding, or a step from one deck to another. A pending
            // recompose is about a deck this is not, so it goes.
            _ => {
                self.ferry_recompose = false;
                false
            }
        };
        self.stow_against(platform);
        carried
    }

    /// Record where the step ended, once per tick and after the step. The
    /// counterpart of [`Self::carry_platform`].
    ///
    /// The offset is the character's place in the platform's own frame, and it
    /// is what the wire carries. vmangos' `HandleMoverRelocation` discards the
    /// world position for a passenger and recomputes it from `t_pos` against
    /// its own copy of the transport, so the server treats the offset as the
    /// position.
    ///
    /// Taking the offset before the stride instead of after froze the
    /// character to the deck. The carry put the passenger where the platform
    /// moved them and the offset was recomputed there; `advance` then took the
    /// stride the keys asked for; and the next tick's carry put the character
    /// back at the offset recorded before the stride. Every stride taken while
    /// the platform was moving was undone by the following tick, forty times
    /// a second. Walking on a stationary lift worked, so the symptom was that
    /// the character could not move while the platform moved.
    ///
    /// This uses the same placement [`Self::carry_platform`] used. Querying
    /// the world again here would cost a second collision query per tick and
    /// would add any mid-tick platform movement to the character's own stride.
    /// A character who walked off the deck is dropped by the next tick's
    /// query, one step later, which is the tick they would have been dropped
    /// on anyway.
    pub(crate) fn stow_platform(&mut self) {
        // Frozen across a transfer. Between the teleport and the first
        // placement on the new map every world position refers to the old
        // map. Only the offset the packet restated is valid, and recomputing
        // it from one of those positions would throw the character thousands
        // of yards. See [`Self::transfer_aboard`].
        if self.ferry_recompose {
            return;
        }
        let Some(ferry) = self.ferry else {
            return;
        };
        self.stow_against(ferry.platform());
    }

    /// Take the offset from wherever the character is now, and set the flag and
    /// the block on the movement info. Both [`Self::carry_platform`] and
    /// [`Self::stow_platform`] end here.
    fn stow_against(&mut self, platform: Platform) {
        let ferry = Ferry::aboard(platform, self.info.position);
        self.ferry = Some(ferry);
        self.info.flags |= move_flags::ONTRANSPORT;
        self.info.transport = Some(TransportInfo {
            guid: platform.guid,
            position: ferry.offset,
        });
    }

    pub fn controls(&self) -> Controls {
        self.controls
    }

    /// What the server has stopped this character doing, as of the last tick
    /// that read its descriptors.
    pub fn restraint(&self) -> Restraint {
        self.restraint
    }

    /// Adopt this tick's reading of the two descriptor fields.
    ///
    /// Called from the session loop rather than from a packet handler, because
    /// neither field has a packet of its own: both arrive inside whichever
    /// values block changed, and a stun is a change to `UNIT_FIELD_FLAGS` in
    /// the same update as the aura that caused it.
    pub fn set_restraint(&mut self, restraint: Restraint) {
        self.restraint = restraint;
    }

    /// Whether this character may travel. Matches the 1.12.1 client's travel
    /// rule.
    ///
    /// A root, and death, block travel. A stun is not tested: every stun also
    /// roots (`HandleAuraModStun` ends in `SetRooted(true)`), so the stun flag
    /// would be redundant here, and testing it instead of the root would make
    /// the two indistinguishable.
    ///
    /// A [`Ride`] also blocks travel, which the 1.12.1 client's rule does not
    /// include. The root and death mean the server refuses movement; a ride
    /// means the server is moving the character along a stated path, so the
    /// keys have no effect. It is tested here rather than beside every caller
    /// so that the stride, the jump and the keypress all agree.
    ///
    /// `UNIT_FLAG_TAXI_FLIGHT` blocks travel too: it is the same condition as
    /// a ride, and it persists across a relog. See [`Restraint::on_taxi`] for
    /// why a ride's absence is not enough on its own.
    pub fn can_move(&self) -> bool {
        !self.restraint.dead
            && !self.restraint.on_taxi
            && !self.info.has(move_flags::ROOT)
            && self.ride.is_none()
    }

    /// Whether this character may turn. Matches the 1.12.1 client's turn rule.
    ///
    /// A stun, and death, block turning. A root does not: a rooted character
    /// turns on the spot, which is why Frost Nova can be fought out of and a
    /// Hammer of Justice cannot.
    ///
    /// A taxi flight blocks turning too (`UNIT_FLAG_TAXI_FLIGHT`).
    ///
    /// A [`Ride`] also blocks turning, for the reason [`Self::can_move`] gives:
    /// the spline states the facing too (a charge carries `SetFacingGUID`), so
    /// a mouse drag mid-charge would be a second source of steering. The
    /// camera still turns, as in 1.12; see `send_input`.
    pub fn can_turn(&self) -> bool {
        !self.restraint.dead
            && !self.restraint.stunned
            && !self.restraint.on_taxi
            && self.ride.is_none()
    }

    pub fn position(&self) -> Position {
        self.info.position
    }

    /// How fast this character is travelling, in yards per second: the number a
    /// gait is chosen from, and zero when it is standing still.
    ///
    /// A [`Ride`] moves at none of the six speeds in [`Speeds`], which is why
    /// this exists rather than the call sites reading `info.speed(&speeds)`. A
    /// charge is run speed × 4 capped at 24 (`ChargeMovementGenerator::
    /// ComputePath`), and the wire does not state it: the server sends a path
    /// and a duration, and the speed follows from those. Reading the run speed
    /// instead draws a character sprinting a third as fast as they are moving.
    pub(crate) fn travel_speed(&self) -> f32 {
        if let Some(ride) = self.ride.as_ref() {
            return ride.spline.speed();
        }
        if self.info.is_moving() {
            self.info.speed(&self.speeds)
        } else {
            0.0
        }
    }

    /// The velocity of a server-driven ride, in yards per second, or `None` when
    /// nothing is driving.
    ///
    /// A three-dimensional vector. A reader continuing the simulation between
    /// ticks needs the vertical component, which [`Self::travel_speed`] and the
    /// block's orientation do not give: a taxi flight spends most of its length
    /// climbing or diving, and speed plus orientation describe only a flat
    /// path. See [`Spline::velocity`].
    pub fn ride_velocity(&self) -> Option<[f32; 3]> {
        Some(self.ride.as_ref()?.spline.velocity())
    }

    /// The path a server-driven ride follows and how far along it the
    /// character is, or `None` when nothing is driving.
    ///
    /// A reader drawing the character between simulation steps samples the
    /// curve at `elapsed` plus the time since the step, with
    /// [`Spline::at_secs`], and lands on the position and heading the next step
    /// will have. Continuing from [`Self::ride_velocity`] instead draws a
    /// straight line that leaves the curve on every bend and is pulled back to
    /// it on each step, and holds the heading still between steps.
    pub fn ride_curve(&self) -> Option<RideCurve> {
        let ride = self.ride.as_ref()?;
        Some(RideCurve {
            spline: ride.published.clone(),
            elapsed: ride.elapsed,
        })
    }

    /// Accept the server's version of the character's position. Called when an
    /// update for the player's own GUID arrives.
    ///
    /// Any arc in progress ends here. The server has just stated a position,
    /// and a parabola anchored at the previous position would carry the
    /// character down through it. The correction and the fall would conflict,
    /// and the character would appear to teleport and then sink into the floor.
    pub fn resync(&mut self, position: Position) {
        self.info.position = position;
        self.sent_orientation = position.orientation;
        if self.air.is_some() {
            self.air = None;
            self.info.flags &= !(move_flags::JUMPING | move_flags::FALLINGFAR);
            self.info.fall_time = 0;
            self.info.jump = JumpInfo::default();
        }
    }

    /// Change which keys are held, returning the packets that owes the server.
    ///
    /// A rooted character reports no keys at all, because of an anticheat
    /// rule. [`transitions`] starts from the flags the block already has and
    /// adds to them, so a `W` pressed while stunned put `MOVEFLAG_FORWARD` on
    /// top of a `MOVEFLAG_ROOT` the server had set, and sent it as
    /// `MSG_MOVE_START_FORWARD`. That matches `CHEAT_TYPE_ROOT_MOVE`:
    ///
    /// ```text
    /// // Moving while rooted. We do not apply root flag until ack, so should be no false positives.
    /// if ((currentMoveFlags & MOVEFLAG_MASK_MOVING) &&
    ///     ((currentMoveFlags & MOVEFLAG_ROOT) || (GetLastMovementInfo().moveFlags & MOVEFLAG_ROOT)) &&
    ///     !me->HasPendingMovementChange(ROOT) && (opcode != CMSG_FORCE_MOVE_UNROOT_ACK))
    /// ```
    ///
    /// It carries a per-tick and a running penalty and can be configured to
    /// reject outright, so any keypress during a stun produced an
    /// `Anticheat.log` entry. The keys are still recorded: the character still
    /// turns, and [`Self::set_rooted`] restores whatever is held when the root
    /// lifts, in the one packet the check exempts
    /// (`CMSG_FORCE_MOVE_UNROOT_ACK`).
    pub fn set_controls(&mut self, controls: Controls) -> Vec<MoveEvent> {
        self.controls = controls;
        if !self.can_move() {
            return Vec::new();
        }
        // The pitch bits are added here rather than left to the reconcile, so
        // that pressing the ascend key in the water takes effect on the press
        // like every other key. [`Self::reconcile_pitch`] is still needed:
        // entering or leaving the water also changes the pitch bits, and that
        // is not a keypress. The two cannot disagree, because both compute
        // [`Self::pitch_flags`].
        let events = transitions(self.info.flags, controls.to_flags() | self.pitch_flags());
        if let Some(last) = events.last() {
            self.info.flags = last.flags;
        }
        events
    }

    /// Set an absolute heading, for mouse-look, rather than turning at a rate.
    ///
    /// It applies the same [`Self::can_turn`] test as the turn keys. The mouse
    /// drag used to write `mover.info.position.orientation` directly from the
    /// session's command queue, so a dead or stunned character could not turn
    /// with A and D but could turn freely with the mouse.
    ///
    /// Returns whether the heading was taken, so the caller can decide whether
    /// it still owes the server a `MSG_MOVE_SET_FACING`.
    pub fn face(&mut self, orientation: f32) -> bool {
        if !self.can_turn() {
            return false;
        }
        self.info.position.orientation = wrap_angle(orientation);
        true
    }

    /// Whether the facing differs enough from the last one sent to need a
    /// `MSG_MOVE_SET_FACING`. A hundredth of a radian is well under what an
    /// observer could see, and the threshold keeps small turns from flooding
    /// the socket.
    pub(crate) fn facing_changed(&self) -> bool {
        (self.info.position.orientation - self.sent_orientation).abs() > 0.01
    }

    pub(crate) fn mark_sent(&mut self) {
        self.sent_orientation = self.info.position.orientation;
        self.sent_ferry = self.ferry.map(|ferry| ferry.guid);
    }

    /// Whether the character has boarded, changed deck or stepped ashore since
    /// the last packet was sent.
    ///
    /// The server learns of boarding and leaving in one place only:
    /// `HandleMoverRelocation` calls `AddPassenger` when a movement packet
    /// arrives carrying `MOVEFLAG_ONTRANSPORT` and a guid it can find, and
    /// `RemovePassenger` when one arrives without the flag. Nothing else in
    /// vmangos boards or removes a passenger. Until the client reports the
    /// change, the server does not know the character is on the boat, and two
    /// things break:
    ///
    /// * The ride. `Transport::TeleportTransport` moves only `m_passengers`,
    ///   so a passenger the server never registered is left in the ocean when
    ///   the boat crosses to the other continent.
    /// * The anticheat. `IsTeleportAllowed3D` returns true for a player the
    ///   server has on a transport, and otherwise measures world distance
    ///   against `CONFIG_FLOAT_AC_MOVEMENT_CHEAT_TELEPORT_DISTANCE`. A boat
    ///   covers 30 yards a second, so the first packet sent after a gap is
    ///   treated as a teleport hack and rejected, and `SendHeartBeat(true)`
    ///   puts the character back where the deck used to be.
    ///
    /// The first shows as the ship disappearing and leaving the character in
    /// the sea. The second shows as the character sliding about the deck with
    /// shuffling feet: a server correction and the local carry conflicting
    /// forty times a second.
    pub(crate) fn ferry_changed(&self) -> bool {
        self.ferry.map(|ferry| ferry.guid) != self.sent_ferry
    }

    /// A far teleport kept the character on the deck: adopt the offset it
    /// restated, and recompose the world position at the deck's next
    /// placement.
    ///
    /// The counterpart of [`Self::disembark`], for the one teleport in which a
    /// character stays on a boat. In this case `SMSG_NEW_WORLD`'s four floats
    /// are the transport offset, not a world position. `Player::SendNewWorld`
    /// writes `m_movementInfo.GetTransportPos()` when `m_transport` is set and
    /// `m_teleportDest` only when it is not, and `HandleMoveWorldportAck` then
    /// relocates the passenger with `UpdatePassengerPosition`, from the offset,
    /// rather than to any coordinate it sent. Reading the floats as a world
    /// position puts the character at the map origin.
    ///
    /// This function replaces the recorded offset with the packet's (they agree
    /// to the millimetre unless a heartbeat was in flight, and the server will
    /// compose with its own copy), keeps the flag and the block, and sets
    /// `ferry_recompose`. The first placement of the kept deck on the new map
    /// then rebuilds the position as `carry(offset, placement)`. Until then the
    /// offset is frozen. In the meantime the world position is first the old
    /// map's coordinates and then whatever the new map's create block restates;
    /// an offset derived from either against the old placement is off by
    /// several thousand yards and goes out on the wire as `t_pos`. The server
    /// composes passengers from `t_pos` alone, so it teleports the character
    /// there, far from the ship.
    ///
    /// A no-op when there is no ferry, which is every teleport but this one.
    pub(crate) fn transfer_aboard(&mut self, offset: Position) {
        let Some(ferry) = self.ferry.as_mut() else {
            return;
        };
        ferry.offset = offset;
        self.ferry_recompose = true;
        self.info.transport = Some(TransportInfo {
            guid: ferry.guid,
            position: offset,
        });
        self.info.flags |= move_flags::ONTRANSPORT;
    }

    /// Forget the deck. Called on a far teleport and nowhere else.
    ///
    /// [`Self::carry_platform`] moves the character by however far the platform
    /// moved since the offset was taken, and after a teleport the two readings
    /// are not comparable: the recorded placement is where the boat stood on
    /// the map that was left, and the first reading on the new map is an ocean
    /// away. Carrying that difference would move the character across the
    /// world.
    ///
    /// Dropping the ferry instead re-boards from wherever the server has just
    /// put the character, on the next tick, through the same first-boarding
    /// case a gangplank uses, which moves nothing.
    ///
    /// It does not report a step ashore. The server has already decided:
    /// `Player::TeleportTo` calls `RemovePassenger` itself unless the caller
    /// passed `TELE_TO_NOT_LEAVE_TRANSPORT`, and the one caller that passes it
    /// is `Transport::TeleportTransport`, which keeps the character aboard on
    /// purpose. So the record of what the server was last told is cleared with
    /// the ferry: a packet without the flag sent in that window would leave the
    /// character in the sea at the moment the boat carried them across it. The
    /// re-boarding a tick later is still a change and is still reported.
    pub(crate) fn disembark(&mut self) {
        if self.ferry.take().is_some() {
            self.info.flags &= !move_flags::ONTRANSPORT;
            self.info.transport = None;
        }
        self.sent_ferry = None;
        self.ferry_recompose = false;
    }

    /// Whether the character is in the air: jumping, knocked back, or falling.
    pub fn is_airborne(&self) -> bool {
        self.air.is_some()
    }

    /// Whether the character is in the air because it jumped, rather than
    /// because it walked off something. The two are the same parabola to the
    /// server and different animations to the renderer.
    pub fn is_jumping(&self) -> bool {
        self.air.is_some_and(|a| a.jumped)
    }

    /// Jump.
    ///
    /// Returns the packet owed to the server, or `None` when there is nothing
    /// to jump from. The main `None` case is being already airborne:
    /// `CHEAT_TYPE_MULTI_JUMP` allows one `MSG_MOVE_JUMP` before a landing and
    /// rejects the rest (`MultiJump.Reject = 1`), so with a held space bar the
    /// server would silently discard every second jump while the client
    /// continued the arc.
    pub fn jump(&mut self) -> Option<MoveEvent> {
        // A swimmer ascends instead of jumping. `MSG_MOVE_JUMP` sent from the
        // water would be a parabola in a medium without gravity; in water the
        // same key sets `MOVEFLAG_PITCH_UP`, a held control with its own three
        // opcodes. See [`Self::pitch_flags`].
        if self.air.is_some() || self.is_swimming() || !self.can_move() {
            return None;
        }
        // A jump carries the current running speed horizontally.
        // `CHEAT_TYPE_OVERSPEED_JUMP` compares `jump.xyspeed` against the
        // server's own speed for the previous packet's flags, so this must be
        // the speed those flags imply, not an independently chosen value.
        let speed = self.info.speed(&self.speeds);
        self.take_off(JUMP_SPEED, speed, self.info.heading(), true);
        Some(MoveEvent {
            opcode: Opcode::MSG_MOVE_JUMP,
            flags: self.info.flags,
        })
    }

    /// The server has knocked this character back: `SMSG_MOVE_KNOCK_BACK`.
    ///
    /// The four numbers are echoed back unchanged in the ack
    /// (`FindPendingMovementKnockbackChange` compares each to within 0.01), so
    /// they are stored rather than recomputed from a heading. `z_speed` arrives
    /// down-positive (`Unit::KnockBack` sends `-verticalSpeed`, with the
    /// comment "notice the - sign"), so a knock up is a negative wire value.
    /// The up-positive simulation negates it, and the jump block keeps the
    /// wire value for the ack.
    pub(crate) fn knock_back(&mut self, cos_angle: f32, sin_angle: f32, xy_speed: f32, z_speed: f32) {
        self.air = Some(Airborne {
            start: [
                self.info.position.x,
                self.info.position.y,
                self.info.position.z,
            ],
            elapsed: 0.0,
            up: -z_speed,
            jumped: false,
        });
        self.info.flags |= move_flags::JUMPING;
        self.info.fall_time = 0;
        self.info.jump = JumpInfo {
            z_speed,
            cos_angle,
            sin_angle,
            xy_speed,
        };
    }

    /// Start a server-driven ride: an `SMSG_MONSTER_MOVE` naming this
    /// character. See [`Ride`] for what that is and why it cannot be ignored.
    ///
    /// `facing_at` is where a [`SplineFacing::Target`] is looking, resolved by
    /// the caller because only the object manager can find another unit.
    ///
    /// A stop packet is also a ride, by the server's own rules:
    /// `ChargeMovementGenerator::Initialize` opens with `if (!unit.IsStopped())
    /// unit.StopMoving()`, which is a second `MoveSplineInit::Launch` with
    /// `SetStop()`, and vmangos comments it "Will trigger CMSG_MOVE_SPLINE_DONE
    /// from client". A charge is therefore two packets, and both set the
    /// pending flag. The second supersedes the first, so an ack owed for a
    /// spline that has already been replaced is dropped rather than sent (the
    /// server compares the id and would discard it anyway).
    pub fn ride(&mut self, mm: &MonsterMove, facing_at: Option<[f32; 3]>) {
        self.finished_ride = None;
        self.resync(Position {
            x: mm.start[0],
            y: mm.start[1],
            z: mm.start[2],
            orientation: self.info.position.orientation,
        });

        if mm.duration_ms == 0 || mm.path.len() < 2 {
            // No path to follow: a stop, or a bare `SetFacingTo`. The server
            // still has the player marked spline-pending, so the ack is owed
            // at once.
            self.ride = None;
            self.finished_ride = Some(mm.spline_id);
            self.stand_down(mm.facing, facing_at);
            return;
        }

        let flying = mm.flags & spline_flags::FLYING != 0;
        let spline = Spline {
            path: mm.path.clone(),
            duration_ms: mm.duration_ms,
            elapsed_ms: 0,
            facing: mm.facing,
            // A cyclic spline never ends, so its ack would never be sent. The
            // server does not send a player one; treating an unexpected one as
            // an ordinary path guarantees the ride ends.
            cyclic: false,
            flying,
            // A ride's vertical is the server's path; the local mover handles
            // the ground for the player and `grounded_z` is never called here.
            // Stored so the flag is not silently dropped.
            falling: mm.flags & spline_flags::FALLING != 0,
            // A ride moves the player's own character, and the server does not
            // put a player on a transport spline: `MoveSplineInit` only sends
            // `SMSG_MONSTER_MOVE_TRANSPORT` for a unit it is also boarding, and
            // a player boards itself. Stored (normally `None`) rather than
            // dropped, so the field is not forgotten if that changes.
            on_transport: mm.transport,
        };
        self.ride = Some(Ride {
            published: std::sync::Arc::new(spline.clone()),
            spline,
            elapsed: 0.0,
            spline_id: mm.spline_id,
            facing_at,
        });
        // The arc and the keys both stop applying. The server's `Launch`
        // clears `MOVEFLAG_MASK_MOVING` and sets `SPLINE_ENABLED | FORWARD` on
        // its copy of this block, so both ends agree on the flags without a
        // packet. `FORWARD` also makes the renderer draw a run rather than a
        // character sliding in its idle pose; it is the same substitution
        // [`crate::state::objects::Entity::move_flags`] makes for a creature on
        // a spline, for the same reason.
        self.air = None;
        self.info.fall_time = 0;
        self.info.jump = JumpInfo::default();
        self.info.flags &= !move_flags::MASK_MOVING;
        self.info.flags |= move_flags::SPLINE_ENABLED | move_flags::FORWARD;
        // `MOVEFLAG_FLYING` is also set for a flying ride. The server holds it
        // for the whole of a taxi flight and removes it in
        // `FlightPathMovementGenerator::Finalize`; [`Self::stand_down`] removes
        // it on this end. The renderer chooses the animation from it, and
        // without it a gryphon at 32 y/s plays a `Run` authored for 6.9.
        if flying {
            self.info.flags |= move_flags::FLYING;
        }
    }

    /// Whether the server is driving this character now.
    pub(crate) fn is_riding(&self) -> bool {
        self.ride.is_some()
    }

    /// Abandon a ride without acknowledging it.
    ///
    /// Called on a teleport, near or far. `HandleMoveSplineDoneOpcode` returns
    /// early while `IsBeingTeleported()`, so an ack sent across a relocation is
    /// discarded, and the ride's path leads somewhere on a map the character
    /// has left. The relocation returns control to the client in place of the
    /// ack.
    pub(crate) fn abandon_ride(&mut self) {
        if self.ride.take().is_some() {
            self.stand_down(SplineFacing::Travel, None);
        }
        self.finished_ride = None;
    }

    /// The spline id an ended ride owes the server, taken once.
    pub(crate) fn take_finished_ride(&mut self) -> Option<u32> {
        self.finished_ride.take()
    }

    /// Restore the block to that of a character not on a ride, and turn to the
    /// facing the packet asked for.
    fn stand_down(&mut self, facing: SplineFacing, facing_at: Option<[f32; 3]>) {
        let p = self.info.position;
        let here = [p.x, p.y, p.z];
        let orientation = match facing {
            SplineFacing::Angle(a) => a,
            SplineFacing::Spot(spot) => bearing(here, spot),
            SplineFacing::Target(_) => match facing_at {
                Some(at) => bearing(here, at),
                None => p.orientation,
            },
            SplineFacing::Travel => p.orientation,
        };
        self.info.position.orientation = wrap_angle(orientation);
        // Nothing was sent during the ride, and the ack carries this
        // orientation, so a `MSG_MOVE_SET_FACING` a tick later would repeat
        // what the server has just been told.
        self.sent_orientation = self.info.position.orientation;
        self.info.flags &=
            !(move_flags::SPLINE_ENABLED | move_flags::MASK_MOVING | move_flags::FLYING);
        // The keys are still held: whatever they imply resumes from the
        // endpoint, as an unroot restores the keys held during the root.
        if self.can_move() {
            self.info.flags |= self.controls.to_flags() | self.pitch_flags();
        }
    }

    /// One step of a ride. Returns whether one was taken.
    fn ride_step(&mut self, dt: f32) -> bool {
        let Some(ride) = self.ride.as_mut() else {
            return false;
        };
        ride.elapsed += dt;
        ride.spline.elapsed_ms = (ride.elapsed * 1000.0) as u32;
        // Sampled at the fractional time rather than the whole milliseconds
        // above, so a renderer sampling the same curve at the same moment with
        // [`Spline::at_secs`] lands on this position exactly.
        let (position, heading) = ride.spline.at_secs(ride.elapsed);
        let finished = ride.spline.finished();
        let facing = ride.spline.facing;
        let facing_at = ride.facing_at;
        let spline_id = ride.spline_id;

        self.info.position.x = position[0];
        self.info.position.y = position[1];
        self.info.position.z = position[2];
        self.info.position.orientation = wrap_angle(heading);
        // Set every step rather than only at the start: a root or a stun
        // arriving mid-charge goes through `set_rooted`, which clears
        // `MOVEFLAG_MASK_MOVING`, and the character would finish the ride in
        // its idle pose. vmangos applies a pending root in
        // `ChargeMovementGenerator::Finalize`, after the spline, so applying it
        // here would apply it early.
        self.info.flags |= move_flags::SPLINE_ENABLED | move_flags::FORWARD;

        if finished {
            self.ride = None;
            self.finished_ride = Some(spline_id);
            self.stand_down(facing, facing_at);
        }
        true
    }

    /// `SMSG_FORCE_MOVE_ROOT` / `UNROOT`, applied.
    ///
    /// Rooting clears the moving flags because the server's
    /// `ResolvePendingMovementChange` does (`RemoveUnitMovementFlag(
    /// MOVEFLAG_MASK_MOVING)`), and a packet carrying both is
    /// `CHEAT_TYPE_ROOT_MOVE`; five of those cause a kick. Turning still works:
    /// a root stops translation only.
    pub fn set_rooted(&mut self, rooted: bool) {
        if rooted {
            self.info.flags &= !move_flags::MASK_MOVING;
            self.info.flags |= move_flags::ROOT;
            self.air = None;
            self.info.fall_time = 0;
        } else {
            self.info.flags &= !move_flags::ROOT;
            // The keys are still held; restore whatever they imply. The unroot
            // ack may carry them because `CheckMoveStart` exempts the ack
            // opcodes. This is skipped if something else still prevents
            // travel: a corpse whose root is lifted still cannot walk, and
            // reporting `FORWARD` while `advance` does not translate is what
            // `CheckSpeedHack` detects.
            if self.can_move() {
                self.info.flags |= self.controls.to_flags();
            }
        }
    }

    /// Begin an arc. Shared by the jump, the knockback and the step off a ledge
    /// so that all three set the start point and flags identically: a fall
    /// without `fall_time` is
    /// `CHEAT_TYPE_NO_FALL_TIME`, and one without the flag is a teleport.
    fn take_off(&mut self, up: f32, xy_speed: f32, heading: f32, jumped: bool) {
        let p = self.info.position;
        self.air = Some(Airborne {
            start: [p.x, p.y, p.z],
            elapsed: 0.0,
            up,
            jumped,
        });
        self.info.flags |= move_flags::JUMPING;
        self.info.fall_time = 0;
        self.info.jump = JumpInfo {
            // The wire value is down-positive: the 1.12.1 client reports a jump
            // as `zspeed = -7.9558`, and the server's knockback packet sends
            // `-verticalSpeed` in the same field (`Unit::KnockBack`, "notice
            // the - sign"). `Airborne::up` stays up-positive; only the wire
            // value is negated.
            z_speed: -up,
            cos_angle: heading.cos(),
            sin_angle: heading.sin(),
            xy_speed,
        };
    }

    /// Whether the character is in water deep enough to swim in.
    pub fn is_swimming(&self) -> bool {
        self.info.has(move_flags::SWIMMING)
    }

    /// The pitch bits the held keys imply. They are legal only in water.
    ///
    /// `MOVEFLAG_PITCH_UP` (0x40) and `PITCH_DOWN` (0x80) are how 1.12 swims
    /// up and down. The 1.12.1 client applies the ascend and descend keys only
    /// while `MOVEFLAG_SWIMMING` (0x200000) is set. It treats the two keys as an
    /// opposed pair that produces one of three opcodes:
    /// `MSG_MOVE_START_PITCH_UP` (191), `START_PITCH_DOWN` (192) and
    /// `STOP_PITCH` (193).
    ///
    /// This is therefore not in [`Controls::to_flags`]: a character holding the
    /// same key on dry land is jumping, and sending `PITCH_UP` there would be a
    /// movement flag the 1.12.1 client never sends.
    fn pitch_flags(&self) -> u32 {
        if !self.is_swimming() {
            return 0;
        }
        if self.controls.ascend && !self.controls.descend {
            move_flags::PITCH_UP
        } else if self.controls.descend && !self.controls.ascend {
            move_flags::PITCH_DOWN
        } else {
            0
        }
    }

    /// Bring the pitch bits into line with the keys, returning the packet owed
    /// to the server.
    ///
    /// One reconcile replaces a branch in each of the two places that can
    /// change the bits: a key press, and entering or leaving the water, which
    /// is not a keypress. It runs once a tick from [`Self::advance`], so a
    /// swimmer who enters the water with the key already held starts rising
    /// 25 ms later.
    ///
    /// It cannot be merged into the swim transition: `CheckMoveFlags` treats
    /// any opcode other than `MSG_MOVE_START_PITCH_UP` that introduces
    /// `MOVEFLAG_PITCH_UP` as a cheat, so the bit may not be sent on the
    /// `MSG_MOVE_START_SWIM` that made it legal.
    fn reconcile_pitch(&mut self) -> Option<Opcode> {
        let pitching = move_flags::PITCH_UP | move_flags::PITCH_DOWN;
        let wanted = self.pitch_flags();
        if self.info.flags & pitching == wanted {
            return None;
        }
        // Take the first event, not the last. A change from up to down is a
        // stop and then a start, and `advance` reports one packet a tick.
        // Taking the last would apply both flag changes while sending only one
        // of the two opcodes, which `CheckMoveFlags` rejects. The next tick
        // reconciles again and sends the other.
        let events = transitions(self.info.flags, (self.info.flags & !pitching) | wanted);
        let first = events.first()?;
        self.info.flags = first.flags;
        Some(first.opcode)
    }

    /// The body's vertical angle, in radians, up-positive.
    ///
    /// Meaningful only while swimming: it is the field the movement block
    /// carries under `MOVEFLAG_SWIMMING` and under nothing else. It is the
    /// camera's pitch rather than a separate control: a diver goes down by
    /// looking down and swimming forward.
    ///
    /// It sets the aim, not the whole of the vertical movement:
    /// `Bindings.xml`'s `PITCHUP` and `PITCHDOWN` are a held control with a
    /// movement flag each, handled by [`Self::pitch_flags`].
    ///
    /// Clamped to just inside the poles, because the value goes on the wire and
    /// `MaNGOS::IsValidMapCoord` requires it finite. It is stored in every
    /// state, so that the first packet after entering the water carries the
    /// pitch the player was already looking at rather than zero.
    pub(crate) fn set_pitch(&mut self, pitch: f32) {
        use std::f32::consts::FRAC_PI_2;
        self.info.pitch = pitch.clamp(-FRAC_PI_2 + 0.01, FRAC_PI_2 - 0.01);
    }

    /// The water half of one step: enter it, swim in it, or leave it.
    ///
    /// Returns the packet the transition owes the server. There are two, and
    /// each has an anticheat rule: `MSG_MOVE_START_SWIM` must carry
    /// `MOVEFLAG_SWIMMING` and no other opcode may introduce it, and
    /// `MSG_MOVE_STOP_SWIM` must not carry it (`CheckMoveFlags`, and
    /// `CHEAT_TYPE_FLY_HACK_SWIM` for the second). The flag is therefore
    /// updated before the return in both directions.
    ///
    /// The thresholds are [`MIN_SWIM_DEPTH`] and its hysteresis, both matching
    /// the 1.12.1 client. What this function decides itself, and states in the
    /// two places it applies, is what happens to the altitude at each
    /// transition.
    fn tick_water(&mut self, surface: Option<f32>, floor: Option<f32>, dt: f32) -> Option<Opcode> {
        let depth = surface.map(|s| s - self.info.position.z);
        if !self.info.has(move_flags::SWIMMING) {
            // Not swimming, and the water is not deep enough to start.
            if !depth.is_some_and(|d| d > MIN_SWIM_DEPTH) {
                return None;
            }
            let line = surface.unwrap_or_default() - MIN_SWIM_DEPTH;
            // The character is placed on the water line, as
            // [`Self::touch_down`] places it on the floor and for the same
            // reason: the transition is defined at a depth, so a tick that
            // crosses it puts the character where the crossing was rather than
            // wherever the 25 ms boundary fell. Walking in from a beach stays
            // continuous (the crossing depth and the ground agree at the
            // water's edge), and falling in from a height moves the character
            // by at most one tick's fall.
            //
            // The 1.12.1 client sinks and then rises back up, and this code
            // does not. That is the one visible difference: the rise rate is
            // not established, and a guessed rate would be an unsupported
            // number.
            self.info.position.z = line.max(floor.unwrap_or(f32::MIN));
            self.info.flags |= move_flags::SWIMMING;
            // The arc ends. Clearing the flags here is legal only because of
            // the opcode returned: `CheckFallStop` treats a packet that clears
            // `JUMPING`/`FALLINGFAR` under anything but a fall-end opcode as a
            // cheat, and `IsFallEndOpcode` accepts `MSG_MOVE_FALL_LAND` or
            // `MSG_MOVE_START_SWIM`. The server treats an arc ending in water
            // as a fall end.
            self.air = None;
            self.info.flags &= !(move_flags::JUMPING | move_flags::FALLINGFAR);
            self.info.fall_time = 0;
            self.info.jump = JumpInfo::default();
            return Some(Opcode::MSG_MOVE_START_SWIM);
        }

        // Swimming. Out of the water entirely, or shallower than the dead band
        // under the entry depth.
        if !depth.is_some_and(|d| d >= MIN_SWIM_DEPTH - SWIM_EXIT_HYSTERESIS) {
            // The pitch bits are cleared too, on the same packet.
            // `CheckMoveFlags` never objects to dropping a movement flag, only
            // to introducing one under the wrong opcode. The alternative is a
            // `MOVEFLAG_PITCH_UP` on dry land for a tick, which the 1.12.1
            // client never sends: it applies the pitch keys only while
            // `MOVEFLAG_SWIMMING` is set.
            self.info.flags &=
                !(move_flags::SWIMMING | move_flags::PITCH_UP | move_flags::PITCH_DOWN);
            return Some(Opcode::MSG_MOVE_STOP_SWIM);
        }

        // No gravity and no buoyancy: the altitude changes only by the
        // swimmer's stroke along their pitch. A diver who stops stays where
        // they are.
        let mut z = self.info.position.z + climbed(&self.info, &self.speeds, dt);
        // The surface is a ceiling (rising through the water line ends the
        // swim, so a swimmer cannot be above it), and the floor is still
        // solid, since a swimmer can rest on the lake bed.
        //
        // The floor is applied second, and the order matters. Where a beach
        // rises past the water line the two clamps disagree. If the ceiling
        // won, the ground would never lift the swimmer out: they would float
        // below the sand, at a depth exactly on the entry threshold, and swim
        // in the shallows indefinitely. The ground takes precedence.
        if let Some(s) = surface {
            z = z.min(s - MIN_SWIM_DEPTH);
        }
        if let Some(h) = floor {
            z = z.max(h);
        }
        self.info.position.z = z;
        None
    }

    /// Land, clearing the arc.
    fn touch_down(&mut self, floor: f32) {
        self.info.position.z = floor;
        self.air = None;
        self.info.flags &= !(move_flags::JUMPING | move_flags::FALLINGFAR);
        self.info.fall_time = 0;
        self.info.jump = JumpInfo::default();
    }

    /// Advance the local simulation by `dt` seconds, returning the packet the
    /// step owes the server, if any.
    ///
    /// `world` is an optional description of what is underfoot and in the way;
    /// when supplied the character follows the landscape instead of moving at a
    /// fixed z, and stops at walls instead of walking through them.
    ///
    /// The three transitions are reported at once rather than at the next
    /// heartbeat, because of two server rules: the server sets the arc's start
    /// from the first packet carrying `MOVEFLAG_JUMPING`, so a late take-off
    /// report puts the parabola in the wrong place; and a packet that drops
    /// the flag under any opcode but `MSG_MOVE_FALL_LAND` is
    /// `CHEAT_TYPE_BAD_FALL_STOP`.
    ///
    /// The step is resolved in pieces no longer than [`MAX_STRIDE`]; see that
    /// constant. When more than one piece owes a packet, this returns the
    /// first. [`Self::advance_through`] returns all of them, and is what the
    /// session uses.
    pub fn advance(&mut self, dt: f32, world: Option<&dyn Footing>) -> Option<Opcode> {
        let mut owed = Vec::new();
        self.advance_pieces(dt, world, &mut owed);
        owed.first().map(|o| o.opcode)
    }

    /// Advance by `dt` seconds with each input applied at its own moment, and
    /// return every packet owed, in the order it became owed.
    ///
    /// `inputs` holds each input with its offset in seconds from the start of
    /// the step, in the order given. An offset outside `0..=dt` is clamped into
    /// it: an input from before the step began takes effect at its start, which
    /// is the earliest moment the step can still change.
    ///
    /// A key change owes one packet per flag it changes ([`transitions`]), each
    /// reported with the position at the moment of the change. A jump owes
    /// `MSG_MOVE_JUMP` with the position it took off from.
    pub fn advance_through(
        &mut self,
        dt: f32,
        inputs: &[(f32, Input)],
        world: Option<&dyn Footing>,
    ) -> Vec<Owed> {
        let dt = dt.max(0.0);
        let mut owed = Vec::new();
        let mut done = 0.0_f32;
        for &(at, input) in inputs {
            let at = at.clamp(done, dt);
            if at > done {
                self.advance_pieces(at - done, world, &mut owed);
                done = at;
            }
            match input {
                Input::Controls(controls) => {
                    // One packet per flag change, in order: the server rejects
                    // a packet whose opcode does not match the flag it added.
                    for event in self.set_controls(controls) {
                        self.info.flags = event.flags;
                        owed.push(Owed { opcode: event.opcode, info: self.info });
                    }
                }
                Input::Jump => {
                    if let Some(event) = self.jump() {
                        owed.push(Owed { opcode: event.opcode, info: self.info });
                    }
                }
            }
        }
        if dt > done {
            self.advance_pieces(dt - done, world, &mut owed);
        }
        owed
    }

    /// How many pieces a step of `dt` seconds is resolved in: enough that no
    /// piece moves the character further than [`MAX_STRIDE`] along the ground
    /// or up and down.
    ///
    /// The speeds are the ones the step will use: the fixed take-off speed in
    /// the air, the keys' speed on the ground, and in the air the larger of the
    /// vertical speeds at the two ends of the step, which bounds the vertical
    /// speed anywhere between them.
    fn pieces(&self, dt: f32) -> u32 {
        let across = match &self.air {
            Some(_) => self.info.jump.xy_speed.abs(),
            None if self.info.has(move_flags::MASK_TRAVEL) => self.info.speed(&self.speeds),
            None => 0.0,
        };
        let vertical = match &self.air {
            Some(air) => {
                let start = (air.up - GRAVITY * air.elapsed).abs();
                let end = (air.up - GRAVITY * (air.elapsed + dt)).abs();
                start.max(end).min(TERMINAL_VELOCITY.max(air.up.abs()))
            }
            None => 0.0,
        };
        let reach = across.max(vertical) * dt;
        if !reach.is_finite() {
            return 1;
        }
        ((reach / MAX_STRIDE).ceil() as u32).clamp(1, MAX_PIECES)
    }

    /// [`Self::advance`] without the packet selection: every piece's packet is
    /// pushed onto `owed` with the block as it stood when it became owed.
    fn advance_pieces(&mut self, dt: f32, world: Option<&dyn Footing>, owed: &mut Vec<Owed>) {
        if dt <= 0.0 {
            return;
        }
        let pieces = self.pieces(dt);
        let piece = dt / pieces as f32;
        for _ in 0..pieces {
            if let Some(opcode) = self.advance_piece(piece, world) {
                owed.push(Owed { opcode, info: self.info });
            }
        }
    }

    /// One piece of a step: the turn, the stride against the walls, and then
    /// the floor, the water and the arc at the end of the stride.
    fn advance_piece(&mut self, dt: f32, world: Option<&dyn Footing>) -> Option<Opcode> {
        if dt <= 0.0 {
            return None;
        }

        // A ride takes precedence over everything below, and owes no packet:
        // the server is following its own copy of this spline and discards
        // anything the client sends until the ride is acknowledged. The ride
        // owes [`Self::take_finished_ride`] instead, which the loop collects.
        if self.ride_step(dt) {
            return None;
        }

        // Turning is local only, so it never enters the flags the server
        // extrapolates from. A rooted character still turns; a stunned or dead
        // one does not. Travel and turning are tested separately, from two
        // different fields. See [`Restraint`].
        if self.can_turn() {
            self.info.position.orientation =
                turned(self.info.position.orientation, &self.speeds, self.controls, dt);
        }

        if !self.can_move() {
            return None;
        }

        // --- horizontal ------------------------------------------------------
        // In the air the velocity is the one fixed at take-off; on the ground
        // it is whatever the keys imply. Either way the stride is proposed and
        // the world decides where it ends, so a wall shortens it before the
        // height is looked up. In the other order, the character would stand
        // on the floor inside the building the wall stopped them entering.
        let (dx, dy) = match &self.air {
            Some(_) => (
                self.info.jump.cos_angle * self.info.jump.xy_speed * dt,
                self.info.jump.sin_angle * self.info.jump.xy_speed * dt,
            ),
            None => {
                let [dx, dy] = strode(&self.info, &self.speeds, dt);
                (dx, dy)
            }
        };
        if dx != 0.0 || dy != 0.0 {
            let p = self.info.position;
            let from = [p.x, p.y, p.z];
            let to = [p.x + dx, p.y + dy, p.z];
            let end = world.map_or(to, |w| w.step(from, to));
            self.info.position.x = end[0];
            self.info.position.y = end[1];
        }

        // --- vertical --------------------------------------------------------
        let p = self.info.position;
        let floor = world.and_then(|w| w.floor(p.x, p.y, p.z));
        let surface = world.and_then(|w| w.liquid(p.x, p.y));
        // The height this character stands at, which is not always the floor;
        // see [`standing_surface`]. The rest of this function uses it in place
        // of `floor`.
        let footing = standing_surface(self.info.flags, floor, surface);

        // The water is tested before the air and takes precedence. A fall
        // that ends in a lake does not land: `IsFallEndOpcode` accepts
        // `MSG_MOVE_FALL_LAND` or `MSG_MOVE_START_SWIM`, so the server counts
        // the swim start as the end of the arc, and a `FALL_LAND` sent as well
        // would end it twice. The airborne state is therefore cleared in
        // [`Self::tick_water`] rather than through [`Self::touch_down`].
        //
        // A water-walker cannot swim; that is what the flag means. It is
        // expressed as "no liquid here" rather than as an early return, so
        // that a character who gains the aura while already swimming leaves the
        // water through the normal exit path (hysteresis, `MSG_MOVE_STOP_SWIM`)
        // instead of keeping `MOVEFLAG_SWIMMING` while standing on the surface.
        let swimmable = match self.info.has(move_flags::WATERWALKING) {
            true => None,
            false => surface,
        };
        if let Some(event) = self.tick_water(swimmable, floor, dt) {
            return Some(event);
        }
        if self.info.has(move_flags::SWIMMING) {
            // The ascend key sets a movement flag of its own, legal only in
            // water. See [`Self::reconcile_pitch`] for why it is reconciled
            // here as well as on the keypress.
            return self.reconcile_pitch();
        }

        if let Some(air) = self.air.as_mut() {
            // Missing floor data holds the arc, as it holds a walking
            // character's altitude. The two used to differ: a walking character
            // over a point with no floor data kept the height the server gave
            // it, while an airborne one kept falling, because the `None` case
            // below wrote the parabola. That caused characters to fall through
            // the floor. The floor query returns `None` at any point inside a
            // building's `MODF` box that its hull does not cover (a doorway, a
            // stair with no `MOPY`, a group not yet read), so a character who
            // left the ground anywhere in Ironforge fell until vmangos'
            // `UndermapRecall` caught them.
            //
            // The arc is held rather than landed, because there is nothing to
            // land on. It does not advance either, so it resumes where it was
            // rather than jumping a second's worth of fall when the geometry
            // arrives. See `Footing::floor`, whose `None` means no data, never
            // no floor.
            //
            // No world at all is a different case from no data here, and only
            // the second holds the arc. A caller with no [`Footing`] (the CLI's
            // snapshot, a unit test of the arc) means there is nothing to stand
            // on anywhere, and an arc held under it would never come down.
            if world.is_some() && footing.is_none() {
                return None;
            }
            air.elapsed += dt;
            let (start_z, elapsed, up) = (air.start[2], air.elapsed, air.up);
            let safe_fall = self.info.has(move_flags::SAFE_FALL);
            let z = start_z - fall_elevation(elapsed, safe_fall, -up);
            // `fallTime` is milliseconds since the arc began, and it has to be
            // non-zero: `CheckNoFallTime` treats a run of packets carrying the
            // flag with a zero `fallTime` as a hack, threshold five.
            self.info.fall_time = (elapsed * 1000.0) as u32;
            // Whether the arc is descending is computed from the arc, not from
            // the position. The two differ once the clamp below applies: a
            // character carried up a hillside is above the parabola, so
            // comparing against their position reads as descending on the way
            // up and lands the jump on its first tick. Two samples of the same
            // expression one step apart reflect only the arc.
            let previous = start_z - fall_elevation(elapsed - dt, safe_fall, -up);
            let descending = z < previous;
            // Only a descending arc lands, or the first upward frame of a
            // jump would land again the moment it left the ground.
            let Some(h) = footing else {
                self.info.position.z = z;
                return None;
            };
            if descending && z <= h {
                self.touch_down(h);
                return Some(Opcode::MSG_MOVE_FALL_LAND);
            }
            // An arc never goes below the ground it is over. Without this, a
            // jump taken while running uphill goes into the hill: the parabola
            // rises at 7.96 y/s and decelerates, while a 45-degree slope under
            // a run rises at a constant 7.6 y/s, so the two cross within a tick
            // or two of the apex, and the character (and the camera framed on
            // them) spends the rest of the ascent inside the hillside. Measured
            // on a synthetic slope: 0.15 yards under at 31 degrees, 1.34 at 45,
            // 2.82 at 56.
            //
            // Clamping is not landing; the branch above keeps the two apart.
            // The character follows the surface with the jump still in
            // progress, and touches down when the arc comes back down through
            // it. `floor`'s ceiling is the feet plus [`STEP_UP`], so this can
            // lift by at most a step per tick and cannot pull a character onto
            // a roof.
            self.info.position.z = z.max(h);
            return None;
        }

        let Some(h) = footing else {
            // No data for this spot: hold the altitude the server last gave
            // rather than inventing one. This happens just after a teleport,
            // before the tile has loaded.
            return None;
        };
        if h < self.info.position.z - FALL_THRESHOLD {
            // Walked off something. The arc starts with no upward speed and
            // carries the horizontal velocity of the stride that left the edge.
            let speed = if self.info.is_moving() {
                self.info.speed(&self.speeds)
            } else {
                0.0
            };
            self.take_off(0.0, speed, self.info.heading(), false);
            return Some(Opcode::MSG_MOVE_HEARTBEAT);
        }
        // A step, a slope, or a stair. Snapped rather than rate-limited: over
        // 25 ms a real slope changes the height by millimetres, so a large
        // change here means the ground data arrived late, and standing on it
        // is the best available answer.
        self.info.position.z = h;
        None
    }
}

/// What the local simulation needs to know about the world it is walking on.
///
/// A trait rather than the height closure it replaced, because collision asks
/// two questions that depend on each other: a wall shortens a stride, and the
/// floor is looked up at the end of the shortened stride. Two closures could be
/// supplied independently and one of them omitted, which would leave a
/// character standing inside the building a wall stopped them entering.
///
/// The protocol crate does not read ADT or WMO files, so the implementation
/// lives with the caller: [`crate::socket::session`] binds the map id, and
/// `vale_assets` supplies the geometry.
pub trait Footing {
    /// The surface to stand on at `(x, y)` for a character whose feet are at
    /// `z`, or `None` where the client has no data for that spot.
    ///
    /// `z` is a parameter because with solid buildings the ground is not a
    /// single height: over Stormwind's canals there are four surfaces above one
    /// point, and which of them is the floor depends on where the character
    /// already is.
    fn floor(&self, x: f32, y: f32, z: f32) -> Option<f32>;

    /// Where a stride from `from` to `to` ends. Only the horizontal part is
    /// decided here; the vertical is decided by [`Self::floor`].
    ///
    /// Defaults to `to`, so a caller with terrain and no buildings (the CLI,
    /// and the renderer before its first tile arrives) gets the behaviour it
    /// had before collision existed.
    fn step(&self, _from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
        to
    }

    /// The height of the liquid surface over `(x, y)`, if there is one.
    ///
    /// Not a function of `z`, unlike [`Self::floor`]: a chunk carries one
    /// surface per liquid, and the 1.12.1 client considers all four liquid
    /// types (mask `0x0f`) and uses whichever surface is present, so there is
    /// no choice between surfaces to make. The caller, [`Mover::advance`],
    /// decides what a depth means.
    ///
    /// Defaults to `None` (no water here), so every existing caller keeps its
    /// previous behaviour of walking on the lake bed.
    fn liquid(&self, _x: f32, _y: f32) -> Option<f32> {
        None
    }

    /// The current placement of the moving platform with this guid.
    ///
    /// Looked up by guid rather than by position, unlike the other methods
    /// here, because the caller already knows which platform it wants: a unit
    /// whose spline arrived on `SMSG_MONSTER_MOVE_TRANSPORT` follows a path in
    /// that transport's frame, and converting it to world coordinates needs
    /// only the transport's placement. See [`Spline::on_transport`].
    ///
    /// Defaults to `None`, meaning the platform is not loaded. A passenger of a
    /// platform this client does not have is left where it was rather than
    /// placed at the map's origin.
    fn platform_of(&self, _guid: u64) -> Option<Platform> {
        None
    }
}

/// A bare height lookup is a [`Footing`] with no walls in it.
///
/// Kept so the terrain-only callers, and the tests that predate collision, can
/// still pass a closure.
impl<F: Fn(f32, f32, f32) -> Option<f32>> Footing for F {
    fn floor(&self, x: f32, y: f32, z: f32) -> Option<f32> {
        self(x, y, z)
    }
}

/// Wrap to `[0, 2pi)`. `MaNGOS::IsValidMapCoord` rejects a non-finite
/// orientation, and letting it grow without bound eventually loses precision.
pub fn wrap_angle(a: f32) -> f32 {
    use std::f32::consts::PI;
    let two_pi = 2.0 * PI;
    let mut a = a % two_pi;
    if a < 0.0 {
        a += two_pi;
    }
    a
}

pub(crate) fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
}

/// The orientation of a walk from `a` to `b`. Zero is +X (north),
/// matching the server's orientation convention.
pub(crate) fn bearing(a: [f32; 3], b: [f32; 3]) -> f32 {
    (b[1] - a[1]).atan2(b[0] - a[0])
}

/// The signed angle from `a` to `b`, in `(-pi, pi]`.
///
/// A unit turning through north crosses the 2pi boundary, and the raw
/// difference turns it the long way round once per revolution. Anything that
/// interpolates or rate-limits an orientation needs this rather than `b - a`.
pub fn shortest_turn(a: f32, b: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut delta = (b - a) % TAU;
    if delta > PI {
        delta -= TAU;
    }
    if delta < -PI {
        delta += TAU;
    }
    delta
}

/// Whether this opcode arrives from the server as `packedGuid` + a
/// [`MovementInfo`], describing another player's movement.
///
/// When a client sends a `MSG_MOVE_*` packet, `HandleMovementOpcodes` rebuilds
/// it as `m_clientMoverGuid.WriteAsPacked()` followed by `movementInfo.Write`
/// and calls `SendMovementMessageToSet(..., _player)`, which sends the same
/// opcode to everyone nearby except the sender. This broadcast is the only way
/// another player's position changes: `SMSG_MONSTER_MOVE` covers server-driven
/// units, and a values update carries no position. Without handling these,
/// every other player stays where they were created.
///
/// The list is the opcodes routed to `HandleMovementOpcodes` in
/// `Protocol/Opcodes.cpp`, minus `CMSG_MOVE_FALL_RESET`, which is never
/// broadcast, plus the observer opcodes commented below. Other opcodes that
/// start with `MSG_MOVE_` have a different body: `MSG_MOVE_TIME_SKIPPED` is a
/// guid and a `u32`, and reading it as a movement block yields a plausible but
/// wrong position.
pub(crate) fn is_broadcast_movement(opcode: Opcode) -> bool {
    use Opcode::*;
    matches!(
        opcode,
        MSG_MOVE_START_FORWARD
            | MSG_MOVE_START_BACKWARD
            | MSG_MOVE_STOP
            | MSG_MOVE_START_STRAFE_LEFT
            | MSG_MOVE_START_STRAFE_RIGHT
            | MSG_MOVE_STOP_STRAFE
            | MSG_MOVE_JUMP
            | MSG_MOVE_START_TURN_LEFT
            | MSG_MOVE_START_TURN_RIGHT
            | MSG_MOVE_STOP_TURN
            | MSG_MOVE_START_PITCH_UP
            | MSG_MOVE_START_PITCH_DOWN
            | MSG_MOVE_STOP_PITCH
            | MSG_MOVE_SET_RUN_MODE
            | MSG_MOVE_SET_WALK_MODE
            | MSG_MOVE_FALL_LAND
            | MSG_MOVE_START_SWIM
            | MSG_MOVE_STOP_SWIM
            | MSG_MOVE_SET_FACING
            | MSG_MOVE_SET_PITCH
            | MSG_MOVE_HEARTBEAT
            // The observer side of the flag changes above. These do not go
            // through `HandleMovementOpcodes`; `SendMovementFlagChangeToObservers`
            // writes them directly. The body is the same packed guid and
            // movement block, and they are the only notice that another player
            // has been rooted, knocked back or given water-walking. Without
            // them a rooted player nearby keeps being extrapolated along the
            // flags they were last seen with, walking on the spot.
            | MSG_MOVE_ROOT
            | MSG_MOVE_UNROOT
            | MSG_MOVE_WATER_WALK
            | MSG_MOVE_HOVER
            | MSG_MOVE_FEATHER_FALL
            // The knockback. Its four trailing floats are already in the
            // movement block's jump section, so the tail is ignored.
            | MSG_MOVE_KNOCK_BACK
            // Another unit moved without walking there. Same body again, sent
            // for a creature as well as for a player:
            // `MovementPacketSender::SendTeleportToObservers`, called from
            // `Player::ExecuteTeleportNear` and `Unit::NearTeleportTo`. It
            // covers a Blink, a Sheep's wander re-anchor, a same-map `.tele`
            // and a creature being returned to its spawn. `STATUS_NEVER` in the
            // server's opcode table: server-to-client only.
            //
            // Sent twice per teleport, to observers around the old position
            // and around the new, both carrying the destination. The second
            // reaches a watcher near either end; the first tells a watcher who
            // is about to lose sight of the unit before the relocation removes
            // it from their cell. Applying both gives the same result as
            // applying one.
            //
            // Without it the unit stays at its old position until its next
            // heartbeat: half a second for a moving player, and indefinitely
            // for a standing one, since a standing player sends nothing.
            | MSG_MOVE_TELEPORT
    )
}

/// Parse one of the [`is_broadcast_movement`] packets: whose movement, and what.
pub(crate) fn parse_movement_broadcast(body: &[u8]) -> Option<(u64, MovementInfo)> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    let info = MovementInfo::read(&mut r)?;
    Some((guid, info))
}

/// Where a unit looks when it finishes a spline.
///
/// `Mask_Final_Facing` in `MoveSplineFlag.h`. It is why a creature that has
/// just walked up to the player turns to face them. [`SplineFacing::Travel`] is
/// the default and the common case: face the direction of travel.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SplineFacing {
    #[default]
    Travel,
    /// `Final_Angle`: an absolute orientation in radians.
    Angle(f32),
    /// `Final_Point`: face a point.
    Spot([f32; 3]),
    /// `Final_Target`: face a unit, by GUID. The combat case: a creature
    /// chasing a player is sent with its victim here.
    Target(u64),
}

/// `SMSG_MONSTER_MOVE`: the server moving a unit along a spline.
///
/// ```text
/// packedGuid
/// f32 x, y, z          spline start (the unit's current position)
/// u32 splineId
/// u8  faceType         0 normal, 1 stop, 2 spot(3 floats), 3 target(u64), 4 angle(f32)
/// u32 splineFlags      (absent when faceType == 1)
/// u32 duration         (absent when faceType == 1)
/// u32 nodeCount
/// f32 x, y, z          destination — the *last* node, not the first
/// ...                  the nodes before it, encoding depending on the flags
/// ```
///
/// Sources: `MoveSplineInit::Launch` for the header and the stop case,
/// `Movement/spline/packet_builder.cpp` for the path.
///
/// The path has two encodings, chosen by a flag, which place the destination
/// differently. `WriteLinearPath` writes the destination first and then the
/// earlier nodes as packed offsets from it; `WriteCatmullRomPath` writes every
/// node as an absolute position, in order, so the first vector after the count
/// is node zero and the destination is the last. Reading the second as the
/// first puts a flying creature's destination at its first waypoint: plausible
/// coordinates in the wrong place. In 1.12 `Mask_CatmullRom` is `Flying`
/// (0x200) and nothing else.
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterMove {
    pub guid: u64,
    pub start: [f32; 3],
    pub spline_id: u32,
    /// The waypoints, in order, ending at the destination. Empty for a stop
    /// packet; the unit halts at `start`.
    ///
    /// Following these rather than only the endpoints keeps a creature on a
    /// patrol route from cutting corners: the server follows the path, and a
    /// client that interpolates from start to destination moves the creature
    /// straight through whatever the path goes around.
    pub path: Vec<[f32; 3]>,
    pub duration_ms: u32,
    pub facing: SplineFacing,
    pub flags: u32,
    /// The transport in whose frame the whole path is expressed:
    /// `SMSG_MONSTER_MOVE_TRANSPORT`. `None` for the ordinary opcode.
    ///
    /// The two packets have the same body except for one extra packed guid
    /// after the unit's: `MoveSplineInit::Launch` builds `SMSG_MONSTER_MOVE`,
    /// then, if the unit is boarding a transport, changes the opcode and writes
    /// the transport's guid before the rest. A reader that treats the second as
    /// the first parses it without error and reads the transport's guid as the
    /// unit's start coordinates.
    ///
    /// Every coordinate after it is also local to the transport. A boat's
    /// deckhand walking three yards has a three-yard path about the boat's
    /// origin, and placing it in the world unconverted puts them three yards
    /// from the map's origin, in the sea off Westfall.
    pub transport: Option<u64>,
}

impl MonsterMove {
    pub fn destination(&self) -> Option<[f32; 3]> {
        self.path.last().copied()
    }
}

/// `enum MonsterMoveType` from `packet_builder.cpp`.
mod monster_move_type {
    pub const NORMAL: u8 = 0;
    pub(crate) const STOP: u8 = 1;
    pub(crate) const FACING_SPOT: u8 = 2;
    pub(crate) const FACING_TARGET: u8 = 3;
    pub(crate) const FACING_ANGLE: u8 = 4;
}

/// `MoveSplineFlag`, the subset this client reads. `Mask_CatmullRom = Flying`.
pub(crate) mod spline_flags {
    /// `MoveSplineFlag::Falling`: "Affects elevation computation". The z along
    /// such a spline is an arc rather than the path's own height, so it is the
    /// one server-driven move whose vertical must not be adjusted. Nothing here
    /// computes the arc yet; the flag is read only to keep
    /// [`super::Spline::grounded_z`] from adjusting it.
    pub(crate) const FALLING: u32 = 0x0000_0002;
    pub const FLYING: u32 = 0x0000_0200;
    pub(crate) const CYCLIC: u32 = 0x0010_0000;
    pub(crate) const CATMULLROM: u32 = FLYING;
}

/// One `appendPackXYZ`: a `u32` holding three signed fixed-point offsets in
/// quarter-yards: 11 bits for x and y, 10 for z (`ByteBuffer.h:469`).
///
/// The sign must be recovered explicitly: the server masks each field to its
/// width, so a negative offset arrives as a positive number with the width's top
/// bit set. Reading the fields unsigned puts nodes up to 512 yards away.
fn unpack_offset(packed: u32) -> [f32; 3] {
    let sign_extend = |v: u32, bits: u32| -> f32 {
        let v = v & ((1 << bits) - 1);
        let signed = if v & (1 << (bits - 1)) != 0 {
            v as i32 - (1 << bits)
        } else {
            v as i32
        };
        signed as f32 * 0.25
    };
    [
        sign_extend(packed, 11),
        sign_extend(packed >> 11, 11),
        sign_extend(packed >> 22, 10),
    ]
}

pub(crate) fn parse_monster_move(body: &[u8]) -> Option<MonsterMove> {
    parse_monster_move_inner(body, false)
}

/// `SMSG_MONSTER_MOVE_TRANSPORT`: the same body with the transport's packed
/// guid inserted after the unit's, and every coordinate after it in that
/// transport's own frame. See [`MonsterMove::transport`].
pub(crate) fn parse_monster_move_transport(body: &[u8]) -> Option<MonsterMove> {
    parse_monster_move_inner(body, true)
}

fn parse_monster_move_inner(body: &[u8], on_transport: bool) -> Option<MonsterMove> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    let transport = on_transport.then(|| r.packed_guid());
    if !r.has(12 + 4 + 1) {
        return None;
    }
    let start = [r.f32(), r.f32(), r.f32()];
    let spline_id = r.u32();
    let face_type = r.u8();

    if face_type == monster_move_type::STOP {
        return Some(MonsterMove {
            guid,
            start,
            spline_id,
            path: Vec::new(),
            duration_ms: 0,
            facing: SplineFacing::Travel,
            flags: 0,
            transport,
        });
    }

    let facing = match face_type {
        monster_move_type::FACING_SPOT => {
            if !r.has(12) {
                return None;
            }
            SplineFacing::Spot([r.f32(), r.f32(), r.f32()])
        }
        // A plain u64: `facing.target` is a bare `uint64` in the union, not an
        // ObjectGuid, so there is no packed form here.
        monster_move_type::FACING_TARGET => {
            if !r.has(8) {
                return None;
            }
            SplineFacing::Target(r.u64())
        }
        monster_move_type::FACING_ANGLE => {
            if !r.has(4) {
                return None;
            }
            SplineFacing::Angle(r.f32())
        }
        monster_move_type::NORMAL => SplineFacing::Travel,
        // An unknown facing mode means the rest of the packet is unreadable.
        _ => return None,
    };

    if !r.has(4 + 4 + 4 + 12) {
        return None;
    }
    let flags = r.u32();
    let duration_ms = r.u32();
    let node_count = r.u32() as usize;

    let mut path = if flags & spline_flags::CATMULLROM != 0 {
        // Every node absolute and in order, the destination last. A cyclic path
        // also starts with a duplicate of its first point, which adds a
        // zero-length first segment and has no other effect.
        let mut path = Vec::with_capacity(node_count.min(256));
        for _ in 0..node_count {
            if !r.has(12) {
                break;
            }
            path.push([r.f32(), r.f32(), r.f32()]);
        }
        if path.is_empty() {
            return None;
        }
        path
    } else {
        // Destination first, then the earlier nodes as packed offsets from the
        // destination, in path order. `WriteLinearPath` writes no offsets at all
        // for a two-point move even though it still counts 2, so the offsets are
        // `count - 1` and only when the count exceeds two.
        let destination = [r.f32(), r.f32(), r.f32()];
        let mut path = Vec::with_capacity(node_count.min(256));
        if node_count > 2 {
            for _ in 0..node_count - 1 {
                if !r.has(4) {
                    // A truncated tail loses the intermediate nodes but keeps
                    // the move: the unit goes straight to the destination, as
                    // this client did for every path before nodes were read.
                    path.clear();
                    break;
                }
                let o = unpack_offset(r.u32());
                path.push([
                    destination[0] - o[0],
                    destination[1] - o[1],
                    destination[2] - o[2],
                ]);
            }
        }
        path.push(destination);
        path
    };

    // The path begins at `start`, and neither encoding writes it.
    //
    // Both writers skip the spline's first control point. `WriteLinearPath`
    // takes `real_path = &spline.getPoint(1)` and emits offsets for
    // `real_path[0..last_idx)`, which is `controls[1..count-1)`;
    // `WriteCatmullRomPath` appends from `getPoint(2)`, which for a
    // catmull-rom spline is also `controls[1]`. In both cases the header
    // carries `controls[0]` instead: `WriteCommonMonsterMovePart` opens with
    // `spline.getPoint(spline.first())`, and `first()` is `index_lo`, which is 0
    // for a linear spline and 1 for a catmull-rom one, so `controls[0]` either
    // way.
    //
    // Omitting it loses the whole first leg, not just a point. A creature on a
    // four-node patrol was placed on its second waypoint at t = 0 and then
    // walked a path shorter than the duration allowed for: it jumped a leg
    // forward when the packet arrived, moved through the rest too slowly, and
    // was snapped back by the next packet. Creatures appeared to teleport and
    // move erratically. A check that only reads coordinates cannot detect this,
    // because every point in the path is a real waypoint the server named; one
    // is simply missing.
    //
    // Compared against the point rather than tested from the flags, because a
    // third case does write it: `WriteCatmullRomCyclicPath` starts with
    // `getPoint(1)` (`controls[0]`) as the extra vertex the `Enter_Cycle` flag
    // says to erase after the first lap. Comparing covers all three cases
    // with one rule, and the epsilon is far below the quarter-yard the packed
    // offsets are quantised to.
    let apart = |a: [f32; 3], b: [f32; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    if path.first().is_none_or(|first| apart(*first, start) > 0.05) {
        path.insert(0, start);
    }

    Some(MonsterMove {
        guid,
        start,
        spline_id,
        path,
        duration_ms,
        facing,
        flags,
        transport,
    })
}

/// A server-driven move in progress, from `SMSG_MONSTER_MOVE`.
///
/// The server sends the whole path once and expects the client to animate it;
/// without this every creature stays where it was created.
#[derive(Debug, Clone, PartialEq)]
pub struct Spline {
    /// Every waypoint, starting where the unit was when the packet was sent and
    /// ending at the destination. Always at least two points.
    pub path: Vec<[f32; 3]>,
    /// How long one lap takes. For a cyclic spline that is not how long the
    /// unit moves: it continues until the server sends something else.
    pub duration_ms: u32,
    pub elapsed_ms: u32,
    /// Where the unit looks when it arrives.
    pub facing: SplineFacing,
    /// `MoveSplineFlag::Cyclic`: this path repeats, and the server will not send
    /// it again.
    ///
    /// `MoveSpline::updateState` wraps `time_passed % Duration()` and continues
    /// indefinitely, so the duration in the packet is one lap and the client
    /// must loop it. A client that treats the end of the lap as the end of the
    /// move leaves the unit there: a patrolling guard walks his route once and
    /// then stands still, and then teleports to his real position the next
    /// time the server states it. Neither symptom suggests a missing flag; the
    /// first looks like an idle NPC and the second like a network fault.
    pub cyclic: bool,
    /// `MoveSplineFlag::Flying`. The flag has two effects, both named in its
    /// vmangos comment: "Smooth movement(Catmullrom interpolation mode), flying
    /// animation".
    ///
    /// It decides how the path is interpolated (see
    /// [`Self::position_and_heading`] and [`curve`]) and, in the client crate,
    /// which animation the unit plays while on it. A taxi flight shows both
    /// effects most clearly: interpolated linearly, it changes direction
    /// abruptly at every waypoint, and without the flying animation the gryphon
    /// plays `Run` scaled to 32 y/s, which is `Run`'s authored 6.9 played four
    /// and a half times too fast.
    pub flying: bool,
    /// `MoveSplineFlag::Falling`: the z along this path is an arc the client is
    /// expected to compute, not the height the nodes state. Read only so that
    /// [`Self::grounded_z`] leaves such a spline unchanged, and a knocked-back
    /// unit is not moved along the ground it is flying over.
    pub falling: bool,
    /// The transport in whose frame every waypoint above is expressed:
    /// `SMSG_MONSTER_MOVE_TRANSPORT`. `None` for every other spline.
    ///
    /// A deckhand walking three yards across a boat has a three-yard path about
    /// the boat's origin. The path is kept local rather than converted on
    /// arrival, because the boat moves: a path converted once is correct for
    /// one instant and drifts for the rest of the leg, which for a zeppelin is
    /// the length of a continent. See [`Self::in_world`].
    pub(crate) on_transport: Option<u64>,
}

/// A server-driven ride as published to a renderer: the path, and the seconds
/// since the ride began at the moment the reading was taken. See
/// [`Mover::ride_curve`].
#[derive(Debug, Clone, PartialEq)]
pub struct RideCurve {
    pub spline: std::sync::Arc<Spline>,
    pub elapsed: f32,
}

/// How far ahead [`Spline::velocity`] looks to measure itself.
///
/// Twenty milliseconds: shorter than a simulation tick, so the result describes
/// the current leg rather than the next one, and long enough that at Azeroth's
/// far corners, where `f32` resolution is about a millimetre, the difference
/// between two positions is dominated by the movement rather than by rounding.
const VELOCITY_PROBE_MS: u32 = 20;

impl Spline {
    /// Fraction complete, 0..=1.
    pub fn progress(&self) -> f32 {
        self.progress_at(self.elapsed_ms)
    }

    /// Fraction complete at another moment on this spline's clock.
    fn progress_at(&self, elapsed_ms: u32) -> f32 {
        if self.duration_ms == 0 {
            1.0
        } else {
            (elapsed_ms as f32 / self.duration_ms as f32).min(1.0)
        }
    }

    pub(crate) fn finished(&self) -> bool {
        !self.cyclic && self.elapsed_ms >= self.duration_ms
    }

    /// Wrap a cyclic spline back to the start of its lap.
    ///
    /// Called after `elapsed_ms` is advanced, and a no-op on an ordinary spline.
    /// Returns whether a lap was completed, which is when the leading duplicate
    /// point is removed. `WriteCatmullRomCyclicPath` writes `spline.getPoint(1)`
    /// and then the whole path starting at the same point, and the packet's
    /// `Enter_Cycle` flag indicates this ("erases first spline vertex after
    /// first cycle done"). The duplicate is a zero-length first leg, so it has
    /// no effect until the lap wraps; left in after that, it stalls the unit
    /// for one frame at the start of every lap.
    pub(crate) fn wrap(&mut self) -> bool {
        if !self.cyclic || self.duration_ms == 0 || self.elapsed_ms < self.duration_ms {
            return false;
        }
        self.elapsed_ms %= self.duration_ms;
        // Checked against the points rather than the flag alone: only the
        // catmull-rom writer emits the duplicate, and removing a real waypoint
        // would shorten the patrol by a leg every lap.
        if self.path.len() > 2 && self.path[0] == self.path[1] {
            self.path.remove(0);
        }
        true
    }

    pub fn from(&self) -> [f32; 3] {
        self.path.first().copied().unwrap_or_default()
    }

    pub fn to(&self) -> [f32; 3] {
        self.path.last().copied().unwrap_or_default()
    }

    /// Total path length in yards: the distance actually travelled. On a
    /// multi-node path it is longer than the straight line between the ends,
    /// and on a [`Self::flying`] one longer again, because the curve bows
    /// outside its polyline.
    pub fn length(&self) -> f32 {
        self.segment_lengths().iter().sum()
    }

    /// Yards per second along the path.
    ///
    /// The server states a path and a duration rather than a speed, and moves
    /// along the spline at a constant rate, so the speed follows from those
    /// two. It is not necessarily the unit's run speed: a creature on a long
    /// patrol leg is sent at its walk speed, which is the distinction the
    /// renderer needs when choosing between Walk and Run.
    pub fn speed(&self) -> f32 {
        if self.duration_ms == 0 {
            return 0.0;
        }
        self.length() * 1000.0 / self.duration_ms as f32
    }

    /// Where the unit is now, and which way it is heading, in radians.
    ///
    /// A spline is followed at a constant speed, so the parameter is distance
    /// along the path rather than a fraction of each segment; otherwise a long
    /// leg and a short one would take the same time and the unit would visibly
    /// speed up and slow down. This is `Spline::computeIndex`: `t × length`,
    /// the segment that contains it, and the local parameter inside it.
    ///
    /// The heading is the path's derivative, as in
    /// `MoveSpline::ComputePosition`'s `atan2(hermite.y, hermite.x)`. For a
    /// straight segment the derivative is the segment vector, so a linear
    /// spline gives the same heading as a per-segment bearing; for a
    /// [`Self::flying`] one it is the curve's tangent, which is continuous, so
    /// turns are smooth without a separate smoothing step.
    pub fn position_and_heading(&self) -> ([f32; 3], f32) {
        self.sample(self.elapsed_ms)
    }

    /// [`Self::position_and_heading`] in world coordinates, for a spline whose
    /// path is in a transport's frame.
    ///
    /// Unchanged for most splines, which name no transport and are already in
    /// world coordinates. For the rest it is
    /// `GenericTransport::CalculatePassengerPosition` applied to the sample:
    /// the same yaw and translation a passenger's offset goes through. A
    /// deckhand and the player standing beside them are both passengers and
    /// must be placed by the same rule.
    ///
    /// A transport this client does not have returns `None`, and the caller
    /// leaves the unit where it was. Taking the local path as world coordinates
    /// instead would put every boat's crew a few yards from the map's origin,
    /// in the sea off Westfall.
    pub fn in_world(&self, ground: Option<&dyn Footing>) -> Option<([f32; 3], f32)> {
        let (local, heading) = self.position_and_heading();
        let Some(guid) = self.on_transport else {
            return Some((local, heading));
        };
        let platform = ground?.platform_of(guid)?;
        let (sin, cos) = platform.facing.sin_cos();
        Some((
            [
                platform.position[0] + local[0] * cos - local[1] * sin,
                platform.position[1] + local[1] * cos + local[0] * sin,
                platform.position[2] + local[2],
            ],
            heading + platform.facing,
        ))
    }

    /// The current three-dimensional velocity, in yards per second.
    ///
    /// The heading `position_and_heading` returns is a bearing: it discards
    /// the vertical, as `MoveSpline::ComputePosition` does, which is correct
    /// for orienting a body but not enough to extrapolate its position. A taxi
    /// flight climbs and dives for most of its length, so a reader that
    /// extrapolates from a bearing and a speed holds its altitude between
    /// readings and then steps to each new one: a sawtooth at the simulation's
    /// 40 Hz, on an axis along which nothing else in the frame moves.
    ///
    /// A forward difference over [`VELOCITY_PROBE_MS`] of this spline's own
    /// evaluator rather than an analytic derivative, so a linear path and a
    /// Catmull-Rom one are differentiated by the same code that positions them
    /// and cannot disagree with it. Past the end it returns zero, because
    /// `sample` stays on the destination, which is correct for a ride that has
    /// arrived.
    pub fn velocity(&self) -> [f32; 3] {
        let (here, _) = self.sample(self.elapsed_ms);
        let (there, _) = self.sample(self.elapsed_ms + VELOCITY_PROBE_MS);
        let dt = VELOCITY_PROBE_MS as f32 / 1000.0;
        [
            (there[0] - here[0]) / dt,
            (there[1] - here[1]) / dt,
            (there[2] - here[2]) / dt,
        ]
    }

    /// The unit's position at a moment on this spline's clock, and its heading
    /// there.
    fn sample(&self, elapsed_ms: u32) -> ([f32; 3], f32) {
        self.sample_progress(self.progress_at(elapsed_ms))
    }

    /// [`Self::sample`] at a moment in fractional seconds rather than whole
    /// milliseconds.
    ///
    /// A ride is walked by this, and a renderer drawing the ride between
    /// simulation steps samples the same curve with it at its own frame time.
    /// Whole milliseconds would draw a 32 y/s flight in 3 cm steps.
    pub fn at_secs(&self, secs: f32) -> ([f32; 3], f32) {
        let progress = if self.duration_ms == 0 {
            1.0
        } else {
            (secs * 1000.0 / self.duration_ms as f32).clamp(0.0, 1.0)
        };
        self.sample_progress(progress)
    }

    fn sample_progress(&self, progress: f32) -> ([f32; 3], f32) {
        match self.locate_progress(progress) {
            Some((index, t)) => self.at(index, t),
            None => (self.from(), bearing(self.from(), self.to())),
        }
    }

    /// Which leg of the path the unit is on, and how far along it. This is the
    /// part of [`Self::sample`] that finds the leg rather than the point. It is
    /// separate because [`Self::grounded_z`] needs the leg's two endpoints, and
    /// recovering them from a position would duplicate the same search.
    ///
    /// `None` for a spline with no length: fewer than two points, or a
    /// path whose every segment is zero-length.
    fn locate(&self, elapsed_ms: u32) -> Option<(usize, f32)> {
        self.locate_progress(self.progress_at(elapsed_ms))
    }

    /// [`Self::locate`] at a fraction of the whole path.
    fn locate_progress(&self, progress: f32) -> Option<(usize, f32)> {
        if self.path.len() < 2 {
            return None;
        }
        let lengths = self.segment_lengths();
        let total: f32 = lengths.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut want = total * progress;
        for (index, leg) in lengths.iter().enumerate() {
            if *leg <= 0.0 {
                continue;
            }
            if want <= *leg {
                return Some((index, want / leg));
            }
            want -= leg;
        }
        // Past the end: stay on the destination, still facing along the last
        // leg.
        Some((self.path.len() - 2, 1.0))
    }

    /// The height to draw a unit at part-way along a leg, following the ground
    /// the leg crosses.
    ///
    /// A linear spline is a straight line in three dimensions between two
    /// points the server states, and the ground between them is not straight.
    /// Over real 1.12 terrain the error is large: sampled at each of the
    /// 31,152 ground creature spawns the world database lists on maps 0 and 1,
    /// a ten-yard leg's straight line departs from the ground under it by more
    /// than half a yard 17% of the time and by more than a yard 8%; for a
    /// twenty-yard leg (a chase) the figures are 50% and 28%. Just over half
    /// of those put the line above the ground, so the creature visibly floats,
    /// and the rest put its feet in the hill.
    ///
    /// Only the interpolated part is corrected. The result is not the ground
    /// height here: it is the ground here plus the height the server's two
    /// endpoints stand above the ground under them, interpolated along the leg.
    /// At `t = 0` and `t = 1` this reproduces the node's stated z exactly, so
    /// the client never disagrees with the server about a position the server
    /// named, and in between it carries the server's height above ground across
    /// the terrain rather than along a chord. This keeps it correct in cases a
    /// plain snap to the ground would break:
    ///
    /// * a creature swimming across a lake has both endpoints the same distance
    ///   above the lake bed, so it keeps swimming instead of walking the bottom;
    /// * one crossing a bridge has both endpoints thirty yards up, so it stays
    ///   on the bridge rather than being dropped into the canyon;
    /// * one walking out of a cave mouth is carried between the two smoothly.
    ///
    /// `None` means no correction, and every case that cannot compute one
    /// returns it: a flying or falling spline (whose vertical comes from the
    /// curve or an arc, not the ground), a path with no leg, and any of the
    /// three lookups with no data. The caller then keeps the chord, which is
    /// what this client did before.
    ///
    /// This is a reconstruction, not a measurement: what the 1.12.1 client
    /// does with an ungrounded spline is not known. What is measured is the
    /// deviation above, and that the correction is zero at every position the
    /// server states.
    ///
    /// ## Cost, and why the node lookups are not cached
    ///
    /// Three [`Footing::floor`] calls per server-driven unit per simulation
    /// step, under the world lock. The terrain part of one call is 0.32 µs,
    /// measured over 626,000 warmed lookups, so sixty creatures on splines cost
    /// 0.058 ms of a 25 ms tick; the buildings part is the same bounded `MODF`
    /// walk the dead-reckoning path above already makes per moving player.
    ///
    /// The two node lookups change only when the leg does and could be cached
    /// on the spline. They are not, because a cyclic path drops its leading
    /// point at the start of every lap ([`Self::wrap`]), which shifts every leg
    /// index under the cache without any error. The resulting stale cache
    /// would show as an occasional creature drifting, which is hard to trace.
    /// If the cost becomes significant it will show as
    /// `SessionStatus::stalls` increasing; check that number first.
    pub(crate) fn grounded_z(&self, world: &dyn Footing, drawn: [f32; 3]) -> Option<f32> {
        if self.flying || self.falling {
            return None;
        }
        let (leg, t) = self.locate(self.elapsed_ms)?;
        let a = *self.path.get(leg)?;
        let b = *self.path.get(leg + 1)?;
        let lift = |p: [f32; 3]| world.floor(p[0], p[1], p[2]).map(|g| p[2] - g);
        let (lift_a, lift_b) = (lift(a)?, lift(b)?);
        let here = world.floor(drawn[0], drawn[1], drawn[2])?;
        Some(here + lift_a + (lift_b - lift_a) * t)
    }

    /// One segment at a local parameter, in whichever mode this spline is.
    fn at(&self, index: usize, t: f32) -> ([f32; 3], f32) {
        if self.flying {
            let controls = curve::controls(&self.path);
            let at = curve::position(&controls, index, t);
            let tangent = curve::derivative(&controls, index, t);
            if let (Some(at), Some(tangent)) = (at, tangent) {
                return (at, wrap_angle(tangent[1].atan2(tangent[0])));
            }
        }
        let (a, b) = (self.path[index], self.path[index + 1]);
        (
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ],
            bearing(a, b),
        )
    }

    /// How long each segment is: the chord for a linear spline, the sampled
    /// curve for a flying one.
    ///
    /// A curve is longer than its polyline, so for a flying spline these, and
    /// [`Self::length`] which sums them, are longer than the chords. Using the
    /// curve lengths keeps the unit at the server's position rather than ahead
    /// of it.
    fn segment_lengths(&self) -> Vec<f32> {
        if self.flying {
            let controls = curve::controls(&self.path);
            return (0..self.path.len().saturating_sub(1))
                .map(|index| curve::segment_length(&controls, index))
                .collect();
        }
        self.path.windows(2).map(|w| distance(w[0], w[1])).collect()
    }
}

/// `SMSG_FORCE_*_SPEED_CHANGE`: `packedGuid`, `u32 movementCounter`,
/// `f32 speed`. Source: `MovementPacketSender::SendSpeedChangeToController`.
///
/// The counter must be returned in the matching `CMSG_FORCE_*_ACK`, or the
/// server leaves a pending movement change outstanding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SpeedChange {
    pub guid: u64,
    pub counter: u32,
    pub speed: f32,
}

pub(crate) fn parse_speed_change(body: &[u8]) -> Option<SpeedChange> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if !r.has(8) {
        return None;
    }
    Some(SpeedChange {
        guid,
        counter: r.u32(),
        speed: r.f32(),
    })
}

/// `MSG_MOVE_TELEPORT_ACK`, server to client: the server has moved this player
/// and is waiting for the client to acknowledge it.
///
/// Layout is `MovementPacketSender::SendTeleportToController`: packed guid,
/// `u32 movementCounter`, then a whole `MovementInfo` carrying the new
/// position. The reply goes back under the same opcode with a plain u64
/// guid, the counter, and a timestamp (`WorldSession::HandleMoveTeleportAck`
/// reads `recvData >> guid` on an `ObjectGuid`).
///
/// Ignoring this gets the account banned. The server relocates the player in
/// `ExecuteTeleportNear` and the client keeps extrapolating from the old
/// position; the next heartbeat then reports a position more than a yard from
/// the server's with no movement flags set, which matches
/// `MovementAnticheat::CheckTeleport`'s definition of a teleport hack. It fires
/// once per packet until the penalty threshold, and `MovementAnticheat` bans
/// the account for a cheat the client never attempted. This happened once: a
/// GM summoned the character, and the result was nine detections and a
/// 24-hour ban.
#[derive(Debug, Clone, PartialEq)]
pub struct Teleport {
    pub guid: u64,
    pub counter: u32,
    pub info: MovementInfo,
}

pub(crate) fn parse_teleport(body: &[u8]) -> Option<Teleport> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if !r.has(4) {
        return None;
    }
    let counter = r.u32();
    Some(Teleport {
        guid,
        counter,
        info: MovementInfo::read(&mut r)?,
    })
}

/// `SMSG_CLIENT_CONTROL_UPDATE`: which unit this client now controls.
///
/// A packed guid and one byte. `Unit::UpdateControl` sends it at the start and
/// end of every charm, possess, fear and confusion: possessing a unit sends
/// `(the possessed unit, 1)`, releasing it sends `(the player, 1)`, and being
/// feared or mind-controlled sends `(the player, 0)`: the guid the client
/// already controls, with movement taken away.
///
/// `allow_move` and the guid therefore carry separate information. A client
/// that reads only the guid cannot tell a fear from a release, and one that
/// reads only the byte cannot tell which unit the keys control.
///
/// The reply is `CMSG_SET_ACTIVE_MOVER` with the same guid, unpacked; see
/// [`crate::socket::world::WorldSession::set_active_mover`], which describes
/// what happens if it is not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClientControl {
    pub guid: u64,
    /// Whether the keys may move it at all. False for a feared or confused
    /// character, which the server moves itself.
    pub(crate) allow_move: bool,
}

pub(crate) fn parse_client_control(body: &[u8]) -> Option<ClientControl> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if guid == 0 || !r.has(1) {
        return None;
    }
    Some(ClientControl { guid, allow_move: r.u8() != 0 })
}

/// The reply to a near teleport, under the same opcode it arrived on.
///
/// The guid in the reply is plain, not packed (`HandleMoveTeleportAck` reads
/// `recvData >> guid` on an `ObjectGuid`, which is `uint64` in 1.12), even
/// though the incoming packet carried a packed one. The asymmetry is the
/// server's.
pub(crate) fn teleport_ack_body(guid: u64, counter: u32, time: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid).u32(counter).u32(time);
    w.buf
}

/// The reply to `SMSG_FORCE_*_SPEED_CHANGE`, on the matching `CMSG_..._ACK`.
///
/// `HandleForceSpeedChangeAckOpcodes`: the guid, the counter the change arrived
/// with, a whole `MovementInfo`, and the speed being confirmed. Echoing the
/// counter resolves the server's pending change; without it the change stays
/// pending indefinitely and the anticheat keeps measuring the client against
/// a speed it is no longer using.
pub(crate) fn speed_change_ack_body(
    guid: u64,
    counter: u32,
    info: &MovementInfo,
    speed: f32,
) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid).u32(counter);
    info.write(&mut w);
    w.f32(speed);
    w.buf
}

/// The end of a ride: `CMSG_MOVE_SPLINE_DONE`.
///
/// `HandleMoveSplineDoneOpcode` reads a whole `MovementInfo`, then the spline id,
/// then `Unused<float>()`, a trailing word the server skips without reading, so
/// its value does not matter; this sends 1.0, a completed fraction.
///
/// If this packet is missing, nothing reports an error and the server never
/// recovers (see [`Ride`]). It must pass four checks, and three of them turn
/// the packet into a no-op rather than an error:
///
/// * the id must be `movespline->GetId()`, so an ack for a spline that has
///   already been superseded is discarded; see [`Mover::ride`], which is why
///   one is never sent;
/// * the same id twice is refused (`HandleSplineDone`);
/// * the position must be within 10 yards of the spline's final destination,
///   which is why the ride is followed rather than skipped to the end;
/// * `VerifyMovementInfo`, as for any block.
///
/// The server relaxes its other checks for this opcode: `CheckSpeedHack`
/// exempts it, and `HandleFlagTests` returns "ignore" rather than "reject",
/// with the comment "Since we dont require client confirmation for flag
/// changes during move splines, it's possible for client to not have yet
/// processed the changes when the move spline expires."
pub(crate) fn spline_done_body(info: &MovementInfo, spline_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    info.write(&mut w);
    w.u32(spline_id).f32(1.0);
    w.buf
}

/// How much real time this client lost: `CMSG_MOVE_TIME_SKIPPED`, a raw `u64`
/// guid and a `u32` of milliseconds.
///
/// The guid is not packed. `WorldPackets::Movement::MoveTimeSkipped::
/// ReadFromWorldPacket` is `recv_data >> guid >> lag`, and `ByteBuffer`'s
/// `ObjectGuid` reader takes eight bytes; the packed form is read by
/// `ReadAsPacked`, which this handler does not use.
///
/// `HandleMoveTimeSkippedOpcode` adds `lag` to the mover's `stime` and `ctime`
/// (the clock `CheckSpeedHack` measures a stride against), so a client with no
/// lost time to report sends zero rather than claiming an allowance.
///
/// This client sends it for a side effect. The same handler re-sends a
/// transport's out-of-range and create blocks to a player it has just boarded,
/// which is the only way to get `UPDATEFLAG_TRANSPORT`'s path-progress word
/// restated after login. See `SessionLoop::tick_movement`, its one caller.
pub(crate) fn time_skipped_body(guid: u64, lag_ms: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid).u32(lag_ms);
    w.buf
}

/// A movement flag the server has toggled and is waiting for the client to
/// acknowledge: `SMSG_FORCE_MOVE_ROOT` / `UNROOT`, `SMSG_MOVE_WATER_WALK` /
/// `LAND_WALK`, `SMSG_MOVE_SET_HOVER` / `UNSET_HOVER`,
/// `SMSG_MOVE_FEATHER_FALL` / `NORMAL_FALL`.
///
/// Not acknowledging these got this client kicked.
/// `AddMovementFlagChangeToController` pushes a `PlayerMovementPendingChange`
/// and sends one of the packets above; if no ack arrives within
/// `Movement.PendingAckResponseTime` (4 s on the reference server),
/// `CheckPendingMovementChanges` calls `OnFailedToAckChange`
/// (`CHEAT_TYPE_PENDING_ACK_DELAY`, threshold 3, kick) and then applies the
/// change anyway. For a root, the server sets `MOVEFLAG_ROOT` on its copy of
/// the client's movement info, so the next packet sent with a moving flag is
/// `CHEAT_TYPE_ROOT_MOVE`. That check is sticky (it ORs the flag back into the
/// stored info) and reaches its five-tick threshold in a second.
/// `Anticheat.log` on the reference server recorded both, repeatedly, for this
/// client's characters.
///
/// The reply body is `u64 guid, u32 counter, MovementInfo`, plus a `u32 apply`
/// for the three that go through `HandleMovementFlagChangeToggleAck`, but not
/// for root, which has its own handler. Hence [`FlagChange::apply_field`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagChange {
    /// The `MOVEFLAG_` being toggled.
    pub flag: u32,
    pub apply: bool,
    pub ack: Opcode,
}

impl FlagChange {
    /// The flag change this server opcode carries, if any.
    pub fn of(opcode: Opcode) -> Option<FlagChange> {
        use move_flags::*;
        let (flag, apply, ack) = match opcode {
            Opcode::SMSG_FORCE_MOVE_ROOT => (ROOT, true, Opcode::CMSG_FORCE_MOVE_ROOT_ACK),
            Opcode::SMSG_FORCE_MOVE_UNROOT => (ROOT, false, Opcode::CMSG_FORCE_MOVE_UNROOT_ACK),
            Opcode::SMSG_MOVE_WATER_WALK => {
                (WATERWALKING, true, Opcode::CMSG_MOVE_WATER_WALK_ACK)
            }
            Opcode::SMSG_MOVE_LAND_WALK => {
                (WATERWALKING, false, Opcode::CMSG_MOVE_WATER_WALK_ACK)
            }
            Opcode::SMSG_MOVE_SET_HOVER => (HOVER, true, Opcode::CMSG_MOVE_HOVER_ACK),
            Opcode::SMSG_MOVE_UNSET_HOVER => (HOVER, false, Opcode::CMSG_MOVE_HOVER_ACK),
            Opcode::SMSG_MOVE_FEATHER_FALL => {
                (SAFE_FALL, true, Opcode::CMSG_MOVE_FEATHER_FALL_ACK)
            }
            Opcode::SMSG_MOVE_NORMAL_FALL => {
                (SAFE_FALL, false, Opcode::CMSG_MOVE_FEATHER_FALL_ACK)
            }
            _ => return None,
        };
        Some(FlagChange { flag, apply, ack })
    }

    /// Whether this ack carries the trailing `u32 apply`.
    ///
    /// Root does not: `HandleMoveRootAck` reads guid, counter and the movement
    /// block and stops, taking the opcode as the answer. The other three go
    /// through `HandleMovementFlagChangeToggleAck`, which reads one more field.
    /// An extra `u32` on a root ack is harmless, because the server ignores the
    /// tail, but a missing one on a water-walk ack makes the server read past
    /// the end, so the difference is modelled explicitly.
    pub(crate) fn apply_field(&self) -> bool {
        self.flag != move_flags::ROOT
    }
}

/// `packedGuid` + `u32 movementCounter`, which is the whole body of all six.
pub(crate) fn parse_flag_change(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if !r.has(4) {
        return None;
    }
    Some((guid, r.u32()))
}

/// The reply to one of the six. `apply` is `None` for the root pair.
pub(crate) fn flag_change_ack_body(
    guid: u64,
    counter: u32,
    info: &MovementInfo,
    apply: Option<bool>,
) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid).u32(counter);
    info.write(&mut w);
    if let Some(apply) = apply {
        w.u32(u32::from(apply));
    }
    w.buf
}

/// `SMSG_MOVE_KNOCK_BACK`: the server has knocked this unit back.
///
/// `packedGuid`, `u32 movementCounter`, then `cos, sin, speedXY, speedZ`. The
/// direction is a vector rather than an angle, and `speedZ` is down-positive:
/// `Unit::KnockBack` sends `-verticalSpeed`, so an upward knock arrives
/// negative, the same convention as the jump block it is echoed in. The four
/// floats must be returned unchanged in the ack's jump block:
/// `FindPendingMovementKnockbackChange` compares each to within 0.01 and
/// rejects the ack otherwise, which leaves the change pending and triggers
/// `CHEAT_TYPE_PENDING_ACK_DELAY` as if no ack had been sent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct KnockBack {
    pub guid: u64,
    pub counter: u32,
    pub cos_angle: f32,
    pub sin_angle: f32,
    pub xy_speed: f32,
    pub z_speed: f32,
}

pub(crate) fn parse_knock_back(body: &[u8]) -> Option<KnockBack> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if !r.has(4 + 16) {
        return None;
    }
    Some(KnockBack {
        guid,
        counter: r.u32(),
        cos_angle: r.f32(),
        sin_angle: r.f32(),
        xy_speed: r.f32(),
        z_speed: r.f32(),
    })
}

/// The reply to a knockback: `u64 guid, u32 counter, MovementInfo`. The
/// movement info's jump block carries the four numbers back.
pub(crate) fn knock_back_ack_body(guid: u64, counter: u32, info: &MovementInfo) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid).u32(counter);
    info.write(&mut w);
    w.buf
}

/// `SMSG_NEW_WORLD`: the server is moving this player to another map, and the
/// session is held until the client confirms it has arrived.
///
/// Layout is `Player::SendNewWorld`: `u32 mapId` then `f32 x, y, z, o`, 20
/// bytes, with no guid because it always refers to the player. (The four
/// floats are the transport-local position when the player is on a transport,
/// which this client does not read; it does not change the parse.)
///
/// Ignoring it has a worse effect than ignoring the near teleport: an ignored
/// [`Teleport`] misreported a position and got the account banned, while this
/// one stops the session. `Player::TeleportTo` calls
/// `SetSemaphoreTeleportFar(true)` before sending it, and every movement opcode
/// the client sends is discarded while that flag is set. Only
/// `HandleMoveWorldportAckOpcode` enters the new world: it clears the flag,
/// creates the destination map, relocates the player and sends the initial
/// packets. A client that does not handle this opcode stays in a world that no
/// longer contains it, receiving nothing, with no error reported.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NewWorld {
    pub map_id: u32,
    pub position: Position,
}

pub(crate) fn parse_new_world(body: &[u8]) -> Option<NewWorld> {
    let mut r = Reader::new(body);
    if !r.has(20) {
        return None;
    }
    let map_id = r.u32();
    let position = Position {
        x: r.f32(),
        y: r.f32(),
        z: r.f32(),
        orientation: r.f32(),
    };
    // The same check as the near teleport, for the same reason: adopting a
    // position the server could not have meant leaves the character outside
    // every loaded tile with nothing to bring it back.
    if !position.is_plausible() {
        return None;
    }
    Some(NewWorld { map_id, position })
}

/// `SMSG_LOGIN_VERIFY_WORLD`: the map the character logged in on. No other
/// packet states it.
///
/// The same twenty bytes as [`NewWorld`] (`u32 mapId` then `f32 x, y, z, o`),
/// sent from `WorldSession::HandlePlayerLoginOpcode` before anything else about
/// the world, so it is the first packet in the login burst.
///
/// ## Why it is not redundant with the character list
///
/// The character-list row the player clicked can be out of date by the time
/// the login completes, and no packet reports the change.
/// `Player::LoadFromDB` ends with:
///
/// ```text
/// // if the player is in an instance and it has been reset in the meantime
/// // teleport him to the entrance
/// if (!state || state->GetInstanceId() != instanceId)
///     if (AreaTriggerTeleport const* at = sObjectMgr.GetGoBackTrigger(GetMapId()))
///     {
///         Relocate(at->destination…);
///         SetLocationMapId(at->destination.mapId);
///     }
/// ```
///
/// That is a `Relocate`, not a `TeleportTo`. It happens during the load,
/// before the session has a map or a socket to announce anything on, so there
/// is no `SMSG_TRANSFER_PENDING` and no `SMSG_NEW_WORLD`. A character who
/// logged out in Scholomance and returns after the instance has expired is on
/// map 0 at Caer Darrow, while `SMSG_CHAR_ENUM` still says map 289.
///
/// A client that takes the map from the list and does not read this packet
/// then loads the instance's tile directory at the continent's coordinates.
/// For Scholomance that is `schoolofnecromancy_36_29.adt`, which does not exist
/// (the map's sixteen tiles are at (30..33, 29..32)), so nine reads fail and
/// there is no ground, no buildings and no collision, with the entrance's
/// creatures standing in the empty space. It stays that way for the whole
/// session: a hearthstone out of Caer Darrow is a near teleport on the same
/// map, with no `SMSG_NEW_WORLD`, so nothing corrects the map until the next
/// login. This parser was added to fix that.
///
/// The position is carried, but this packet is not read for it: the
/// character's create block states it a few packets later, along with its
/// speeds and movement flags, and the login takes those. The map id has no
/// other source.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoginVerifyWorld {
    pub map_id: u32,
    pub position: Position,
}

pub(crate) fn parse_login_verify_world(body: &[u8]) -> Option<LoginVerifyWorld> {
    let mut r = Reader::new(body);
    if !r.has(20) {
        return None;
    }
    let map_id = r.u32();
    let position = Position {
        x: r.f32(),
        y: r.f32(),
        z: r.f32(),
        orientation: r.f32(),
    };
    // The same check as both teleports. A login has no previous position to
    // fall back on, so a body that fails it leaves the character-list row in
    // effect, which is wrong only in the case described above.
    if !position.is_plausible() {
        return None;
    }
    Some(LoginVerifyWorld { map_id, position })
}

/// `SMSG_TRANSFER_PENDING`: a far teleport is about to happen.
///
/// The announcement that precedes [`NewWorld`]. It is defined beside it rather
/// than beside `SMSG_TRANSFER_ABORTED` because it concerns a transfer that is
/// happening: vmangos sends it from `Player::ExecuteTeleportFar`, immediately
/// before removing the character from the old map, with the comment `// send
/// transfer packet to display load screen` on the line above
/// (`Player.cpp:2037`). It carries no position and needs no reply; it covers
/// the second or two between leaving and `SMSG_NEW_WORLD` stating the arrival.
///
/// No other packet marks that moment. Taking the map change from
/// `SMSG_NEW_WORLD` instead is late by the interval in which the old world is
/// torn down, which is the interval a loading screen exists to cover.
///
/// `u32 mapId`, then, only when the character is standing on a transport,
/// `u32 transportEntry, u32 oldMapId`. Both tail fields are read because a
/// body whose length the client does not know cannot be told apart from a
/// truncated one. Nothing here uses them yet; a boat leaving with a character
/// on it is handled through `SMSG_MONSTER_MOVE_TRANSPORT`, not this packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferPending {
    /// The destination map.
    pub map_id: u32,
    /// `(transport entry, the map being left)`, for a transfer that begins on a
    /// boat or a zeppelin. Absent for every ordinary teleport.
    pub transport: Option<(u32, u32)>,
}

pub(crate) fn parse_transfer_pending(body: &[u8]) -> Option<TransferPending> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let map_id = r.u32();
    // `SendPacket` writes both extra dwords or neither, so a body with
    // four spare bytes means this parse is wrong, not that the transport block
    // is half present.
    let transport = if r.has(8) {
        Some((r.u32(), r.u32()))
    } else {
        None
    };
    Some(TransferPending { map_id, transport })
}

/// Which of the six speeds an `SMSG_FORCE_*_SPEED_CHANGE` carries, and the
/// opcode that acknowledges it. Order matches [`Speeds`].
pub(crate) fn speed_change_slot(opcode: Opcode) -> Option<(usize, Opcode)> {
    Some(match opcode {
        Opcode::SMSG_FORCE_WALK_SPEED_CHANGE => (0, Opcode::CMSG_FORCE_WALK_SPEED_CHANGE_ACK),
        Opcode::SMSG_FORCE_RUN_SPEED_CHANGE => (1, Opcode::CMSG_FORCE_RUN_SPEED_CHANGE_ACK),
        Opcode::SMSG_FORCE_RUN_BACK_SPEED_CHANGE => {
            (2, Opcode::CMSG_FORCE_RUN_BACK_SPEED_CHANGE_ACK)
        }
        Opcode::SMSG_FORCE_SWIM_SPEED_CHANGE => (3, Opcode::CMSG_FORCE_SWIM_SPEED_CHANGE_ACK),
        Opcode::SMSG_FORCE_SWIM_BACK_SPEED_CHANGE => {
            (4, Opcode::CMSG_FORCE_SWIM_BACK_SPEED_CHANGE_ACK)
        }
        Opcode::SMSG_FORCE_TURN_RATE_CHANGE => (5, Opcode::CMSG_FORCE_TURN_RATE_CHANGE_ACK),
        _ => return None,
    })
}

/// How a speed change for a unit other than the player arrives.
///
/// `MovementPacketSender`'s table has three opcodes per speed, and this client
/// previously read only the middle column. The other two are all an observer
/// receives:
///
/// ```text
/// moveTypeToOpcode[MOVE_RUN] = { SMSG_SPLINE_SET_RUN_SPEED,   // server-driven
///                                SMSG_FORCE_RUN_SPEED_CHANGE, // the controller
///                                MSG_MOVE_SET_RUN_SPEED }     // everyone else
/// ```
///
/// No other packet restates a unit's speed. The six speeds are in a movement
/// block, which arrives when an object is created and not again, so a player
/// who mounts nearby, or a creature that is hasted, slowed, enraged or fleeing,
/// keeps being extrapolated at the speed it had when it came into view. At a
/// 60% mount that is 4.2 yards of error per half-second heartbeat, applied and
/// corrected twice a second in alternating directions, so the unit jitters
/// back and forth.
///
/// The `*_CHEAT` opcodes beside these (204, 206, 208, 210, 212, 215) are
/// deliberately absent: `moveTypeToOpcode` uses them for no build, and
/// `Opcodes.cpp` maps them to `Handle_NULL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpeedBroadcast {
    /// `MSG_MOVE_SET_*_SPEED`: packed guid, a whole movement block, then the
    /// speed. Sent for a player-controlled unit whose spline has finalised.
    /// The block is included on purpose: `HandleForceSpeedChangeAck` relocates
    /// the mover first, and the comment over the send says "send the speed
    /// change to others (with updated position if all is fine)".
    Observed,
    /// `SMSG_SPLINE_SET_*`: packed guid and the speed, no block. Sent for a
    /// server-driven unit (every creature, via `SendSpeedChangeToAll`), and
    /// for a player-controlled one whose spline has not finalised, where a
    /// position mid-path would be wrong.
    Spline,
}

/// Which of the six speeds one of the two observer families carries, and which
/// family it is. Order matches [`Speeds`] and vmangos' `UnitMoveType`.
pub(crate) fn broadcast_speed_slot(opcode: Opcode) -> Option<(usize, SpeedBroadcast)> {
    use SpeedBroadcast::{Observed, Spline};
    Some(match opcode {
        Opcode::MSG_MOVE_SET_WALK_SPEED => (0, Observed),
        Opcode::MSG_MOVE_SET_RUN_SPEED => (1, Observed),
        Opcode::MSG_MOVE_SET_RUN_BACK_SPEED => (2, Observed),
        Opcode::MSG_MOVE_SET_SWIM_SPEED => (3, Observed),
        Opcode::MSG_MOVE_SET_SWIM_BACK_SPEED => (4, Observed),
        Opcode::MSG_MOVE_SET_TURN_RATE => (5, Observed),
        Opcode::SMSG_SPLINE_SET_WALK_SPEED => (0, Spline),
        Opcode::SMSG_SPLINE_SET_RUN_SPEED => (1, Spline),
        Opcode::SMSG_SPLINE_SET_RUN_BACK_SPEED => (2, Spline),
        Opcode::SMSG_SPLINE_SET_SWIM_SPEED => (3, Spline),
        Opcode::SMSG_SPLINE_SET_SWIM_BACK_SPEED => (4, Spline),
        Opcode::SMSG_SPLINE_SET_TURN_RATE => (5, Spline),
        _ => return None,
    })
}

/// [`SpeedBroadcast::Observed`]: packed guid, `MovementInfo`, `f32 speed`.
///
/// The block is always present, and the packet is not a heartbeat. Parsing it
/// as one ([`parse_movement_broadcast`]) succeeds and discards the trailing
/// float, which is how this family of packets used to be handled: the position
/// updated and the speed did not.
pub(crate) fn parse_observed_speed(body: &[u8]) -> Option<(u64, MovementInfo, f32)> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    let info = MovementInfo::read(&mut r)?;
    if !r.has(4) {
        return None;
    }
    Some((guid, info, r.f32()))
}

/// [`SpeedBroadcast::Spline`]: packed guid and `f32 speed`, and nothing else.
pub(crate) fn parse_spline_speed(body: &[u8]) -> Option<(u64, f32)> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if !r.has(4) {
        return None;
    }
    Some((guid, r.f32()))
}

/// A movement flag set or cleared on a unit this client does not control: the
/// twelve `SMSG_SPLINE_MOVE_*` opcodes.
///
/// `MovementPacketSender` sends a flag change three ways, depending on who is
/// moving the unit. This project handled two of the three before this type:
///
/// | function | audience | opcodes | read here by |
/// |---|---|---|---|
/// | `AddMovementFlagChangeToController` | the player moving the unit | six `SMSG_MOVE_*`/`SMSG_FORCE_MOVE_*` | [`FlagChange`]; must be acknowledged |
/// | `SendMovementFlagChangeToObservers` | everyone but that player | five `MSG_MOVE_*` | [`is_broadcast_movement`]; packed guid + a whole block |
/// | `SendMovementFlagChangeToAll` / `SendToggleRunWalkToAll` | everyone, for a server-controlled unit | these twelve | this |
///
/// The third is the creature case, and it has no movement block: under 1.9.4
/// the body is a packed guid and nothing else, so the opcode carries the
/// entire message and the flag comes from a table. It is not acknowledged; it
/// is a notification, not a request.
///
/// Without these the flags never change after creation. A creature's movement
/// block arrives once, inside its create block; no values update carries one,
/// and `SMSG_MONSTER_MOVE` carries a path with no flags. For every unit the
/// server moves, these twelve packets are the only later notice that it has
/// been rooted, set to walk, given water-walking, set hovering or given a
/// feather fall. A client that ignores them keeps the unit's creation flags
/// for the rest of the session.
///
/// ## Two of the twelve that vmangos never sends
///
/// `SMSG_SPLINE_MOVE_START_SWIM` and `_STOP_SWIM` are in the opcode table and
/// in `Opcodes.cpp`, but nothing in vmangos writes either, and the server never
/// sets `MOVEFLAG_SWIMMING` on a creature. They are included because the 1.12.1
/// client accepts them, and because a gap in the table would invite someone to
/// work the missing entries out again; they are marked as unused rather than
/// left out. The consequence of the server never sending them is a swimming
/// creature drawn running; see `Entity::is_swimming`, which describes the same
/// gap from the entity side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SplineFlagChange {
    /// The `MOVEFLAG_` being stated.
    pub flag: u32,
    /// Set it, or clear it.
    pub apply: bool,
}

impl SplineFlagChange {
    /// The flag change this opcode carries, if it is one of the twelve.
    ///
    /// The pairing follows `MovementPacketSender::SendMovementFlagChangeToAll`'s
    /// switch, plus `SendToggleRunWalkToAll` for the run/walk pair. The server
    /// has a separate function for run/walk because it does not treat it as a
    /// flag change; this client does.
    pub fn of(opcode: Opcode) -> Option<SplineFlagChange> {
        use move_flags::*;
        let (flag, apply) = match opcode {
            Opcode::SMSG_SPLINE_MOVE_ROOT => (ROOT, true),
            Opcode::SMSG_SPLINE_MOVE_UNROOT => (ROOT, false),
            Opcode::SMSG_SPLINE_MOVE_WATER_WALK => (WATERWALKING, true),
            Opcode::SMSG_SPLINE_MOVE_LAND_WALK => (WATERWALKING, false),
            Opcode::SMSG_SPLINE_MOVE_FEATHER_FALL => (SAFE_FALL, true),
            Opcode::SMSG_SPLINE_MOVE_NORMAL_FALL => (SAFE_FALL, false),
            Opcode::SMSG_SPLINE_MOVE_SET_HOVER => (HOVER, true),
            Opcode::SMSG_SPLINE_MOVE_UNSET_HOVER => (HOVER, false),
            // Reversed relative to every other pair here, matching the server:
            // the flag is `MOVEFLAG_WALK_MODE`, so the run opcode clears it.
            // `SendToggleRunWalkToAll(unit, run)` picks
            // `run ? SET_RUN_MODE : SET_WALK_MODE`.
            Opcode::SMSG_SPLINE_MOVE_SET_WALK_MODE => (WALK_MODE, true),
            Opcode::SMSG_SPLINE_MOVE_SET_RUN_MODE => (WALK_MODE, false),
            // The two vmangos never sends; see the type's doc.
            Opcode::SMSG_SPLINE_MOVE_START_SWIM => (SWIMMING, true),
            Opcode::SMSG_SPLINE_MOVE_STOP_SWIM => (SWIMMING, false),
            _ => return None,
        };
        Some(SplineFlagChange { flag, apply })
    }
}

/// A packed guid, which is the whole body of all twelve.
///
/// A zero-length body is rejected. `Reader::packed_guid` returns 0 for an empty
/// read, the same as for a real mask byte of zero, and a guid of 0 names no
/// entity, so the empty case is refused here rather than applied to no entity
/// later.
pub(crate) fn parse_spline_flag(body: &[u8]) -> Option<u64> {
    if body.is_empty() {
        return None;
    }
    let mut r = Reader::new(body);
    match r.packed_guid() {
        0 => None,
        guid => Some(guid),
    }
}

/// The movement flags the server has stated for a unit outside its movement
/// block: which bits it has stated, and their values.
///
/// Two words rather than one, because these packets both set and clear, and
/// they override a flag word that arrived earlier and still applies to every
/// bit they have not mentioned. With one word, a cleared bit (such as an
/// unroot) would be indistinguishable from a bit never mentioned.
///
/// Applied by [`Self::over`], which is the only place the two are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct FlagOverride {
    /// The bits any of the twelve has stated, set or cleared.
    pub known: u32,
    /// The stated value of each bit in [`Self::known`].
    pub value: u32,
}

impl FlagOverride {
    /// Record one statement.
    pub fn state(&mut self, change: SplineFlagChange) {
        self.known |= change.flag;
        if change.apply {
            self.value |= change.flag;
        } else {
            self.value &= !change.flag;
        }
    }

    /// The unit's effective flag word: `base` for every bit not stated, this
    /// override for the rest.
    pub fn over(&self, base: u32) -> u32 {
        (base & !self.known) | (self.value & self.known)
    }

    /// Whether nothing has been stated. Usually true, which lets
    /// [`Self::over`] be skipped rather than run per entity per frame.
    pub fn is_empty(&self) -> bool {
        self.known == 0
    }
}

pub mod curve;

#[cfg(test)]
mod tests;
