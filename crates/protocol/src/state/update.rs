//! `SMSG_UPDATE_OBJECT` — how the server tells the client that things exist,
//! where they are, and what changed about them.
//!
//! This is the largest single piece of the protocol. Layout:
//!
//! ```text
//! u32 blockCount
//! u8  hasTransport
//! blockCount x {
//!   u8 updateType
//!   VALUES(0)          packedGuid, valuesUpdate
//!   MOVEMENT(1)        u64 guid (NOT packed), movementUpdate
//!   CREATE_OBJECT(2)   packedGuid, u8 objectTypeId, movementUpdate, valuesUpdate
//!   CREATE_OBJECT2(3)  as above
//!   OUT_OF_RANGE(4)    u32 count, count x packedGuid
//!   NEAR_OBJECTS(5)    u32 count, count x packedGuid
//! }
//! ```
//!
//! Source: vmangos `src/game/Objects/UpdateData.{h,cpp}` and
//! `src/game/Objects/Object.cpp` (`BuildMovementUpdate`, `BuildValuesUpdate`).
//!
//! **Parsing is best-effort by design.** The block layout is gated by flags, so
//! one misunderstood field would otherwise desynchronise the rest of the packet
//! and panic. Every read is bounds-checked and parsing stops early rather than
//! failing — a partially understood update is far more useful than a dead
//! session, and the leftovers are reported for diagnosis.

use crate::bytes::Reader;

/// `enum ObjectUpdateType` — what a block does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum UpdateType {
    Values = 0,
    Movement = 1,
    CreateObject = 2,
    CreateObject2 = 3,
    OutOfRange = 4,
    NearObjects = 5,
}

impl UpdateType {
    pub fn from_code(code: u8) -> Option<Self> {
        use UpdateType::*;
        Some(match code {
            0 => Values,
            1 => Movement,
            2 => CreateObject,
            3 => CreateObject2,
            4 => OutOfRange,
            5 => NearObjects,
            _ => return None,
        })
    }

    /// Both create variants carry identical payloads; `CreateObject2` only
    /// tells the client the object is brand new rather than newly visible.
    pub fn is_create(self) -> bool {
        matches!(self, UpdateType::CreateObject | UpdateType::CreateObject2)
    }
}

/// `enum TypeID` — what kind of thing a created object is.
///
/// `Object` is the default because it is the base every other one extends and
/// because it is already what the renderer falls back to for an object whose
/// type never arrived (`e.object_type.unwrap_or(ObjectType::Object)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ObjectType {
    #[default]
    Object = 0,
    Item = 1,
    Container = 2,
    Unit = 3,
    Player = 4,
    GameObject = 5,
    DynamicObject = 6,
    Corpse = 7,
}

impl ObjectType {
    pub fn from_code(code: u8) -> Option<Self> {
        use ObjectType::*;
        Some(match code {
            0 => Object,
            1 => Item,
            2 => Container,
            3 => Unit,
            4 => Player,
            5 => GameObject,
            6 => DynamicObject,
            7 => Corpse,
            _ => return None,
        })
    }
}

/// `enum ObjectUpdateFlags` — which movement fields are present.
/// vmangos marks this enum "checked for 1.12.1".
pub mod update_flags {
    pub const NONE: u8 = 0x00;
    pub const SELF: u8 = 0x01;
    pub const TRANSPORT: u8 = 0x02;
    pub const MELEE_ATTACKING: u8 = 0x04;
    pub const HIGHGUID: u8 = 0x08;
    pub const ALL: u8 = 0x10;
    pub const LIVING: u8 = 0x20;
    pub const HAS_POSITION: u8 = 0x40;
}

/// `enum MovementFlags`. Defined once, in [`crate::state::movement`], because the same
/// values gate both this block and the outgoing `MSG_MOVE_*` packets — two
/// copies would be two chances to disagree.
pub use crate::state::movement::move_flags;

