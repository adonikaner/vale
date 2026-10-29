//! `creature_display_info_addon`: the row the server needs beside a
//! creature display before it spawns a creature drawn with it.
//!
//! ## Why a new display needs a row here
//!
//! vmangos reads `CreatureDisplayInfo.dbc` as a file (it is one of the tables
//! in [`crate::datadir::SERVER_DBCS`]), and reads a second table of its own for
//! each display: how wide the creature is, how far it reaches in melee, how
//! fast it walks and runs, and its gender. `Creature::InitEntry`
//! (`Creature.cpp`) looks the display up with
//! `ObjectMgr::GetCreatureDisplayInfoRandomGender` and, when the table has no
//! row for it, logs "has no display id data defined in table
//! `creature_display_info_addon`, can't load" and does not spawn the creature.
//! So a display the project adds to `CreatureDisplayInfo.dbc` is unusable on
//! the server until this table has a row for it.
//!
//! ## The row
//!
//! ```text
//! display_id               mediumint unsigned  CreatureDisplayInfo field 0
//! build                    smallint unsigned   5875 for a row this project writes
//! bounding_radius          float               the creature's width, in yards
//! combat_reach             float               its melee reach, in yards
//! speed_walk, speed_run    float               multipliers of the base speeds
//! gender                   tinyint unsigned    0 male, 1 female, 2 none
//! display_id_other_gender  mediumint unsigned  a display used half the time, or 0
//! ```
//!
//! `ObjectMgr::LoadCreatureDisplayInfoAddon` loads it progressively by build:
//! the highest `build` at or below 5875 wins. It is read at startup.
//!
//! ## Where a new row's values come from
//!
//! `Unit::UpdateModelData` (`Unit.cpp`) reads `bounding_radius` and
//! `combat_reach` as the values at the display's own scale, the model's
//! `ModelScale` times the display's `Scale`, and multiplies them by the
//! creature's scale over that. Two displays of one model therefore differ by
//! the ratio of their `Scale` fields. A new row is copied from a shipped
//! display of the same model, its radius and reach multiplied by that ratio,
//! with that display's speeds and gender and no other-gender display. The
//! copy is made by the statement itself, from the server's own row, so the
//! project needs no connection to write it.
//!
//! A display whose model no shipped display uses has nothing to copy, and
//! takes the server's own defaults: `DEFAULT_WORLD_OBJECT_SIZE` (0.389) and
//! `DEFAULT_COMBAT_REACH` (1.5) from `ObjectDefines.h`, the table's default
//! speeds, and gender 2. A row of zeros would not do: `UpdateModelData`
//! treats a zero radius as missing data, logs an error at every spawn, and
//! sets both values to 1.5.
//!
//! Only a display the archives do not ship gets a row. A shipped display
//! already has one, and an edit to its scale does not change it.
//!
//! The `CreatureDisplayInfo` fields read here, `ModelId` (1) and `Scale` (4),
//! are `vale-assets`' schema's.

use std::collections::HashMap;

use crate::row::{self, Assignment, Key};
use vale_edit::dbc::DbcFile;

/// The server table.
pub const TABLE: &str = "creature_display_info_addon";

/// The build a row this project writes is at.
pub const BUILD: u32 = crate::CLIENT_BUILD;

/// `display_id` is a `mediumint unsigned`.
pub const MAX_ID: u32 = 0x00FF_FFFF;

/// `DEFAULT_WORLD_OBJECT_SIZE` in vmangos' `ObjectDefines.h`.
pub const DEFAULT_BOUNDING_RADIUS: f32 = 0.389;

/// `DEFAULT_COMBAT_REACH` in vmangos' `ObjectDefines.h`.
pub const DEFAULT_COMBAT_REACH: f32 = 1.5;

/// The table's defaults for the two speeds.
pub const DEFAULT_SPEED_WALK: f32 = 1.0;
pub const DEFAULT_SPEED_RUN: f32 = 1.14286;

/// `GENDER_NONE`.
pub const GENDER_NONE: u32 = 2;

/// The `CreatureDisplayInfo` fields this module reads.
const MODEL_ID_FIELD: usize = 1;
const SCALE_FIELD: usize = 4;

