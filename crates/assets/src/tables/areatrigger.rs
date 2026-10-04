//! `AreaTrigger.dbc`: the volumes the client reports standing in.
//!
//! An instance portal is not a game object, a spell or anything the server
//! sends: it is a sphere or a box in this shipped table. Entering a dungeon
//! starts when the client notices it is inside one and says so.
//! `CMSG_AREATRIGGER` carries a single `u32`, the row id, and the server
//! decides what that means (a teleport, a tavern's rest state, a quest's "area
//! explored", a battleground's flag capture). A client that never sends it
//! walks through the portal at the Deadmines and nothing happens, with no
//! packet missing on the wire.
//!
//! The rules below are the 1.12.1 client's, and no other source states them:
//! the server only validates what it is told
//! (`WorldSession::HandleAreaTriggerOpcode` re-runs the same containment test
//! with a 5-yard tolerance and drops the packet if it fails), and the file
//! carries the volumes without saying how they are polled.
//!
//! ## The layout
//!
//! ```text
//! AreaTrigger.dbc      432 rows x 10 fields, 40 bytes a record
//!   [ 0] id
//!   [ 1] mapId
//!   [ 2..4] x, y, z          the centre, in the server's own axes
//!   [ 5] radius              > 0 makes it a sphere; the box fields are then 0
//!   [ 6..8] boxLength, boxWidth, boxHeight    full lengths, not half
//!   [ 9] boxYaw              radians, about +Z
//! ```
//!
//! Measured over the shipped file: 352 of the 432 rows are spheres (radius 1.0
//! to 150.0) and the other 80 are boxes, none with a zero extent. The
//! `radius > 0` test therefore partitions the table cleanly, and a reader that
//! got the two the wrong way round would reduce every trigger to a point.
//!
//! ## When the client tests the triggers
//!
//! The test runs on a 100 ms timer, not every frame. This client draws at two
//! to three times 1.12's rate, and a test on the frame would send the packet
//! from a different position than 1.12 does.
//!
//! Three rules decide which trigger is sent:
//!
//! * Only the current map's block of rows is tested. The client finds the
//!   first row whose map matches and stops at the first row past it, so a row
//!   of that map outside the block is never tested. The shipped file is sorted
//!   (133 rows on map 0, 121 on map 1, then the instances in ascending id
//!   order). [`AreaTriggers`] groups by map explicitly rather than relying on
//!   the order, since a grouping that depended on it would answer "no triggers
//!   on this map" when the order was wrong.
//!
//! * The client remembers the trigger it last sent, and each tick first tests
//!   that row alone: while the player is still inside it, nothing else is
//!   tested and nothing is sent. Leaving it clears the latch and lets any
//!   trigger fire again. Without the latch a portal would send ten packets a
//!   second for as long as the player stood in it; the Deadmines' 7-yard sphere
//!   takes a couple of seconds to walk across.
//!
//! * The first row containing the point is sent, not the nearest, and no
//!   later row is tested.
//!
//! A map change clears the latch, takes the new map's block and restarts the
//! timer, so the trigger a teleport lands the player in is not held out by the
//! latch.
//!
//! ## The two volumes
//!
//! Both tests first refuse a row on another map. A sphere contains the point
//! when `dist² <= radius²` in three dimensions, closed. A box contains it when
//! the point, taken into the box's own frame (relative to the centre and
//! turned by `-boxYaw` about up), lies strictly inside half of each declared
//! length: it fails on `<= -half` and on `>= +half`. vmangos'
//! `IsPointInAreaTriggerZone` is the same test with the rotation written as
//! `2π - boxYaw` (identical) and the bound as `fabs(d) > half` (closed rather
//! than open, which differs only for a point exactly on a face).
//!
//! One step is not modelled because it is not identified: before the packet
//! goes out the client does something per unit with the value 238. 238 is not
//! an `AnimationData` id (that table stops at 207), an `Emotes` id, or a
//! `SoundEntries` id that fits here.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// The field indices.
pub mod fields {
    pub const ID: usize = 0;
    pub const MAP: usize = 1;
    pub const X: usize = 2;
    pub const Y: usize = 3;
    pub const Z: usize = 4;
    pub const RADIUS: usize = 5;
    pub const BOX_LENGTH: usize = 6;
    pub const BOX_WIDTH: usize = 7;
    pub const BOX_HEIGHT: usize = 8;
    pub const BOX_YAW: usize = 9;
    /// How many fields a record has.
    pub const COUNT: usize = 10;
}