/// `MoveSplineFlag` — only the bits that change the wire layout.
pub mod spline_flags {
    pub const FINAL_POINT: u32 = 0x0001_0000;
    pub const FINAL_TARGET: u32 = 0x0002_0000;
    pub const FINAL_ANGLE: u32 = 0x0004_0000;
}

/// A position with facing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub orientation: f32,
}

impl Position {
    /// Could the server have meant this?
    ///
    /// `MaNGOS::IsValidMapCoord` (`GridDefines.h:186`): finite, and within half a
    /// map of the origin on X and Y, `|z| <= 400000`. The server rejects anything
    /// else outright, so a coordinate that fails here is a *client* mistake — a
    /// mis-read field, or a block parsed at the wrong offset — and the one thing
    /// worse than noticing is not noticing. See `Entity::set_server_position`.
    pub fn is_plausible(&self) -> bool {
        // 64 tiles of 1600/3 yards, halved: the same MAP_HALFSIZE the server uses.
        const HALF: f32 = 32.0 * 1600.0 / 3.0;
        let ok = |c: f32| c.is_finite() && c.abs() <= HALF - 0.5;
        ok(self.x) && ok(self.y) && self.z.is_finite() && self.z.abs() <= 400_000.0
            && self.orientation.is_finite()
    }
}

/// The movement block attached to create/movement updates.
#[derive(Debug, Clone, Default)]
pub struct MovementUpdate {
    pub update_flags: u8,
    pub move_flags: u32,
    pub position: Option<Position>,
    /// Walk, run, run-back, swim, swim-back, turn-rate.
    pub speeds: Option<[f32; 6]>,
    /// The object was moving along a spline path when this update was built.
    pub has_spline: bool,
    /// Whether that spline block was consumed cleanly. `false` means everything
    /// after it in the packet is untrustworthy.
    pub spline_ok: bool,
    /// The spline itself — the path, how far through it the server is, and how
    /// long the whole thing takes.
    ///
    /// **A creature that comes into view mid-walk is walking**, and this block
    /// is the only thing that says so. `SMSG_MONSTER_MOVE` is sent when a move
    /// *starts*, to whoever could see it then; someone who arrives afterwards
    /// gets the state of the move inline in the create block and nothing else
    /// until the creature is next given a new one. Reading the block only far
    /// enough to skip past it — which is what this client did, because getting
    /// the *size* right is what stops the rest of the packet desynchronising —
    /// leaves every such creature standing frozen at the position the create
    /// block happened to state, and then **teleporting** the moment its next
    /// spline arrives, by however far it had walked in the meantime. Ten to
    /// fifteen yards, in practice, and once for every creature that streams in
    /// while moving.
    pub spline: Option<SplineUpdate>,
    /// **`UPDATEFLAG_TRANSPORT`'s trailing word: how far into its own cycle a
    /// moving platform is**, in milliseconds, at the moment this block was
    /// built.
    ///
    /// The one thing on the wire that says anything at all about an elevator.
    /// `Object::BuildMovementUpdate` appends `uint32 GetPathProgress()` for a
    /// game object carrying the flag, and `ElevatorTransport::Update` keeps that
    /// at `GetTimeSinceCreation() % TotalTime` — so it is a *phase*, not a
    /// timestamp, and it is the only way a client can be in step with the
    /// server about a thing neither of them ever sends a position for. The
    /// motion itself is `TransportAnimation.dbc`, which both ends read.
    ///
    /// Read and thrown away until the round that fixed *"elevators are drawn and
    /// stood on at their spawn position"*, which it is the missing half of. Note
    /// that the same flag also changes what the position above **means**: for a
    /// transport, `HAS_POSITION` carries the *stationary* point
    /// (`GetStationaryX/Y/Z`) rather than where the platform is now, which is
    /// exactly the base an offset is added to.
    pub path_progress: Option<u32>,
}

