//! Movement: the `MovementInfo` block, the `MSG_MOVE_*` opcodes that carry it,
//! and a local simulation faithful enough to survive vmangos's anticheat.
//!
//! Sources: `Objects/MovementInfo.{h,cpp}` (the flags and the wire layout),
//! `Handlers/MovementHandler.cpp` (what the server accepts), `Objects/Unit.cpp`
//! (`ExtrapolateMovement`, `GetSpeedForMovementInfo`) and
//! `Anticheat/MovementAnticheat/MovementAnticheat.cpp` (what gets you rejected).
//!
//! ## What the anticheat actually checks
//!
//! `Anticheat.Enable = 1` on the reference server, so this is not optional. The
//! rules that shape the design here, all with `Reject = 1`:
//!
//! * **`ctime` must be non-zero and monotonic.** `CHEAT_TYPE_NULL_CLIENT_TIME`
//!   fires on zero, `CHEAT_TYPE_TIME_BACK` on any decrease. It is the client's
//!   own millisecond tick — the server only ever compares it against itself.
//! * **`ctime` deltas must track real elapsed time.** `CheckTimeDesync`
//!   subtracts the server's receive-time delta from the client's; more than a
//!   second of drift in one packet is `CHEAT_TYPE_NUM_DESYNC`.
//! * **A moving unit must send something every 500 ms.** `CheckSpeedHack`
//!   raises `CHEAT_TYPE_SKIPPED_HEARTBEATS` when a gap exceeds 1000 ms and the
//!   *previous* packet had a moving flag set.
//! * **The opcode has to match the flags.** `CheckMoveStart`: only
//!   `MSG_MOVE_START_FORWARD` may introduce `FORWARD`, only
//!   `MSG_MOVE_START_STRAFE_LEFT` may introduce `STRAFE_LEFT`, and so on. Two
//!   flags changing at once therefore need two packets, not one — see
//!   [`transitions`].
//! * **Reported positions must be reachable.** `CheckSpeedHack` re-runs the
//!   server's own extrapolation from the previous packet and allows 10% slack,
//!   so [`Mover::advance`] reproduces `Unit::ExtrapolateMovement` exactly rather
//!   than approximating it.
//!
//! ## The vertical is a simulation, not a clamp, and that is an anticheat rule
//!
//! [`Mover::advance`] used to pull the character towards the floor at a clamped
//! 40 yards a second whatever was underneath. That is invisible in a log — every
//! number is a plausible position — and it is `CHEAT_TYPE_TELEPORT`:
//! `CheckTeleport` compares two consecutive packets that **both** carry no
//! horizontal and no falling flags, and calls a z difference over 2.0 yards a
//! teleport hack. Standing still while a tile arrives under the character is
//! exactly that shape. The measured cost was a 24-hour ban with `TeleportHack
//! (Total:6)` in `Anticheat.log`, from a character standing in Elwynn.
//!
//! The client the server expects does not glide: it **falls**, with
//! `MOVEFLAG_JUMPING` set the whole way (which exempts the check, since the
//! flag is in `MOVEFLAG_MASK_XZ | MOVEFLAG_JUMPING`'s exemption list), a
//! `fallTime` that counts, and a `MSG_MOVE_FALL_LAND` at the bottom. So gravity
//! is not a feature added beside the anticheat work — it *is* the fix, and the
//! jump is the same arc with a non-zero initial speed.
//!
//! ## Why there are no turn flags here
//!
//! `MOVEFLAG_TURN_LEFT` / `TURN_RIGHT` make the server extrapolate along a
//! *circular arc* (`ExtrapolateMovement`, the `R = 1.295 * speed / pi` branch).
//! Reproducing that arc to within 10% while also running the straight-line case
//! is a lot of surface area for one keypress. Mouse-turning in the real client
//! does not use those flags either: it sends `MSG_MOVE_SET_FACING`. So turning
//! here only rotates the orientation locally and rides out on the next packet,
//! which leaves every extrapolation the server does a straight line. The
//! resulting path is a curve *shorter* than the allowance, which is the safe
//! side of the comparison.

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
    pub const PITCH_UP: u32 = 0x0000_0040;
    pub const PITCH_DOWN: u32 = 0x0000_0080;
    pub const WALK_MODE: u32 = 0x0000_0100;
    pub const LEVITATING: u32 = 0x0000_0400;
    pub const FIXED_Z: u32 = 0x0000_0800;
    pub const ROOT: u32 = 0x0000_1000;
    pub const JUMPING: u32 = 0x0000_2000;
    pub const FALLINGFAR: u32 = 0x0000_4000;
    pub const SWIMMING: u32 = 0x0020_0000;
    pub const SPLINE_ENABLED: u32 = 0x0040_0000;
    pub const FLYING: u32 = 0x0100_0000;
    pub const ONTRANSPORT: u32 = 0x0200_0000;
    pub const SPLINE_ELEVATION: u32 = 0x0400_0000;
    pub const WATERWALKING: u32 = 0x1000_0000;
    pub const SAFE_FALL: u32 = 0x2000_0000;
    pub const HOVER: u32 = 0x4000_0000;

    /// `MOVEFLAG_MASK_MOVING`. The anticheat's "was it moving?" test, and the
    /// trigger for the 500 ms heartbeat requirement.
    pub const MASK_MOVING: u32 = FORWARD
        | BACKWARD
        | STRAFE_LEFT
        | STRAFE_RIGHT
        | PITCH_UP
        | PITCH_DOWN
        | JUMPING
        | FALLINGFAR
        | SPLINE_ELEVATION;

    /// The bits that mean **travel across the ground**, which is a narrower
    /// question than [`MASK_MOVING`] and the one [`super::strode`] asks.
    ///
    /// `MASK_MOVING` is the *anticheat's* mask and it folds in the two pitch
    /// bits and the two air bits, none of which is a stride: a swimmer holding
    /// the ascend key is "moving" by that definition and travels nowhere on the
    /// map. Keying the horizontal step on it would swim a character forward at
    /// their swim speed for as long as they held the key to go straight up.
    pub const MASK_TRAVEL: u32 = FORWARD | BACKWARD | STRAFE_LEFT | STRAFE_RIGHT;
}

/// Baseline run speed in yards/second (`baseMoveSpeed[MOVE_RUN]`). Only used
/// when the server has not told us the real value yet.
pub const BASE_RUN_SPEED: f32 = 7.0;

/// Downward acceleration, yards per second squared. `Movement::gravity` in
/// `Movement/spline/util.cpp`, and the client's own number — the server carries
/// it because it re-runs the client's fall to check it.
pub const GRAVITY: f32 = 19.291_105_270_385_74;

/// Upward speed a jump starts with.
///
/// **Stated by the server about the client**, which is the only reason it is not
/// a guess: `HandleMovementOpcodes` does `SetJumpInitialSpeed(7.95797334f)` the
/// moment a `MSG_MOVE_JUMP` arrives, because that is what it needs to reproduce
/// the arc the client is about to walk. Reporting a different one is not
/// rejected outright — `CheckSpeedHack` only measures the horizontal — but it
/// puts the two simulations on different parabolas, and the client is the one
/// that has to land.
pub const JUMP_SPEED: f32 = 7.957_973_34;

/// Fall speed ceilings, `Movement::terminalVelocity` and
/// `terminalSavefallVelocity`. The second is Slow Fall / Levitate, which arrives
/// as `MOVEFLAG_SAFE_FALL`.
pub const TERMINAL_VELOCITY: f32 = 60.148_003;
pub const TERMINAL_SAFE_FALL_VELOCITY: f32 = 7.0;

/// How far a fall has dropped after `t` seconds, given the **downward** speed it
/// started with.
///
/// A direct port of `Movement::computeFallElevation(t, isSafeFall,
/// start_velocity)`, which is what `Unit::ExtrapolateMovement` runs to decide
/// where a jumping client should be — so this is not "a plausible gravity", it
/// is the same function on both ends of the socket. Note the sign convention it
/// inherits: a *jump* is passed `-JUMP_SPEED`, i.e. a negative downward speed,
/// and the caller subtracts the result from the arc's starting height.
///
/// **`safe_fall` clamps the start velocity and nothing else**, which looks like
/// a bug in the original and is copied faithfully anyway: the linear part past
/// terminal time still runs at [`TERMINAL_VELOCITY`], so an ordinary Slow Fall
/// (start velocity zero) descends at exactly the ordinary rate. Correcting it
/// here would put the client on a different parabola from the server that is
/// checking it, which is the one thing this function must not do.
pub fn fall_elevation(t: f32, safe_fall: bool, start_velocity: f32) -> f32 {
    let terminal = if safe_fall {
        TERMINAL_SAFE_FALL_VELOCITY
    } else {
        TERMINAL_VELOCITY
    };
    let start_velocity = start_velocity.min(terminal);
    // The moment terminal velocity is reached, measured from the arc's start —
    // negative for a jump, which is why the branch below is on `t` and not on
    // "have we been falling long".
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

/// **Where a moving platform is right now** — the answer to
/// [`crate::socket::session::World::platform`].
///
/// A game object's position and facing, taken from the collision world at the
/// moment it is asked, which for a transport is the placement the *renderer*
/// last built its hull at. So the platform this reports and the floor the
/// character is standing on are the same thing by construction, which is the
/// property the carry below depends on: a ferry computed against a stale
/// placement would slide the passenger off the deck it is holding them to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Platform {
    pub guid: u64,
    pub position: [f32; 3],
    pub facing: f32,
}

/// **The character's relationship to the platform under its feet** — the state
/// behind `MOVEFLAG_ONTRANSPORT`.
///
/// ## Why the client owns this and the server does not
///
/// `WorldSession::HandleMoverRelocation` boards a player **only when their own
/// movement packet arrives carrying the flag and the transport's guid**: it
/// looks the guid up in the map and calls `AddPassenger`. Nothing else does. So
/// a client that never sets the flag is never a passenger, `UpdatePassengerPositions`
/// never moves it, and the deck slides out from under it — which is the report
/// this exists for, on the Deeprun Tram, whose car travels 2,482 yards.
///
/// ## What is held, and why it is the offset rather than the position
///
/// The offset is the character's place **in the platform's own frame**, and it
/// is what stays constant while the platform moves. Each step the world
/// position is recomputed from it and the platform's new placement, which is
/// the carry; the character then walks from there as usual, and the offset is
/// re-derived from where they ended up.
///
/// The transform is a yaw and a translation, byte for byte
/// `GenericTransport::CalculatePassengerPosition` and its inverse — no pitch, no
/// roll, and the same `Rz` this client already places a game object's model and
/// hull with. See [`Ferry::aboard`] and [`Ferry::carry`].
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
    /// **Board**: take a world position into the platform's frame.
    ///
    /// `CalculatePassengerOffset` — subtract the platform's origin, then rotate
    /// by `-facing`. The orientation is the difference of the two, so a
    /// character facing along the deck goes on facing along it however the deck
    /// turns.
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

    /// **…and be carried**: put the held offset back into the world against the
    /// platform's *current* placement.
    ///
    /// `CalculatePassengerPosition`, the exact inverse of [`Self::aboard`]. This
    /// is the whole of the ride: the offset does not change, the platform moves,
    /// and the character moves with it.
    pub fn carry(&self, platform: Platform) -> Position {
        let (sin, cos) = platform.facing.sin_cos();
        Position {
            x: platform.position[0] + self.offset.x * cos - self.offset.y * sin,
            y: platform.position[1] + self.offset.y * cos + self.offset.x * sin,
            z: platform.position[2] + self.offset.z,
            orientation: normalize_orientation(platform.facing + self.offset.orientation),
        }
    }

    /// Has the platform moved since the offset was taken? The carry is skipped
    /// when it has not, so a character standing on a *stationary* game object —
    /// which is nearly all of them — pays one comparison and nothing else.
    pub fn moved(&self, platform: Platform) -> bool {
        self.at != platform.position || self.at_facing != platform.facing
    }

    /// **Where the platform stood when this offset was taken.**
    ///
    /// The offset alone does not name a place: it is a coordinate in a frame,
    /// and the frame is this. A reader that wants to put the passenger back
    /// into the world against a *different* reading of the same platform — the
    /// renderer, which draws the deck on its own clock and must draw whoever is
    /// standing on it at the same moment — needs both halves, which is why this
    /// is published rather than kept private to the carry.
    pub fn platform(&self) -> Platform {
        Platform {
            guid: self.guid,
            position: self.at,
            facing: self.at_facing,
        }
    }
}

/// `Geometry::NormalizeOrientation` — the server's own, and it is a modulo
/// rather than a clamp: an orientation outside `[0, 2pi)` is refused by
/// `IsValidMapCoord`, and both of [`Ferry`]'s transforms add or subtract two
/// angles that are each already in range.
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
/// Field order is `zspeed, cosAngle, sinAngle, xyspeed` — note that cosine
/// comes *before* sine, which is the opposite of the usual convention.
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
    /// The client's own millisecond tick (`ctime` server-side). Must be
    /// non-zero and never go backwards — see the module docs.
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

    /// Is any flag set that makes the server expect 500 ms heartbeats?
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

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        self.write(&mut w);
        w.buf
    }

    /// Read a movement block, or `None` if it is truncated.
    ///
    /// Bounds-checked at every field group rather than indexing blindly: this
    /// same layout appears *inline* inside `SMSG_UPDATE_OBJECT`, where reading
    /// one field short turns the next block's first byte into a garbage update
    /// type several blocks later.
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

    /// Heading actually travelled, which is the facing rotated by the strafe
    /// keys.
    ///
    /// The forward half is straight out of `Unit::ExtrapolateMovement`. **The
    /// backward diagonals are not, and that is deliberate**: the server's
    /// version lets `MOVEFLAG_BACKWARD` absorb a strafe outright, so
    /// back-and-left is walked straight backwards — and holding S and then
    /// pressing a strafe key does nothing whatsoever, which is not what the
    /// game does. 1.12 moves a character back-and-left at 135°, the mirror of
    /// the forward-and-left 45° the same function already encodes; whoever
    /// wrote the server's copy modelled one pair and not the other.
    ///
    /// **Diverging here is free, and that is why it is allowed.**
    /// `CheckSpeedHack` extrapolates from the last packet and accumulates
    /// `m_overspeedDistance` only when the *distance* travelled exceeds the
    /// distance the extrapolation allows, by more than 10%
    /// (`MovementAnticheat.cpp`, `realDistance2D_sq > (allowedDX + allowedDY) *
    /// 1.1f`). A different heading over the same interval covers exactly the
    /// same ground, so the comparison is unmoved; and vanilla is
    /// client-authoritative about position, so the server takes the reported
    /// place either way. The one rule this must not break is that the
    /// *renderer's* prediction and the mover integrate the same expression, and
    /// they both call this.
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

/// Which movement keys are held. Purely an input snapshot — [`Mover`] turns it
/// into flags, opcodes and positions.
///
/// `turn_left` / `turn_right` deliberately do **not** map to
/// `MOVEFLAG_TURN_*`; see the module docs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Controls {
    pub forward: bool,
    pub backward: bool,
    pub strafe_left: bool,
    pub strafe_right: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub walk: bool,
    /// **Swim straight up, while held** — `PITCHUP` in `Bindings.xml`, and the
    /// space bar in this client.
    ///
    /// Deliberately *not* in [`Self::to_flags`]: the flags it implies are only
    /// legal while swimming, and the gate is the client's own — it tests
    /// `MOVEFLAG_SWIMMING` and only then reaches its pitch handler. See
    /// [`Mover::pitch_flags`], which is where that gate lives.
    pub ascend: bool,
    /// …and down, `PITCHDOWN`. **No key in this client**, which is a gap rather
    /// than a decision: the binding ships and a swimmer aims down with the
    /// mouse instead. The field is here because the two are one opposed pair in
    /// the client's own handler and reading only half of it would put the
    /// asymmetry somewhere it cannot be seen.
    pub descend: bool,
}