/// One row: a volume on one map, and the id to report from inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaTrigger {
    pub id: u32,
    pub map: u32,
    /// The centre, in the server's own axes — the same frame
    /// `vale_protocol::state::update::Position` is in, so no conversion happens
    /// anywhere on this path.
    pub position: [f32; 3],
    /// A sphere's radius. Zero (or less) means this row is a box instead; see
    /// the module note for why the discriminator is trustworthy.
    pub radius: f32,
    /// Full lengths along the box's own x, y and z, halved at the test.
    pub extent: [f32; 3],
    /// The box's yaw about +Z, in radians.
    pub yaw: f32,
}

impl AreaTrigger {
    /// Whether this row is a sphere: a radius above zero. A row whose radius
    /// is zero or less is a box.
    pub fn is_sphere(&self) -> bool {
        self.radius > 0.0
    }

    /// Whether `point` is inside this volume, on `map`.
    ///
    /// The client's rule, including the map refusal it starts with: a trigger's
    /// coordinates mean nothing without its map, and tile coordinates repeat
    /// across continents, so testing the raw position would put the character
    /// inside Blackrock Depths while standing in a field in Elwynn.
    pub fn contains(&self, map: u32, point: [f32; 3]) -> bool {
        if map != self.map {
            return false;
        }
        let d = [
            point[0] - self.position[0],
            point[1] - self.position[1],
            point[2] - self.position[2],
        ];
        if self.is_sphere() {
            // Three dimensions, not two — a trigger directly above another one
            // (the multi-level instance entrances) relies on the z term.
            let dist2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            return dist2 <= self.radius * self.radius;
        }
        // The point taken into the box's own frame: relative to the centre,
        // turned by `-yaw`, which is these two lines.
        let (sin, cos) = self.yaw.sin_cos();
        let local = [d[0] * cos + d[1] * sin, d[1] * cos - d[0] * sin, d[2]];
        let half = [self.extent[0] / 2.0, self.extent[1] / 2.0, self.extent[2] / 2.0];
        // Strict on both sides, as the client compares; see the module note
        // for where vmangos differs and by how much.
        (0..3).all(|i| local[i] > -half[i] && local[i] < half[i])
    }
}

/// The whole table, grouped by map.
#[derive(Debug, Clone, Default)]
pub struct AreaTriggers {
    /// Every row, in the file's order, which is the order the client tests
    /// them in and so decides which of two overlapping triggers is sent.
    rows: Vec<AreaTrigger>,
    /// Map id -> indices into `rows`, ascending. The client tests one
    /// contiguous block per map; this is the same set without assuming the
    /// file is sorted.
    by_map: HashMap<u32, Vec<usize>>,
}

