//! **The dots on the minimap** — which units and objects get one, which cell
//! of the sheet each takes, and where inside the disc it lands.
//!
//! Nothing here crosses the wire and no file states any of it, so all of it is
//! the client's own rule. The sheet is
//! `Interface\Minimap\ObjectIcons.blp`, 128x128, and the client reads it as a
//! **4x4 grid of 32-texel cells**: five uv records, one per
//! blip kind, from `column = kind & 3`, `row = kind >> 2`, each scaled by
//! `0.25`. (The 16-texel `POIIcons` cells are a different sheet's 8x8 grid,
//! and not the blips'.)
//! The five cells are the five dots the picture actually holds: yellow, red,
//! green and yellow across the top row, and blue at the start of the second.
//!
//! ## Five lists, rebuilt every update
//!
//! The client keeps one record list per kind and refills them on every
//! minimap update: all five are emptied, then the object manager is walked
//! with one callback. The callback is the rule, and it is short:
//!
//! ```text
//! the local player is skipped
//! OBJECT_FIELD_TYPE 0x21 (game object)   -> game object
//! OBJECT_FIELD_TYPE 0x19 (player)        -> unit, but skipped if in
//!                                           my group — those
//!                                           come from the party list
//! OBJECT_FIELD_TYPE 0x09 (unit)          -> unit
//!
//! game object:
//! see [`resource_kind`]                                  -> kind 0
//!
//! unit or player:
//! UNIT_FIELD_CHARMEDBY, else UNIT_FIELD_SUMMONEDBY == me -> skipped
//! UNIT_FIELD_HEALTH <= 0                                 -> skipped
//! UNIT_FIELD_BYTES_1 byte 3 & 4 (UNTRACKABLE)            -> skipped
//! the quest-giver status this client last heard == 7     -> kind 3
//! see [`tracked_unit`]                                   -> kind 1
//!
//! party:
//! the five party members, whichever carry a position     -> kind 4
//! ```
//!
//! **Kind 2 — the green dot — is never produced by 5875.** Every append site
//! is the one function above and it writes 0, 1, 3 and 4; the green cell is on
//! the sheet and on no list. It is kept in the enum so the cell arithmetic is
//! the client's, and named so nobody adds a rule to fill it.
//!
//! The status test reads the slot `SMSG_QUESTGIVER_STATUS`
//! writes (gated on `UNIT_NPC_FLAG_QUESTGIVER`), and 7 is
//! `DIALOG_STATUS_REWARD2` — a quest to hand in. vmangos' own enum comment on
//! that value reads "yellow dot on minimap", which is what pins the cell: kind 3
//! is the top row's fourth cell, and it is yellow. The same comment calls 6 a
//! "red dot", and the client tests no such value; a status-6 giver that is not
//! otherwise tracked draws nothing here, as it draws nothing there.
//!
//! ## What the party dot is drawn at
//!
//! The client has five `(scale, colour)` pairs, one per kind: every scale is
//! `1.0` except the party's, which is `1.3`, so a group member's dot is drawn
//! a third larger. The colours beside them are not the dots' — those come off
//! the sheet — and are not read here.
//!
//! ## What is stated rather than measured
//!
//! * **The size of a dot.** The reference's quad size was not measured.
//!   [`SIZE`] is taste: a dot
//!   about a fourteenth of the frame across, which is what a 32-texel cell
//!   looks like on the reference's 140-unit frame.
//! * **A blip on a different transport from the character is drawn dimmed**
//!   (`0xffb0b0b0`). Not drawn here; every dot is full white.
//! * **The hover.** The reference names the blips under the pointer in a
//!   tooltip. Not answered.

use crate::tables::lock::{Key, KeyKind};

/// The sheet the dots are cut from — the widget's `SetBlipTexture` default,
/// and the texture the client loads.
pub const SHEET: &str = r"Interface\Minimap\ObjectIcons";

/// The sheet is read as this many cells a side — the `& 3` and `>> 2`, and
/// the `0.25`.
pub const GRID: u32 = 4;

/// How wide a dot is drawn, as a fraction of the frame's smaller side. Stated
/// rather than measured — see the module comment.
pub const SIZE: f32 = 1.0 / 14.0;

/// The five kinds, numbered as the client's five lists are — and the
/// number is the cell: `column = kind & 3`, `row = kind >> 2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Kind {
    /// A herb, a vein or a chest whose lock skill is in
    /// `PLAYER_TRACK_RESOURCES`. Yellow.
    Resource = 0,
    /// A unit `PLAYER_TRACK_CREATURES` covers, a hunter's-marked one, or a
    /// stealthed one the character can see. Red.
    Tracked = 1,
    /// On the sheet and on no list — see the module comment. Green.
    Unused = 2,
    /// A quest giver with a quest to hand in. Yellow.
    QuestTurnIn = 3,
    /// A member of the character's own group. Blue, and a third larger.
    Party = 4,
}

