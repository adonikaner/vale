//! `Map.dbc`: the maps, by id, and the folder each one's files are in.
//!
//! ```text
//! Map.dbc    44 rows x 42 fields, 168 bytes a record
//!   [ 0] id
//!   [ 1] directory          World\Maps\<directory>\<directory>.wdt, and the
//!                           prefix of every tile's file name
//!   [ 2] instance type      0 world, 1 dungeon, 2 raid, 3 battleground
//!   [ 3] PvP                1 on four rows
//!   [ 4] name               the enUS name, then seven locale columns
//!   [12] name flags
//!   [13] min level, [14] max level, [15] max players
//!   [19] area               an AreaTable row
//!   [20] description 0, [29] description 1, each with seven locales and flags
//!   [38] loading screen     a LoadingScreens row; see `super::loading`
//! ```
//!
//! The counts are measured over the shipped file; the meaning of fields 16 to
//! 18 and 39 to 41 is not known and their values are stated in
//! `super::schema::MAP`.
//!
//! vmangos does not read this file. `DBCfmt.h` still defines a format for it
//! (`MapEntryfmt`) that nothing loads; the server's maps are its own
//! `map_template` table (`SQLStorages.cpp:43`), which holds the type, the
//! player limit, the area and where a ghost enters. A new map needs a row in
//! both: this file for the client, that table for the server.
//!
//! ## What a new map's folder may be called
//!
//! [`directory_problem`] is the rule. The directory is a path segment of every
//! file the map has, in the archives and in the server's own extracted tiles,
//! so it is held to ASCII letters, digits and underscores, starting with a
//! letter. The archives compare paths without regard to case, so two maps
//! whose directories differ only in case would read each other's tiles.

use crate::tables::dbc::Dbc;

/// The field indices.
pub mod fields {
    pub const ID: usize = 0;
    pub const DIRECTORY: usize = 1;
    pub const INSTANCE_TYPE: usize = 2;
    pub const PVP: usize = 3;
    pub const NAME: usize = 4;
    pub const NAME_FLAGS: usize = 12;
    pub const MIN_LEVEL: usize = 13;
    pub const MAX_LEVEL: usize = 14;
    pub const MAX_PLAYERS: usize = 15;
    /// -1 on 41 of the 44 shipped rows and 0 on the other three.
    pub const FIELD_16: usize = 16;
    pub const AREA: usize = 19;
    pub const DESCRIPTION_0: usize = 20;
    pub const DESCRIPTION_1: usize = 29;
    /// The two descriptions' locale flags, after their eight columns each.
    pub const DESCRIPTION_0_FLAGS: usize = 28;
    pub const DESCRIPTION_1_FLAGS: usize = 37;
    pub const LOADING_SCREEN: usize = 38;
    /// 1 on 43 of the 44 shipped rows.
    pub const FIELD_40: usize = 40;
    /// An `f32`, 1.0 on 43 of the 44 shipped rows.
    pub const FIELD_41: usize = 41;
    /// How many fields a record has.
    pub const COUNT: usize = 42;
}

/// What kind of place a map is: field 2. vmangos' `map_template.map_type`
/// takes the same values (`MAP_COMMON`, `MAP_INSTANCE`, `MAP_RAID`,
/// `MAP_BATTLEGROUND`).
pub const INSTANCE_TYPES: [(u32, &str); 4] =
    [(0, "World"), (1, "Dungeon"), (2, "Raid"), (3, "Battleground")];

/// The name flags 36 of the 37 shipped rows carry in field 12.
pub const SHIPPED_NAME_FLAGS: u32 = 4_128_894;

/// The flags most shipped rows carry for a description that is empty: fields
/// 28 and 37 on 29 of the 37.
pub const EMPTY_DESCRIPTION_FLAGS: u32 = 4_128_892;

/// The largest id a new map may take. The server's extracted tiles are named
/// `maps/%03u%02u%02u.map` (vmangos' `GridMap.cpp:641` and its extractor), so
/// an id of 1,000 or more prints a fourth digit and the names stop being fixed
/// width. The shipped ids stop at 533.
pub const MAX_NEW_ID: u32 = 999;

/// The longest directory a new map is given. No shipped one is longer than 24
/// characters; the limit keeps a tile's file name within what every archive
/// tool accepts.
pub const MAX_DIRECTORY: usize = 64;

/// Why `directory` cannot be a new map's folder, or `None` when it can.
/// `taken` is every directory the table already holds.
pub fn directory_problem<'a>(directory: &str, taken: impl IntoIterator<Item = &'a str>) -> Option<String> {
    if directory.is_empty() {
        return Some("a map needs a directory".to_string());
    }
    if directory.len() > MAX_DIRECTORY {
        return Some(format!("a directory is at most {MAX_DIRECTORY} characters"));
    }
    if !directory.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Some("a directory starts with a letter".to_string());
    }
    if !directory.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Some("a directory holds only letters, digits and underscores".to_string());
    }
    if taken.into_iter().any(|had| had.eq_ignore_ascii_case(directory)) {
        return Some(format!(
            "{directory} is already a map's directory; the archives do not tell two apart by case"
        ));
    }
    None
}

/// The id a new map takes: one past the largest in the table, or `None` when
/// that is past [`MAX_NEW_ID`].
pub fn next_id(ids: impl IntoIterator<Item = u32>) -> Option<u32> {
    let next = ids.into_iter().max().map_or(0, |max| max + 1);
    (next <= MAX_NEW_ID).then_some(next)
}

/// Every map's id and directory, in file order. A file that does not parse
/// reads as no rows.
pub fn directories(raw: &[u8]) -> Vec<(u32, String)> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return Vec::new();
    };
    (0..dbc.record_count)
        .filter_map(|record| {
            Some((
                dbc.u32_at(record, fields::ID)?,
                dbc.string_at(record, fields::DIRECTORY)?,
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_is_a_plain_unused_name() {
        let taken = ["Azeroth", "DeadminesInstance"];
        assert_eq!(directory_problem("Islands", taken), None);
        assert_eq!(directory_problem("Isle_2", taken), None);
        assert!(directory_problem("", taken).is_some());
        assert!(directory_problem("2Isle", taken).is_some());
        assert!(directory_problem("Isle of", taken).is_some());
        assert!(directory_problem("Isle\\x", taken).is_some());
        assert!(directory_problem("azeroth", taken).unwrap().contains("by case"));
        assert!(directory_problem(&"a".repeat(MAX_DIRECTORY + 1), taken).is_some());
    }

    #[test]
    fn a_new_id_follows_the_largest_and_stays_under_a_thousand() {
        assert_eq!(next_id([0, 1, 533]), Some(534));
        assert_eq!(next_id([]), Some(0));
        assert_eq!(next_id([998]), Some(999));
        assert_eq!(next_id([999]), None);
    }
}
