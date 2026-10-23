//! Writing a field of a row: a number, several numbers at once, or a string,
//! each as one entry on the undo stack.

use crate::session::EditSession;
use vale_edit::dbc::{cell, Cell};
use bevy::prelude::*;

/// Write one field and record it on the undo stack.
///
/// The gesture key is the field, so a number dragged through forty values in
/// one motion is one entry, and the same field changed again half a second
/// later is a second entry. That is `History::begin_gesture`'s rule, which the
/// panels also use for a dragged rotation.
pub(crate) fn set_field(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    field: usize,
    value: u32,
    label: &str,
    now: f64,
) {
    let Some(table) = session.tables.get_mut(table_name) else {
        return;
    };
    let Some(edit) = Cell::new(table, record, field, value) else {
        return;
    };
    if !edit.moves() {
        return;
    }
    edit.apply(table);
    session
        .history
        .begin_gesture(label, format!("{table_name} {record} field {field}"), now);
    session.history.record_cell(table_name, edit);
    session.history.end();
    session.table_edited(table_name);
}

/// Write several fields of one row as a single gesture.
///
/// [`set_field`] keys its gesture on the field, so a drag that changes two
/// columns would make two entries on the stack, and one `Ctrl+Z` would undo
/// half of the movement. A light dragged across the map changes `InternalX`
/// and `InternalZ`. A dragged radius changes one column, and writing it through
/// this function as well keeps both drags on the same key, so a drag that
/// changes from one to the other is still one entry.
///
/// `subject` is the gesture key. Pass a name for the movement rather than for
/// the columns; [`vale_edit::undo::History::begin_gesture`] merges writes
/// with the same key inside its time window into one change.
pub(crate) fn set_fields(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    fields: &[(usize, u32)],
    label: &str,
    subject: &str,
    now: f64,
) {
    let Some(table) = session.tables.get_mut(table_name) else {
        return;
    };
    let edits: Vec<Cell> = fields
        .iter()
        .filter_map(|&(field, value)| Cell::new(table, record, field, value))
        .filter(|edit| edit.moves())
        .collect();
    // Return without opening a gesture when nothing changes. A drag reports a
    // position every frame, and on most frames the movement is too small to
    // change a float's bits. Opening a gesture for each such frame would push
    // an empty change and end the run of entries the next real change should
    // merge into.
    if edits.is_empty() {
        return;
    }
    session.history.begin_gesture(label, subject, now);
    for edit in edits {
        edit.apply(table_mut(session, table_name));
        session.history.record_cell(table_name, edit);
    }
    session.history.end();
    session.table_edited(table_name);
}

/// The open table, looked up again on each iteration of [`set_fields`]' loop:
/// `apply` needs it mutably, and so does the history between iterations.
pub(super) fn table_mut<'a>(
    session: &'a mut EditSession,
    table_name: &str,
) -> &'a mut vale_edit::dbc::DbcFile {
    session
        .tables
        .get_mut(table_name)
        .expect("checked by the caller")
}

/// Write one string field and record it on the undo stack, as [`set_field`]
/// does for a number. A string write appends to the string block and then
/// writes the field.
pub(crate) fn set_text(
    session: &mut EditSession,
    table_name: &str,
    record: usize,
    field: usize,
    text: &str,
    label: &str,
    now: f64,
) {
    let Some(table) = session.tables.get_mut(table_name) else {
        return;
    };
    let Some(edit) = cell::set_text(table, record, field, text) else {
        return;
    };
    if !edit.moves() {
        return;
    }
    session
        .history
        .begin_gesture(label, format!("{table_name} {record} field {field}"), now);
    session.history.record_cell(table_name, edit);
    session.history.end();
    session.table_edited(table_name);
}