impl Controls {
    /// The movement flags these keys imply. Opposite keys cancel, exactly as
    /// the real client does — sending both `FORWARD` and `BACKWARD` would make
    /// the server extrapolate backwards while we walked forwards.
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
    /// Flags *after* this transition. Each event is a single flag change, so
    /// these must be applied in order.
    pub flags: u32,
}

/// Split a flag change into the sequence of packets the server will accept.
///
/// `CheckMoveStart` rejects any packet that introduces a movement flag under
/// the wrong opcode, so a change of two flags is two packets. Stops are emitted
/// before starts, which is both what the real client does and what keeps
/// "turn around on the spot" (`FORWARD` -> `BACKWARD`) legal.
pub fn transitions(old: u32, new: u32) -> Vec<MoveEvent> {
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
    // **The same shape as the strafe pair and with the same rule behind it.**
    // `CheckMoveFlags` tests `MOVEFLAG_PITCH_UP` and `MOVEFLAG_PITCH_DOWN`
    // exactly the way it tests the four travel bits: the start opcode must
    // carry its own flag, and no other opcode may introduce it. The three
    // opcodes are consecutive (191/192/193) because the client reaches them
    // through consecutive indices into its own table — see [`Mover::ascend`],
    // which is where the measurement is.
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
/// **The horizontal velocity is frozen at take-off**, which is the client's own
/// rule (1.12 has no air control) *and* the server's: `ExtrapolateMovement`
/// walks `jump.start + (cosAngle, sinAngle) * xyspeed * t` and does not consult
/// the movement flags at all while `MOVEFLAG_JUMPING` is set. Steering in the
/// air would therefore be measured against a straight line and accumulate
/// `m_overspeedDistance` for as long as the jump lasted.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Airborne {
    /// Where the arc began. The server anchors its own copy of this at the
    /// **first packet carrying the flag** (`MovementInfo::Read`, `if
    /// (!jump.startClientTime)`), which is why a take-off is reported at once
    /// rather than on the next heartbeat.
    start: [f32; 3],
    /// Seconds since it began.
    elapsed: f32,
    /// Upward speed at t = 0. [`JUMP_SPEED`] for a jump, zero for walking off
    /// an edge, whatever the server said for a knockback.
    up: f32,
    /// Did this begin with a `MSG_MOVE_JUMP`? A knockback and a step off a
    /// ledge both end in `MSG_MOVE_FALL_LAND` too, but only a jump counts
    /// against `CHEAT_TYPE_MULTI_JUMP`, which allows exactly one before a
    /// landing.
    jumped: bool,
}

/// **The server is driving this character**: an `SMSG_MONSTER_MOVE` addressed to
/// our own guid, which the local simulation has to *ride* rather than ignore.
///
/// Warrior Charge is the case this was written for, and it is neither a teleport
/// nor a knockback. `Spell::OnSpellLaunch` ends in
/// `GetMotionMaster()->MoveCharge(unitTarget, …)`, which mutates the *player's*
/// own motion master into a `ChargeMovementGenerator` and launches an ordinary
/// `MoveSpline` — the same machinery a patrolling creature is walked with. So
/// the packet that arrives is the same packet every creature in the world sends,
/// with one difference that changes everything: it names us.
///
/// **And it is owed an answer.** `MoveSplineInit::Launch` raises
/// `SetSplineDonePending(true)` for any player mover, and
/// `WorldSession::HandleMovementOpcodes` opens with
///
/// ```text
/// if (pMover->HasPendingSplineDone())
///     return;
/// ```
///
/// — so from the moment a charge is cast until `CMSG_MOVE_SPLINE_DONE` arrives,
/// **every movement packet this client sends is discarded without a word**. Only
/// two things clear the flag: that packet, and `Map::Add`, which is a login. That
/// is the whole of the report this exists to fix — "it glitches the char's
/// position and requires a relog" is one cause with two halves, the position
/// being the ride nobody rode and the relog being the ack nobody sent.
struct Ride {
    /// The path, walked exactly as [`crate::state::objects::ObjectManager::advance`]
    /// walks a creature's — the same type, so the character on the wire and the
    /// creature beside it cannot end up on two different curves.
    spline: Spline,
    /// Seconds since the ride began. Kept beside [`Spline::elapsed_ms`] rather
    /// than replacing it because the mover steps in fractional seconds and the
    /// spline counts whole milliseconds: truncating each step instead of the
    /// total loses up to a millisecond a tick, which over a one-second charge is
    /// forty of them.
    elapsed: f32,
    /// Echoed back in `CMSG_MOVE_SPLINE_DONE`, which the server drops outright
    /// unless it matches `movespline->GetId()`.
    spline_id: u32,
    /// Where to look on arrival for a [`SplineFacing::Target`] — resolved by the
    /// handler, because only the object manager can find another unit. A charge
    /// carries this: `ChargeMovementGenerator::Initialize` ends in
    /// `init.SetFacingGUID(unit.GetTargetGuid())`.
    facing_at: Option<[f32; 3]>,
}

/// How far the ground may be below the feet before the character is falling
/// rather than walking down a slope.
///
/// Under this it is a step and the character is simply placed on it. A slope
/// steep enough to matter still produces far less than this per 25 ms tick — at
/// a 7.6 y/s run that is 19 cm of travel — so this only ever catches a real
/// edge.
///
/// Shared with [`crate::state::objects::ObjectManager::advance`], which asks the same
/// question about a unit it is *not* simulating — "is this a stair or a cliff?"
/// — and answers it the other way round: a stair is followed, a cliff is left
/// for the server to state.
///
/// Public because the *prediction* needs it too, and for the reason that whole
/// module exists: `crates/client`'s `world::predict` continues the same step
/// this function takes, so the number that decides "step or cliff" has to be
/// one value rather than two that agree today.
pub const FALL_THRESHOLD: f32 = 0.7;

/// The fraction of a unit's own height the water has to reach before it is
/// swimming in it — **0.75**.
///
/// The client's water tick has one shape: two thresholds against one quantity,
/// the *depth* — the liquid surface minus the feet, from the liquid query's
/// level and the unit's own position. The query mask it asks with is **0x0f**,
/// all four liquids, so lava and slime are swum in too. The unit starts
/// swimming once the depth exceeds 0.75 of its own height.
///
/// vmangos states the same rule from the other side —
/// `GetMinSwimDepth() { return GetCollisionHeight() * 0.75f; }` with the comment
/// *"client switches to swim animation at this depth"* — so the two agree.
pub const SWIM_DEPTH_FRACTION: f32 = 0.75;

/// The height the fraction above is *of*, and **the one number here that is not
/// this character's own.**
///
/// The height is a per-unit field — the same one the gait cascade doubles —
/// and the reference fills it from the model. Nothing in the
/// protocol crate can see a model, and 2.0 is what vmangos initialises
/// `m_modelCollisionHeight` to for a unit whose own model states none, which
/// makes the threshold **1.5 yards**. A character noticeably taller or shorter
/// than a human therefore begins swimming a few inches early or late; it is
/// stated here rather than hidden because the shape of the rule (a fraction of
/// the swimmer's height) is the reference's and only the height is a stand-in.
pub const COLLISION_HEIGHT: f32 = 2.0;

/// How deep the water must be before the character swims — see
/// [`SWIM_DEPTH_FRACTION`] and [`COLLISION_HEIGHT`].
pub const MIN_SWIM_DEPTH: f32 = COLLISION_HEIGHT * SWIM_DEPTH_FRACTION;

/// The dead band under [`MIN_SWIM_DEPTH`] at which swimming *stops* —
/// **1/36 of a yard** (the float `0x3ce38e39`).
///
/// The client subtracts it once, before either branch runs, and the *stop*
/// branch compares the depth against the difference. Two and a half
/// centimetres, which is not a tuning value: it is
/// exactly enough that a swimmer floating at the entry depth — where this client
/// holds one — does not chatter between the two states, one packet apiece, for
/// as long as they stay in the water.
pub const SWIM_EXIT_HYSTERESIS: f32 = 1.0 / 36.0;

/// **How far off the ground [`move_flags::HOVER`] holds a unit — one yard.**
///
/// The flag is recorded by two whole packet families and nothing here drew it,
/// because nothing here knew the height: 1.12 has no `UNIT_FIELD_HOVERHEIGHT`
/// (that is 3.x) and vmangos' `Aura::HandleAuraHover` is one line —
/// `GetTarget()->SetHover(apply)` — so the server states the flag and never a
/// distance. It is the client's own constant, and the client says it twice.
///
/// The movement step is the one that matters. It puts the unit on the surface
/// (`z -= drop`); if the unit is not `HOVER` that is the end of it. A hovering
/// unit has the drop undone, then takes `t = drop - 1.0`: if `t < 0` it is left
/// where it is, otherwise `z -= t`.
///
/// That comes to `z = surface + 1.0` when the drop is more than a yard, and
/// **no change at all when it is less** — a hovering unit is lowered onto its
/// yard of air and never lifted onto it.
///
/// The second site is the selection box: a hovering unit's box has
/// its floor dropped by the same 1.0, so the thing a player clicks still reaches
/// the ground under it.
pub const HOVER_HEIGHT: f32 = 1.0;

