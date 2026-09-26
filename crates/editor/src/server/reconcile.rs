//! The one order every subject's Apply runs in.
//!
//! ## What an Apply has to make true
//!
//! ```text
//! the database  =  what it held before this project  +  what the project says now
//! ```
//!
//! The project's store is the record of the second term and the revert file is
//! the record of the first. An Apply that only *adds* to the database keeps that
//! equation while the project only grows, and breaks it the first time the
//! project stops saying something it had applied:
//!
//! * **A created row given another id.** The store re-keys its claim, the next
//!   Apply inserts the row at the new id, and the row at the old id stays in the
//!   database with nothing claiming it. Reported as an item created, applied,
//!   renumbered and applied again, and present twice.
//! * **A created row discarded, a column edit taken back, a removal undone.**
//!   Each leaves the database holding what the project no longer says.
//! * **A column first edited after an Apply.** The revert file named the columns
//!   the first Apply wrote and was never added to for a row it already covered,
//!   so Put back left that column where the project had set it.
//!
//! All three are one fault — the second Apply starts from a database it does not
//! describe — and one order removes it:
//!
//! ```text
//! 1  put back    run the revert file, newest entry first: the database is what
//!                it was before this project
//! 2  forget      remove the revert file, which now describes nothing
//! 3  per row     read the row as it stands and refuse a created key that is
//!                already there, append the statements that put it back to the
//!                revert file, then run the row's own statements
//! ```
//!
//! So every Apply is the first Apply, and the revert file describes exactly
//! what step 3 wrote.
//!
//! ## Why step 3 is a row at a time
//!
//! A row's undo is the inverse of its statements *given the database they ran
//! against*, and a plan's rows are not independent: a quest moved to another
//! entry takes its relation rows with it, so a relation this project removes is
//! at the new entry by the time its `DELETE` runs and at the old one before.
//! Snapshots taken all together before anything ran read the second state for a
//! statement that meets the first. Taken immediately before each row's own
//! statements, every entry is exact, and running the entries newest first
//! unwinds the plan through the same states it went through.
//!
//! ## Every step leaves the two records agreeing
//!
//! The reference install's tables are MyISAM, which ignores a transaction — see
//! `vale_mangos::conn` — so a list of statements can stop half way and stay
//! half written. The order above is safe against that without a rollback,
//! because an entry is on disk before its row's first statement runs and
//! **every statement of an undo can be run against a row it has already
//! restored**: an `UPDATE` back to read values, a `DELETE`, or a `DELETE`
//! followed by the `INSERT`s that restore what it took. Never a bare `INSERT`,
//! which fails the second time. Whatever a failed Apply or a failed Put back
//! leaves, Put back finishes.
//!
//! A failure after step 1 leaves the project partly applied or not at all,
//! rather than as it was applied before, and the message says so.
//!
//! ## One subject at a time is not enough
//!
//! The equation holds per subject only while no other subject's statements
//! move its rows, and an item or quest renumber does move them. So this order
//! is run for one subject inside a larger one — [`super::stack`] — which puts
//! back every later subject first and applies it again after. Two checks here
//! exist because of it: [`refuse_a_taken_id`] leaves out rows the project
//! itself sets to name the new id, which an earlier subject has already
//! written, and [`undo_of_an_update`] refuses a renumber whose row is gone,
//! whose other statements would otherwise run with nothing to put them back.

use super::creatures::Undo;
use crate::session::EditSession;
use vale_edit::project::Project;
use vale_mangos::conn::Db;
use vale_mangos::row::{Edits, Key, Life};

/// One row of a plan, as this module needs it: what it is called in the revert
/// file, and the statements it becomes.
pub struct Step {
    pub table: String,
    pub key: Key,
    pub statements: Vec<String>,
}

/// What one Apply did.
#[derive(Debug, Default)]
pub struct Reconciled {
    /// Rows the plan's own statements affected.
    pub affected: u64,
    /// How many rows the new revert file covers.
    pub undoable: usize,
    /// **How many rows the project had applied and no longer claims**, which
    /// were put back and not written again: a discarded creation, a re-keyed
    /// one's old id, a removal that was undone.
    pub taken_back: usize,
}

impl Reconciled {
    /// `" — 1 row(s) the project no longer claims put back"`, or nothing.
    pub fn taken_back_words(&self) -> String {
        match self.taken_back {
            0 => String::new(),
            n => format!(" — {n} row(s) the project no longer claims put back"),
        }
    }
}

