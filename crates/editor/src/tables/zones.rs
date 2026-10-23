//! `AreaTable`'s zones and sub-areas: the columns read, the sub-areas of a
//! zone, and a new zone or sub-area with an explore bit of its own.

use crate::session::EditSession;
use bevy::prelude::*;
use super::browser::Browser;
use super::rows::{push_row, rows_naming};
use vale_edit::dbc::places::next_explore_bit;

/// `AreaTable.dbc`'s fields, as the Zones workspace reads and writes them.
/// The indices are `vale_assets::tables::area::fields`'.
pub(crate) mod area {
    use vale_assets::tables::area::fields;
    pub(crate) const TABLE: &str = "AreaTable";
    pub(crate) const MAP: usize = fields::MAP_ID;
    pub(crate) const PARENT: usize = fields::PARENT;
    pub(crate) const EXPLORE_BIT: usize = fields::AREA_BIT;
    pub(crate) const FLAGS: usize = fields::FLAGS;
    pub(crate) const NAME: usize = fields::NAME;
    pub(crate) const NAME_FLAGS: usize = fields::NAME_FLAGS;
    /// `AREA_FLAG_DUEL`, which 969 of the 1,081 shipped rows carry and a new
    /// zone starts with.
    pub(crate) const DUELS: u32 = 0x40;
    /// The word after the name columns on 724 of the 1,081 shipped rows; the
    /// other 357 hold 8,323,198. No source describes the column, so a new row
    /// takes the commoner shipped value and not zero, which no shipped row
    /// holds.
    pub(crate) const SHIPPED_NAME_FLAGS: u32 = 4_128_894;
    /// What a new sub-area takes from its zone: the flags, the two reverb
    /// presets, the ambience and the music. Northshire Valley holds Elwynn
    /// Forest's values in all five.
    pub(crate) const FROM_ZONE: [usize; 5] = [4, 5, 6, 7, 8];
    pub(crate) const NEW_ZONE: &str = "New zone";
    pub(crate) const NEW_SUB_AREA: &str = "New area";
}

/// The sub-areas of a zone: the rows whose parent is `zone`, in file order.
pub(crate) fn sub_areas_of(browser: &mut Browser, session: &EditSession, zone: u32) -> Vec<usize> {
    rows_naming(browser, session, area::TABLE, zone, area::TABLE, area::PARENT)
}

/// A new zone on `map`, as one undo entry: no parent, the next explore bit,
/// duels allowed, and a name to be replaced. Returns its record.
pub(crate) fn add_zone(session: &mut EditSession, map: u32) -> Option<usize> {
    let bit = next_explore_bit(session.table(area::TABLE)?);
    session.history.begin("Add zone");
    let at = push_row(
        session,
        area::TABLE,
        &[
            (area::MAP, map),
            (area::EXPLORE_BIT, bit),
            (area::FLAGS, area::DUELS),
            (area::NAME_FLAGS, area::SHIPPED_NAME_FLAGS),
        ],
        &[(area::NAME, area::NEW_ZONE.to_string())],
    );
    session.history.end();
    at
}

/// A new sub-area of the zone `of` is, or is in, as one undo entry. It takes
/// the zone's map and [`area::FROM_ZONE`]'s columns, the next explore bit, and
/// a name to be replaced. Asked of a sub-area, it makes a sibling: the table
/// is two levels, and no shipped row's parent has a parent. Returns the new
/// row's record.
pub(crate) fn add_sub_area(session: &mut EditSession, of: u32) -> Option<usize> {
    let areas = session.table(area::TABLE)?;
    let asked = areas.row_of(of)?;
    let zone = match areas.u32_at(asked, area::PARENT)? {
        0 => of,
        parent => parent,
    };
    // A parent the table does not hold: the row asked of stands in for it.
    let from = areas.row_of(zone).unwrap_or(asked);
    let mut numbers = vec![
        (area::MAP, areas.u32_at(from, area::MAP)?),
        (area::PARENT, zone),
        (area::EXPLORE_BIT, next_explore_bit(areas)),
        (area::NAME_FLAGS, area::SHIPPED_NAME_FLAGS),
    ];
    for field in area::FROM_ZONE {
        numbers.push((field, areas.u32_at(from, field)?));
    }
    session.history.begin("Add sub-area");
    let at = push_row(
        session,
        area::TABLE,
        &numbers,
        &[(area::NAME, area::NEW_SUB_AREA.to_string())],
    );
    session.history.end();
    at
}