/// The in-flight spline a create block carries.
#[derive(Debug, Clone, PartialEq)]
pub struct SplineUpdate {
    /// `MoveSplineFlag`, the same word `SMSG_MONSTER_MOVE` carries.
    pub flags: u32,
    /// Where the unit looks when it arrives.
    pub facing: crate::state::movement::SplineFacing,
    /// How far into the move the server already is.
    pub elapsed_ms: u32,
    /// How long the whole move takes — **not** the remainder.
    /// `PacketBuilder::WriteCreate` writes `Duration()` raw, where
    /// `WriteMonsterMove` writes `duration - time_passed`.
    pub duration_ms: u32,
    /// The waypoints, phantom control points trimmed off.
    pub path: Vec<[f32; 3]>,
}

impl MovementUpdate {
    /// Is this the object the session controls?
    pub fn is_self(&self) -> bool {
        self.update_flags & update_flags::SELF != 0
    }
}

/// A parsed values block: field index -> raw u32.
///
/// Values are kept raw rather than decoded into named fields. The meaning of an
/// index depends on the object's type and comes from `UpdateFields.h`; mapping
/// them is a separate, additive job, and storing the raw words means no data is
/// lost in the meantime.
#[derive(Debug, Clone, Default)]
pub struct ValuesUpdate {
    pub fields: Vec<(u16, u32)>,
}

impl ValuesUpdate {
    pub fn get(&self, index: u16) -> Option<u32> {
        self.fields.iter().find(|(i, _)| *i == index).map(|(_, v)| *v)
    }

    pub fn get_f32(&self, index: u16) -> Option<f32> {
        self.get(index).map(f32::from_bits)
    }
}

/// One block from the packet.
#[derive(Debug, Clone)]
pub enum UpdateBlock {
    Create {
        guid: u64,
        object_type: Option<ObjectType>,
        movement: MovementUpdate,
        values: ValuesUpdate,
        /// True for `CREATE_OBJECT2` (brand new rather than newly visible).
        is_new: bool,
    },
    Values {
        guid: u64,
        values: ValuesUpdate,
    },
    Movement {
        guid: u64,
        movement: MovementUpdate,
    },
    OutOfRange {
        guids: Vec<u64>,
    },
    NearObjects {
        guids: Vec<u64>,
    },
}

/// The result of parsing one `SMSG_UPDATE_OBJECT`.
#[derive(Debug, Clone, Default)]
pub struct ObjectUpdate {
    pub has_transport: bool,
    pub blocks: Vec<UpdateBlock>,
    /// Set when parsing stopped early. Non-empty means we do not fully
    /// understand some block; the session continues regardless.
    pub warning: Option<String>,
}