/// **Run one subject's Apply**, in the module's order.
///
/// `undo_for` is the subject's own half of step 3, asked for one step at a time
/// with the database as that step will meet it: it answers the statements that
/// put the row back, `None` for a row with nothing to put back (an edit to a row
/// that is not there), or an `Err` that stops the Apply.
pub fn apply(
    project: &Project,
    db: &mut Db,
    vpath: &str,
    subject: &str,
    steps: &[Step],
    mut undo_for: impl FnMut(&mut Db, usize) -> Result<Option<Vec<String>>, String>,
) -> Result<Reconciled, String> {
    let had = Undo::open_at(project, vpath)?;
    if !had.entries.is_empty() {
        db.run(&had.put_back()).map_err(|e| {
            format!("putting back what this project applied before did not finish: {e}")
        })?;
        remove(project, vpath)?;
    }
    let mut undo = Undo {
        entries: Vec::new(),
    };
    let mut out = Reconciled::default();
    for (index, step) in steps.iter().enumerate() {
        let done = (|| {
            if let Some(statements) = undo_for(db, index)? {
                undo.add_many(&step.table, &step.key, &statements);
                // **On disk before the row's first statement runs.**
                undo.append_newest_at(project, vpath, subject)?;
            }
            db.run(&step.statements)
        })();
        match done {
            Ok(affected) => out.affected += affected,
            Err(e) => {
                let before = match (index, had.entries.is_empty()) {
                    (0, true) => "Nothing has been applied.".to_string(),
                    (0, false) => "What this project had applied before has been put back, \
                                   so the database holds none of it now."
                        .to_string(),
                    (n, _) => format!(
                        "{n} of {} row(s) were written before this; Put back returns them.",
                        steps.len()
                    ),
                };
                return Err(format!("{e} {before}"));
            }
        }
    }
    // The file again, in the order a person running it by hand wants.
    undo.write_finished_at(project, vpath, subject)?;
    out.undoable = undo.entries.len();
    out.taken_back = had
        .entries
        .iter()
        .filter(|(table, key, _)| !undo.covers(table, key))
        .count();
    Ok(out)
}

/// **Put back**, which is steps 1 and 2 and nothing after them. Answers how
/// many rows the file covered.
pub fn put_back(project: &Project, db: &mut Db, vpath: &str) -> Result<usize, String> {
    let had = Undo::open_at(project, vpath)?;
    if had.entries.is_empty() {
        return Ok(0);
    }
    db.run(&had.put_back())?;
    remove(project, vpath)?;
    Ok(had.entries.len())
}

/// Whether this project has anything in the database for that subject.
pub fn has_applied(session: &EditSession, vpath: &str) -> bool {
    Undo::open_at(&session.project, vpath).is_ok_and(|undo| !undo.entries.is_empty())
}

/// **Refuse a row arriving at an id that is taken or that something names.**
///
/// Asked of every row a plan creates or moves, immediately before it runs.
///
/// *Taken*: a new row is numbered from a reserved range and from the highest id
/// this editor could see, but the database moves under it — another project, a
/// GM command, an upstream update. The statements are a `DELETE` of the key and
/// an `INSERT`, which would destroy a row somebody else owns. The database
/// holds none of this project's rows at this key here, so a row at it is not
/// this project's.
///
/// *Named*: the undo of a followed reference is the same `UPDATE` with the ids
/// exchanged, which is exact only while nothing named the target before the
/// move. A vendor line left pointing at an id with no row behind it would be
/// taken along with the real ones when the move is put back.
///
/// **Rows this project itself sets to name the id are not counted.** `own` is
/// the project's store. An item whose `start_quest` the project sets to a
/// quest's new entry is written by the item Apply, which runs before the quest
/// Apply — see [`super::stack`] — so by the time the quest is moved the item
/// already names its new id. That row is the project's, and the item's own
/// undo puts its column back after the quest's is run, so it is not a row the
/// move could not be told from.
pub fn refuse_a_taken_id(
    db: &mut Db,
    table: &str,
    key: &Key,
    references: &[vale_mangos::row::Reference],
    own: &Edits,
) -> Result<(), String> {
    if db
        .row(&vale_mangos::row::id_exists_query(table, key))?
        .is_some()
    {
        return Err(format!(
            "{table} {} is already in the database and is not this project's.",
            key.text()
        ));
    }
    let Some(id) = key.first() else { return Ok(()) };
    for reference in references {
        let written = reference.literal(id);
        let mine: Vec<Key> = own
            .rows()
            .filter(|(claimed, _, row)| {
                *claimed == reference.table
                    && row.life != Life::Delete
                    && row.columns.get(reference.column) == Some(&written)
            })
            .flat_map(|(_, key, row)| std::iter::once(key.clone()).chain(row.from.clone()))
            .collect();
        let named = db
            .row(&reference.count_except(id, &mine))?
            .and_then(|row| row.get("n").cloned().flatten())
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0);
        if named > 0 {
            return Err(format!(
                "{named} row(s) of {}.{} already name {id}, which has no row of its own; \
                 a row moved onto it could not be told from them when it is put back.",
                reference.table, reference.column
            ));
        }
    }
    Ok(())
}

