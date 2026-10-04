//! `WorldSafeLocs.dbc`: the places a dead character's spirit appears.
//!
//! A row is a place on a map with a name. The server picks one when a
//! character releases: `ObjectMgr::GetClosestGraveYard` takes the rows linked
//! to the zone the corpse is in by `game_graveyard_zone`, and the nearest of
//! those for the character's side. The client reads the file too, for the
//! position it is told to fly the spirit to.
//!
//! ```text
//! WorldSafeLocs.dbc    122 rows x 14 fields, 56 bytes a record
//!   [ 0] id
//!   [ 1] map
//!   [ 2..4] x, y, z            in the server's axes: north, west, up
//!   [ 5] name                  the enUS name, then seven locale columns
//!   [13] name flags
//! ```
//!
//! vmangos reads this file from `DataDir\5875\dbc\` (`DBCStores.cpp:418`), not
//! from a SQL table, so an edited copy reaches the server as a file, copied by
//! a publish. Which zone uses which place is the server's `game_graveyard_zone`
//! table, and which way the spirit faces is `world_safe_locs_facing`; neither
//! is in the file.

use crate::tables::dbc::Dbc;

/// The field indices.
pub mod fields {
    pub const ID: usize = 0;
    pub const MAP: usize = 1;
    pub const X: usize = 2;
    pub const Y: usize = 3;
    pub const Z: usize = 4;
    pub const NAME: usize = 5;
    pub const NAME_FLAGS: usize = 13;
    /// How many fields a record has.
    pub const COUNT: usize = 14;
}

/// One row.
#[derive(Debug, Clone, PartialEq)]
pub struct SafeLoc {
    pub id: u32,
    pub map: u32,
    /// x, y, z in the server's axes: north, west, up.
    pub at: [f32; 3],
    pub name: String,
}

/// Every row of the file, in file order. A file that does not parse reads as
/// no rows.
pub fn read(raw: &[u8]) -> Vec<SafeLoc> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return Vec::new();
    };
    (0..dbc.record_count)
        .filter_map(|record| {
            let float = |field: usize| dbc.u32_at(record, field).map(f32::from_bits);
            Some(SafeLoc {
                id: dbc.u32_at(record, fields::ID)?,
                map: dbc.u32_at(record, fields::MAP)?,
                at: [float(fields::X)?, float(fields::Y)?, float(fields::Z)?],
                name: dbc.string_at(record, fields::NAME).unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing;

    #[test]
    fn a_row_reads_its_place_and_name() {
        let strings = b"\0Redridge Mountains\0";
        let mut row = vec![0u32; fields::COUNT];
        row[fields::ID] = 2;
        row[fields::X] = (-9271.0f32).to_bits();
        row[fields::Y] = (-2305.0f32).to_bits();
        row[fields::Z] = 71.0f32.to_bits();
        row[fields::NAME] = 1;
        let rows = read(&testing::dbc(&[row], fields::COUNT, strings));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, 2);
        assert_eq!(rows[0].map, 0);
        assert_eq!(rows[0].at, [-9271.0, -2305.0, 71.0]);
        assert_eq!(rows[0].name, "Redridge Mountains");
        assert!(read(b"not a dbc").is_empty());
    }
}