/// Parse an `SMSG_UPDATE_OBJECT` body.
pub fn parse(body: &[u8]) -> ObjectUpdate {
    let mut out = ObjectUpdate::default();
    let mut r = Reader::new(body);

    if !r.has(5) {
        out.warning = Some("packet too short for header".into());
        return out;
    }
    let block_count = r.u32();
    out.has_transport = r.u8() != 0;

    for i in 0..block_count {
        if !r.has(1) {
            out.warning = Some(format!("ran out of data at block {i}/{block_count}"));
            break;
        }
        let raw_type = r.u8();
        let Some(update_type) = UpdateType::from_code(raw_type) else {
            // Cannot know this block's length, so nothing after it is
            // trustworthy either.
            out.warning = Some(format!("unknown update type {raw_type} at block {i}"));
            break;
        };

        match update_type {
            UpdateType::OutOfRange | UpdateType::NearObjects => {
                if !r.has(4) {
                    out.warning = Some(format!("truncated guid list at block {i}"));
                    break;
                }
                let count = r.u32();
                let mut guids = Vec::new();
                for _ in 0..count {
                    if !r.has(1) {
                        break;
                    }
                    guids.push(r.packed_guid());
                }
                out.blocks.push(if update_type == UpdateType::OutOfRange {
                    UpdateBlock::OutOfRange { guids }
                } else {
                    UpdateBlock::NearObjects { guids }
                });
            }

            UpdateType::Values => {
                let guid = r.packed_guid();
                let values = parse_values(&mut r);
                out.blocks.push(UpdateBlock::Values { guid, values });
            }

            UpdateType::Movement => {
                // Note the asymmetry: vmangos writes a plain u64 here
                // (`BuildMovementUpdateBlock` uses GetObjectGuid()), not the
                // packed form used by create/values blocks.
                if !r.has(8) {
                    out.warning = Some(format!("truncated movement guid at block {i}"));
                    break;
                }
                let guid = r.u64();
                let movement = parse_movement(&mut r);
                out.blocks.push(UpdateBlock::Movement { guid, movement });
            }

            UpdateType::CreateObject | UpdateType::CreateObject2 => {
                let guid = r.packed_guid();
                if !r.has(1) {
                    out.warning = Some(format!("truncated create block {i}"));
                    break;
                }
                let object_type = ObjectType::from_code(r.u8());
                let movement = parse_movement(&mut r);
                let values = parse_values(&mut r);
                out.blocks.push(UpdateBlock::Create {
                    guid,
                    object_type,
                    movement,
                    values,
                    is_new: update_type == UpdateType::CreateObject2,
                });
            }
        }
    }

    if out.warning.is_none() && r.remaining() > 0 {
        // Leftover bytes mean a field group was mis-sized somewhere. Worth
        // surfacing: the blocks parsed so far may still be wrong.
        out.warning = Some(format!("{} trailing bytes unparsed", r.remaining()));
    }
    out
}

fn parse_movement(r: &mut Reader) -> MovementUpdate {
    let mut m = MovementUpdate::default();
    if !r.has(1) {
        return m;
    }
    m.update_flags = r.u8();

    if m.update_flags & update_flags::LIVING != 0 {
        // Identical layout to an outgoing MSG_MOVE_* body, so it is the same
        // reader. If the two ever drift apart, one direction of movement will
        // silently misparse while the other looks fine.
        let Some(info) = crate::state::movement::MovementInfo::read(r) else {
            return m;
        };
        m.move_flags = info.flags;
        m.position = Some(info.position);

        if !r.has(24) {
            return m;
        }
        m.speeds = Some([r.f32(), r.f32(), r.f32(), r.f32(), r.f32(), r.f32()]);

        // A moving creature carries its spline path inline. Skipping it
        // desynchronises the whole rest of the packet, which shows up as a
        // bogus "unknown update type" several blocks later.
        if m.move_flags & move_flags::SPLINE_ENABLED != 0 {
            m.has_spline = true;
            m.spline = read_spline(r);
            m.spline_ok = m.spline.is_some();
        }
    } else if m.update_flags & update_flags::HAS_POSITION != 0 {
        if !r.has(16) {
            return m;
        }
        m.position = Some(Position {
            x: r.f32(),
            y: r.f32(),
            z: r.f32(),
            orientation: r.f32(),
        });
    }

    if m.update_flags & update_flags::HIGHGUID != 0 && r.has(4) {
        let _unk = r.u32();
    }
    if m.update_flags & update_flags::ALL != 0 && r.has(4) {
        let _unk = r.u32();
    }
    if m.update_flags & update_flags::MELEE_ATTACKING != 0 && r.has(1) {
        let _victim = r.packed_guid();
    }
    if m.update_flags & update_flags::TRANSPORT != 0 && r.has(4) {
        m.path_progress = Some(r.u32());
    }

    m
}