/// **The undo of an edit to a row that is already in the database**, and the
/// refusal of a move that could not have one.
///
/// `None` for an edit to a row that is not there: its `UPDATE` matches nothing
/// and there is nothing to put back. **A move is different.** Its statements
/// are the row's `UPDATE`, one more for the row's other content patches, and
/// one per column elsewhere that names the id — and only the first depends on
/// the row being there. With the row gone upstream, the others would still
/// renumber every other patch of it and every reference to it, and Put back
/// would have nothing to run for any of them.
pub fn undo_of_an_update(
    table: &str,
    at: &Key,
    key: &Key,
    changes: &[vale_mangos::row::Assignment],
    references: &[vale_mangos::row::Reference],
    now: Option<&std::collections::HashMap<String, Option<String>>>,
) -> Result<Option<Vec<String>>, String> {
    let undo = vale_mangos::row::undo_move_statements(table, at, key, changes, references, now);
    if undo.is_none() && at != key {
        return Err(format!(
            "{table} {} is not in the database, so it cannot be moved to {}: the other \
             content patches and every reference to it would move with nothing to put \
             them back. Discard the move, or restore the row.",
            at.text(),
            key.text()
        ));
    }
    Ok(undo)
}

/// **Refuse a spawn created past the guid space** — `creature::MAX_GUID` or
/// `gameobject::MAX_GUID`. A guid above it stops the server: the guid
/// generator refuses to hand out a counter past its range.
pub fn refuse_a_guid_past_the_limit(table: &str, key: &Key, limit: u64) -> Result<(), String> {
    match key.first() {
        Some(guid) if guid > limit => Err(format!(
            "{table} guid {guid} is above {limit}, the highest guid the server accepts."
        )),
        _ => Ok(()),
    }
}

/// Remove a revert file whose statements have run. **A failure is an error**:
/// a file left behind would be put back again on every later Apply.
fn remove(project: &Project, vpath: &str) -> Result<(), String> {
    let Some(disk) = project.path_for(vpath) else {
        return Ok(());
    };
    match std::fs::remove_file(&disk) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "{vpath} has been put back but could not be removed: {e}. Remove it by hand \
             before the next Apply."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_mangos::row::Assignment;

    /// **A renumber whose row has gone is refused**, rather than run as its
    /// other statements with nothing to put them back. An edit to a row that
    /// has gone is not: its `UPDATE` matches nothing.
    #[test]
    fn a_move_with_no_row_behind_it_is_refused() {
        let at = Key::two(("entry", 852), ("patch", 10));
        let to = Key::two(("entry", 2_000_456), ("patch", 10));
        let change = [Assignment { column: "name", value: "'Axe'".into() }];
        let refs = &vale_mangos::item::REFERENCES;
        assert!(undo_of_an_update("item_template", &at, &to, &change, refs, None).is_err());
        assert_eq!(
            undo_of_an_update("item_template", &at, &at, &change, refs, None),
            Ok(None)
        );
        let mut now = std::collections::HashMap::new();
        now.insert("name".to_string(), Some("Old Axe".to_string()));
        let undo = undo_of_an_update("item_template", &at, &to, &change, refs, Some(&now))
            .expect("a row that is there")
            .expect("an undo");
        assert!(undo.len() > 2, "the references are followed back too");
    }

    #[test]
    fn a_guid_past_the_space_is_refused() {
        let limit = vale_mangos::creature::MAX_GUID;
        assert!(refuse_a_guid_past_the_limit("creature", &Key::one("guid", limit), limit).is_ok());
        assert!(refuse_a_guid_past_the_limit("creature", &Key::one("guid", limit + 1), limit).is_err());
    }
}