impl Kind {
    /// The cell, as `[u0, v0, u1, v1]` on the sheet.
    pub fn uv(self) -> [f32; 4] {
        let cell = 1.0 / GRID as f32;
        let column = (self as u32 & (GRID - 1)) as f32;
        let row = (self as u32 / GRID) as f32;
        [column * cell, row * cell, (column + 1.0) * cell, (row + 1.0) * cell]
    }

    /// The per-kind scale: `1.3` for the party, `1.0` for the rest.
    pub fn scale(self) -> f32 {
        match self {
            Kind::Party => 1.3,
            _ => 1.0,
        }
    }
}

/// **What the character is tracking**, off their own three fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tracking {
    /// `PLAYER_TRACK_CREATURES` (1104): bit `type - 1` for a `CreatureType.dbc`
    /// id, written by `SPELL_AURA_TRACK_CREATURES`.
    pub creatures: u32,
    /// `PLAYER_TRACK_RESOURCES` (1105): bit `lockType - 1` for a `LockType.dbc`
    /// id, written by `SPELL_AURA_TRACK_RESOURCES`.
    pub resources: u32,
    /// `PLAYER_FIELD_BYTES` byte 0, bit `0x02` — `PLAYER_FIELD_BYTE_TRACK_STEALTHED`.
    pub stealthed: bool,
}

/// `UNIT_DYNAMIC_FLAGS` bit 1 — `UNIT_DYNFLAG_TRACK_UNIT`, a hunter's mark.
pub const UNIT_DYNFLAG_TRACK_UNIT: u32 = 0x0002;

/// What the unit rule reads off one unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unit {
    /// The `CreatureType.dbc` id off the creature cache; 0 for a player, and for
    /// a creature nobody has queried yet.
    pub creature_type: u32,
    /// `UNIT_FIELD_BYTES_1`'s fourth byte.
    pub vis_flags: u8,
    /// [`UNIT_DYNFLAG_TRACK_UNIT`] is set.
    pub hunters_marked: bool,
    /// `UNIT_FIELD_HEALTH > 0`.
    pub alive: bool,
    /// `UNIT_FIELD_CHARMEDBY`, or when that is zero `UNIT_FIELD_SUMMONEDBY`,
    /// is the character's own guid — their pet, their totem, their minion.
    pub mine: bool,
    /// The last `SMSG_QUESTGIVER_STATUS` for this unit was `DIALOG_STATUS_REWARD2`.
    pub quest_turn_in: bool,
}

/// **Is this unit tracked?** Three doors, tried in this order:
///
/// ```text
/// vis flags & CREEP, and PLAYER_FIELD_BYTES & TRACK_STEALTHED  -> yes
/// UNIT_DYNAMIC_FLAGS & TRACK_UNIT                             -> yes
/// creature type 0 (a player, or an unqueried creature)         -> no
/// PLAYER_TRACK_CREATURES & (1 << (type - 1))                   -> the answer
/// ```
///
/// The creature-type mask is read only for the local player (the client
/// compares the asking object's guid with the player's); for anybody else it
/// is zero, which is the branch this client never takes.
pub fn tracked_unit(tracking: &Tracking, unit: &Unit) -> bool {
    const UNIT_VIS_FLAGS_CREEP: u8 = 0x02;
    if unit.vis_flags & UNIT_VIS_FLAGS_CREEP != 0 && tracking.stealthed {
        return true;
    }
    if unit.hunters_marked {
        return true;
    }
    if unit.creature_type == 0 || unit.creature_type > 32 {
        return false;
    }
    tracking.creatures & (1 << (unit.creature_type - 1)) != 0
}

/// **The unit branch of the minimap callback**: which dot a unit or a player
/// gets, or none. The local player, and a player in the character's own
/// group, are the caller's to leave out.
pub fn unit_kind(tracking: &Tracking, unit: &Unit) -> Option<Kind> {
    const UNIT_VIS_FLAGS_UNTRACKABLE: u8 = 0x04;
    if unit.mine || !unit.alive || unit.vis_flags & UNIT_VIS_FLAGS_UNTRACKABLE != 0 {
        return None;
    }
    if unit.quest_turn_in {
        return Some(Kind::QuestTurnIn);
    }
    tracked_unit(tracking, unit).then_some(Kind::Tracked)
}