/// Read the inline spline written by `PacketBuilder::WriteCreate`.
///
/// ```text
/// u32 splineFlags
/// Final_Angle  -> f32 angle          (checked first)
/// Final_Target -> u64 target guid
/// Final_Point  -> f32 x, y, z
/// u32 timePassed, u32 duration, u32 id
/// u32 nodeCount, nodeCount x (f32 x, y, z)
/// f32 x, y, z   final destination (zero when cyclic)
/// ```
///
/// The facing branches are an else-if chain server-side, so exactly one applies
/// even if several bits are set. Returns `None` if the block was truncated, in
/// which case everything after it in the packet is untrustworthy.
///
/// **The node array is the spline's raw point buffer, phantom points and all.**
/// `getPath()` is `spline.getPoints()`, not the control list: `SplineBase`
/// reserves entries at one or both ends for `C_Evaluate`, and which ones depends
/// on the interpolation mode. The real path is `[index_lo ..= index_hi]`, and
/// those bounds are set by `InitLinear` / `InitCatmullRom`:
///
/// ```text
///   linear,     n controls -> size n+1, lo 0, hi n-1   (cyclic: hi n)
///   catmullrom, n controls -> size n+2, lo 1, hi n     (cyclic: size n+3, hi n+1)
/// ```
///
/// which is `[lo .. count-1)` in every case but linear-cyclic, where it is the
/// whole array. Taking the buffer verbatim walks the creature onto a phantom —
/// a duplicate of the destination for the two common cases, so it would cost
/// only a zero-length final leg, but a *leading* phantom on a catmull-rom spline
/// is `controls[0].lerp(controls[1], -1)`: a point extrapolated backwards past
/// the start, which would fling the creature away from its path and then back.
fn read_spline(r: &mut Reader) -> Option<SplineUpdate> {
    use crate::state::movement::SplineFacing;

    if !r.has(4) {
        return None;
    }
    let flags = r.u32();

    let facing = if flags & spline_flags::FINAL_ANGLE != 0 {
        if !r.has(4) {
            return None;
        }
        SplineFacing::Angle(r.f32())
    } else if flags & spline_flags::FINAL_TARGET != 0 {
        if !r.has(8) {
            return None;
        }
        SplineFacing::Target(r.u64())
    } else if flags & spline_flags::FINAL_POINT != 0 {
        if !r.has(12) {
            return None;
        }
        SplineFacing::Spot([r.f32(), r.f32(), r.f32()])
    } else {
        SplineFacing::Travel
    };

    // timePassed, duration, id (id exists for builds above 1.7.1).
    if !r.has(12) {
        return None;
    }
    let elapsed_ms = r.u32();
    let duration_ms = r.u32();
    let _id = r.u32();

    if !r.has(4) {
        return None;
    }
    let nodes = r.u32() as usize;
    // Path nodes plus the trailing final-destination vector. `has` also guards
    // against a corrupt count demanding an absurd read.
    let path_bytes = nodes.saturating_mul(12).saturating_add(12);
    if !r.has(path_bytes) {
        return None;
    }
    let points: Vec<[f32; 3]> = (0..nodes).map(|_| [r.f32(), r.f32(), r.f32()]).collect();
    let _final_destination = [r.f32(), r.f32(), r.f32()];

    // The interpolation mode and the cycle bit decide which ends of the buffer
    // are phantom; both live in `movement`'s copy of `MoveSplineFlag`, which is
    // the same word.
    let catmullrom = flags & crate::state::movement::spline_flags::CATMULLROM != 0;
    let cyclic = flags & crate::state::movement::spline_flags::CYCLIC != 0;
    let lo = if catmullrom { 1 } else { 0 };
    let hi = if cyclic && !catmullrom {
        nodes
    } else {
        nodes.saturating_sub(1)
    };
    let path: Vec<[f32; 3]> = points.get(lo..hi).unwrap_or_default().to_vec();

    Some(SplineUpdate {
        flags,
        facing,
        elapsed_ms,
        duration_ms,
        path,
    })
}

