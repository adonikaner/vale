//! The files of a `SoundEntries` row. A row holds up to ten file names in its
//! File 1 to File 10 columns, a weight for each in Weight 1 to Weight 10, and
//! the folder of all of them in Directory. This module reads them, and writes
//! an added, changed or removed file as one undo entry.

use crate::session::EditSession;
use vale_assets::tables::sound::fields::entry;
use vale_edit::dbc::{cell, Cell, DbcFile};

pub(crate) const TABLE: &str = "SoundEntries";

/// The Weight a file added to an empty slot is given. 8,933 of the 8,961
/// shipped slots that hold a file have Weight 1.
pub(crate) const WEIGHT: u32 = 1;

/// One slot of a row that holds a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Slot {
    /// The slot's index, 0 to 9: File `at + 1` and Weight `at + 1`.
    pub(crate) at: usize,
    /// The File column: the file's name, without the row's Directory.
    pub(crate) file: String,
    /// The Weight column: the file's share of the picks, against the sum of
    /// the row's weights.
    pub(crate) weight: u32,
}

/// The slots of the row at `record` whose File column is not empty, in slot
/// order.
pub(crate) fn slots(table: &DbcFile, record: usize) -> Vec<Slot> {
    (0..entry::FILE_COUNT)
        .filter_map(|at| {
            let file = table.string_at(record, entry::FILE + at).unwrap_or_default();
            (!file.is_empty()).then(|| Slot { at, file, weight: table.u32_at(record, entry::FREQ + at).unwrap_or(0) })
        })
        .collect()
}

/// The first slot of the row whose File column is empty, or `None` when all
/// ten hold a file.
pub(crate) fn free_slot(table: &DbcFile, record: usize) -> Option<usize> {
    (0..entry::FILE_COUNT).find(|&at| table.string_at(record, entry::FILE + at).unwrap_or_default().is_empty())
}

/// The row's Directory column, without a trailing `\` or `/`. Three shipped
/// rows end it with `\`.
pub(crate) fn directory(table: &DbcFile, record: usize) -> String {
    table
        .string_at(record, entry::DIRECTORY)
        .unwrap_or_default()
        .trim_end_matches(['\\', '/'])
        .to_string()
}

/// The archive path of `file` in `directory`, joined as
/// `vale_assets::tables::sound::SoundEntry::path` joins it.
pub(crate) fn joined(directory: &str, file: &str) -> String {
    let directory = directory.trim_end_matches(['\\', '/']);
    match directory.is_empty() {
        true => file.to_string(),
        false => format!("{directory}\\{file}"),
    }
}

/// The probability, from 0 to 1, that one play of the row picks the file of
/// `weight` among `slots`: `weight` over the sum of the weights. When every
/// weight is 0, each file has the same probability, as in
/// `SoundEntry::pick`.
pub(crate) fn chance(slots: &[Slot], weight: u32) -> f32 {
    let total: u32 = slots.iter().map(|slot| slot.weight).sum();
    match total {
        0 => 1.0 / slots.len().max(1) as f32,
        _ => weight as f32 / total as f32,
    }
}

/// The mixer channel a row plays on, chosen by its SoundType `kind`: 28
/// (Music) and 30 (Fly-by music) on `Channel::Music`, 50 (Ambience) on
/// `Channel::Ambience`, and every other type on `Channel::Effects`.
pub(crate) fn channel_of_type(kind: u32) -> vale_client::sound::mixer::Channel {
    use vale_client::sound::mixer::Channel;
    match kind {
        28 | 30 => Channel::Music,
        50 => Channel::Ambience,
        _ => Channel::Effects,
    }
}