/// **Is this game object a tracked resource?** Its lock's eight
/// slots are walked, and a slot whose type is `LOCK_KEY_SKILL`
/// (2) answers yes when `PLAYER_TRACK_RESOURCES` carries bit `index - 1` —
/// `LockType.dbc`'s Herbalism is 2 and Mining is 3, which is what Find Herbs
/// and Find Minerals write.
///
/// `keys` is the lock row out of [`crate::tables::lock::Locks::keys`]; an
/// object with no lock, or a lock with no skill slot, is never a resource.
pub fn resource_kind(tracking: &Tracking, keys: &[Key]) -> Option<Kind> {
    keys.iter()
        .any(|key| match key.kind {
            KeyKind::Skill { lock_type, .. } => {
                lock_type >= 1 && lock_type <= 32 && tracking.resources & (1 << (lock_type - 1)) != 0
            }
            KeyKind::Item(_) => false,
        })
        .then_some(Kind::Resource)
}

/// **Where another position lands on the disc**, as `(u, v)` in `0..1` from
/// the frame's top left — or `None` beyond the radius, which is what the
/// round mask would have cut anyway.
///
/// North (`+x`) is up and west (`+y`) is left, the same orientation
/// [`crate::tables::minimap::tiles_in_view`] places the pictures in.
pub fn on_disc(player: (f32, f32), other: (f32, f32), radius: f32) -> Option<(f32, f32)> {
    if !(radius > 0.0) {
        return None;
    }
    let north = other.0 - player.0;
    let west = other.1 - player.1;
    if north * north + west * west > radius * radius {
        return None;
    }
    Some((0.5 - west / (2.0 * radius), 0.5 - north / (2.0 * radius)))
}

/// **The world heading from the character to a place**, in the server's own
/// radians — `0` is north (`+x`), growing towards west (`+y`) — which is the
/// number an edge arrow is turned by, through the same
/// `arrow_angle` the player's own arrow uses.
pub fn bearing(player: (f32, f32), other: (f32, f32)) -> f32 {
    (other.1 - player.1).atan2(other.0 - player.0)
}

/// **The markers' sheet** — `Interface\Minimap\POIIcons.blp`, 128x128, read as
/// an 8x8 grid of 16-texel cells: `WorldMap_GetPOITextureCoords` in
/// `WorldMapFrame.lua` does the arithmetic for the parchment, and the minimap
/// draws the same cell. See [`crate::tables::areapoi`].
pub const POI_SHEET: &str = r"Interface\Minimap\POIIcons";

/// …and its cell count a side.
pub const POI_GRID: u32 = 8;

/// The cell of [`POI_SHEET`] a marker index names, as `[u0, v0, u1, v1]`.
pub fn poi_uv(index: u32) -> [f32; 4] {
    let cell = 1.0 / POI_GRID as f32;
    let column = (index % POI_GRID) as f32;
    let row = ((index / POI_GRID) % POI_GRID) as f32;
    [column * cell, row * cell, (column + 1.0) * cell, (row + 1.0) * cell]
}

/// The cell a ghost's own body is drawn as when it is inside the disc — the
/// plain tombstone at the end of the sheet's first row. **Taste**: the
/// reference's corpse marker on the minimap was not traced to a cell.
pub const CORPSE_CELL: u32 = 7;

/// The arrow drawn at the rim for a place beyond it. Two are shipped, both
/// 32x32 and both pointing up: the widget's own `minimapArrowModel`
/// (`Interface\Minimap\Rotating-MinimapArrow.mdl`, grey) and the gold
/// `Rotating-MinimapGuideArrow` beside it. **A reading, not a measurement**:
/// the gold one *guides*, so it points at the body; the grey one points at the
/// guard's flag.
pub const POI_ARROW: &str = r"Interface\Minimap\Rotating-MinimapArrow";
pub const GUIDE_ARROW: &str = r"Interface\Minimap\Rotating-MinimapGuideArrow";

/// How wide a rim arrow is drawn, as a fraction of the frame's smaller side —
/// the sheet's 32 texels on the reference's 140-unit frame.
pub const ARROW_SIZE: f32 = 32.0 / 140.0;

/// **How far along the disc's radius an edge arrow sits**, as a fraction —
/// inside the rim rather than on it, so the whole arrow shows. Taste.
pub const EDGE: f32 = 0.86;

#[cfg(test)]
mod tests {
    use super::*;

    /// The five cells are the five dots the sheet holds: the top row's four
    /// and the second row's first.
    #[test]
    fn the_five_kinds_take_the_five_painted_cells() {
        assert_eq!(Kind::Resource.uv(), [0.0, 0.0, 0.25, 0.25]);
        assert_eq!(Kind::Tracked.uv(), [0.25, 0.0, 0.5, 0.25]);
        assert_eq!(Kind::Unused.uv(), [0.5, 0.0, 0.75, 0.25]);
        assert_eq!(Kind::QuestTurnIn.uv(), [0.75, 0.0, 1.0, 0.25]);
        assert_eq!(Kind::Party.uv(), [0.0, 0.25, 0.25, 0.5]);
        assert_eq!(Kind::Party.scale(), 1.3);
    }