/// **What a unit with these movement flags stands on**, given the floor under it
/// and the liquid surface over that — `None` where the client has no data at
/// all.
///
/// Two flags this client recorded correctly for rounds and drew nothing for:
///
/// * **[`move_flags::WATERWALKING`]** puts the unit on the liquid surface rather
///   than the floor under it. The floor still wins where it is the higher of the
///   two, which is a lake seen from a bridge over it: a water-walker on the
///   bridge stands on the bridge.
/// * **[`move_flags::HOVER`]** adds [`HOVER_HEIGHT`] to whichever of those it
///   came to. The two compose — the reference's hover arm runs on the drop that
///   the surface choice produced, whatever chose it.
///
/// One function because both callers need exactly this and neither may differ
/// from the other: [`Mover::advance`] walks the local player and
/// `ObjectManager::stand_on_the_ground` puts everybody else down, and a unit
/// drawn on the lake bed while the server has it on the surface is the same bug
/// either way round.
///
/// **The liquid is the caller's to fetch or not.** It is passed rather than
/// queried so that the observer path can skip [`Footing::liquid`] entirely for
/// the overwhelming majority of units, which carry neither flag.
pub fn standing_surface(flags: u32, floor: Option<f32>, liquid: Option<f32>) -> Option<f32> {
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
/// **Free function, and that is the whole reason it exists.** The renderer
/// predicts the local player forward from the last reading it was given so that
/// a keypress moves the character in the frame it was pressed rather than a
/// round trip later, and a prediction that does not integrate the *same*
/// expression the simulation does is a prediction that has to be corrected
/// every step — which is the judder it was meant to remove. There is one copy
/// of the arithmetic and both callers use it.
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

/// How far the **drawn body** sits from the direction a unit is aiming, in
/// radians, given the keys it is holding.
///
/// **This is the whole of how 1.12 strafes, and it is not an animation.** The
/// ground ships no sideways gait — `RunLeft`/`RunRight` are rows in
/// `AnimationData.dbc` that no model carries, and the Shuffles are the
/// turn-in-place foot-shuffle — so a strafing character plays *Run*. What makes
/// it read as running sideways is that the rendered root is turned into the
/// slide while the aim (the camera, the server's orientation, the thing the
/// character is facing) holds still: ±90° for a pure strafe, ±45° when forward
/// or backward is held too. The renderer eases the model onto it and twists the
/// spine and head back — see `vale_assets::world::m2::BodyTwist`.
///
/// Yaw is left-positive, so a strafe left is `+` — **mirrored while
/// backpedalling**, which is the part that has to be stated rather than
/// guessed: a back-and-left diagonal faces the body forward-right and
/// backpedals along the line of travel, so the legs never cross.
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
/// Zero when the block says the unit is not moving, which is what makes it safe
/// to call unconditionally. The airborne case is **not** here: in the air the
/// velocity is the one frozen at take-off and lives in the jump block, which is
/// the mover's own business — and it is also why the renderer's prediction
/// stops at the moment the character leaves the ground (see [`turned`]).
pub fn strode(info: &MovementInfo, speeds: &Speeds, dt: f32) -> [f32; 2] {
    // **`MASK_TRAVEL`, not `is_moving`** — see the mask's own note. A swimmer
    // holding the ascend key carries `MOVEFLAG_PITCH_UP`, which is in the
    // anticheat's moving mask and is not a stride: keyed on that, holding the
    // key to go straight up would also swim the character forwards.
    if !info.has(move_flags::MASK_TRAVEL) {
        return [0.0, 0.0];
    }
    let speed = info.speed(speeds);
    let heading = info.heading();
    // **A swimmer's stroke is not all travel across the ground.** The body is
    // pitched — that is what [`MovementInfo::pitch`] is, and it is the one field
    // the wire carries *only* under `MOVEFLAG_SWIMMING` — so a stroke along the
    // body covers `cos(pitch)` of its length horizontally and the rest of it in
    // [`climbed`]. Diving straight down at `pitch = -pi/2` therefore moves the
    // character nowhere on the map, which is what it looks like.
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

/// …and the **vertical** half of the same stroke, which only a swimmer has.
///
/// Zero on the ground and zero in the air: a fall is a parabola the mover owns
/// and a walk has no vertical of its own at all, which is why the ground path
/// asks [`Footing::floor`] instead. In the water there is no gravity and the
/// only thing that changes the altitude is the swimmer's own pitch.
///
/// **A strafe has no vertical**, and neither does turning: the pitch is the
/// body's and it is `FORWARD`/`BACKWARD` that travel along it. Backpedalling
/// mirrors it, for the same reason [`MovementInfo::heading`] mirrors the
/// horizontal — a character pitched up and swimming backwards goes down.
///
/// A free function beside [`strode`] because the two are one velocity split into
/// its parts, and a caller that integrated one without the other would move a
/// diver horizontally at their full speed.
pub fn climbed(info: &MovementInfo, speeds: &Speeds, dt: f32) -> f32 {
    if !info.has(move_flags::SWIMMING) {
        return 0.0;
    }
    let speed = info.speed(speeds);
    // **The ascend key is a whole stroke of its own and it outranks the aim.**
    // `MOVEFLAG_PITCH_UP` is a movement bit in its own right — it is in
    // `MOVEFLAG_MASK_MOVING`, so the server measures it as travel at the swim
    // speed, which is where this rate comes from rather than from a constant.
    // Held alone it is straight up: [`strode`] gives no horizontal without a
    // travel key, so there is nothing else to compose with.
    //
    // **What the reference does with *both* held is not established**, and this
    // adds them: swimming forward with the ascend key covers ground and rises
    // at the same time, which is faster through the water than either alone.
    // Stated rather than clamped, because a clamp would be a second invented
    // rule on top of an unmeasured one.
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

/// What the server has taken away from this character — as distinct from the
/// movement flags, which say what it is *doing*.
///
/// **The two halves are separate because the client keeps them separate**, and
/// that is the whole of why a root and a stun feel different to play against: a
/// rooted character still turns on the spot and still swings at what it is
/// facing, a stunned one is a statue. The rule lives in the client's input
/// tick — see [`crate::state::objects::Entity::is_stunned`], which states
/// it — and it reads two different fields for the two answers:
/// `MOVEFLAG_ROOT` out of the movement block for the stride, `UNIT_FLAG_STUNNED`
/// out of `UNIT_FIELD_FLAGS` for the turn, with health gating both.
///
/// Nothing here is on the wire as such. `ROOT` arrives as its own packet and
/// lands in [`MovementInfo::flags`]; these two are *descriptor* fields on the
/// player's own entity, so they are read out of the object manager once a tick
/// and handed to the mover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Restraint {
    /// `UNIT_FIELD_FLAGS & UNIT_FLAG_STUNNED` (0x40000) — no turning.
    pub stunned: bool,
    /// `UNIT_FIELD_HEALTH == 0`. The client tests health and the stand state
    /// separately and both fail a corpse; one boolean
    /// covers them because vmangos writes the health and the stand state
    /// together in `KillPlayer`.
    pub dead: bool,
    /// **`UNIT_FIELD_FLAGS & UNIT_FLAG_TAXI_FLIGHT` — the server is flying this
    /// character**, and neither the keys nor the mouse have any say until it
    /// lands.
    ///
    /// A ride already stops both ([`Mover::can_move`] tests `ride.is_none()`),
    /// so during an ordinary flight this says nothing new. What it covers is the
    /// case where the two ends disagree: **the flag outlives a session and the
    /// ride does not**. A character who logs out mid-flight logs back in wearing
    /// it, and if the resumed spline is ever missed the alternative to obeying
    /// this is a character walking around on the ground while the server holds
    /// them on a gryphon and discards every packet they send — which is not a
    /// hypothetical, it is what `SessionLoop::enter_world` used to do.
    ///
    /// Safe to obey because it means one thing: vmangos sets and clears it in
    /// `FlightPathMovementGenerator`'s `Initialize` and `Finalize` and nowhere
    /// else, always beside `UNIT_FLAG_REMOVE_CLIENT_CONTROL`, whose own comment
    /// is *"disable player movement"*.
    pub on_taxi: bool,
}

/// The client's own copy of where it is and where it is going.
///
/// The server is authoritative, but it only *corrects* us — between corrections
/// the client is expected to simulate itself and report the result. Getting
/// that simulation wrong does not desynchronise quietly; it trips the speed
/// hack check.
pub struct Mover {
    pub info: MovementInfo,
    pub speeds: Speeds,
    controls: Controls,
    /// Orientation last sent, so a facing change can be detected.
    sent_orientation: f32,
    /// The arc, while there is one. `None` is "standing on something".
    air: Option<Airborne>,
    /// The server-driven ride, while there is one — see [`Ride`]. It outranks
    /// the keys, the arc and the ground.
    ride: Option<Ride>,
    /// The spline id a finished ride owes the server, until the loop sends it.
    ///
    /// Held rather than sent because a handler under the world lock must not
    /// write to the socket, and because `CMSG_MOVE_SPLINE_DONE` is the one
    /// outbound movement packet that is not a bare [`MovementInfo`] — see
    /// [`spline_done_body`].
    finished_ride: Option<u32>,
    /// What the server has stopped this character doing — see [`Restraint`].
    restraint: Restraint,
    /// **The moving platform this character is standing on**, or `None` for the
    /// overwhelming majority of a session. See [`Ferry`], which is the whole of
    /// what it means.
    ferry: Option<Ferry>,
    /// Which platform the **server** was last told about, so a boarding and a
    /// step ashore can each be reported the moment they happen rather than at
    /// the next heartbeat. See [`Self::ferry_changed`].
    sent_ferry: Option<u64>,
    /// A transport teleport has restated the offset and the world position is
    /// owed a fresh composition against the deck's next placement — see
    /// [`Self::transfer_aboard`]. While it is set the offset is **frozen**:
    /// neither the carry nor the stow may re-derive it from a world position,
    /// because every world position between the teleport and the first
    /// placement on the new map is about somewhere else.
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

    /// **Board or be carried**, once per tick and *before* the step.
    ///
    /// `under` is whatever the world says is directly underfoot *and is a thing
    /// that moves* — the caller filters, because nothing in this crate knows
    /// which game objects are transports and the object manager does
    /// (`UPDATEFLAG_TRANSPORT`, `Entity::transport_phase_ms`).
    ///
    /// Three cases and they are all here rather than at the call site, because
    /// the ordering between them is the whole subject:
    ///
    /// * **Still on the same platform**: the character is moved by however far
    ///   it moved, *before* their own step is taken, so they walk from where
    ///   the deck put them. This is the carry, and it is the report.
    /// * **A different platform, or a first boarding**: the offset is taken
    ///   fresh from where they already are. Nothing moves.
    /// * **Nothing underfoot**: the flag and the block come off. **The position
    ///   is left exactly where it is** — stepping off a moving deck leaves you
    ///   where you were standing, and recomputing anything here would teleport
    ///   a character who simply walked down the gangway.
    ///
    /// **This is half of a pair and it is useless on its own**: the offset it
    /// leaves behind describes where the character stood *before* their stride,
    /// so [`Self::stow_platform`] must run after the step to record where they
    /// ended up. See that function, which is where the whole of the reason is
    /// written down.
    ///
    /// **And what it carries is a delta rather than a stored place.** The
    /// offset is recomputed from wherever the character is at the moment this
    /// runs, so a heading the mouse set, a position the server corrected and a
    /// knockback the server threw — all three drained before `tick_movement`
    /// even begins — survive being aboard. Reading the recorded offset instead
    /// puts the state of a tick ago back over every one of them.
    ///
    /// Answers whether the world position changed, which the caller uses to
    /// decide whether the server has to be told sooner than the heartbeat.
    pub fn carry_platform(&mut self, under: Option<Platform>) -> bool {
        let Some(platform) = under else {
            // **A transfer in flight is not a step ashore.** The deck was kept
            // across a far teleport on the server's own say-so
            // (`TELE_TO_NOT_LEAVE_TRANSPORT`), and the world merely has not
            // placed it on the new map yet. Taking the ferry here would send a
            // no-flag packet — `RemovePassenger`, and the character is left
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
            // **A transport teleport's first placement on the new map: the
            // offset is composed absolutely, not carried as a delta.** The
            // recorded placement describes the map that was left, and between
            // the teleport and this moment the position was resynced — first to
            // `SMSG_NEW_WORLD`'s body, then to the far side's own create block —
            // so a delta between those two frames is thousands of yards of
            // garbage folded into the offset. The offset itself is the one
            // thing both ends still agree on (`HandleMoveWorldportAck`
            // relocates a passenger with `UpdatePassengerPosition`, from the
            // offset, rather than to the coordinates it sent), so the position
            // is rebuilt from it and nothing else.
            Some(ferry) if ferry.guid == platform.guid && self.ferry_recompose => {
                // Against the *held* record there is nothing to compose yet:
                // the placement handed back is the stale one the hold repeats,
                // and the offset stays frozen until a real one arrives.
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
                // **The offset is recomputed from where the character actually
                // is, not read off the record.** Between the last tick's
                // `stow_platform` and this line, three things can have written
                // the position or the heading — `Mover::face` from the mouse,
                // `Mover::resync` from a server correction, and a knockback —
                // and every one of them is drained *before* `tick_movement`
                // runs. Carrying the stored offset overwrites all three with
                // the state of a tick ago, which is the same fault the frozen
                // stride was and one axis over.
                //
                // Mouse-look is where it showed: `face` writes an absolute
                // world heading and the carry put the old one straight back, so
                // a passenger could turn with A and D — those run inside
                // `advance`, after this — and not with the mouse. Recomputed
                // this way the whole thing is a *delta*: the character is moved
                // and turned by however much the deck was, and nothing else
                // about them is touched. It is exactly the identity when
                // nothing wrote, so the ride is unchanged.
                let ferry = Ferry::aboard(ferry.platform(), before);
                self.info.position = ferry.carry(platform);
                // **An arc in flight is carried too, by translating where it
                // began.** `advance` walks the vertical from `Airborne::start`
                // and overwrites whatever the carry above put in `z`, so a jump
                // taken on a rising lift would otherwise be measured against
                // the world: the parabola comes down where the deck used to be
                // and the floor query reads that as having fallen through it.
                // Moving the anchor keeps the arc in the platform's frame,
                // which is the frame the character jumped in.
                //
                // All three axes, though only `z` is ever read — the horizontal
                // is frozen at take-off in the jump block and the carry above
                // has already moved `x`/`y`. Keeping the whole anchor in step
                // costs two adds and means the record still names the point in
                // space the arc began at, rather than a place with one axis
                // from each of two frames.
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

    /// **…and record where the step ended**, once per tick and *after* it.
    ///
    /// The offset is the character's place in the platform's own frame and it
    /// is what the wire carries — vmangos' `HandleMoverRelocation` throws the
    /// world position away for a passenger and recomputes it from `t_pos`
    /// against its own copy of the transport, so the offset is not a hint, it
    /// is the position.
    ///
    /// **Taking it before the stride instead of after is a character frozen to
    /// the deck**, and that is the report this exists for. The carry above puts
    /// the passenger where the platform moved them and then the offset was
    /// re-derived on the spot; `advance` then took the stride the keys asked
    /// for; and the *next* tick's carry put the character back at the offset
    /// recorded before it — so every stride taken while the platform was moving
    /// was undone by the following tick, forty times a second. Walking on a
    /// stationary lift worked, which is why it read as "you cannot move while
    /// it is moving" rather than as anything to do with the offset.
    ///
    /// Against the *same* placement [`Self::carry_platform`] used, deliberately:
    /// asking the world again here would be a second collision query a tick and
    /// would fold a platform that moved mid-tick into the character's own
    /// stride. A character who walked off the deck is dropped by the next
    /// tick's query, one step later, which is the tick they would have been
    /// dropped on anyway.
    pub fn stow_platform(&mut self) {
        // **Frozen across a transfer.** Between the teleport and the first
        // placement on the new map every world position is about somewhere
        // else — the offset the packet restated is the only truth, and
        // re-deriving it from one of those positions is the fling this flag
        // exists to prevent. See [`Self::transfer_aboard`].
        if self.ferry_recompose {
            return;
        }
        let Some(ferry) = self.ferry else {
            return;
        };
        self.stow_against(ferry.platform());
    }

    /// Take the offset from wherever the character is now, and put the flag and
    /// the block on the movement info. Both halves of the pair end here.
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
    /// neither field has a packet of its own: both arrive inside whatever values
    /// block happened to change, and a stun is `UNIT_FIELD_FLAGS` moving in the
    /// same update as the aura that caused it.
    pub fn set_restraint(&mut self, restraint: Restraint) {
        self.restraint = restraint;
    }

    /// May this character **travel**? The client's own travel check.
    ///
    /// A root, and death. Not a stun as such — every stun in the game roots as
    /// well (`HandleAuraModStun` ends in `SetRooted(true)`), so the flag is
    /// redundant here and reading it here instead would have made the two
    /// indistinguishable.
    ///
    /// **And a third reason that is not in the client's check**: a [`Ride`]. The two
    /// above are the server refusing; this one is the server *driving*, and the
    /// difference is that there is nothing to obey — the keys have no say
    /// because the path is already stated. Folded in here rather than checked
    /// beside every caller so that the stride, the jump and the keypress cannot
    /// disagree about who is steering.
    ///
    /// **…and a fourth, which is the same reason surviving a relog**: the
    /// server's own `UNIT_FLAG_TAXI_FLIGHT`. See [`Restraint::on_taxi`], which
    /// is why a ride's absence is not enough on its own.
    pub fn can_move(&self) -> bool {
        !self.restraint.dead
            && !self.restraint.on_taxi
            && !self.info.has(move_flags::ROOT)
            && self.ride.is_none()
    }

    /// May this character **turn**? The client's own turn check.
    ///
    /// A stun, and death. **Not a root**: a rooted character turns on the spot,
    /// which is what makes Frost Nova survivable and a Hammer of Justice not.
    ///
    /// …and a [`Ride`], for the reason [`Self::can_move`] gives: the spline
    /// states the facing too (a charge carries `SetFacingGUID`), so a mouse
    /// dragged mid-charge would be a second thing steering one character. The
    /// *camera* still turns, which is 1.12's own split — see `send_input`.
    pub fn can_turn(&self) -> bool {
        !self.restraint.dead
            && !self.restraint.stunned
            && !self.restraint.on_taxi
            && self.ride.is_none()
    }

    pub fn position(&self) -> Position {
        self.info.position
    }

    /// How fast this character is actually travelling, yards per second — the
    /// number a gait is chosen from, and zero when it is standing still.
    ///
    /// **A [`Ride`] is not any of the six speeds in [`Speeds`]**, which is why
    /// this exists rather than the call sites reading `info.speed(&speeds)`: a
    /// charge is run speed × 4 capped at 24 (`ChargeMovementGenerator::
    /// ComputePath`) and nothing on the wire states it — the server sends a path
    /// and a duration, and those two *are* the speed. Reading the run speed
    /// instead draws a character sprinting a third as fast as they are moving.
    pub fn travel_speed(&self) -> f32 {
        if let Some(ride) = self.ride.as_ref() {
            return ride.spline.speed();
        }
        if self.info.is_moving() {
            self.info.speed(&self.speeds)
        } else {
            0.0
        }
    }

    /// **Which way a server-driven ride is going**, in yards per second, or
    /// `None` when nothing is driving.
    ///
    /// The three-dimensional answer, which is what a reader continuing the
    /// simulation between ticks needs and what [`Self::travel_speed`] and the
    /// block's own orientation cannot give it: a taxi flight spends most of its
    /// length climbing or diving, and those two together describe a path that
    /// is flat. See [`Spline::velocity`].
    pub fn ride_velocity(&self) -> Option<[f32; 3]> {
        Some(self.ride.as_ref()?.spline.velocity())
    }

    /// Accept the server's version of where we are. Called when an update for
    /// the player's own GUID arrives.
    ///
    /// **Any arc in flight ends here.** The server has just stated a position,
    /// and a parabola anchored at where we *were* would carry the character on
    /// down through it — the correction and the fall would fight, which is the
    /// shape of "teleported and then sank into the floor".
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
    /// **A rooted character reports no keys at all**, and this is an anticheat
    /// rule rather than a nicety. [`transitions`] starts from the flags the
    /// block already has and *adds* to them, so a `W` pressed while stunned put
    /// `MOVEFLAG_FORWARD` on top of a `MOVEFLAG_ROOT` the server itself had set
    /// and sent it as `MSG_MOVE_START_FORWARD` — which is
    /// `CHEAT_TYPE_ROOT_MOVE` verbatim:
    ///
    /// ```text
    /// // Moving while rooted. We do not apply root flag until ack, so should be no false positives.
    /// if ((currentMoveFlags & MOVEFLAG_MASK_MOVING) &&
    ///     ((currentMoveFlags & MOVEFLAG_ROOT) || (GetLastMovementInfo().moveFlags & MOVEFLAG_ROOT)) &&
    ///     !me->HasPendingMovementChange(ROOT) && (opcode != CMSG_FORCE_MOVE_UNROOT_ACK))
    /// ```
    ///
    /// It carries a per-tick and a running penalty and can be configured to
    /// reject outright, so every stun in the game was one keypress away from an
    /// `Anticheat.log` entry. The keys are still **recorded** — the character
    /// still turns, and [`Self::set_rooted`] puts back whatever is held the
    /// moment the root lifts, which is the one packet the check exempts
    /// (`CMSG_FORCE_MOVE_UNROOT_ACK`).
    pub fn set_controls(&mut self, controls: Controls) -> Vec<MoveEvent> {
        self.controls = controls;
        if !self.can_move() {
            return Vec::new();
        }
        // **The pitch bits are added here rather than left to the reconcile**,
        // so that pressing the ascend key in the water is answered on the press
        // like every other key. [`Self::reconcile_pitch`] still exists and is
        // still needed: the *other* thing that changes the answer is entering
        // or leaving the water, which is not a keypress. They cannot disagree,
        // because both compute [`Self::pitch_flags`].
        let events = transitions(self.info.flags, controls.to_flags() | self.pitch_flags());
        if let Some(last) = events.last() {
            self.info.flags = last.flags;
        }
        events
    }

    /// Point the character somewhere outright — mouse-look, which sets an
    /// absolute heading rather than turning at a rate.
    ///
    /// **It goes through the same gate the keys do**, and that is what this
    /// method exists for: the drag used to write
    /// `mover.info.position.orientation` directly from the session's command
    /// queue, so a dead or stunned character could not turn with A and D and
    /// span freely on the mouse. Same rule, one door.
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

    /// Has the facing drifted far enough from the last one sent to be worth a
    /// `MSG_MOVE_SET_FACING`? A hundredth of a radian is well under what any
    /// observer could see, and keeps idle turning from flooding the socket.
    pub fn facing_changed(&self) -> bool {
        (self.info.position.orientation - self.sent_orientation).abs() > 0.01
    }

    pub fn mark_sent(&mut self) {
        self.sent_orientation = self.info.position.orientation;
        self.sent_ferry = self.ferry.map(|ferry| ferry.guid);
    }

    /// **Has the character boarded, changed deck or stepped ashore since the
    /// last packet went out?**
    ///
    /// The server learns both edges from one place and only from one place:
    /// `HandleMoverRelocation` calls `AddPassenger` when a movement packet
    /// arrives carrying `MOVEFLAG_ONTRANSPORT` and a guid it can find, and
    /// `RemovePassenger` when one arrives without the flag. Nothing else in
    /// vmangos boards or lands anybody. So a client that waits for the next
    /// heartbeat to mention it is a client the server does not know is on the
    /// boat, and it costs both halves:
    ///
    /// * **the ride**. `Transport::TeleportTransport` walks `m_passengers` and
    ///   nobody else, so a passenger the server never registered is left in the
    ///   ocean when the boat crosses to the other continent.
    /// * **the anticheat**. `IsTeleportAllowed3D` returns true outright for a
    ///   player the server has on a transport and otherwise measures world
    ///   distance against `CONFIG_FLOAT_AC_MOVEMENT_CHEAT_TELEPORT_DISTANCE`.
    ///   A boat covers 30 yards a second, so the first packet sent after a
    ///   silence is a teleport hack — rejected, with `SendHeartBeat(true)`
    ///   putting the character back where the deck used to be.
    ///
    /// The first of those is *"the ship vanished and left me in the sea"*; the
    /// second is *"I slide about the deck and my feet shuffle"*, which is a
    /// correction and a carry fighting each other forty times a second.
    pub fn ferry_changed(&self) -> bool {
        self.ferry.map(|ferry| ferry.guid) != self.sent_ferry
    }

    /// **A far teleport kept us on the deck: adopt the offset it restated and
    /// owe the world a composition.**
    ///
    /// The other half of [`Self::disembark`], for the one teleport that is a
    /// character *staying* on a boat rather than leaving one — and the packet
    /// is stranger than it looks: **`SMSG_NEW_WORLD`'s four floats are the
    /// transport offset, not a place.** `Player::SendNewWorld` writes
    /// `m_movementInfo.GetTransportPos()` when `m_transport` is set and
    /// `m_teleportDest` only when it is not, and `HandleMoveWorldportAck` then
    /// relocates the passenger with `UpdatePassengerPosition` — from the
    /// offset — rather than to any coordinate it ever sent. So the offset is
    /// the whole of what the packet says, and reading it as a world position
    /// puts the character at the map origin.
    ///
    /// What this therefore does: replace the recorded offset with the packet's
    /// (they agree to the millimetre unless a heartbeat was in flight, and the
    /// server's copy is the one it will compose with), keep the flag and the
    /// block, and mark the position as **owed** — the first real placement of
    /// the kept deck on the new map rebuilds it as `carry(offset, placement)`,
    /// absolutely. Until then the offset is frozen: the world position in the
    /// meantime is first the old map's coordinates and then whatever the far
    /// side's create block restates, and deriving an offset from either against
    /// the old placement is a several-thousand-yard error that goes out on the
    /// wire as `t_pos` — which the server, composing passengers from `t_pos`
    /// alone, obligingly teleports the character to. That was *"the ship
    /// teleported me far out into the middle of nowhere"*.
    ///
    /// A no-op when there is no ferry, which is every teleport but this one.
    pub fn transfer_aboard(&mut self, offset: Position) {
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

    /// **Forget the deck** — a far teleport, and nothing else.
    ///
    /// [`Self::carry_platform`] moves the character by however far the platform
    /// moved *since the offset was taken*, and after a teleport the two
    /// readings are not comparable: the recorded placement is where the boat
    /// stood on the map that was left, and the first reading on the new one is
    /// an ocean away. Carrying that difference throws the character across the
    /// world.
    ///
    /// Dropping it instead re-boards from wherever the server has just put
    /// them, on the next tick, through the same first-boarding arm a gangplank
    /// goes through — which moves nobody.
    ///
    /// **What it does not do is report a step ashore.** The server has already
    /// decided the question: `Player::TeleportTo` calls `RemovePassenger`
    /// itself unless the caller passed `TELE_TO_NOT_LEAVE_TRANSPORT`, and the
    /// one caller that passes it is `Transport::TeleportTransport`, which is
    /// keeping us aboard on purpose. So the record of what the server was last
    /// told goes with the ferry: a no-flag packet fired into that window would
    /// land us in the sea at exactly the moment the boat carried us across it.
    /// The re-boarding a tick later is still an edge and is still reported.
    pub fn disembark(&mut self) {
        if self.ferry.take().is_some() {
            self.info.flags &= !move_flags::ONTRANSPORT;
            self.info.transport = None;
        }
        self.sent_ferry = None;
        self.ferry_recompose = false;
    }

    /// Is the character in the air — jumping, knocked back, or falling?
    pub fn is_airborne(&self) -> bool {
        self.air.is_some()
    }

    /// …and did it *jump*, rather than walk off something? The two are one
    /// parabola to the server and two different animations to the renderer.
    pub fn is_jumping(&self) -> bool {
        self.air.is_some_and(|a| a.jumped)
    }

    /// Leave the ground under our own power.
    ///
    /// Returns the packet that owes the server, or `None` when there is nothing
    /// to jump from: **already airborne is the case that matters**, because
    /// `CHEAT_TYPE_MULTI_JUMP` allows exactly one `MSG_MOVE_JUMP` before a
    /// landing and rejects the rest outright (`MultiJump.Reject = 1`), so a held
    /// space bar would otherwise have every second jump silently discarded by
    /// the server while the client flew on.
    pub fn jump(&mut self) -> Option<MoveEvent> {
        // **A swimmer does not jump — it ascends.** `MSG_MOVE_JUMP` sent from
        // the water would be a parabola through a medium with no gravity in it;
        // the same key is `MOVEFLAG_PITCH_UP` there, which is a held control
        // with its own three opcodes. See [`Self::pitch_flags`].
        if self.air.is_some() || self.is_swimming() || !self.can_move() {
            return None;
        }
        // The horizontal velocity a jump carries is the one being run at, and
        // `CHEAT_TYPE_OVERSPEED_JUMP` compares `jump.xyspeed` against the
        // server's own speed for the *previous* packet's flags — so this is the
        // speed the flags already imply, never a number of our own.
        let speed = self.info.speed(&self.speeds);
        self.take_off(JUMP_SPEED, speed, self.info.heading(), true);
        Some(MoveEvent {
            opcode: Opcode::MSG_MOVE_JUMP,
            flags: self.info.flags,
        })
    }

    /// The server has thrown this character: `SMSG_MOVE_KNOCK_BACK`.
    ///
    /// The four numbers are echoed back verbatim in the ack
    /// (`FindPendingMovementKnockbackChange` compares each to 0.01), so they are
    /// adopted rather than recomputed from a heading. `z_speed` arrives
    /// **down-positive** — `Unit::KnockBack` sends `-verticalSpeed`, its own
    /// comment saying "notice the - sign" — so a knock *up* is a negative wire
    /// value, and the up-positive simulation negates it while the jump block
    /// keeps the wire's own number for the ack.
    pub fn knock_back(&mut self, cos_angle: f32, sin_angle: f32, xy_speed: f32, z_speed: f32) {
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

    /// **The server has taken the wheel**: an `SMSG_MONSTER_MOVE` naming this
    /// character. See [`Ride`] for what that is and what it costs to ignore.
    ///
    /// `facing_at` is where a [`SplineFacing::Target`] is looking, resolved by
    /// the caller because only the object manager can find another unit.
    ///
    /// **A stop packet is a ride too**, and that is not a special case bolted on:
    /// `ChargeMovementGenerator::Initialize` opens with `if (!unit.IsStopped())
    /// unit.StopMoving()`, which is a second `MoveSplineInit::Launch` with
    /// `SetStop()` — vmangos writes *"Will trigger CMSG_MOVE_SPLINE_DONE from
    /// client"* beside it. So a charge is two packets and both raise the pending
    /// flag; the second supersedes the first, which is why an ack owed for a
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
            // Nothing to walk — a stop, or a bare `SetFacingTo`. The server is
            // still holding us as spline-pending, so the ack is owed at once.
            self.ride = None;
            self.finished_ride = Some(mm.spline_id);
            self.stand_down(mm.facing, facing_at);
            return;
        }

        let flying = mm.flags & spline_flags::FLYING != 0;
        self.ride = Some(Ride {
            spline: Spline {
                path: mm.path.clone(),
                duration_ms: mm.duration_ms,
                elapsed_ms: 0,
                facing: mm.facing,
                // A cyclic spline never ends and would owe an ack that never
                // came; the server does not send a player one, and treating a
                // stray as an ordinary path at least terminates.
                cyclic: false,
                flying,
                // A ride's vertical is the server's own path; the local mover
                // owns the ground for the player and `grounded_z` is never
                // asked here. Carried so the flag is not silently dropped.
                falling: mm.flags & spline_flags::FALLING != 0,
                // A ride is our own character being driven, and the server does
                // not put a player on a transport spline: `MoveSplineInit` only
                // reaches `SMSG_MONSTER_MOVE_TRANSPORT` for a unit it is also
                // boarding, and a player boards itself. Carried as `None` rather
                // than dropped so the field cannot be forgotten if that changes.
                on_transport: mm.transport,
            },
            elapsed: 0.0,
            spline_id: mm.spline_id,
            facing_at,
        });
        // The arc and the keys are both over: the server's own `Launch` clears
        // `MOVEFLAG_MASK_MOVING` and sets `SPLINE_ENABLED | FORWARD` on its copy
        // of this block, so the two ends agree about what we are doing without
        // anything being sent. `FORWARD` is also what makes the renderer draw a
        // run rather than a character sliding along in its idle pose — the same
        // substitution [`crate::state::objects::Entity::move_flags`] makes for a
        // creature on a spline, for the same reason.
        self.air = None;
        self.info.fall_time = 0;
        self.info.jump = JumpInfo::default();
        self.info.flags &= !move_flags::MASK_MOVING;
        self.info.flags |= move_flags::SPLINE_ENABLED | move_flags::FORWARD;
        // **…and `MOVEFLAG_FLYING` for a flying ride**, which is the server's
        // own state for the whole of a taxi flight —
        // `FlightPathMovementGenerator::Finalize` is what removes it again, and
        // [`Self::stand_down`] is this end of that. It is what the renderer
        // chooses the *clip* from, and without it a gryphon at 32 y/s plays a
        // `Run` authored for 6.9.
        if flying {
            self.info.flags |= move_flags::FLYING;
        }
    }

    /// Is the server driving this character right now?
    pub fn is_riding(&self) -> bool {
        self.ride.is_some()
    }

    /// **Abandon a ride without acknowledging it.**
    ///
    /// A teleport, near or far. `HandleMoveSplineDoneOpcode` returns early while
    /// `IsBeingTeleported()`, so an ack sent across a relocation is thrown away
    /// and the ride's own path leads somewhere on a map we have left. The
    /// relocation *is* the hand-back.
    pub fn abandon_ride(&mut self) {
        if self.ride.take().is_some() {
            self.stand_down(SplineFacing::Travel, None);
        }
        self.finished_ride = None;
    }

    /// The spline id an ended ride owes the server, taken once.
    pub fn take_finished_ride(&mut self) -> Option<u32> {
        self.finished_ride.take()
    }

    /// Put the block back the way an unridden character's looks, and turn to
    /// whatever the packet asked for.
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
        // Nothing was sent while the ride lasted, and the ack carries this
        // orientation itself — so a `MSG_MOVE_SET_FACING` chasing it a tick
        // later would restate what the server has just been told.
        self.sent_orientation = self.info.position.orientation;
        self.info.flags &=
            !(move_flags::SPLINE_ENABLED | move_flags::MASK_MOVING | move_flags::FLYING);
        // The keys are still held: whatever they imply resumes from the
        // endpoint, exactly as an unroot puts back what was pressed through it.
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
        let (position, heading) = ride.spline.position_and_heading();
        let finished = ride.spline.finished();
        let facing = ride.spline.facing;
        let facing_at = ride.facing_at;
        let spline_id = ride.spline_id;

        self.info.position.x = position[0];
        self.info.position.y = position[1];
        self.info.position.z = position[2];
        self.info.position.orientation = wrap_angle(heading);
        // Re-asserted every step rather than only at the start: a root or a stun
        // landing mid-charge goes through `set_rooted`, which clears
        // `MOVEFLAG_MASK_MOVING` — and the character would finish the ride in
        // its idle pose. `ChargeMovementGenerator::Finalize` is where vmangos
        // applies a pending root, i.e. *after* the spline, so obeying it here
        // would be obeying it early.
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
    /// Rooting **clears the moving flags** because the server's own
    /// `ResolvePendingMovementChange` does (`RemoveUnitMovementFlag(
    /// MOVEFLAG_MASK_MOVING)`), and a packet carrying both is
    /// `CHEAT_TYPE_ROOT_MOVE` — five of which is a kick. Turning still works: a
    /// root stops translation, not the character.
    pub fn set_rooted(&mut self, rooted: bool) {
        if rooted {
            self.info.flags &= !move_flags::MASK_MOVING;
            self.info.flags |= move_flags::ROOT;
            self.air = None;
            self.info.fall_time = 0;
        } else {
            self.info.flags &= !move_flags::ROOT;
            // The keys are still held; put back whatever they imply, which is
            // also what makes the unroot's own packet legal — `CheckMoveStart`
            // exempts the ack opcodes. **Unless something else is still holding
            // the character**: a corpse whose root is lifted is still not
            // walking, and reporting `FORWARD` while `advance` refuses to
            // translate is the shape `CheckSpeedHack` measures.
            if self.can_move() {
                self.info.flags |= self.controls.to_flags();
            }
        }
    }

    /// Begin an arc. Shared by the jump, the knockback and the step off a ledge
    /// so that all three anchor and flag themselves identically — a fall that
    /// forgets `fall_time` is `CHEAT_TYPE_NO_FALL_TIME`, and one that forgets
    /// the flag is a teleport.
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
            // The wire is **down-positive**: the real 1.12 client reports a jump
            // as `zspeed = -7.9558`, and the server's own knockback packet sends
            // `-verticalSpeed` into the same field (`Unit::KnockBack`, "notice
            // the - sign"). `Airborne::up` stays up-positive; only the wire
            // flips.
            z_speed: -up,
            cos_angle: heading.cos(),
            sin_angle: heading.sin(),
            xy_speed,
        };
    }

    /// Is the character in water deep enough to swim in?
    pub fn is_swimming(&self) -> bool {
        self.info.has(move_flags::SWIMMING)
    }

    /// **The pitch bits the held keys imply, which are legal only in water.**
    ///
    /// `MOVEFLAG_PITCH_UP` (0x40) and `PITCH_DOWN` (0x80) are how 1.12 goes up
    /// and down, and the gate is the client's own rather than a caution here:
    /// the client's input tick reaches its ascend/descend handler only after
    /// testing `MOVEFLAG_SWIMMING` (0x200000) — otherwise there is nothing to
    /// do. That handler is two opposed input bits resolving to one of three
    /// commands — 11, 12 and 13 into the client's own movement-opcode table,
    /// which are `MSG_MOVE_START_PITCH_UP` (191), `START_PITCH_DOWN` (192) and
    /// `STOP_PITCH` (193).
    ///
    /// So this is **not** in [`Controls::to_flags`]: a character holding the
    /// same key on dry land is jumping, and putting `PITCH_UP` on the wire
    /// there would be a movement flag the reference never sends.
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

    /// Bring the pitch bits into line with the keys, returning the packet that
    /// owes the server.
    ///
    /// **One reconcile rather than a branch in each of the two places that can
    /// change the answer**, which is the point: the bits move when the *key* is
    /// pressed and when the *water* is entered or left, and the second is not a
    /// keypress. Asked once a tick from [`Self::advance`], so a swimmer who
    /// enters the water with the key already held starts rising 25 ms later
    /// rather than never.
    ///
    /// It cannot be folded into the swim transition itself: `CheckMoveFlags`
    /// calls any opcode other than `MSG_MOVE_START_PITCH_UP` that *introduces*
    /// `MOVEFLAG_PITCH_UP` a cheat, so the bit may not ride out on the
    /// `MSG_MOVE_START_SWIM` that made it legal.
    fn reconcile_pitch(&mut self) -> Option<Opcode> {
        let pitching = move_flags::PITCH_UP | move_flags::PITCH_DOWN;
        let wanted = self.pitch_flags();
        if self.info.flags & pitching == wanted {
            return None;
        }
        // **The first event, not the last.** A flip from up to down is a stop
        // and then a start, and `advance` reports one packet a tick — taking
        // the last would apply both flag changes while sending only one of the
        // two opcodes, which is `CheckMoveFlags` verbatim. The next tick
        // reconciles again and sends the other.
        let events = transitions(self.info.flags, (self.info.flags & !pitching) | wanted);
        let first = events.first()?;
        self.info.flags = first.flags;
        Some(first.opcode)
    }

    /// **Which way the body is pointed in the vertical**, radians, up-positive.
    ///
    /// Only meaningful while swimming — it is the one field the movement block
    /// carries under `MOVEFLAG_SWIMMING` and under nothing else — and it is the
    /// **camera's** pitch rather than a control of its own: looking down and
    /// swimming forward is how a diver goes down.
    ///
    /// It is the *aim* and not the whole of the vertical, which is the
    /// correction this doc comment carries: `Bindings.xml`'s `PITCHUP` and
    /// `PITCHDOWN` are a held control with a movement flag apiece, and they are
    /// [`Self::pitch_flags`]. An earlier version of this note said 1.12 had
    /// neither, which was a grep for the wrong word.
    ///
    /// Clamped to just inside the poles, because the serialised value goes on
    /// the wire and `MaNGOS::IsValidMapCoord` wants it finite; and it is stored
    /// whatever the state, so that the first packet after entering the water
    /// carries the pitch the player was already looking at rather than zero.
    pub fn set_pitch(&mut self, pitch: f32) {
        use std::f32::consts::FRAC_PI_2;
        self.info.pitch = pitch.clamp(-FRAC_PI_2 + 0.01, FRAC_PI_2 - 0.01);
    }

    /// The water half of one step: enter it, swim in it, or leave it.
    ///
    /// Returns the packet the transition owes the server, of which there are
    /// exactly two and each has an anticheat rule attached:
    /// `MSG_MOVE_START_SWIM` **must** carry `MOVEFLAG_SWIMMING` and no other
    /// opcode may introduce it, and `MSG_MOVE_STOP_SWIM` must **not** carry it
    /// (`CheckMoveFlags`, and `CHEAT_TYPE_FLY_HACK_SWIM` for the second) — so
    /// the flag is written before the return in both directions.
    ///
    /// The thresholds are [`MIN_SWIM_DEPTH`] and its hysteresis, both out of
    /// the client's water tick; what is *this* function's, and is stated in the two places it
    /// shows, is what happens to the altitude in between.
    fn tick_water(&mut self, surface: Option<f32>, floor: Option<f32>, dt: f32) -> Option<Opcode> {
        let depth = surface.map(|s| s - self.info.position.z);
        if !self.info.has(move_flags::SWIMMING) {
            // Not swimming, and the water is not deep enough to start.
            if !depth.is_some_and(|d| d > MIN_SWIM_DEPTH) {
                return None;
            }
            let line = surface.unwrap_or_default() - MIN_SWIM_DEPTH;
            // **The character is placed on the water line**, which is the same
            // thing [`Self::touch_down`] does with the floor and for the same
            // reason: the transition is defined at a depth, so a tick that
            // crosses it lands the character where the crossing was rather than
            // wherever the 25 ms boundary happened to fall. Walking in off a
            // beach is continuous under it — the crossing depth and the ground
            // agree at the water's edge — and falling in from a height is a
            // step of at most one tick's fall.
            //
            // **The reference sinks and bobs back up and this does not**, which
            // is the one visible difference: what rate it rises at is not
            // established and a guessed one would be a number in this file with
            // nothing behind it.
            self.info.position.z = line.max(floor.unwrap_or(f32::MIN));
            self.info.flags |= move_flags::SWIMMING;
            // **Everything about the arc goes, and dropping the flag here is
            // legal precisely because of the opcode this returns.**
            // `CheckFallStop` calls a packet that clears `JUMPING`/`FALLINGFAR`
            // under anything but a *fall-end* opcode a cheat — and
            // `IsFallEndOpcode` is `MSG_MOVE_FALL_LAND` **or**
            // `MSG_MOVE_START_SWIM`. The server has an explicit name for "the
            // arc ended in the water"; this is it.
            self.air = None;
            self.info.flags &= !(move_flags::JUMPING | move_flags::FALLINGFAR);
            self.info.fall_time = 0;
            self.info.jump = JumpInfo::default();
            return Some(Opcode::MSG_MOVE_START_SWIM);
        }

        // Swimming. Out of the water entirely, or shallower than the dead band
        // under the entry depth.
        if !depth.is_some_and(|d| d >= MIN_SWIM_DEPTH - SWIM_EXIT_HYSTERESIS) {
            // **The pitch bits leave with it**, on the same packet. Dropping a
            // movement flag is never what `CheckMoveFlags` objects to — only
            // introducing one under the wrong opcode is — and the alternative
            // is a `MOVEFLAG_PITCH_UP` standing on dry land for a tick, which
            // the reference cannot produce at all: its own handler is reachable
            // only while `MOVEFLAG_SWIMMING` is set.
            self.info.flags &=
                !(move_flags::SWIMMING | move_flags::PITCH_UP | move_flags::PITCH_DOWN);
            return Some(Opcode::MSG_MOVE_STOP_SWIM);
        }

        // **No gravity, and no buoyancy either**: the altitude moves only by
        // the swimmer's own stroke along their own pitch. A diver who stops
        // stops where they are, which is what the water feels like to play in.
        let mut z = self.info.position.z + climbed(&self.info, &self.speeds, dt);
        // The surface is a ceiling — rising through the water line is what
        // *stops* the swimming, so it cannot also be a place to be — and the
        // floor is still solid, since a swimmer can rest on the lake bed.
        //
        // **The floor is applied second and that order is load-bearing.** Where
        // a beach shelves up past the water line the two clamps disagree, and
        // the ceiling winning means the ground never lifts the swimmer out: they
        // float below the sand, at a depth that stays exactly on the entry
        // threshold, and swim in the shallows for ever. Ground beats water.
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
    /// when supplied the character follows the landscape instead of gliding at a
    /// fixed z, and stops at walls instead of walking through them.
    ///
    /// **The three transitions are reported rather than left to the next
    /// heartbeat**, and each has a rule behind it: the server anchors the arc at
    /// the first packet carrying `MOVEFLAG_JUMPING`, so a late take-off report
    /// is a parabola drawn from the wrong place; and a packet that *drops* the
    /// flag under any opcode but `MSG_MOVE_FALL_LAND` is
    /// `CHEAT_TYPE_BAD_FALL_STOP`.
    pub fn advance(&mut self, dt: f32, world: Option<&dyn Footing>) -> Option<Opcode> {
        if dt <= 0.0 {
            return None;
        }

        // **A ride outranks everything below it**, and it owes no packet: the
        // server is walking its own copy of this spline and discards anything we
        // send until the ride is acknowledged. What it owes instead is
        // [`Self::take_finished_ride`], which the loop drains.
        if self.ride_step(dt) {
            return None;
        }

        // Turning is local only, so it never enters the flags the server
        // extrapolates from. **A rooted character still turns; a stunned or
        // dead one does not** — the two questions are asked separately and
        // answered off two different fields. See [`Restraint`].
        if self.can_turn() {
            self.info.position.orientation =
                turned(self.info.position.orientation, &self.speeds, self.controls, dt);
        }

        if !self.can_move() {
            return None;
        }

        // --- horizontal ------------------------------------------------------
        // In the air the velocity is the one frozen at take-off; on the ground
        // it is whatever the keys imply. Either way the stride is *proposed* and
        // the world decides where it ends, so a wall shortens it before the
        // height is looked up — asking the two in the other order stands the
        // character on the floor *inside* the building they were stopped from
        // entering.
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
        // **What this character stands on**, which is not always the floor — see
        // [`standing_surface`], and note that it is what the *rest* of this
        // function uses in place of `floor` from here down.
        let footing = standing_surface(self.info.flags, floor, surface);

        // **The water is asked before the air, and it outranks it.** A fall
        // that ends in a lake does not land — `IsFallEndOpcode` is
        // `MSG_MOVE_FALL_LAND` *or* `MSG_MOVE_START_SWIM`, so the server counts
        // the swim start as the end of the arc and a `FALL_LAND` sent as well
        // would be a second one. Everything about being airborne is therefore
        // cleared here rather than through [`Self::touch_down`].
        //
        // **…and a water-walker has no water to be in**, which is the whole of
        // what the flag is for. It is spelled as "there is no liquid here"
        // rather than as an early return so that a character who gains the aura
        // while already swimming leaves the water through the existing exit arm
        // — hysteresis, `MSG_MOVE_STOP_SWIM` and all — instead of silently
        // keeping `MOVEFLAG_SWIMMING` on a surface they are now standing on.
        let swimmable = match self.info.has(move_flags::WATERWALKING) {
            true => None,
            false => surface,
        };
        if let Some(event) = self.tick_water(swimmable, floor, dt) {
            return Some(event);
        }
        if self.info.has(move_flags::SWIMMING) {
            // …and the ascend key, which is a movement flag of its own and is
            // legal only here. See [`Self::reconcile_pitch`] for why it is a
            // reconcile rather than part of the keypress.
            return self.reconcile_pitch();
        }

        if let Some(air) = self.air.as_mut() {
            // **No data for this spot holds the arc, exactly as it holds a
            // walking character's altitude.** The two used to be opposite: a
            // walking character over a point the client has no floor for keeps
            // the height the server gave it, and an airborne one carried on
            // down for ever, because the `None` arm below simply wrote the
            // parabola. That is the whole of *"you fall through the floor"* —
            // the answer is `None` at any point inside a building's own `MODF`
            // box that its hull does not cover (a doorway, a stair with no
            // `MOPY`, a group that has not been read yet), so a character who
            // leaves the ground anywhere in Ironforge falls until vmangos'
            // `UndermapRecall` catches them.
            //
            // Held rather than landed: there is nothing to land *on*. The arc
            // does not advance either, so it resumes where it was rather than
            // teleporting a second's worth of fall the moment the geometry
            // arrives. See `Footing::floor`, whose `None` is *no data* and
            // never *no floor*.
            //
            // **"No world at all" is a different answer from "no data here"**,
            // and only the second holds. A caller with no [`Footing`] — the
            // CLI's snapshot, a unit test of the arc itself — is saying there
            // is nothing to stand on anywhere, and an arc that froze under it
            // would never come down.
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
            // **Whether the arc has turned over, asked of the arc rather than
            // of the position.** They used to be the same number and are not
            // once the clamp below fires: a character riding up a hillside sits
            // *above* the parabola, so comparing against where they are reads
            // "descending" on the way up and lands the jump on its first tick.
            // Two samples of the same expression a step apart cannot say
            // anything but what the arc is doing.
            let previous = start_z - fall_elevation(elapsed - dt, safe_fall, -up);
            let descending = z < previous;
            // Only a *descending* arc lands, or the first upward frame of a
            // jump would land again the moment it left the ground.
            let Some(h) = footing else {
                self.info.position.z = z;
                return None;
            };
            if descending && z <= h {
                self.touch_down(h);
                return Some(Opcode::MSG_MOVE_FALL_LAND);
            }
            // **An arc is never walked below the ground it is over.** Nothing
            // stopped it before, and a jump taken while running uphill goes
            // *into* the hill: the parabola rises at 7.96 y/s and decelerating
            // while a 45-degree slope under a run rises at 7.6 y/s and does
            // not, so the two cross within a tick or two of the apex and the
            // character — and the camera framed on them — spends the rest of
            // the ascent inside the hillside. Measured on a synthetic slope:
            // 0.15 yards under at 31 degrees, 1.34 at 45, 2.82 at 56.
            //
            // Clamping is **not** landing, which is the distinction the arm
            // above keeps: the character rides the surface with the jump still
            // in flight and touches down when the arc really comes back through
            // it. `floor`'s own ceiling is the feet plus [`STEP_UP`], so this
            // can lift by at most a step in a tick and cannot suck anybody onto
            // a roof.
            self.info.position.z = z.max(h);
            return None;
        }

        let Some(h) = footing else {
            // No data for this spot: hold the altitude the server last gave us
            // rather than inventing one. This is the moments after a teleport,
            // before the tile arrives.
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
        // A step, a slope, or a stair. Snapped rather than rate-limited: at 25 ms
        // a real slope is millimetres, so anything large here is the ground
        // arriving late, and the least-wrong answer to that is to stand on it.
        self.info.position.z = h;
        None
    }
}

/// What the local simulation needs to know about the world it is walking on.
///
/// A trait rather than the height closure it replaced, because collision asks
/// two questions and they are not separable: a wall shortens a stride, and the
/// floor is looked up *at the end of the stride that actually happened*. Two
/// closures could be supplied independently and one of them forgotten, which is
/// a character standing inside the building they were stopped from entering.
///
/// The protocol crate deliberately knows nothing about ADT or WMO files, so the
/// implementation lives with the caller — [`crate::socket::session`] binds the map id,
/// and `vale_assets` supplies the geometry.
pub trait Footing {
    /// The surface to stand on at `(x, y)` for a character whose feet are at
    /// `z`, or `None` where the client has no data for that spot.
    ///
    /// `z` is a parameter because "the ground" stopped being a single number
    /// the moment buildings became solid: over Stormwind's canals there are
    /// four surfaces above one point, and which of them is the floor depends
    /// entirely on where the character already is.
    fn floor(&self, x: f32, y: f32, z: f32) -> Option<f32>;

    /// Where a stride from `from` to `to` actually ends. Only the horizontal
    /// part is decided here; the vertical is [`Self::floor`]'s.
    ///
    /// Defaulted to "wherever it was going", so a caller with terrain and no
    /// buildings — the CLI, and the renderer before its first tile arrives —
    /// gets the behaviour it had before collision existed.
    fn step(&self, _from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
        to
    }

    /// **The height of the liquid surface over `(x, y)`, if there is one.**
    ///
    /// Deliberately *not* a function of `z`, where [`Self::floor`] is: a chunk
    /// carries one surface per liquid and the client's own query asks with a
    /// mask of all four kinds and takes what it finds (mask `0x0f`),
    /// so there is no "which of the surfaces above me" question to ask. What a
    /// depth means is decided by the caller, which is [`Mover::advance`].
    ///
    /// Defaulted to `None` — *no water here* — so every existing caller keeps
    /// the behaviour it had, which is walking on the lake bed.
    fn liquid(&self, _x: f32, _y: f32) -> Option<f32> {
        None
    }

    /// **Where a named moving platform is right now.**
    ///
    /// Asked by guid rather than by position, unlike everything else here,
    /// because the caller already knows which platform it wants: a unit whose
    /// spline arrived on `SMSG_MONSTER_MOVE_TRANSPORT` walks a path in that
    /// transport's own frame, and turning it into world coordinates needs the
    /// transport's placement and nothing else. See [`Spline::on_transport`].
    ///
    /// Defaulted to `None`, which reads as *the platform is not loaded* — and a
    /// passenger of a platform this client is not holding is left where it was
    /// rather than placed at the map's origin.
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

/// Which way you are looking if you walk from `a` to `b`. Zero is +X (north),
/// matching the server's orientation convention.
pub(crate) fn bearing(a: [f32; 3], b: [f32; 3]) -> f32 {
    (b[1] - a[1]).atan2(b[0] - a[0])
}

/// The signed angle from `a` to `b`, in `(-pi, pi]`.
///
/// A unit turning through north crosses the 2pi seam, and taking the raw
/// difference spins it the long way round once per revolution. Anything that
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

/// Does this opcode arrive from the server as `packedGuid` + a
/// [`MovementInfo`], describing *another* player's movement?
///
/// When a client sends a `MSG_MOVE_*` packet, `HandleMovementOpcodes` rebuilds
/// it as `m_clientMoverGuid.WriteAsPacked()` followed by `movementInfo.Write`
/// and calls `SendMovementMessageToSet(..., _player)` — i.e. it echoes the same
/// opcode to everyone nearby *except* the sender. That broadcast is the only way
/// another player's position ever changes: `SMSG_MONSTER_MOVE` covers
/// server-driven units, and a values update carries no position at all. Without
/// handling these, every other player stands frozen where they were created.
///
/// The list is exactly the opcodes routed to `HandleMovementOpcodes` in
/// `Protocol/Opcodes.cpp`, minus `CMSG_MOVE_FALL_RESET`, which is never
/// broadcast. Anything else that merely *starts* with `MSG_MOVE_` has a
/// different body — `MSG_MOVE_TIME_SKIPPED` is a guid and a `u32`, and reading
/// it as a movement block yields a plausible, silently wrong position.
pub fn is_broadcast_movement(opcode: Opcode) -> bool {
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
            // The observer half of the flag changes above. These do *not* go
            // through `HandleMovementOpcodes` — `SendMovementFlagChangeToObservers`
            // writes them directly — but the body is the same packed guid and
            // movement block, and they are the only statement that another
            // player has been rooted, thrown or given water-walking. Without
            // them a rooted player nearby goes on being dead-reckoned along the
            // flags they were last seen with, walking on the spot.
            | MSG_MOVE_ROOT
            | MSG_MOVE_UNROOT
            | MSG_MOVE_WATER_WALK
            | MSG_MOVE_HOVER
            | MSG_MOVE_FEATHER_FALL
            // …and the knockback, whose four trailing floats are already in the
            // movement block's own jump section. The tail is ignored.
            | MSG_MOVE_KNOCK_BACK
            // …and **somebody else being moved without walking there**, which
            // is the same body again and is sent for a creature as well as for
            // a player: `MovementPacketSender::SendTeleportToObservers`, called
            // from `Player::ExecuteTeleportNear` and `Unit::NearTeleportTo`, so
            // it covers a Blink, a Sheep's wander re-anchor, a same-map `.tele`
            // and a creature being put back on its spawn. `STATUS_NEVER` on the
            // server's own table: this direction only.
            //
            // **Twice per teleport, around the old position and the new**, both
            // carrying the *destination*. The second is the one that lands for a
            // watcher near either end; the first exists so that a watcher who is
            // about to lose sight of the unit gets told before the relocation
            // takes it out of their cell. Applying both is idempotent.
            //
            // Without it the unit stands at its old position until its next
            // heartbeat — half a second for a player who is moving, and
            // *indefinitely* for one who is not, since a standing player sends
            // nothing at all.
            | MSG_MOVE_TELEPORT
    )
}

/// Parse one of the [`is_broadcast_movement`] packets: whose movement, and what.
pub fn parse_movement_broadcast(body: &[u8]) -> Option<(u64, MovementInfo)> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    let info = MovementInfo::read(&mut r)?;
    Some((guid, info))
}

/// Where a unit looks when it finishes a spline.
///
/// `Mask_Final_Facing` in `MoveSplineFlag.h`, and the reason a creature that has
/// just walked up to you turns to look at you. [`SplineFacing::Travel`] is the
/// default and the common case: face the way you are going.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SplineFacing {
    #[default]
    Travel,
    /// `Final_Angle` — an absolute orientation in radians.
    Angle(f32),
    /// `Final_Point` — look at a place.
    Spot([f32; 3]),
    /// `Final_Target` — look at a unit, by GUID. The combat case: a creature
    /// chasing you is sent with its victim here.
    Target(u64),
}

/// `SMSG_MONSTER_MOVE` — the server moving a unit along a spline.
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
/// **The path is two encodings, chosen by a flag, and they disagree about where
/// the destination is.** `WriteLinearPath` writes the destination first and then
/// the earlier nodes as *packed offsets from it*; `WriteCatmullRomPath` writes
/// every node absolutely, in order, so the first vector after the count is node
/// zero and the destination is the last. Reading the second as the first puts a
/// flying creature's destination at its first waypoint — plausible coordinates,
/// wrong place. `Mask_CatmullRom` is `Flying` (0x200) and nothing else in 1.12.
#[derive(Debug, Clone, PartialEq)]
pub struct MonsterMove {
    pub guid: u64,
    pub start: [f32; 3],
    pub spline_id: u32,
    /// The waypoints, in order, ending at the destination. Empty for a stop
    /// packet — the unit halts at `start`.
    ///
    /// Reading these rather than just the endpoints is what stops a creature on
    /// a patrol route cutting the corner: the server walks the path, and a client
    /// that interpolates start-to-destination sends it straight through whatever
    /// the path was going around.
    pub path: Vec<[f32; 3]>,
    pub duration_ms: u32,
    pub facing: SplineFacing,
    pub flags: u32,
    /// **The transport this whole path is expressed in the frame of** —
    /// `SMSG_MONSTER_MOVE_TRANSPORT`, and `None` for the ordinary opcode.
    ///
    /// The two packets are the same body with one extra packed guid inserted
    /// after the unit's: `MoveSplineInit::Launch` builds `SMSG_MONSTER_MOVE`,
    /// then, if the unit is boarding a transport, changes the opcode and writes
    /// the transport's guid before everything else. So a reader that treats the
    /// second as the first parses it happily and takes the transport's guid as
    /// the unit's start coordinates.
    ///
    /// **And every coordinate after it is local**, which is the half that
    /// cannot be papered over: a boat's deckhand walking three yards is a
    /// three-yard path about the boat's origin, and dropping it into the world
    /// unconverted puts them three yards from the map's origin, in the sea off
    /// Westfall.
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
    pub const STOP: u8 = 1;
    pub const FACING_SPOT: u8 = 2;
    pub const FACING_TARGET: u8 = 3;
    pub const FACING_ANGLE: u8 = 4;
}

/// `MoveSplineFlag`, the subset this client reads. `Mask_CatmullRom = Flying`.
pub mod spline_flags {
    /// `MoveSplineFlag::Falling` — *"Affects elevation computation"*. The z along
    /// such a spline is an arc rather than the path's own height, so it is the
    /// one server-driven move whose vertical must not be touched. Nothing here
    /// computes the arc yet; what the flag is read for is to keep
    /// [`super::Spline::grounded_z`] off it.
    pub const FALLING: u32 = 0x0000_0002;
    pub const FLYING: u32 = 0x0000_0200;
    pub const CYCLIC: u32 = 0x0010_0000;
    pub const CATMULLROM: u32 = FLYING;
}

/// One `appendPackXYZ`: a `u32` holding three signed fixed-point offsets in
/// quarter-yards — 11 bits for x and y, 10 for z (`ByteBuffer.h:469`).
///
/// The sign has to be recovered by hand: the server masks each field to its
/// width, so a negative offset arrives as a positive number with the width's top
/// bit set. Reading them unsigned puts every node up to 512 yards away.
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

pub fn parse_monster_move(body: &[u8]) -> Option<MonsterMove> {
    parse_monster_move_inner(body, false)
}

/// `SMSG_MONSTER_MOVE_TRANSPORT` — the same body with the transport's packed
/// guid inserted after the unit's, and every coordinate after it in that
/// transport's own frame. See [`MonsterMove::transport`].
pub fn parse_monster_move_transport(body: &[u8]) -> Option<MonsterMove> {
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
        // additionally leads with a duplicate of its first point, which costs a
        // zero-length first segment and nothing else.
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
        // Destination first, then the earlier nodes as packed offsets *from the
        // destination*, in path order. `WriteLinearPath` writes no offsets at all
        // for a two-point move even though it still counts 2, so the offsets are
        // `count - 1` and only when the count exceeds two.
        let destination = [r.f32(), r.f32(), r.f32()];
        let mut path = Vec::with_capacity(node_count.min(256));
        if node_count > 2 {
            for _ in 0..node_count - 1 {
                if !r.has(4) {
                    // A truncated tail costs the intermediate nodes, not the
                    // move: straight to the destination is what this client did
                    // for every path until now.
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

    // **The path begins at `start`, and neither encoding writes it.**
    //
    // Both writers skip the spline's own first control point. `WriteLinearPath`
    // takes `real_path = &spline.getPoint(1)` and emits offsets for
    // `real_path[0..last_idx)`, which is `controls[1..count-1)`;
    // `WriteCatmullRomPath` appends from `getPoint(2)`, which for a
    // catmull-rom spline is also `controls[1]`. In both cases `controls[0]` is
    // carried by the header instead — `WriteCommonMonsterMovePart` opens with
    // `spline.getPoint(spline.first())`, and `first()` is `index_lo`, which is 0
    // for a linear spline and 1 for a catmull-rom one, i.e. `controls[0]` either
    // way.
    //
    // Dropping it does not lose a *point*, it loses the whole **first leg**. A
    // creature on a four-node patrol was placed on its second waypoint at t = 0
    // and then walked a path shorter than the duration accounted for — so it
    // jumped a leg forward the instant the packet arrived, ran the rest too
    // slowly, and got snapped back by the next packet. That is the "mobs
    // teleport around and move strangely" report, and it is invisible to every
    // check that only reads coordinates: every point in the path is a real
    // waypoint the server named, just not all of them.
    //
    // Checked against the point rather than against the flags, because the third
    // case writes it and the other two do not: `WriteCatmullRomCyclicPath` leads
    // with `getPoint(1)` — `controls[0]` — as the fake vertex the `Enter_Cycle`
    // flag says to erase after the first lap. Comparing makes one rule cover all
    // three, and the epsilon is far under the quarter-yard the packed offsets
    // are quantised to.
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
/// without this every creature in the world is frozen where it was created.
#[derive(Debug, Clone, PartialEq)]
pub struct Spline {
    /// Every waypoint, starting where the unit was when the server spoke and
    /// ending at the destination. Always at least two points.
    pub path: Vec<[f32; 3]>,
    /// How long **one lap** takes. For a cyclic spline that is not how long the
    /// unit walks for: it walks until the server says otherwise.
    pub duration_ms: u32,
    pub elapsed_ms: u32,
    /// Where the unit looks when it arrives.
    pub facing: SplineFacing,
    /// `MoveSplineFlag::Cyclic` — this path repeats, and the server will never
    /// mention it again.
    ///
    /// `MoveSpline::updateState` wraps `time_passed % Duration()` and carries on
    /// forever, so the duration in the packet is one lap and **the client is
    /// what loops**. A client that treats the end of the lap as the end of the
    /// move parks the unit there, which is a patrolling guard who walks his beat
    /// once and then stands still for the rest of the session — and then
    /// *teleports* to wherever the server really has him the next time anything
    /// makes the server state his position. Neither half reads as a missing
    /// flag; the first reads as a lazy NPC and the second as a network fault.
    pub cyclic: bool,
    /// **`MoveSplineFlag::Flying` — one bit with two consequences**, and the
    /// flag's own comment in vmangos says both: *"Smooth movement(Catmullrom
    /// interpolation mode), flying animation"*.
    ///
    /// So this decides how the path is *walked* — see
    /// [`Self::position_and_heading`] and [`curve`] — and, one crate up, which
    /// clip the unit plays while it is on it. A taxi flight is the loud case
    /// for both: linear, it snaps direction at every waypoint, and without the
    /// animation half the gryphon plays `Run` scaled to 32 y/s, which is
    /// `Run`'s authored 6.9 played four and a half times too fast.
    pub flying: bool,
    /// **`MoveSplineFlag::Falling`** — the z along this path is an arc the
    /// client is expected to compute, not the height the nodes state. Read for
    /// one thing only: [`Self::grounded_z`] refuses to touch such a spline, so
    /// a knocked-back unit is not walked along the ground it is sailing over.
    pub falling: bool,
    /// **The transport every waypoint above is expressed in the frame of** —
    /// `SMSG_MONSTER_MOVE_TRANSPORT`, and `None` for every other spline in the
    /// game.
    ///
    /// A deckhand walking three yards across a boat is a three-yard path about
    /// the *boat's* origin. Kept local rather than converted at arrival,
    /// because the boat moves: a path converted once is right for one instant
    /// and drifts for the rest of the leg, which for a zeppelin is the length
    /// of a continent. See [`Self::in_world`].
    pub on_transport: Option<u64>,
}

/// How far ahead [`Spline::velocity`] looks to measure itself.
///
/// Twenty milliseconds: under a simulation tick, so the answer is about the leg
/// being walked rather than about the next one, and far enough above the `f32`
/// noise of two world positions at Azeroth's far corners — where a step is a
/// millimetre — that the difference is a speed and not a subtraction of two
/// nearly equal large numbers.
const VELOCITY_PROBE_MS: u32 = 20;

impl Spline {
    /// Fraction complete, 0..=1.
    pub fn progress(&self) -> f32 {
        self.progress_at(self.elapsed_ms)
    }

    /// The same at some other moment on this spline's own clock.
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
    /// Returns whether a lap was completed, which is the moment the leading
    /// duplicate point has to go — `WriteCatmullRomCyclicPath` writes
    /// `spline.getPoint(1)` and then the whole path *starting at the same
    /// point*, and the packet's `Enter_Cycle` flag is the server saying so
    /// ("erases first spline vertex after first cycle done"). The duplicate is
    /// only ever a zero-length first leg, so it costs nothing until the lap
    /// wraps, at which point leaving it in stalls the unit for one frame at the
    /// top of every lap.
    pub(crate) fn wrap(&mut self) -> bool {
        if !self.cyclic || self.duration_ms == 0 || self.elapsed_ms < self.duration_ms {
            return false;
        }
        self.elapsed_ms %= self.duration_ms;
        // Checked against the points rather than against the flag alone: only
        // the catmull-rom writer emits the duplicate, and dropping a real
        // waypoint would shorten the patrol by a leg every lap.
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

    /// Total path length in yards — the distance actually walked, which on a
    /// multi-node path is longer than the straight line between its ends, and
    /// on a [`Self::flying`] one is longer again because the curve bulges
    /// outside its own polyline.
    pub fn length(&self) -> f32 {
        self.segment_lengths().iter().sum()
    }

    /// Yards per second along the path.
    ///
    /// The server states a path and a duration rather than a speed, and walks
    /// the spline at a constant rate, so those two *are* the speed. It is not
    /// necessarily the unit's run speed: a creature on a long patrol leg is sent
    /// at its walk speed, which is exactly the distinction the renderer wants
    /// when choosing between Walk and Run.
    pub fn speed(&self) -> f32 {
        if self.duration_ms == 0 {
            return 0.0;
        }
        self.length() * 1000.0 / self.duration_ms as f32
    }

    /// Where the unit is now, and which way it is heading, in radians.
    ///
    /// A spline is walked at a constant *speed*, so the parameter is distance
    /// along the path rather than a fraction of each segment — otherwise a long
    /// leg and a short one would take the same time and the unit would visibly
    /// speed up and slow down. That is `Spline::computeIndex`: `t × length`,
    /// the segment that brackets it, and the local parameter inside it.
    ///
    /// **The heading is the path's own derivative**, which is
    /// `MoveSpline::ComputePosition`'s `atan2(hermite.y, hermite.x)` and not a
    /// choice here. For a straight segment the derivative *is* the segment
    /// vector, so a linear spline reads exactly as it did before this; for a
    /// [`Self::flying`] one it is the curve's tangent, which is continuous —
    /// so the turn smooths itself and there is no second mechanism for it.
    pub fn position_and_heading(&self) -> ([f32; 3], f32) {
        self.sample(self.elapsed_ms)
    }

    /// **…and the same answer in world coordinates**, for a spline whose path is
    /// in a transport's own frame.
    ///
    /// The identity for the great majority of splines, which name no transport
    /// and are already in the world. For the rest it is
    /// `GenericTransport::CalculatePassengerPosition` applied to the sample —
    /// the same yaw and translation a passenger's own offset goes through, and
    /// necessarily so: a deckhand and the player standing beside them are the
    /// same kind of passenger and must not be placed by two different rules.
    ///
    /// **A transport this client is not holding answers `None`**, and the
    /// caller leaves the unit where it was. The alternative — taking the local
    /// path as world coordinates — puts a boat's crew a few yards from the map's
    /// origin, which is in the sea off Westfall for every unit on every boat at
    /// once.
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

    /// **How fast, and in which of the three directions**, this instant —
    /// yards per second.
    ///
    /// The heading `position_and_heading` returns is a *bearing*: it discards
    /// the vertical exactly as `MoveSpline::ComputePosition` does, which is
    /// right for pointing a body and useless for continuing one. A taxi flight
    /// climbs and dives for most of its length, so a reader that walks the path
    /// from a bearing and a speed holds its altitude between readings and then
    /// steps down to each new one — a sawtooth at the simulation's own 40 Hz,
    /// on the one axis nothing else in the frame is moving along.
    ///
    /// A forward difference over [`VELOCITY_PROBE_MS`] of this spline's own
    /// evaluator rather than an analytic derivative, so a linear path and a
    /// Catmull-Rom one are differentiated by the same code that positions them
    /// and cannot disagree with it. Past the end it answers zero, because
    /// `sample` parks on the destination — which is the honest answer for a
    /// ride that has arrived.
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

    /// Where the unit stands at some moment on this spline's own clock, and
    /// which way it is heading there.
    fn sample(&self, elapsed_ms: u32) -> ([f32; 3], f32) {
        match self.locate(elapsed_ms) {
            Some((index, t)) => self.at(index, t),
            None => (self.from(), bearing(self.from(), self.to())),
        }
    }

    /// **Which leg of the path is being walked, and how far along it** — the
    /// half of [`Self::sample`] that is about the *path* rather than about the
    /// point, split out because [`Self::grounded_z`] needs the leg's two
    /// endpoints and re-deriving them from a position would be a second
    /// implementation of the same search.
    ///
    /// `None` for a spline with nothing to walk: fewer than two points, or a
    /// path whose every segment is zero-length.
    fn locate(&self, elapsed_ms: u32) -> Option<(usize, f32)> {
        if self.path.len() < 2 {
            return None;
        }
        let lengths = self.segment_lengths();
        let total: f32 = lengths.iter().sum();
        if total <= 0.0 {
            return None;
        }
        let mut want = total * self.progress_at(elapsed_ms);
        for (index, leg) in lengths.iter().enumerate() {
            if *leg <= 0.0 {
                continue;
            }
            if want <= *leg {
                return Some((index, want / leg));
            }
            want -= leg;
        }
        // Past the end: park on the destination, still facing the last leg.
        Some((self.path.len() - 2, 1.0))
    }

    /// **The height to draw a unit at part-way along a leg, put back on the
    /// ground the leg crosses.**
    ///
    /// A linear spline is a straight line in *three* dimensions between two
    /// points the server states, and the ground between them is not straight.
    /// Over real 1.12 terrain that is not a small error: sampled at every one
    /// of the 31,152 ground creature spawns the world database names on maps 0
    /// and 1, a ten-yard leg's straight line leaves the ground under it by more
    /// than half a yard **17% of the time** and by more than a yard 8%; a
    /// twenty-yard leg — a chase — does so **50%** and **28%**. Just over half
    /// of those are the line *above* the ground, which is a creature visibly
    /// floating, and the rest are its feet in the hill.
    ///
    /// **What is corrected is only the part the client invented.** The answer
    /// is not "the ground here": it is the ground here plus the height the
    /// *server's own two endpoints* stand above the ground under them,
    /// interpolated along the leg. So at `t = 0` and `t = 1` this reproduces
    /// the node's stated z exactly — the client can never disagree with the
    /// server about a position the server actually named — and in between it
    /// carries whatever lift the server asked for across the terrain rather
    /// than across a chord. That is what makes it safe for the cases a plain
    /// snap-to-ground would ruin:
    ///
    /// * a creature swimming across a lake has both endpoints the same distance
    ///   above the lake bed, so it keeps swimming instead of walking the bottom;
    /// * one crossing a bridge has both endpoints thirty yards up, so it stays
    ///   on the bridge rather than being dropped into the canyon;
    /// * one walking out of a cave mouth is carried between the two smoothly.
    ///
    /// **`None` means "no opinion", and every branch that cannot answer takes
    /// it** — a flying or falling spline (whose vertical is the curve's or an
    /// arc's and not the ground's), a path with no leg, and any of the three
    /// lookups the client has no data for. The caller keeps the chord, which is
    /// what this client did before.
    ///
    /// This is a **reconstruction and not a measurement**: what the reference
    /// does with an ungrounded spline is not known. What is
    /// measured is the deviation above, and that it is invisible at every
    /// position the server states.
    ///
    /// ## What it costs, and the cache that is deliberately not here
    ///
    /// Three [`Footing::floor`] calls per server-driven unit per simulation
    /// step, under the world lock. The terrain half of one is **0.32 µs**
    /// measured over 626,000 warmed lookups, so sixty creatures on splines is
    /// 0.058 ms of a 25 ms tick; the buildings half is the same bounded `MODF`
    /// walk the dead-reckoning path above already makes per moving player.
    ///
    /// The two *node* lookups only change when the leg does and could be cached
    /// on the spline — and are not, because a cyclic path drops its leading
    /// point at the top of every lap ([`Self::wrap`]) and every leg index
    /// shifts under the cache with nothing failing. That is a stale-cache bug
    /// that would show as one creature in a hundred drifting, which is the
    /// worst kind. If the cost ever shows up it will show up as
    /// `SessionStatus::stalls` moving, and that is the number to look at first.
    pub fn grounded_z(&self, world: &dyn Footing, drawn: [f32; 3]) -> Option<f32> {
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

    /// How long each segment is — the chord for a linear spline, the sampled
    /// curve for a flying one.
    ///
    /// **A curve is longer than its polyline**, so this is not the same number
    /// as [`Self::length`]'s sum of chords and the difference is what keeps the
    /// unit at the server's own position rather than ahead of it.
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
/// The counter has to come back in the matching `CMSG_FORCE_*_ACK` or the
/// server leaves a pending movement change outstanding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeedChange {
    pub guid: u64,
    pub counter: u32,
    pub speed: f32,
}

pub fn parse_speed_change(body: &[u8]) -> Option<SpeedChange> {
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
/// and is waiting to be told the client knows.
///
/// Layout is `MovementPacketSender::SendTeleportToController`: packed guid,
/// `u32 movementCounter`, then a whole `MovementInfo` carrying the *new*
/// position. The reply goes back under the same opcode with a **plain** u64
/// guid, the counter, and a timestamp (`WorldSession::HandleMoveTeleportAck`
/// reads `recvData >> guid` on an `ObjectGuid`).
///
/// **Ignoring this gets the account banned, and not for anything the client is
/// doing wrong.** The server relocates the player on `ExecuteTeleportNear` and
/// the client keeps dead-reckoning from where it was; the next heartbeat then
/// reports a position more than a yard from the server's with no movement flags
/// set, which is exactly `MovementAnticheat::CheckTeleport`'s definition of a
/// teleport hack. It fires once per packet until the penalty threshold, and the
/// account is banned by `MovementAnticheat` with a reason naming a cheat the
/// client never attempted — which is what happened here, nine detections and a
/// 24-hour ban, from a GM summoning the character.
#[derive(Debug, Clone, PartialEq)]
pub struct Teleport {
    pub guid: u64,
    pub counter: u32,
    pub info: MovementInfo,
}

pub fn parse_teleport(body: &[u8]) -> Option<Teleport> {
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

/// **`SMSG_CLIENT_CONTROL_UPDATE` — which unit this client is now driving.**
///
/// A packed guid and one byte. `Unit::UpdateControl` sends it on both edges of
/// every charm, possess, fear and confusion: possessing something says
/// `(the possessed unit, 1)`, releasing it says `(the player, 1)`, and being
/// feared or mind-controlled says `(the player, 0)` — the same guid the client
/// already drives, with movement taken away.
///
/// **So `allow_move` and the guid are two questions.** A client that reads only
/// the guid cannot tell a fear from a release, and one that reads only the byte
/// cannot tell which body the keys are pointed at.
///
/// The answer is `CMSG_SET_ACTIVE_MOVER` with the same guid, plainly
/// encoded — see [`crate::socket::world::WorldSession::set_active_mover`],
/// which is where the consequence of not sending one is written down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientControl {
    pub guid: u64,
    /// Whether the keys may move it at all. False for a feared or confused
    /// character, whose body the server is walking itself.
    pub allow_move: bool,
}

pub fn parse_client_control(body: &[u8]) -> Option<ClientControl> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if guid == 0 || !r.has(1) {
        return None;
    }
    Some(ClientControl { guid, allow_move: r.u8() != 0 })
}

/// The reply to a near teleport, under the same opcode it arrived on.
///
/// **The guid going back is plain, not packed** — `HandleMoveTeleportAck` reads
/// `recvData >> guid` on an `ObjectGuid`, which is `uint64` in 1.12 — even
/// though the packet that arrived carried a packed one. The asymmetry is the
/// server's, not a mistake here.
pub fn teleport_ack_body(guid: u64, counter: u32, time: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid).u32(counter).u32(time);
    w.buf
}

/// The reply to `SMSG_FORCE_*_SPEED_CHANGE`, on the matching `CMSG_..._ACK`.
///
/// `HandleForceSpeedChangeAckOpcodes`: the guid, the counter the change arrived
/// with, a whole `MovementInfo`, and the speed being confirmed. Echoing the
/// counter is what retires the server's pending change; without it the change
/// stays pending forever and the anticheat goes on measuring us against a speed
/// we are no longer using.
pub fn speed_change_ack_body(
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

/// **The ride is over**: `CMSG_MOVE_SPLINE_DONE`.
///
/// `HandleMoveSplineDoneOpcode` reads a whole `MovementInfo`, then the spline id,
/// then `Unused<float>()` — a trailing word the server skips without looking at
/// it, so its value is free and this sends 1.0, a completed fraction.
///
/// **The one movement packet whose absence is silent and permanent.** The four
/// checks it has to pass are all cheap and all worth stating, because three of
/// them make the packet a no-op rather than an error:
///
/// * the id must be `movespline->GetId()`, so an ack for a spline that has
///   already been superseded is discarded — see [`Mover::ride`], which is why one
///   is never sent;
/// * the same id twice is refused (`HandleSplineDone`);
/// * the position must be within **10 yards** of the spline's own final
///   destination, which is why the ride is *walked* rather than skipped to;
/// * `VerifyMovementInfo`, as for any block.
///
/// Everything else is relaxed for this opcode on purpose: `CheckSpeedHack`
/// exempts it outright, and `HandleFlagTests` returns "ignore" rather than
/// "reject" — *"Since we dont require client confirmation for flag changes
/// during move splines, it's possible for client to not have yet processed the
/// changes when the move spline expires."*
pub fn spline_done_body(info: &MovementInfo, spline_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    info.write(&mut w);
    w.u32(spline_id).f32(1.0);
    w.buf
}

/// **How much real time this client lost**: `CMSG_MOVE_TIME_SKIPPED`, a raw
/// `u64` guid and a `u32` of milliseconds.
///
/// The guid is *not* packed. `WorldPackets::Movement::MoveTimeSkipped::
/// ReadFromWorldPacket` is `recv_data >> guid >> lag`, and `ByteBuffer`'s
/// `ObjectGuid` reader takes eight bytes; the packed form is spelled
/// `ReadAsPacked` and this handler does not use it.
///
/// `HandleMoveTimeSkippedOpcode` adds `lag` to the mover's `stime` and `ctime`
/// — the clock `CheckSpeedHack` measures a stride against — so a client with
/// nothing to declare sends zero rather than inventing an allowance.
///
/// **What it is sent for here is the side effect.** The same handler re-sends a
/// transport's out-of-range and create blocks to a player it has just boarded,
/// which is the only thing that restates `UPDATEFLAG_TRANSPORT`'s path-progress
/// word after login. See `SessionLoop::tick_movement`, which is its one caller.
pub fn time_skipped_body(guid: u64, lag_ms: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid).u32(lag_ms);
    w.buf
}

/// A movement flag the server has toggled and is **waiting to be told about**:
/// `SMSG_FORCE_MOVE_ROOT` / `UNROOT`, `SMSG_MOVE_WATER_WALK` / `LAND_WALK`,
/// `SMSG_MOVE_SET_HOVER` / `UNSET_HOVER`, `SMSG_MOVE_FEATHER_FALL` /
/// `NORMAL_FALL`.
///
/// **These are the packets whose absence got this client kicked.**
/// `AddMovementFlagChangeToController` pushes a `PlayerMovementPendingChange`
/// and sends one of the six; if no ack arrives within
/// `Movement.PendingAckResponseTime` (4 s here), `CheckPendingMovementChanges`
/// calls `OnFailedToAckChange` — `CHEAT_TYPE_PENDING_ACK_DELAY`, threshold 3,
/// kick — and then **enforces the change anyway**. For a root that means the
/// server sets `MOVEFLAG_ROOT` on its copy of our movement info, so the next
/// packet we send with a moving flag is `CHEAT_TYPE_ROOT_MOVE`, which is sticky
/// (the check ORs the flag back into the stored info) and reaches its
/// five-tick threshold in a second. `Anticheat.log` on the reference server has
/// both, repeatedly, on this client's own characters.
///
/// The reply body is `u64 guid, u32 counter, MovementInfo` — plus a `u32 apply`
/// for the three that go through `HandleMovementFlagChangeToggleAck`, and *not*
/// for root, which has its own handler. Hence [`FlagChange::apply_field`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagChange {
    /// The `MOVEFLAG_` being toggled.
    pub flag: u32,
    pub apply: bool,
    pub ack: Opcode,
}

impl FlagChange {
    /// Which server packet is this, if any?
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

    /// Does this ack carry the trailing `u32 apply`?
    ///
    /// Root does not: `HandleMoveRootAck` reads guid, counter and the movement
    /// block and stops, taking the *opcode* as the answer. The other three go
    /// through `HandleMovementFlagChangeToggleAck`, which reads one more field.
    /// One extra `u32` on a root ack pushes nothing off the end — the server
    /// simply ignores the tail — but a missing one on a water-walk ack reads
    /// garbage, so the asymmetry is stated rather than papered over.
    pub fn apply_field(&self) -> bool {
        self.flag != move_flags::ROOT
    }
}

/// `packedGuid` + `u32 movementCounter`, which is the whole body of all six.
pub fn parse_flag_change(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if !r.has(4) {
        return None;
    }
    Some((guid, r.u32()))
}

/// The reply to one of the six. `apply` is `None` for the root pair.
pub fn flag_change_ack_body(
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

/// `SMSG_MOVE_KNOCK_BACK`: the server has thrown this unit.
///
/// `packedGuid`, `u32 movementCounter`, then `cos, sin, speedXY, speedZ` —
/// note that the direction comes as a vector rather than an angle, and that
/// `speedZ` is **down-positive**: `Unit::KnockBack` sends `-verticalSpeed`, so
/// a throw upward arrives negative, the same convention as the jump block it
/// is echoed in. The four floats have to come back **verbatim** in the
/// ack's jump block: `FindPendingMovementKnockbackChange` compares each to
/// within 0.01 and rejects the ack otherwise, which leaves the change pending
/// and lands on `CHEAT_TYPE_PENDING_ACK_DELAY` exactly as if we had never
/// answered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KnockBack {
    pub guid: u64,
    pub counter: u32,
    pub cos_angle: f32,
    pub sin_angle: f32,
    pub xy_speed: f32,
    pub z_speed: f32,
}

pub fn parse_knock_back(body: &[u8]) -> Option<KnockBack> {
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

/// The reply to a knockback: `u64 guid, u32 counter, MovementInfo` — where the
/// movement info's jump block is what carries the four numbers back.
pub fn knock_back_ack_body(guid: u64, counter: u32, info: &MovementInfo) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid).u32(counter);
    info.write(&mut w);
    w.buf
}

/// `SMSG_NEW_WORLD`: the server is moving this player to **another map**, and
/// the session is held until the client says it has arrived.
///
/// Layout is `Player::SendNewWorld`: `u32 mapId` then `f32 x, y, z, o` — 20
/// bytes, no guid, because it can only ever be about us. (The four floats are
/// the *transport-local* position when the player is on a transport, which this
/// client does not read; it does not change the parse.)
///
/// **This is the half of the teleport with the worse consequence.** The near
/// teleport ([`Teleport`]) misreported a position and got the account banned;
/// this one stops the session dead. `Player::TeleportTo` sets
/// `SetSemaphoreTeleportFar(true)` before sending it, and every movement opcode
/// the client sends is discarded while that flag is up — the world is only
/// entered by `HandleMoveWorldportAckOpcode`, which is what clears it, creates
/// the destination map, relocates the player and sends the initial packets. A
/// client that files this under "unhandled opcodes" is left standing in a world
/// that no longer contains it, receiving nothing, with no error anywhere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NewWorld {
    pub map_id: u32,
    pub position: Position,
}

pub fn parse_new_world(body: &[u8]) -> Option<NewWorld> {
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
    // The same guard the near teleport gets, and for the same reason: adopting a
    // position the server could not have meant strands the character outside
    // every loaded tile with nothing to bring it back.
    if !position.is_plausible() {
        return None;
    }
    Some(NewWorld { map_id, position })
}

/// **`SMSG_LOGIN_VERIFY_WORLD`: which map the character actually logged in
/// on**, and the only packet that ever says so.
///
/// Same twenty bytes as [`NewWorld`] — `u32 mapId` then `f32 x, y, z, o` — and
/// sent from `WorldSession::HandlePlayerLoginOpcode` before anything else about
/// the world, so it is the first thing in the login burst.
///
/// ## Why it is not redundant with the character list
///
/// Because the row the player clicked can be **stale by the time the login
/// completes, with nothing on the wire to say so**. `Player::LoadFromDB` ends
/// with:
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
/// That is a **`Relocate`, not a `TeleportTo`** — it happens inside the load,
/// before the session has a map or a socket to announce anything on, so there
/// is no `SMSG_TRANSFER_PENDING` and no `SMSG_NEW_WORLD`. A character who
/// logged out in Scholomance and comes back after the instance has expired is
/// on map 0 at Caer Darrow, and `SMSG_CHAR_ENUM` still says map 289.
///
/// A client that takes the map from the list and never reads this packet then
/// streams the *instance's* directory at the *continent's* coordinates. For
/// Scholomance that is `schoolofnecromancy_36_29.adt`, which does not exist —
/// the map's sixteen tiles are at (30..33, 29..32) — so nine reads fail and
/// there is no ground, no buildings and no collision, with the entrance's
/// creatures standing in the middle of it. It stays that way for the whole
/// session: a hearthstone out of Caer Darrow is a *near* teleport, same map,
/// no `SMSG_NEW_WORLD`, so nothing ever corrects the mistake and the next login
/// is the fix. That was the report this parser answers.
///
/// The **position** is carried but is not what this is for: the character's own
/// create block states it a few packets later, along with its speeds and its
/// movement flags, and that is what the login adopts. The map id has no second
/// source at all.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoginVerifyWorld {
    pub map_id: u32,
    pub position: Position,
}

pub fn parse_login_verify_world(body: &[u8]) -> Option<LoginVerifyWorld> {
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
    // The same guard both teleports get. A login is the one moment with no
    // previous position to fall back on, so a body that will not parse leaves
    // the character-list row standing — which is wrong in exactly the case
    // above and right in every other one.
    if !position.is_plausible() {
        return None;
    }
    Some(LoginVerifyWorld { map_id, position })
}

/// **`SMSG_TRANSFER_PENDING`: the far teleport is about to happen.**
///
/// The announcement half of [`NewWorld`], and it lives beside it rather than
/// beside `SMSG_TRANSFER_ABORTED` because it is about the transfer that *is*
/// happening: vmangos sends it from `Player::ExecuteTeleportFar`, immediately
/// before removing the character from the old map, with the comment `// send
/// transfer packet to display load screen` on the line above it
/// (`Player.cpp:2037`). It carries no position and needs no reply; the whole of
/// what it is for is the second or two between "you are leaving" and
/// `SMSG_NEW_WORLD` saying where you arrived.
///
/// **Nothing else on the wire marks that moment.** Reading the map change off
/// `SMSG_NEW_WORLD` instead is late by exactly the interval in which the old
/// world is torn down, which is the interval a loading screen exists to cover.
///
/// `u32 mapId`, then — **only when the character is standing on a transport** —
/// `u32 transportEntry, u32 oldMapId`. Both tails are read because a body this
/// client does not understand the length of is a body it cannot tell from a
/// truncated one; nothing here uses them yet, and a boat leaving with a
/// character on it is `SMSG_MONSTER_MOVE_TRANSPORT`'s subject rather than this
/// one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferPending {
    /// The map being transferred **to**.
    pub map_id: u32,
    /// `(transport entry, the map being left)`, for a transfer that begins on a
    /// boat or a zeppelin. Absent for every ordinary teleport.
    pub transport: Option<(u32, u32)>,
}

pub fn parse_transfer_pending(body: &[u8]) -> Option<TransferPending> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let map_id = r.u32();
    // The two extra dwords are all-or-nothing: `SendPacket` wrote both or
    // neither, so a body with four spare bytes is a body this parse has got
    // wrong rather than a transport with half its fields.
    let transport = if r.has(8) {
        Some((r.u32(), r.u32()))
    } else {
        None
    };
    Some(TransferPending { map_id, transport })
}

/// Which of the six speeds an `SMSG_FORCE_*_SPEED_CHANGE` carries, and the
/// opcode that acknowledges it. Order matches [`Speeds`].
pub fn speed_change_slot(opcode: Opcode) -> Option<(usize, Opcode)> {
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

/// How a speed change about a unit that is **not us** arrives.
///
/// `MovementPacketSender`'s table is three opcodes wide per speed and this
/// client only ever read the middle column. The other two are the whole of what
/// an observer is told:
///
/// ```text
/// moveTypeToOpcode[MOVE_RUN] = { SMSG_SPLINE_SET_RUN_SPEED,   // server-driven
///                                SMSG_FORCE_RUN_SPEED_CHANGE, // the controller
///                                MSG_MOVE_SET_RUN_SPEED }     // everyone else
/// ```
///
/// **Nothing else on the wire ever restates a unit's speed.** The six speeds are
/// in a movement block, which arrives when an object is *created* and never
/// again — so a player who mounts in front of you, or a creature that is hasted,
/// slowed, enraged or fleeing, goes on being dead-reckoned at the speed they had
/// when they came into sight. At a 60% mount that is 4.2 yards of error per
/// half-second heartbeat, applied and undone twice a second in alternating
/// directions, which is the "jitter back and forth aggressively" report.
///
/// The `*_CHEAT` opcodes beside these (204, 206, 208, 210, 212, 215) are
/// deliberately absent: `moveTypeToOpcode` names them for no build, and
/// `Opcodes.cpp` files them under `Handle_NULL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedBroadcast {
    /// `MSG_MOVE_SET_*_SPEED` — packed guid, a whole movement block, then the
    /// speed. Sent about a **player-controlled** unit whose spline has
    /// finalised, and the block is deliberately part of it:
    /// `HandleForceSpeedChangeAck` relocates the mover first and the comment
    /// over the send says so — *"send the speed change to others (with updated
    /// position if all is fine)"*.
    Observed,
    /// `SMSG_SPLINE_SET_*` — packed guid and the speed, no block. Sent about a
    /// **server-driven** unit (every creature in the game, via
    /// `SendSpeedChangeToAll`), and about a player-controlled one whose spline
    /// has *not* finalised, where the position would be a lie mid-path.
    Spline,
}

/// Which of the six speeds one of the two observer families carries, and which
/// family it is. Order matches [`Speeds`] and vmangos' `UnitMoveType`.
pub fn broadcast_speed_slot(opcode: Opcode) -> Option<(usize, SpeedBroadcast)> {
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
/// **The block is not optional and it is not the same packet as a heartbeat.**
/// Parsing it as one ([`parse_movement_broadcast`]) succeeds and drops the
/// trailing float on the floor, which is the failure this whole family had:
/// the position would move and the speed it is moving at would not.
pub fn parse_observed_speed(body: &[u8]) -> Option<(u64, MovementInfo, f32)> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    let info = MovementInfo::read(&mut r)?;
    if !r.has(4) {
        return None;
    }
    Some((guid, info, r.f32()))
}

/// [`SpeedBroadcast::Spline`]: packed guid and `f32 speed`, and nothing else.
pub fn parse_spline_speed(body: &[u8]) -> Option<(u64, f32)> {
    let mut r = Reader::new(body);
    let guid = r.packed_guid();
    if !r.has(4) {
        return None;
    }
    Some((guid, r.f32()))
}

/// **A movement flag stated about a unit this client does not control** —
/// the twelve `SMSG_SPLINE_MOVE_*` opcodes.
///
/// `MovementPacketSender` splits one question three ways by *who is moving the
/// unit*, and this project had two of the three:
///
/// | function | audience | opcodes | read here by |
/// |---|---|---|---|
/// | `AddMovementFlagChangeToController` | the player moving the unit | six `SMSG_MOVE_*`/`SMSG_FORCE_MOVE_*` | [`FlagChange`] — and it must be **acknowledged** |
/// | `SendMovementFlagChangeToObservers` | everyone *but* that player | five `MSG_MOVE_*` | [`is_broadcast_movement`] — packed guid + a whole block |
/// | `SendMovementFlagChangeToAll` / `SendToggleRunWalkToAll` | everyone, for a **server-controlled** unit | these twelve | this |
///
/// The third is the creature case, and it is the one with no movement block:
/// under 1.9.4 the body is a **packed guid and nothing else**, so the opcode
/// carries the entire message and the flag has to come from a table. It is not
/// acknowledged — nobody is being asked to obey it, they are being told.
///
/// **The flags are otherwise frozen at creation.** A creature's movement block
/// arrives once, inside its create block; no values update carries one and
/// `SMSG_MONSTER_MOVE` carries a path with no flags at all. So for every unit
/// the server moves, these twelve packets are the *only* statement after the
/// first that it has been rooted, told to walk, given water-walking, set
/// hovering or handed a feather fall — and a client that drops them runs on the
/// flags the unit was born with for the rest of the session.
///
/// ## Two of the twelve this server never sends
///
/// `SMSG_SPLINE_MOVE_START_SWIM` and `_STOP_SWIM` are in the opcode table and
/// in `Opcodes.cpp`, and **nothing in vmangos writes either**: no call site
/// exists, and `MOVEFLAG_SWIMMING` is set on a creature nowhere in the server
/// at all. They are here because the reference client reads them and because a
/// table with a hole in it invites the hole to be re-derived; they are named as
/// dead rather than left out. The cost of that gap is a swimming *creature*
/// drawn running — see `Entity::is_swimming`, which says the same thing from
/// the other end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplineFlagChange {
    /// The `MOVEFLAG_` being stated.
    pub flag: u32,
    /// Set it, or clear it.
    pub apply: bool,
}

impl SplineFlagChange {
    /// Which of the twelve is this, if any?
    ///
    /// The pairing is `MovementPacketSender::SendMovementFlagChangeToAll`'s own
    /// switch, plus `SendToggleRunWalkToAll` for the last pair — which is a
    /// separate function on the server because run/walk is not a "flag change"
    /// there, and is the same thing here.
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
            // **Backwards against every other pair here**, and it is the
            // server's own asymmetry rather than a slip: the flag is
            // `MOVEFLAG_WALK_MODE`, so the *run* opcode is the one that clears
            // it. `SendToggleRunWalkToAll(unit, run)` picks
            // `run ? SET_RUN_MODE : SET_WALK_MODE`.
            Opcode::SMSG_SPLINE_MOVE_SET_WALK_MODE => (WALK_MODE, true),
            Opcode::SMSG_SPLINE_MOVE_SET_RUN_MODE => (WALK_MODE, false),
            // The two nothing sends — see the type's own note.
            Opcode::SMSG_SPLINE_MOVE_START_SWIM => (SWIMMING, true),
            Opcode::SMSG_SPLINE_MOVE_STOP_SWIM => (SWIMMING, false),
            _ => return None,
        };
        Some(SplineFlagChange { flag, apply })
    }
}

