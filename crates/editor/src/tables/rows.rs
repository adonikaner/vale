//! Adding, copying and removing rows, a reference pointed at a new or copied
//! row, the light chain's new rows, and an item's place in the item sets.

use crate::session::EditSession;
use vale_edit::dbc::{cell, chain, Cell, Row};
use bevy::prelude::*;
use super::browser::Browser;
use super::fields::table_mut;
use vale_assets::tables::schema::blank_defaults;

/// `ItemSet.dbc`'s seventeen item columns.
pub(crate) const SET_ITEMS: std::ops::Range<usize> = 10..27;

/// The records of `table` whose reference column `field` names row `id` of
/// `target`, in file order, through the reverse index.
pub(super) fn rows_naming(
    browser: &mut Browser,
    session: &EditSession,
    target: &str,
    id: u32,
    table: &str,
    field: usize,
) -> Vec<usize> {
    let mut rows: Vec<usize> = browser
        .used_by(session, target, id)
        .into_iter()
        .filter(|at| at.table == table && at.field == field)
        .map(|at| at.record)
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// A new row of `table_name` under the next id with `numbers` and `texts`
/// written, recorded in the history entry that is open. Returns its record.
pub(super) fn push_row(
    session: &mut EditSession,
    table_name: &str,
    numbers: &[(usize, u32)],
    texts: &[(usize, String)],
) -> Option<usize> {
    let table = session.tables.get_mut(table_name)?;
    let id = table.max_id().checked_add(1)?;
    let bytes = table.blank_record(id);
    let at = table.push_record(&bytes)?;
    let row = Row::added(table, at)?;
    session.history.record_row(table_name, row);
    for &(field, value) in numbers {
        let table = session.tables.get_mut(table_name)?;
        if let Some(edit) = Cell::new(table, at, field, value) {
            edit.apply(table);
            session.history.record_cell(table_name, edit);
        }
    }
    for (field, text) in texts {
        let table = session.tables.get_mut(table_name)?;
        if let Some(edit) = cell::set_text(table, at, *field, text) {
            session.history.record_cell(table_name, edit);
        }
    }
    session.table_edited(table_name);
    Some(at)
}

/// A new `Light` row on `map`, as one undo entry. At `centre`, in the world's
/// axes, it is a sphere with the two falloff radii in yards; with `None` it is
/// the map's default light, whose falloff is zero and whose position is not
/// read. Its five `LightParams` ids are copied from the `Light` row at
/// `template`, so it is lit and fogged like that light until its own are
/// chosen. Returns the new row's record.
pub(crate) fn add_light(
    session: &mut EditSession,
    map: u32,
    centre: Option<[f32; 3]>,
    radii: (f32, f32),
    template: Option<usize>,
) -> Option<usize> {
    use vale_assets::tables::light::{light_field as lf, YARDS_PER_UNIT};
    let lights = session.table("Light")?;
    let params: Vec<(usize, u32)> = (lf::PARAMS_CLEAR..lf::PARAMS_CLEAR + 5)
        .filter_map(|field| Some((field, lights.u32_at(template?, field)?)))
        .collect();
    let mut numbers = vec![(lf::MAP, map)];
    if let Some(centre) = centre {
        let placement = vale_assets::world::adt::placement_from_world(centre);
        numbers.extend([
            (lf::INTERNAL_X, (placement[0] / YARDS_PER_UNIT).to_bits()),
            (lf::INTERNAL_Y, (placement[1] / YARDS_PER_UNIT).to_bits()),
            (lf::INTERNAL_Z, (placement[2] / YARDS_PER_UNIT).to_bits()),
            (lf::FALLOFF_START, (radii.0 / YARDS_PER_UNIT).to_bits()),
            (lf::FALLOFF_END, (radii.1 / YARDS_PER_UNIT).to_bits()),
        ]);
    }
    numbers.extend(params);
    session.history.begin("Add light");
    let at = push_row(session, "Light", &numbers, &[]);
    session.history.end();
    at
}

/// A new `LightParams` row copied from row `from`, with its 18 `LightIntBand`
/// and 6 `LightFloatBand` rows, as one undo entry. When `assign` names a
/// reference, `(table, record, field)`, that field is pointed at the new row in
/// the same entry. Returns the new row's id.
///
/// No column points at a band: band `b` of params `p` is the row whose id is
/// `(p - 1) * 18 + b + 1` (and `* 6` for the float bands; see
/// [`vale_assets::tables::light::int_band_row`]). So the generic row copy
/// gives a params row with no bands, which the client draws with the colours
/// of whatever light it lies over. This copies the bands to the ids the new row
/// owns. The new id is the next one above every `LightParams` id whose band ids
/// are all free; a band the source does not have is written as a row with no
/// keys. `from` that names no row copies a blank row with the shipped rows'
/// usual glow and water opacities.
///
/// All three tables must be open; `None` when one is not.
pub(crate) fn copy_light_params(
    session: &mut EditSession,
    from: u32,
    assign: Option<(&str, usize, usize)>,
) -> Option<u32> {
    use vale_assets::tables::light::{float_band_row, int_band_row, BANDS_PER_PARAMS, FLOAT_BANDS_PER_PARAMS};
    const PARAMS: &str = "LightParams";
    const INT: &str = "LightIntBand";
    const FLOAT: &str = "LightFloatBand";
    let bands = |params: u32| {
        let ints = (0..BANDS_PER_PARAMS).filter_map(move |b| Some((INT, int_band_row(params, b)?)));
        let floats =
            (0..FLOAT_BANDS_PER_PARAMS).filter_map(move |b| Some((FLOAT, float_band_row(params, b)?)));
        ints.chain(floats)
    };
    let mut id = session.table(PARAMS)?.max_id().checked_add(1)?;
    session.table(INT)?;
    session.table(FLOAT)?;
    while bands(id).any(|(table, row)| session.table(table).is_some_and(|t| t.row_of(row).is_some())) {
        id = id.checked_add(1)?;
    }

    session.history.begin("New LightParams");
    // The params row: the source's bytes, or a blank row with 0.5 glow and
    // the water opacities 0.5, 1, 0.75 and 1, which most shipped rows hold.
    let params = session.tables.get_mut(PARAMS)?;
    let mut bytes = match params.row_of(from) {
        Some(record) => params.record_bytes(record)?.to_vec(),
        None => {
            let mut blank = params.blank_record(id);
            for (field, value) in [(4, 0.5f32), (5, 0.5), (6, 1.0), (7, 0.75), (8, 1.0)] {
                if let Some(slot) = blank.get_mut(field * 4..field * 4 + 4) {
                    slot.copy_from_slice(&value.to_bits().to_le_bytes());
                }
            }
            blank
        }
    };
    bytes[..4].copy_from_slice(&id.to_le_bytes());
    let at = params.push_record(&bytes)?;
    let row = Row::added(params, at)?;
    session.history.record_row(PARAMS, row);

    // The bands, each copied to the id the new row owns.
    for ((table, source), (_, target)) in bands(from).zip(bands(id)) {
        let file = session.tables.get_mut(table)?;
        let mut bytes = match file.row_of(source) {
            Some(record) => file.record_bytes(record)?.to_vec(),
            None => file.blank_record(target),
        };
        bytes[..4].copy_from_slice(&target.to_le_bytes());
        let at = file.push_record(&bytes)?;
        let row = Row::added(file, at)?;
        session.history.record_row(table, row);
    }
    if let Some((table_name, record, field)) = assign {
        if let Some(table) = session.tables.get_mut(table_name) {
            if let Some(edit) = Cell::new(table, record, field, id) {
                edit.apply(table);
                session.history.record_cell(table_name, edit);
            }
        }
    }
    session.history.end();
    for table in [PARAMS, INT, FLOAT] {
        session.table_edited(table);
    }
    if let Some((table_name, _, _)) = assign {
        session.table_edited(table_name);
    }
    Some(id)
}

/// Which item column of an `ItemSet` row lists `item`, if one does.
pub(crate) fn set_lists(sets: &vale_edit::dbc::DbcFile, record: usize, item: u32) -> Option<usize> {
    let mut fields = SET_ITEMS;
    fields.find(|&field| item != 0 && sets.u32_at(record, field) == Some(item))
}

/// The first item column of an `ItemSet` row that holds no item.
fn empty_item_column(sets: &vale_edit::dbc::DbcFile, record: usize) -> Option<usize> {
    let mut fields = SET_ITEMS;
    fields.find(|&field| sets.u32_at(record, field) == Some(0))
}

/// Move `item` between two sets' item lists in the open `ItemSet` table: it
/// is taken out of every item column of set `from` that lists it and put in
/// the first empty item column of set `to`, where `to` does not list it
/// already. Zero for either is no set. Returns one sentence per change made
/// or refused.
///
/// The edits are recorded under `subject`, so that when the caller has just
/// written the item's `set_id` under the same subject the two are one undo
/// entry: the item's column and the set's list are one fact stored twice.
pub(crate) fn move_between_sets(
    session: &mut EditSession,
    item: u32,
    from: u32,
    to: u32,
    subject: &str,
    now: f64,
) -> Vec<String> {
    let mut said = Vec::new();
    if item == 0 || from == to {
        return said;
    }
    let Some(sets) = session.table("ItemSet") else {
        return said;
    };
    let mut edits: Vec<(usize, usize, u32)> = Vec::new();
    if let Some(record) = sets.row_of(from).filter(|_| from != 0) {
        for field in SET_ITEMS.filter(|&field| sets.u32_at(record, field) == Some(item)) {
            edits.push((record, field, 0));
        }
        if !edits.is_empty() {
            said.push(format!("item {item} removed from ItemSet {from}"));
        }
    }
    if to != 0 {
        match sets.row_of(to) {
            None => said.push(format!("ItemSet has no row {to}; item {item} is not listed in any set")),
            Some(record) if set_lists(sets, record, item).is_some() => {}
            Some(record) => match empty_item_column(sets, record) {
                Some(field) => {
                    edits.push((record, field, item));
                    said.push(format!("item {item} added to ItemSet {to}"));
                }
                None => said.push(format!(
                    "ItemSet {to} has no empty item column; item {item} is not added to it"
                )),
            },
        }
    }
    if edits.is_empty() {
        return said;
    }
    session.history.begin_gesture("Edit item", subject, now);
    for (record, field, value) in edits {
        let table = table_mut(session, "ItemSet");
        if let Some(edit) = Cell::new(table, record, field, value) {
            edit.apply(table);
            session.history.record_cell("ItemSet", edit);
        }
    }
    session.history.end();
    session.table_edited("ItemSet");
    said
}

/// The tables a deep copy of a `SpellVisual` follows into: its kits, and the
/// effects those kits and the visual's own missile and area columns name.
pub(crate) const CHAIN_DEEP: [&str; 2] = ["SpellVisualKit", "SpellVisualEffectName"];

/// The suffix appended to a copied row's name, so it can be told from the
/// original.
pub(crate) const COPY_SUFFIX: &str = " (copy)";


/// Add a blank row to an open table under the next id, as one undo entry.
/// Returns its record index.
pub(crate) fn add_row(session: &mut EditSession, table_name: &str) -> Option<usize> {
    let table = session.tables.get_mut(table_name)?;
    let id = table.max_id().checked_add(1)?;
    let bytes = table.blank_record(id);
    let at = table.push_record(&bytes)?;
    let row = Row::added(table, at)?;
    session.history.begin("Add row");
    session.history.record_row(table_name, row);
    for &(field, value) in blank_defaults(table_name) {
        let table = session.tables.get_mut(table_name)?;
        if let Some(edit) = Cell::new(table, at, field, value) {
            edit.apply(table);
            session.history.record_cell(table_name, edit);
        }
    }
    session.history.end();
    session.table_edited(table_name);
    Some(at)
}

/// Copy one row under the next id, with its name suffixed, as one undo entry.
/// Returns the copy's record index.
pub(crate) fn clone_row(session: &mut EditSession, table_name: &str, record: usize) -> Option<usize> {
    let id = session.table(table_name)?.u32_at(record, 0)?;
    let done = chain::clone_row(&mut session.tables, table_name, id, Some(COPY_SUFFIX))?;
    let new_id = done.new_id(table_name, id)?;
    session.history.begin("Clone row");
    record_cloned(session, done);
    session.history.end();
    session.table_edited(table_name);
    session.table(table_name)?.row_of(new_id)
}

/// Remove one row, as one undo entry. Every record index after it shifts down
/// by one, so a caller holding one must look its row up again by id.
pub(crate) fn delete_row(session: &mut EditSession, table_name: &str, record: usize) -> bool {
    let Some(table) = session.tables.get_mut(table_name) else {
        return false;
    };
    let Some(row) = Row::removed(table, record) else {
        return false;
    };
    table.remove_record(record);
    session.history.begin("Remove row");
    session.history.record_row(table_name, row);
    session.history.end();
    session.table_edited(table_name);
    true
}

/// Copy a visual with every kit and effect it names, with the copies'
/// references pointed at the other copies, as one undo entry. When `assign`
/// names a reference, that reference is pointed at the copied visual in the
/// same entry. Returns what was copied.
///
/// `assign` is `(table, record, field)`, typically a spell's `SpellVisual`
/// column, so that spell casts the copy while every other spell keeps casting
/// the original.
pub(crate) fn clone_chain(
    session: &mut EditSession,
    visual: u32,
    assign: Option<(&str, usize, usize)>,
) -> Option<chain::Cloned> {
    let done = chain::clone_chain(
        &mut session.tables,
        "SpellVisual",
        visual,
        &CHAIN_DEEP,
        Some(COPY_SUFFIX),
    )?;
    let new_id = done.new_id("SpellVisual", visual)?;
    session.history.begin("Clone chain");
    let touched: Vec<String> = done.ids.iter().map(|(table, _, _)| table.clone()).collect();
    record_cloned(session, done.clone());
    if let Some((table_name, record, field)) = assign {
        if let Some(table) = session.tables.get_mut(table_name) {
            if let Some(edit) = Cell::new(table, record, field, new_id) {
                edit.apply(table);
                session.history.record_cell(table_name, edit);
                session.table_edited(table_name);
            }
        }
    }
    session.history.end();
    for table in touched {
        session.table_edited(&table);
    }
    Some(done)
}

/// Add a blank row to the table a reference names and point the reference at
/// it, as one undo entry. Returns the new row's id.
pub(crate) fn link_new(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    field: usize,
    target: &str,
) -> Option<u32> {
    let target_table = session.tables.get_mut(target)?;
    let id = target_table.max_id().checked_add(1)?;
    let bytes = target_table.blank_record(id);
    let at = target_table.push_record(&bytes)?;
    let row = Row::added(target_table, at)?;
    session.history.begin("New linked row");
    session.history.record_row(target, row);
    for &(blank_field, value) in blank_defaults(target) {
        let target_table = session.tables.get_mut(target)?;
        if let Some(edit) = Cell::new(target_table, at, blank_field, value) {
            edit.apply(target_table);
            session.history.record_cell(target, edit);
        }
    }
    let table = session.tables.get_mut(table_name)?;
    if let Some(edit) = Cell::new(table, record, field, id) {
        edit.apply(table);
        session.history.record_cell(table_name, edit);
    }
    session.history.end();
    session.table_edited(target);
    session.table_edited(table_name);
    Some(id)
}

/// Copy the row a reference names and point the reference at the copy, as one
/// undo entry. Returns the copy's id.
///
/// This makes a shared kit one visual's own: the other visuals keep naming the
/// original.
pub(crate) fn unshare(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    field: usize,
    target: &str,
) -> Option<u32> {
    let old = session.table(table_name)?.u32_at(record, field)?;
    let done = chain::clone_row(&mut session.tables, target, old, Some(COPY_SUFFIX))?;
    let new_id = done.new_id(target, old)?;
    session.history.begin("Copy linked row");
    record_cloned(session, done);
    let table = session.tables.get_mut(table_name)?;
    if let Some(edit) = Cell::new(table, record, field, new_id) {
        edit.apply(table);
        session.history.record_cell(table_name, edit);
    }
    session.history.end();
    session.table_edited(target);
    session.table_edited(table_name);
    Some(new_id)
}

/// Put a copy's edits on the open history entry, rows first.
fn record_cloned(session: &mut EditSession, done: chain::Cloned) {
    for (table, row) in done.rows {
        session.history.record_row(&table, row);
    }
    for (table, cell) in done.cells {
        session.history.record_cell(&table, cell);
    }
}