    /// [`tracked_unit`]'s three doors, and the creature-type mask's bit arithmetic.
    #[test]
    fn a_unit_is_tracked_by_stealth_by_mark_or_by_type() {
        let tracking = Tracking { creatures: 1 << (7 - 1), resources: 0, stealthed: false };
        let humanoid = Unit { creature_type: 7, alive: true, ..Unit::default() };
        let beast = Unit { creature_type: 1, alive: true, ..Unit::default() };
        assert!(tracked_unit(&tracking, &humanoid), "Track Humanoids covers type 7");
        assert!(!tracked_unit(&tracking, &beast));
        assert!(!tracked_unit(&tracking, &Unit { creature_type: 0, ..humanoid }), "a player has no type");
        assert!(tracked_unit(&tracking, &Unit { hunters_marked: true, ..beast }));
        let creeping = Unit { vis_flags: 0x02, ..beast };
        assert!(!tracked_unit(&tracking, &creeping));
        assert!(tracked_unit(&Tracking { stealthed: true, ..tracking }, &creeping));
    }

    /// The four refusals before the kind, and the quest giver ahead of the mask.
    #[test]
    fn the_unit_branch_refuses_then_chooses() {
        let tracking = Tracking { creatures: !0, resources: 0, stealthed: false };
        let unit = Unit { creature_type: 7, alive: true, ..Unit::default() };
        assert_eq!(unit_kind(&tracking, &unit), Some(Kind::Tracked));
        assert_eq!(unit_kind(&tracking, &Unit { mine: true, ..unit }), None, "my own pet");
        assert_eq!(unit_kind(&tracking, &Unit { alive: false, ..unit }), None);
        assert_eq!(unit_kind(&tracking, &Unit { vis_flags: 0x04, ..unit }), None, "untrackable");
        assert_eq!(
            unit_kind(&Tracking::default(), &Unit { quest_turn_in: true, creature_type: 0, ..unit }),
            Some(Kind::QuestTurnIn),
            "a hand-in needs no tracking at all"
        );
        assert_eq!(
            unit_kind(&tracking, &Unit { quest_turn_in: true, ..unit }),
            Some(Kind::QuestTurnIn),
            "…and outranks the mask"
        );
    }

    /// A vein under Find Minerals, and the same vein under Find Herbs.
    #[test]
    fn a_resource_is_its_locks_skill_against_the_mask() {
        let mining = [Key { kind: KeyKind::Skill { lock_type: 3, rank: 1 }, slot: 0, action: 0 }];
        let herbs = Tracking { resources: 1 << (2 - 1), ..Tracking::default() };
        let minerals = Tracking { resources: 1 << (3 - 1), ..Tracking::default() };
        assert_eq!(resource_kind(&minerals, &mining), Some(Kind::Resource));
        assert_eq!(resource_kind(&herbs, &mining), None);
        assert_eq!(resource_kind(&minerals, &[]), None, "no lock, no dot");
        let keyed = [Key { kind: KeyKind::Item(1), slot: 0, action: 0 }];
        assert_eq!(resource_kind(&minerals, &keyed), None, "an item lock is not a skill");
    }

    /// The parchment's own arithmetic: eight 16-texel cells a row.
    #[test]
    fn a_poi_cell_is_the_parchments_own() {
        assert_eq!(poi_uv(0), [0.0, 0.0, 0.125, 0.125]);
        assert_eq!(poi_uv(7), [0.875, 0.0, 1.0, 0.125]);
        assert_eq!(poi_uv(9), [0.125, 0.125, 0.25, 0.25]);
    }

    /// North is up, west is left, and the rim is the radius.
    #[test]
    fn the_disc_is_north_up_and_west_left() {
        let me = (100.0, 100.0);
        assert_eq!(on_disc(me, me, 100.0), Some((0.5, 0.5)));
        assert_eq!(on_disc(me, (150.0, 100.0), 100.0), Some((0.5, 0.25)), "north is up");
        assert_eq!(on_disc(me, (100.0, 150.0), 100.0), Some((0.25, 0.5)), "west is left");
        assert_eq!(on_disc(me, (300.0, 100.0), 100.0), None, "past the rim");
        assert!((bearing(me, (150.0, 100.0))).abs() < 1e-6, "north is a facing of 0");
        assert!((bearing(me, (100.0, 150.0)) - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }
}