/// A packed guid, which is the whole body of all twelve.
///
/// **A zero-length body is not one of these.** `Reader::packed_guid` answers 0
/// for an empty read the same way it answers 0 for a genuine mask byte of zero,
/// and a guid of 0 names no entity, so the empty case is refused here rather
/// than being applied to nothing further down.
pub fn parse_spline_flag(body: &[u8]) -> Option<u64> {
    if body.is_empty() {
        return None;
    }
    let mut r = Reader::new(body);
    match r.packed_guid() {
        0 => None,
        guid => Some(guid),
    }
}

/// **What the server has said about a unit's movement flags outside its
/// movement block** — which bits it has spoken about, and what it said.
///
/// Two words rather than one, because these packets both *set* and *clear*, and
/// the thing they are overriding is a flag word that arrived earlier and is
/// still the answer for every bit they have not mentioned. One mask of
/// "unrooted" bits would be indistinguishable from silence.
///
/// Applied by [`Self::over`], which is the only place the two are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FlagOverride {
    /// The bits any of the twelve has stated, set or cleared.
    pub known: u32,
    /// …and the value it stated them at.
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

    /// The flag word a unit is actually on: `base` for every bit nothing has
    /// stated, this for the rest.
    pub fn over(&self, base: u32) -> u32 {
        (base & !self.known) | (self.value & self.known)
    }

    /// Has anything been stated at all? The common case is `false`, and it is
    /// what lets [`Self::over`] be skipped rather than run per entity per frame.
    pub fn is_empty(&self) -> bool {
        self.known == 0
    }
}

pub mod curve;

#[cfg(test)]
mod tests;