impl AreaTriggers {
    /// Parse `AreaTrigger.dbc`.
    pub fn load(raw: &[u8]) -> Result<AreaTriggers, crate::AssetError> {
        let dbc = Dbc::parse(raw)?;
        let mut rows = Vec::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(record, 0) else {
                continue;
            };
            let Some(map) = dbc.u32_at(record, fields::MAP) else {
                continue;
            };
            let f = |field| dbc.f32_at(record, field).unwrap_or(0.0);
            rows.push(AreaTrigger {
                id,
                map,
                position: [f(fields::X), f(fields::Y), f(fields::Z)],
                radius: f(fields::RADIUS),
                extent: [
                    f(fields::BOX_LENGTH),
                    f(fields::BOX_WIDTH),
                    f(fields::BOX_HEIGHT),
                ],
                yaw: f(fields::BOX_YAW),
            });
        }
        let mut by_map: HashMap<u32, Vec<usize>> = HashMap::new();
        for (index, row) in rows.iter().enumerate() {
            by_map.entry(row.map).or_default().push(index);
        }
        Ok(AreaTriggers { rows, by_map })
    }

    /// Read the table out of the archives at `gamedata_dir`.
    ///
    /// A convenience for the two callers that have a game directory and no
    /// archive chain open at the point they need this — the session starts
    /// before the renderer's assets exist.
    pub fn open(gamedata_dir: &str) -> Result<AreaTriggers, crate::AssetError> {
        AreaTriggers::open_with(gamedata_dir, None)
    }

    /// …with a host's overlay consulted before the archives, as
    /// `MapTerrain::open_with` does. A host that holds an edited copy of the
    /// table in its overlay gets that copy, so a trigger it added is one the
    /// session tests.
    pub fn open_with(
        gamedata_dir: &str,
        overlay: Option<crate::archive::Overlay>,
    ) -> Result<AreaTriggers, crate::AssetError> {
        let mut assets = crate::Assets::open(gamedata_dir)?;
        assets.set_overlay(overlay);
        let raw = assets.read(&crate::tables::dbc::dbc_path("AreaTrigger"))?;
        AreaTriggers::load(&raw)
    }

    /// Every row, in the file's own order.
    pub fn rows(&self) -> &[AreaTrigger] {
        &self.rows
    }

    /// The rows on one map, in the file's own order.
    pub fn on_map(&self, map: u32) -> impl Iterator<Item = &AreaTrigger> {
        self.by_map
            .get(&map)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .map(|&i| &self.rows[i])
    }

    /// The first trigger on `map` containing `point`, in table order. This is
    /// the trigger the client sends, not the nearest one: no row after the
    /// first that contains the point is tested.
    pub fn containing(&self, map: u32, point: [f32; 3]) -> Option<u32> {
        self.on_map(map)
            .find(|row| row.contains(map, point))
            .map(|row| row.id)
    }

    /// Whether one specific trigger still holds `point`: the latch's test,
    /// made before any other row is tested. See the module note.
    pub fn holds(&self, id: u32, map: u32, point: [f32; 3]) -> bool {
        self.rows
            .iter()
            .find(|row| row.id == id)
            .is_some_and(|row| row.contains(map, point))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Deadmines entrance, read out of the shipped file: a 7-yard sphere on
    /// map 0. Used as the fixture rather than an invented one so that a reader
    /// changing the field indices has something real to fail against.
    fn deadmines() -> AreaTrigger {
        AreaTrigger {
            id: 78,
            map: 0,
            position: [-11208.5, 1685.34, 25.761],
            radius: 7.0,
            extent: [0.0; 3],
            yaw: 0.0,
        }
    }

    #[test]
    fn a_sphere_is_measured_in_three_dimensions() {
        let trigger = deadmines();
        assert!(trigger.is_sphere());
        assert!(trigger.contains(0, [-11208.5, 1685.34, 25.761]));
        // Six yards up is inside a seven-yard sphere; ten is not, and would be
        // inside a client that dropped the z term.
        assert!(trigger.contains(0, [-11208.5, 1685.34, 31.761]));
        assert!(!trigger.contains(0, [-11208.5, 1685.34, 35.761]));
    }

    #[test]
    fn a_trigger_on_another_map_is_never_entered() {
        // The same coordinates on Kalimdor. Tile coordinates repeat across
        // continents, so without the map refusal this is a portal in a field.
        assert!(!deadmines().contains(1, [-11208.5, 1685.34, 25.761]));
    }

    #[test]
    fn a_box_is_tested_in_its_own_frame() {
        // A 10 x 2 box turned a quarter turn: the long axis now runs along
        // world y, so a point four yards out along y is inside and the same
        // distance along x is not. An unrotated test gets both backwards.
        let trigger = AreaTrigger {
            id: 1,
            map: 0,
            position: [0.0, 0.0, 0.0],
            radius: 0.0,
            extent: [10.0, 2.0, 4.0],
            yaw: std::f32::consts::FRAC_PI_2,

        };
        assert!(!trigger.is_sphere());
        assert!(trigger.contains(0, [0.0, 4.0, 0.0]));
        assert!(!trigger.contains(0, [4.0, 0.0, 0.0]));
        // …and the short axis still bounds it.
        assert!(trigger.contains(0, [0.9, 0.0, 0.0]));
        assert!(!trigger.contains(0, [1.1, 0.0, 0.0]));
    }

    #[test]
    fn a_box_extent_is_a_full_length_rather_than_a_half() {
        let trigger = AreaTrigger {
            id: 1,
            map: 0,
            position: [0.0, 0.0, 0.0],
            radius: 0.0,
            extent: [10.0, 10.0, 10.0],
            yaw: 0.0,
        };
        // 4.9 yards out is inside a 10-yard box. Reading the extent as a half
        // length would make every box trigger twice its authored size.
        assert!(trigger.contains(0, [4.9, 0.0, 0.0]));
        assert!(!trigger.contains(0, [5.1, 0.0, 0.0]));
    }

    #[test]
    fn the_first_row_in_table_order_wins() {
        // Two concentric triggers. The client stops at the first row that
        // contains the point, so the answer is the earlier row rather than the
        // tighter fit.
        let rows = vec![
            AreaTrigger {
                id: 10,
                map: 0,
                position: [0.0; 3],
                radius: 50.0,
                extent: [0.0; 3],
                yaw: 0.0,
            },
            AreaTrigger {
                id: 11,
                map: 0,
                position: [0.0; 3],
                radius: 5.0,
                extent: [0.0; 3],
                yaw: 0.0,
            },
        ];
        let mut by_map = HashMap::new();
        by_map.insert(0, vec![0, 1]);
        let table = AreaTriggers { rows, by_map };
        assert_eq!(table.containing(0, [0.0; 3]), Some(10));
        assert_eq!(table.containing(1, [0.0; 3]), None);
        assert!(table.holds(11, 0, [0.0; 3]));
        assert!(!table.holds(11, 0, [40.0, 0.0, 0.0]));
    }
}
