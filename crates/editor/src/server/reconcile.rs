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

use crate::session::EditSession;
use vale_edit::project::Project;
use vale_mangos::conn::Db;
use vale_mangos::row::{Edits, Key, Life};

/// One row of a plan, as this module needs it: what it is called in the revert
/// file, and the statements it becomes.
pub(crate) struct Step {
    pub(crate) table: String,
    pub(crate) key: Key,
    pub(crate) statements: Vec<String>,
}

/// What one Apply did.
#[derive(Debug, Default)]
pub(crate) struct Reconciled {
    /// Rows the plan's own statements affected.
    pub(crate) affected: u64,
    /// How many rows the new revert file covers.
    pub(crate) undoable: usize,
    /// **How many rows the project had applied and no longer claims**, which
    /// were put back and not written again: a discarded creation, a re-keyed
    /// one's old id, a removal that was undone.
    pub(crate) taken_back: usize,
}

impl Reconciled {
}

/// **Run one subject's Apply**, in the module's order.
///
/// `undo_for` is the subject's own half of step 3, asked for one step at a time
/// with the database as that step will meet it: it answers the statements that
/// put the row back, `None` for a row with nothing to put back (an edit to a row
/// that is not there), or an `Err` that stops the Apply.
pub(crate) fn apply(
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
            format!("could not restore the rows this project applied earlier: {e}")
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
                    (0, false) => "The rows this project applied earlier were restored; \
                                   the database holds none of this project's rows."
                        .to_string(),
                    (n, _) => format!(
                        "{n} of {} row(s) were written before the failure; Restore reverts them.",
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
pub(crate) fn put_back(project: &Project, db: &mut Db, vpath: &str) -> Result<usize, String> {
    let had = Undo::open_at(project, vpath)?;
    if had.entries.is_empty() {
        return Ok(0);
    }
    db.run(&had.put_back())?;
    remove(project, vpath)?;
    Ok(had.entries.len())
}

/// Whether this project has anything in the database for that subject.
pub(crate) fn has_applied(session: &EditSession, vpath: &str) -> bool {
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
pub(crate) fn refuse_a_taken_id(
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
                "{named} row(s) of {}.{} already reference {id}, which has no row; \
                 putting back a move to {id} would also change those rows.",
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
pub(crate) fn undo_of_an_update(
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
            "{table} {} is not in the database, so it cannot be moved to {}: its other \
             content patches and every reference to it would be renumbered with no \
             statement to restore them. Discard the move, or restore the row.",
            at.text(),
            key.text()
        ));
    }
    Ok(undo)
}

/// **Refuse a spawn created past the guid space** — `creature::MAX_GUID` or
/// `gameobject::MAX_GUID`. A guid above it stops the server: the guid
/// generator refuses to hand out a counter past its range.
pub(crate) fn refuse_a_guid_past_the_limit(table: &str, key: &Key, limit: u64) -> Result<(), String> {
    match key.first() {
        Some(guid) if guid > limit => Err(format!(
            "{table} guid {guid} is above {limit}, the highest guid the server accepts."
        )),
        _ => Ok(()),
    }
}

/// A revert file: how to put a subject's rows back. Every row subject keeps
/// one (`sql\<subject>-revert.sql`), and the client-table rows keep
/// `sql\revert.sql`; all of them are read and written here.
///
/// Plain SQL with a marker line before each statement naming the table and the
/// row. The marker is read, not decorative: it is how an apply knows which rows
/// it has already snapshotted. The alternative, a side file in some
/// serialisation, would be a second format and would stop the undo being a
/// file a person can read and run by hand, which is the property that makes an
/// automatic write to a database checkable.
///
/// ```text
/// -- row creature_template entry=68;patch=0
/// UPDATE `creature_template` SET `name` = 'Stormwind City Guard' WHERE …;
///
/// -- row creature_movement id=12345
/// DELETE FROM `creature_movement` WHERE `id` = 12345;
/// INSERT INTO `creature_movement` (...) VALUES (12345, 1, ...);
/// INSERT INTO `creature_movement` (...) VALUES (12345, 2, ...);
/// ```
///
/// An entry runs to the next marker, which is what lets a path's undo be
/// several statements. Reading one line per marker restored a path as its
/// `DELETE` and none of its nodes, so the undo emptied the path it was meant
/// to put back.
pub(crate) struct Undo {
    /// `(table, key, statements)`, oldest first.
    ///
    /// Several statements per entry, because a waypoint path's undo is a
    /// `DELETE` and an `INSERT` a node rather than one `UPDATE`. A row's entry
    /// holds exactly one and reads back as one.
    pub(crate) entries: Vec<(String, Key, Vec<String>)>,
}

impl Undo {
    const MARKER: &'static str = "-- row ";
    /// The client-table marker before the file named a table, `-- entry <id>`,
    /// which always meant a `spell_template` dev row. Read, never written: a
    /// project applied by an earlier build has a file of them, and failing to
    /// recognise one would re-snapshot a row that is already edited.
    const OLD_MARKER: &'static str = "-- entry ";
    /// The line a finished file carries to say its entries are newest first.
    /// A file without it is oldest first, which is every file written before
    /// the order mattered and any file an apply was stopped in the middle of.
    const NEWEST_FIRST: &'static str = "-- order: newest first";
    /// The line that closes an entry. See [`Self::from_text`].
    const END: &'static str = "-- end";

    /// One of the project's revert files; empty when it is not there.
    ///
    /// The marker format is one format, which is why the path is a parameter
    /// rather than a second parser: each subject keeps a revert file of its
    /// own because each is applied by its own gesture, and a second reading of
    /// `-- row <table> <key>` would be a place for the two to drift apart
    /// silently.
    pub(crate) fn open_at(project: &Project, vpath: &str) -> Result<Undo, String> {
        let Some(text) = project.read(vpath) else {
            return Ok(Undo {
                entries: Vec::new(),
            });
        };
        let text = String::from_utf8(text).map_err(|e| format!("{vpath}: {e}"))?;
        Ok(Undo::from_text(&text))
    }

    /// The parse on its own, so it can be tested without a project on disk.
    ///
    /// An entry without its [`Self::END`] line is not read, when the file has
    /// any. An entry is appended before its row's statements run, so one cut
    /// short by a failed or interrupted append is a row that was never
    /// written; running what survived of it would be half an undo (the
    /// `DELETE`s of a removed spawn's snapshot without the `INSERT`s that
    /// restore it) against a row that is still there. A file with no `END`
    /// line at all predates them and is read as it always was.
    pub(crate) fn from_text(text: &str) -> Undo {
        let mut entries: Vec<(String, Key, Vec<String>)> = Vec::new();
        let mut ended: Vec<bool> = Vec::new();
        let mut any_end = false;
        let mut at: Option<(String, Key)> = None;
        let mut newest_first = false;
        for line in text.lines() {
            let line = line.trim();
            if line == Self::NEWEST_FIRST {
                newest_first = true;
                continue;
            }
            if line == Self::END {
                any_end = true;
                if let Some(last) = ended.last_mut() {
                    *last = true;
                }
                at = None;
                continue;
            }
            if let Some(rest) = line.strip_prefix(Self::MARKER) {
                at = rest
                    .split_once(' ')
                    .and_then(|(table, key)| Some((table.trim().to_string(), Key::parse(key)?)));
                continue;
            }
            if let Some(rest) = line.strip_prefix(Self::OLD_MARKER) {
                at = rest.trim().parse::<u64>().ok().map(|entry| {
                    (
                        vale_mangos::spell::TABLE.to_string(),
                        Key::two(("entry", entry), ("build", u64::from(vale_mangos::spell::BUILD))),
                    )
                });
                continue;
            }
            // A comment that is not a marker clears nothing and carries
            // nothing, so the file's own header cannot become an entry.
            if line.is_empty() || line.starts_with("--") {
                continue;
            }
            // The marker is not taken, so every statement until the next one
            // joins this entry. See the type's own doc.
            let Some((table, key)) = at.as_ref() else {
                continue;
            };
            match (entries.last_mut(), ended.last()) {
                (Some((had_table, had_key, statements)), Some(false))
                    if had_table == table && had_key == key =>
                {
                    statements.push(line.to_string());
                }
                _ => {
                    entries.push((table.clone(), key.clone(), vec![line.to_string()]));
                    ended.push(false);
                }
            }
        }
        if any_end {
            let mut ended = ended.into_iter();
            entries.retain(|_| ended.next().unwrap_or(false));
        }
        if newest_first {
            entries.reverse();
        }
        Undo { entries }
    }

    /// Whether this row has already been snapshotted.
    ///
    /// By table and key together. `creature_template` entry 68 and
    /// `creature` guid 68 are different rows, and `(68, patch 0)` and
    /// `(68, patch 3)` are two more; a check on the id alone would give the
    /// second of any pair the first's statement.
    pub(crate) fn covers(&self, table: &str, key: &Key) -> bool {
        self.entries
            .iter()
            .any(|(had_table, had_key, _)| had_table == table && had_key == key)
    }

    pub(crate) fn add(&mut self, table: &str, key: &Key, statement: &str) {
        self.add_many(table, key, &[statement.to_string()]);
    }

    /// The same for the several statements a path's undo is.
    pub(crate) fn add_many(&mut self, table: &str, key: &Key, statements: &[String]) {
        self.entries
            .push((table.to_string(), key.clone(), statements.to_vec()));
    }

    /// Every statement, in the order that puts the database back: the newest
    /// entry first, and each entry's own statements in the order it holds
    /// them.
    ///
    /// Newest first because entries are written in the order the plan ran and
    /// each is the inverse of its own step given the steps before it; see
    /// [`super::reconcile`]. A row created at an id another row was moved away
    /// from has to go before that row can come back.
    pub(crate) fn put_back(&self) -> Vec<String> {
        self.entries
            .iter()
            .rev()
            .flat_map(|(_, _, statements)| statements.iter().cloned())
            .collect()
    }

    /// Add the newest entry to the file, which is [`Self::write_at`] for a
    /// caller that adds one entry per row of a plan: rewriting the whole file
    /// each time is quadratic in the plan, and a project that moves two
    /// thousand spawns is four hundred megabytes of it.
    ///
    /// The file is the same text either way. The first entry writes the whole
    /// file, header included.
    pub(crate) fn append_newest_at(
        &self,
        project: &Project,
        vpath: &str,
        subject: &str,
    ) -> Result<(), String> {
        let (Some((table, key, statements)), true) = (self.entries.last(), self.entries.len() > 1)
        else {
            return self.write_at(project, vpath, subject);
        };
        let Some(disk) = project.path_for(vpath) else {
            return self.write_at(project, vpath, subject);
        };
        let body = Self::entry_text(&(table.clone(), key.clone(), statements.clone()));
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&disk)
            .and_then(|mut file| file.write_all(body.as_bytes()))
            .map_err(|e| format!("{vpath}: {e}"))
    }

    /// Write to any of the project's revert files. See [`Undo::open_at`].
    ///
    /// Oldest entry first, which is the order an apply adds them in and the
    /// order [`Self::append_newest_at`] continues. It is the file as it stands
    /// while an apply is running; [`Self::write_finished_at`] is what replaces
    /// it when the apply is done.
    pub(crate) fn write_at(
        &self,
        project: &Project,
        vpath: &str,
        subject: &str,
    ) -> Result<(), String> {
        if self.entries.is_empty() {
            return Ok(());
        }
        let mut body = format!(
            "-- {} — how to put {subject} back.\n\
             -- An apply is writing this file: one entry per row, each added\n\
             -- immediately before that row's own statements run. Run the entries\n\
             -- from the LAST to the first. Every statement can be run twice.\n\n",
            project.name
        );
        for entry in &self.entries {
            body.push_str(&Self::entry_text(entry));
        }
        project
            .write(vpath, body.as_bytes())
            .map_err(|e| format!("{vpath}: {e}"))?;
        Ok(())
    }

    /// The file as a finished apply leaves it: newest entry first, so that
    /// running it top to bottom by hand is [`Self::put_back`]. The
    /// [`Self::NEWEST_FIRST`] line is what tells [`Self::from_text`] which way
    /// round it is.
    pub(crate) fn write_finished_at(
        &self,
        project: &Project,
        vpath: &str,
        subject: &str,
    ) -> Result<(), String> {
        if self.entries.is_empty() {
            return Ok(());
        }
        let mut body = format!(
            "-- {} — how to put {subject} back.\n\
             -- One entry per row this project applied, each read from the database\n\
             -- immediately before that row was written. Run top to bottom, it\n\
             -- returns the database to what it was before this project. Every\n\
             -- statement can be run twice. Plain SQL: it can be run by hand.\n\
             {}\n\n",
            project.name,
            Self::NEWEST_FIRST
        );
        for entry in self.entries.iter().rev() {
            body.push_str(&Self::entry_text(entry));
        }
        project
            .write(vpath, body.as_bytes())
            .map_err(|e| format!("{vpath}: {e}"))?;
        Ok(())
    }

    pub(crate) fn entry_text((table, key, statements): &(String, Key, Vec<String>)) -> String {
        let mut body = format!("{}{table} {}\n", Self::MARKER, key.text());
        for statement in statements {
            body.push_str(statement);
            body.push('\n');
        }
        body.push_str(Self::END);
        body.push_str("\n\n");
        body
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
            "{vpath} has been run but could not be removed: {e}. Remove it by hand \
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

    /// The revert file round-trips through its own marker.
    ///
    /// That property is what stops a second apply from overwriting the first
    /// snapshot. A marker that stopped being recognised would read as "this
    /// row has no undo yet", re-snapshot the already-edited row, and leave
    /// Revert restoring the edit instead of removing it, with no error
    /// reported.
    #[test]
    fn the_revert_file_reads_back_what_it_wrote() {
        let written = "-- a project — how to put the server's creatures back.\n\
             -- a second comment line that is not a marker\n\n\
             -- row creature_template entry=68;patch=0\n\
             UPDATE `creature_template` SET `level_min` = 55 WHERE `entry` = 68;\n\n\
             -- row creature guid=19272\n\
             UPDATE `creature` SET `position_z` = 98.7 WHERE `guid` = 19272;\n";
        let undo = Undo::from_text(written);
        assert_eq!(undo.entries.len(), 2);
        assert_eq!(undo.entries[0].0, vale_mangos::creature::TEMPLATE);
        assert_eq!(undo.entries[1].0, vale_mangos::creature::SPAWN);
        assert!(undo.covers(vale_mangos::creature::TEMPLATE, &Key::two(("entry", 68), ("patch", 0))));
        assert!(undo.covers(vale_mangos::creature::SPAWN, &Key::one("guid", 19272)));
        assert!(!undo.covers(vale_mangos::creature::SPAWN, &Key::one("guid", 1)));
    }

    /// An entry cut short by an interrupted append is not run. Its row's
    /// statements never ran either, and half a removal's snapshot (the
    /// `DELETE`s without the `INSERT`s) would delete a spawn that is still
    /// there.
    #[test]
    fn an_entry_without_its_end_is_not_read() {
        let mut undo = Undo {
            entries: Vec::new(),
        };
        undo.add_many(
            vale_mangos::creature::SPAWN,
            &Key::one("guid", 1),
            &["DELETE FROM `creature` WHERE `guid` = 1;".into(), "INSERT INTO `creature` …;".into()],
        );
        undo.add_many(
            vale_mangos::creature::SPAWN,
            &Key::one("guid", 2),
            &["DELETE FROM `creature` WHERE `guid` = 2;".into(), "INSERT INTO `creature` …;".into()],
        );
        let whole: String = undo.entries.iter().map(Undo::entry_text).collect();
        assert_eq!(Undo::from_text(&whole).entries, undo.entries);
        // The second entry torn after its `DELETE`.
        let cut = whole.find("INSERT INTO `creature` …;\n-- end\n\n-- row creature guid=2").unwrap();
        let torn = format!(
            "{}-- row creature guid=2\nDELETE FROM `creature` WHERE `guid` = 2;\n",
            &whole[..cut + "INSERT INTO `creature` …;\n-- end\n\n".len()]
        );
        let read = Undo::from_text(&torn);
        assert_eq!(read.entries.len(), 1);
        assert!(read.covers(vale_mangos::creature::SPAWN, &Key::one("guid", 1)));
        assert!(!read.covers(vale_mangos::creature::SPAWN, &Key::one("guid", 2)));
    }

    /// The same id in two tables is two rows, and so is the same entry at
    /// two patches.
    #[test]
    fn the_table_and_the_whole_key_are_what_an_undo_covers() {
        let mut undo = Undo {
            entries: Vec::new(),
        };
        let key = Key::two(("entry", 68), ("patch", 0));
        undo.add(vale_mangos::creature::TEMPLATE, &key, "UPDATE `creature_template` …;");
        assert!(undo.covers(vale_mangos::creature::TEMPLATE, &key));
        assert!(!undo.covers(vale_mangos::creature::SPAWN, &key));
        assert!(!undo.covers(vale_mangos::creature::TEMPLATE, &Key::two(("entry", 68), ("patch", 3))));
    }

    /// A comment that is not a marker carries no statement with it, so the
    /// file's own header cannot become an entry.
    #[test]
    fn a_comment_is_not_an_entry() {
        assert!(Undo::from_text("-- just a note\nUPDATE x;\n")
            .entries
            .is_empty());
    }

    /// A file written before the marker named a table is still read: the old
    /// marker was `-- entry <id>`, and its table was always `spell_template`.
    #[test]
    fn the_old_marker_is_still_read_as_a_spell() {
        let undo = Undo::from_text(
            "-- entry 133\n\
             UPDATE `spell_template` SET `effectBasePoints1` = 13 WHERE `entry` = 133;\n",
        );
        assert_eq!(undo.entries.len(), 1);
        assert_eq!(undo.entries[0].0, vale_mangos::spell::TABLE);
        assert!(undo.covers(vale_mangos::spell::TABLE, &Key::two(("entry", 133), ("build", 5875))));
    }

    /// A client-table file of one statement per marker reads back one entry
    /// per row, each its own statement.
    #[test]
    fn a_file_of_one_statement_per_row_reads_back() {
        let written = "-- a project — how to put the server back.\n\n\
             -- row spell_template entry=133;build=5875\n\
             UPDATE `spell_template` SET `effectBasePoints1` = 13 WHERE `entry` = 133;\n\n\
             -- row spell_template entry=2136;build=5875\n\
             DELETE FROM `spell_template` WHERE `entry` = 2136 AND `build` = 5875;\n";
        let undo = Undo::from_text(written);
        assert_eq!(undo.entries.len(), 2);
        assert!(undo.entries[0].2[0].starts_with("UPDATE"));
        assert!(undo.entries[1].2[0].starts_with("DELETE"));
        assert!(!undo.covers(vale_mangos::spell::TABLE, &Key::two(("entry", 133), ("build", 5464))));
    }

    #[test]
    fn a_guid_past_the_space_is_refused() {
        let limit = vale_mangos::creature::MAX_GUID;
        assert!(refuse_a_guid_past_the_limit("creature", &Key::one("guid", limit), limit).is_ok());
        assert!(refuse_a_guid_past_the_limit("creature", &Key::one("guid", limit + 1), limit).is_err());
    }
}