/// At most this many shipped displays are named as a new row's source. The
/// first that has a row in the server's table is copied.
const MAX_SOURCES: usize = 8;

/// The columns, in table order.
const COLUMNS: [&str; 8] = [
    "display_id",
    "build",
    "bounding_radius",
    "combat_reach",
    "speed_walk",
    "speed_run",
    "gender",
    "display_id_other_gender",
];

/// Whether `id` fits `display_id`.
pub fn fits(id: u32) -> bool {
    id <= MAX_ID
}

/// The row of `id` this project writes: `(display_id, build)`.
pub fn key(id: u32) -> Key {
    Key::two(("display_id", u64::from(id)), ("build", u64::from(BUILD)))
}

/// The displays in `edited` that `shipped` does not have, in id order.
pub fn new_displays(shipped: &DbcFile, edited: &DbcFile) -> Vec<u32> {
    let mut out: Vec<u32> = (0..edited.record_count())
        .filter_map(|record| edited.u32_at(record, 0))
        .filter(|&id| shipped.row_of(id).is_none())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The shipped displays of the same model as `id` in `edited`, nearest in
/// scale first, each with the ratio of `id`'s scale to its own. At most
/// [`MAX_SOURCES`]; none when the model is not used by a shipped display.
pub fn sources(shipped: &DbcFile, edited: &DbcFile, id: u32) -> Vec<(u32, f32)> {
    let Some(record) = edited.row_of(id) else {
        return Vec::new();
    };
    let (Some(model), Some(scale)) = (edited.u32_at(record, MODEL_ID_FIELD), edited.f32_at(record, SCALE_FIELD)) else {
        return Vec::new();
    };
    let mut out: Vec<(u32, f32)> = (0..shipped.record_count())
        .filter(|&r| shipped.u32_at(r, MODEL_ID_FIELD) == Some(model))
        .filter_map(|r| {
            let other = shipped.f32_at(r, SCALE_FIELD)?;
            let ratio = scale / other;
            (other > 0.0 && ratio.is_finite() && ratio > 0.0).then_some((shipped.u32_at(r, 0)?, ratio))
        })
        .collect();
    out.sort_by(|a, b| (a.1 - 1.0).abs().total_cmp(&(b.1 - 1.0).abs()).then(a.0.cmp(&b.0)));
    out.truncate(MAX_SOURCES);
    out
}

/// The columns a new row sets, for the plan: its key. The other values are
/// copied by the statement, from a row the server holds.
pub fn changes(id: u32) -> Vec<Assignment> {
    vec![
        Assignment { column: "display_id", value: id.to_string() },
        Assignment { column: "build", value: BUILD.to_string() },
    ]
}

/// The statements that give the new display `id` its row, in order: a copy
/// from the first of `sources` the server has a row for, then the defaults.
/// Both are `INSERT IGNORE`, so the second does nothing when the first
/// wrote the row, and neither changes a row that is already there. Empty
/// when `id` does not fit.
pub fn statements(id: u32, sources: &[(u32, f32)]) -> Vec<String> {
    if !fits(id) {
        return Vec::new();
    }
    let table = crate::sql::name(TABLE);
    let columns: Vec<String> = COLUMNS.iter().map(|c| crate::sql::name(c)).collect();
    let columns = columns.join(", ");
    let mut out = Vec::new();
    if !sources.is_empty() {
        let ratio: Vec<String> = sources
            .iter()
            .map(|(source, ratio)| format!("WHEN {source} THEN {}", crate::sql::float(*ratio)))
            .collect();
        let ratio = format!("CASE a.{} {} END", crate::sql::name("display_id"), ratio.join(" "));
        let ids: Vec<String> = sources.iter().map(|(source, _)| source.to_string()).collect();
        let ids = ids.join(", ");
        let a = |column: &str| format!("a.{}", crate::sql::name(column));
        out.push(format!(
            "INSERT IGNORE INTO {table} ({columns}) SELECT {id}, {BUILD}, {} * {ratio}, {} * {ratio}, \
             {}, {}, {}, 0 FROM {table} a WHERE {} IN ({ids}) AND {} <= {BUILD} \
             ORDER BY FIELD({}, {ids}), {} DESC LIMIT 1;",
            a("bounding_radius"),
            a("combat_reach"),
            a("speed_walk"),
            a("speed_run"),
            a("gender"),
            a("display_id"),
            a("build"),
            a("display_id"),
            a("build"),
        ));
    }
    out.push(format!(
        "INSERT IGNORE INTO {table} ({columns}) VALUES ({id}, {BUILD}, {}, {}, {}, {}, {GENDER_NONE}, 0);",
        crate::sql::float(DEFAULT_BOUNDING_RADIUS),
        crate::sql::float(DEFAULT_COMBAT_REACH),
        crate::sql::float(DEFAULT_SPEED_WALK),
        crate::sql::float(DEFAULT_SPEED_RUN),
    ));
    out
}

/// The row of `id` this project would write, for [`undo`] to read before an
/// apply.
pub fn dev_row_query(id: u32) -> String {
    format!("SELECT * FROM {} WHERE {};", crate::sql::name(TABLE), key(id).where_clause())
}

/// The statement that puts the table back after an apply: `DELETE` of the
/// row when the server has none now, since the apply creates it, and nothing
/// when it has one, since the apply leaves it.
pub fn undo(id: u32, now: Option<&HashMap<String, Option<String>>>) -> Option<String> {
    match (fits(id), now) {
        (true, None) => Some(row::delete(TABLE, &key(id))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of displays as `(id, model, scale)`, twelve fields wide.
    fn displays(rows: &[(u32, u32, f32)]) -> DbcFile {
        let mut bytes = b"WDBC".to_vec();
        for word in [rows.len() as u32, 12, 48, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        for &(id, model, scale) in rows {
            let mut fields = [0u32; 12];
            fields[0] = id;
            fields[MODEL_ID_FIELD] = model;
            fields[SCALE_FIELD] = scale.to_bits();
            for field in fields {
                bytes.extend_from_slice(&field.to_le_bytes());
            }
        }
        bytes.push(0);
        DbcFile::parse(&bytes).unwrap()
    }

    /// Only a display the archives do not ship is new, and its sources are
    /// the shipped displays of its model, nearest in scale first, with the
    /// ratio of the scales.
    #[test]
    fn a_new_display_is_copied_from_its_model_s_displays() {
        let shipped = displays(&[(793, 40, 1.0), (794, 40, 2.0), (800, 41, 1.0)]);
        let edited = displays(&[(793, 40, 1.0), (794, 40, 2.0), (800, 41, 1.0), (17549, 40, 1.5), (17550, 99, 1.0)]);
        assert_eq!(new_displays(&shipped, &edited), vec![17549, 17550]);
        assert_eq!(sources(&shipped, &edited, 17549), vec![(794, 0.75), (793, 1.5)]);
        assert!(sources(&shipped, &edited, 17550).is_empty(), "no shipped display uses model 99");
    }

    /// The copy comes first and the defaults after it, both INSERT IGNORE; a
    /// display with no source gets the defaults alone, never zeros.
    #[test]
    fn the_defaults_follow_the_copy() {
        let both = statements(17549, &[(794, 0.75), (793, 1.5)]);
        assert_eq!(both.len(), 2);
        assert!(both[0].starts_with("INSERT IGNORE INTO `creature_display_info_addon`"));
        assert!(both[0].contains("WHEN 794 THEN 0.75 WHEN 793 THEN 1.5"));
        assert!(both[0].contains("IN (794, 793)"));
        assert!(both[0].contains("ORDER BY FIELD(a.`display_id`, 794, 793)"));
        let alone = statements(17550, &[]);
        assert_eq!(alone.len(), 1);
        assert!(alone[0].ends_with("VALUES (17550, 5875, 0.389, 1.5, 1, 1.14286, 2, 0);"), "{}", alone[0]);
        assert!(statements(MAX_ID + 1, &[]).is_empty());
    }

    /// An apply that creates the row is undone by removing it; one that
    /// finds a row there leaves it, and is undone by nothing.
    #[test]
    fn the_undo_removes_only_a_row_the_apply_made() {
        assert_eq!(
            undo(17549, None).as_deref(),
            Some("DELETE FROM `creature_display_info_addon` WHERE `display_id` = 17549 AND `build` = 5875;")
        );
        assert_eq!(undo(17549, Some(&HashMap::new())), None);
    }
}