/// `u8 blockCount`, `blockCount` u32 mask words, then one u32 per set bit.
fn parse_values(r: &mut Reader) -> ValuesUpdate {
    let mut v = ValuesUpdate::default();
    if !r.has(1) {
        return v;
    }
    let block_count = r.u8() as usize;
    if !r.has(block_count * 4) {
        return v;
    }
    let mask: Vec<u32> = (0..block_count).map(|_| r.u32()).collect();

    for (word_index, word) in mask.iter().enumerate() {
        for bit in 0..32 {
            if word & (1 << bit) == 0 {
                continue;
            }
            if !r.has(4) {
                return v;
            }
            let field_index = (word_index * 32 + bit) as u16;
            v.fields.push((field_index, r.u32()));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;

    #[test]
    fn packed_guid_round_trips() {
        // GUID 0x0000_0000_0000_1234 -> bytes 0x34 (bit0), 0x12 (bit1).
        let buf = [0b0000_0011u8, 0x34, 0x12];
        let mut r = Reader::new(&buf);
        assert_eq!(r.packed_guid(), 0x1234);
    }

    #[test]
    fn values_mask_selects_the_right_indices() {
        let mut w = Writer::new();
        w.u8(1); // one mask word
        w.u32(0b1001); // bits 0 and 3 set
        w.u32(111);
        w.u32(222);
        let mut r = Reader::new(&w.buf);
        let v = parse_values(&mut r);
        assert_eq!(v.fields, vec![(0, 111), (3, 222)]);
        assert_eq!(v.get(3), Some(222));
        assert_eq!(v.get(1), None);
    }

    #[test]
    fn truncated_packet_warns_instead_of_panicking() {
        // Claims 5 blocks but supplies none.
        let mut w = Writer::new();
        w.u32(5).u8(0);
        let parsed = parse(&w.buf);
        assert!(parsed.blocks.is_empty());
        assert!(parsed.warning.is_some());
    }

    #[test]
    fn out_of_range_block_reads_guid_list() {
        let mut w = Writer::new();
        w.u32(1).u8(0); // one block, no transport
        w.u8(UpdateType::OutOfRange as u8);
        w.u32(2);
        w.u8(0b1).u8(7); // guid 7
        w.u8(0b1).u8(9); // guid 9
        let parsed = parse(&w.buf);
        assert!(parsed.warning.is_none(), "{:?}", parsed.warning);
        match &parsed.blocks[0] {
            UpdateBlock::OutOfRange { guids } => assert_eq!(guids, &vec![7, 9]),
            other => panic!("wrong block: {other:?}"),
        }
    }

    /// **The in-flight spline a create block carries, with its phantom control
    /// points trimmed.**
    ///
    /// `getPath()` is the spline's raw point buffer, not its control list:
    /// `SplineBase` reserves entries at one or both ends for `C_Evaluate`. For a
    /// linear spline that is a trailing duplicate of the destination; for a
    /// catmull-rom one there is also a **leading** phantom at
    /// `controls[0].lerp(controls[1], -1)` — a point extrapolated backwards past
    /// the start, which would fling the creature away from its path and back
    /// again if it were walked.
    ///
    /// Reading this block at all is what stops a creature that streams into view
    /// mid-walk from standing frozen and then teleporting when its next spline
    /// arrives.
    #[test]
    fn a_create_blocks_spline_is_taken_up_where_the_server_already_is() {
        let write_spline = |w: &mut Writer, flags: u32, points: &[[f32; 3]]| {
            w.u32(flags);
            w.u32(1500); // timePassed
            w.u32(4000); // duration
            w.u32(7); // spline id
            w.u32(points.len() as u32);
            for p in points {
                w.f32(p[0]).f32(p[1]).f32(p[2]);
            }
            w.f32(0.0).f32(0.0).f32(0.0); // final destination
        };

        // Linear: points = [c0, c1, c2, phantom(=c2)].
        let mut w = Writer::new();
        write_spline(
            &mut w,
            0x100,
            &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [20.0, 0.0, 0.0], [20.0, 0.0, 0.0]],
        );
        let spline = read_spline(&mut Reader::new(&w.buf)).expect("a spline");
        assert_eq!(spline.elapsed_ms, 1500, "the server's own progress was dropped");
        assert_eq!(spline.duration_ms, 4000);
        assert_eq!(
            spline.path,
            vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [20.0, 0.0, 0.0]],
            "the trailing phantom was walked"
        );

        // Catmull-rom: points = [phantom, c0, c1, phantom]. The leading one is
        // extrapolated *backwards* past the start, so walking it is not a
        // harmless zero-length leg — it is a detour away from the path.
        let mut w = Writer::new();
        write_spline(
            &mut w,
            0x100 | crate::state::movement::spline_flags::CATMULLROM,
            &[[-10.0, 0.0, 0.0], [0.0, 0.0, 0.0], [20.0, 0.0, 0.0], [20.0, 0.0, 0.0]],
        );
        let spline = read_spline(&mut Reader::new(&w.buf)).expect("a spline");
        assert_eq!(
            spline.path,
            vec![[0.0, 0.0, 0.0], [20.0, 0.0, 0.0]],
            "a phantom control point was walked"
        );
    }

    /// A truncated spline block still has to be *detected*, because everything
    /// after it in the packet is then untrustworthy — that is what turns into a
    /// bogus "unknown update type" several blocks later.
    #[test]
    fn a_truncated_spline_block_is_reported_rather_than_guessed_at() {
        let mut w = Writer::new();
        w.u32(0x100);
        w.u32(1500);
        w.u32(4000);
        w.u32(7);
        w.u32(9); // claims nine nodes and carries none
        assert!(read_spline(&mut Reader::new(&w.buf)).is_none());
    }

    #[test]
    fn has_position_movement_is_read_without_living_fields() {
        let mut w = Writer::new();
        w.u8(update_flags::HAS_POSITION);
        for f in [1.0f32, 2.0, 3.0, 0.5] {
            w.u32(f.to_bits());
        }
        let mut r = Reader::new(&w.buf);
        let m = parse_movement(&mut r);
        let p = m.position.expect("position");
        assert_eq!((p.x, p.y, p.z), (1.0, 2.0, 3.0));
        assert!(m.speeds.is_none());
    }

    /// **`UPDATEFLAG_TRANSPORT`'s trailing word is read, and it is read last.**
    ///
    /// The whole of what the wire ever says about an elevator, and it sits
    /// behind two optional fields that a transport does not carry — so a reader
    /// that had the order wrong would consume a victim guid as the progress and
    /// desynchronise everything after it in the packet. Written with the
    /// melee-attacking field in front of it for exactly that reason.
    ///
    /// The position beside it is the **stationary** point, not where the
    /// platform is now (`Object::BuildMovementUpdate` writes
    /// `GetStationaryX/Y/Z` under this flag), which is what makes it the base an
    /// offset is added to.
    #[test]
    fn a_transports_path_progress_is_the_last_word_in_the_block() {
        let mut w = Writer::new();
        w.u8(update_flags::HAS_POSITION | update_flags::MELEE_ATTACKING | update_flags::TRANSPORT);
        for f in [10.0f32, 20.0, 30.0, 1.5] {
            w.u32(f.to_bits());
        }
        w.u8(0); // an empty packed victim guid
        w.u32(17_500);
        let m = parse_movement(&mut Reader::new(&w.buf));
        let p = m.position.expect("the stationary position");
        assert_eq!((p.x, p.y, p.z), (10.0, 20.0, 30.0));
        assert_eq!(m.path_progress, Some(17_500));
    }

    /// …and everything that is not a transport says nothing about a phase,
    /// which is what keeps `Entity::transport_phase_ms` `None` for every object
    /// in the world but a few dozen.
    #[test]
    fn a_block_without_the_flag_carries_no_phase() {
        let mut w = Writer::new();
        w.u8(update_flags::HAS_POSITION);
        for f in [1.0f32, 2.0, 3.0, 0.5] {
            w.u32(f.to_bits());
        }
        assert_eq!(parse_movement(&mut Reader::new(&w.buf)).path_progress, None);
    }
}