/// Splits archive path `path` into the values `slot` of the row at `record`
/// is written with: a new Directory when the row's Directory changes, and the
/// file name for the File column. An error when the file cannot go there.
///
/// Every file of a row is in its Directory. A file in another folder is
/// accepted only when no other slot holds a file, and the row's Directory
/// becomes that file's folder.
pub(crate) fn place(table: &DbcFile, record: usize, slot: usize, path: &str) -> Result<(Option<String>, String), String> {
    let (folder, name) = path.rsplit_once('\\').unwrap_or(("", path));
    if name.is_empty() {
        return Err(format!("{path} is a folder, not a file"));
    }
    let current = directory(table, record);
    if folder.eq_ignore_ascii_case(&current) {
        return Ok((None, name.to_string()));
    }
    match slots(table, record).iter().any(|other| other.at != slot) {
        false => Ok((Some(folder.to_string()), name.to_string())),
        true => Err(format!(
            "{path} is not in the row's Directory, {current}; every file of a SoundEntries row is in its Directory"
        )),
    }
}

/// Writes the file at archive path `path` into `slot` of the row at
/// `record`, and the row's new Directory when [`place`] gives one, as one undo
/// entry. An empty slot is given Weight [`WEIGHT`]; a slot that held a file
/// keeps its Weight.
pub(crate) fn put_file(session: &mut EditSession, record: usize, slot: usize, path: &str) -> Result<(), String> {
    let table = session.table(TABLE).ok_or("SoundEntries is not open")?;
    let (folder, name) = place(table, record, slot, path)?;
    let was_empty = table.string_at(record, entry::FILE + slot).unwrap_or_default().is_empty();
    let mut texts = vec![(entry::FILE + slot, name)];
    if let Some(folder) = folder {
        texts.push((entry::DIRECTORY, folder));
    }
    let numbers = match was_empty {
        true => vec![(entry::FREQ + slot, WEIGHT)],
        false => Vec::new(),
    };
    let label = match was_empty {
        true => "Add sound file",
        false => "Change sound file",
    };
    write(session, label, record, &numbers, &texts);
    Ok(())
}

/// Removes the file in `slot` of the row at `record`, as one undo entry. The
/// File and Weight of each later slot move up one slot, and slot 10 is
/// cleared, so the row's files stay contiguous from File 1.
pub(crate) fn remove_file(session: &mut EditSession, record: usize, slot: usize) {
    let Some(table) = session.table(TABLE) else { return };
    // A slot's File and Weight; past slot 10, an empty name and 0.
    let at = |index: usize| {
        (index < entry::FILE_COUNT)
            .then(|| (table.string_at(record, entry::FILE + index).unwrap_or_default(), table.u32_at(record, entry::FREQ + index).unwrap_or(0)))
            .unwrap_or_default()
    };
    let moved: Vec<(usize, (String, u32))> = (slot..entry::FILE_COUNT).map(|index| (index, at(index + 1))).collect();
    let texts: Vec<(usize, String)> = moved.iter().map(|(index, (file, _))| (entry::FILE + index, file.clone())).collect();
    let numbers: Vec<(usize, u32)> = moved.iter().map(|(index, (_, weight))| (entry::FREQ + index, *weight)).collect();
    write(session, "Remove sound file", record, &numbers, &texts);
}

/// Writes numbers and strings to one row of `SoundEntries` as one undo
/// entry. A field that already holds its value is not written.
fn write(session: &mut EditSession, label: &str, record: usize, numbers: &[(usize, u32)], texts: &[(usize, String)]) {
    session.history.begin(label);
    for &(field, value) in numbers {
        let Some(table) = session.tables.get_mut(TABLE) else { break };
        if let Some(edit) = Cell::new(table, record, field, value).filter(Cell::moves) {
            edit.apply(table);
            session.history.record_cell(TABLE, edit);
        }
    }
    for (field, text) in texts {
        let Some(table) = session.tables.get_mut(TABLE) else { break };
        if table.string_at(record, *field).unwrap_or_default() == *text {
            continue;
        }
        if let Some(edit) = cell::set_text(table, record, *field, text) {
            session.history.record_cell(TABLE, edit);
        }
    }
    session.history.end();
    session.table_edited(TABLE);
}
