//! Keeping what was read from the world database no longer than it is true.
//!
//! ## The rule
//!
//! Every value the editor keeps across frames that came out of the world
//! database (a list of rows, a name, a count, a template row for a form) is
//! keyed on [`EditSession::database_writes`], and is read again after that
//! counter moves. The counter moves on every Apply and every Put back of any
//! subject, whether or not it succeeded, and when the editor is pointed at a
//! different database ([`watch_the_database`]).
//!
//! A value that names a row the project also claims is drawn with the
//! project's claim over it: a creature this project creates is called what
//! the project calls it, whatever the database says or does not say. See
//! `crate::tools::quests::Quests::holder` and `Quests::item`.
//!
//! ## Why
//!
//! A name read once and kept for the session showed a creature under the
//! name an earlier row with the same entry had: the project created it,
//! applied it, put it back and discarded it, and a new creature at that entry
//! was still drawn with the old name, because the cache had never been told
//! the database changed and did not look at the project's own row. The
//! editor's reads had grown one cache at a time, each with its own idea of
//! when it was stale, and nine of the functions that start a read checked
//! nothing.
//!
//! ## What enforces it
//!
//! * One counter for every subject, so a list is re-read after another
//!   subject's write moves its rows (a renumber does).
//! * [`Fresh`], a cache that empties itself when the counter has moved.
//! * The test `every_read_is_started_by_a_function_that_checks_the_counter`,
//!   which reads this crate's source and fails when a function that starts a
//!   database read (`queue::read(`) does not mention the counter first.

use crate::session::EditSession;
use vale_mangos::row::{Edits, Life};
use bevy::prelude::*;

/// A value read from the world database, and the
/// [`EditSession::database_writes`] it was read at. [`Self::renew`] empties it
/// when the database has been written since.
#[derive(Debug, Default)]
pub struct Fresh<T> {
    value: T,
    read_at: Option<u64>,
}

impl<T: Default> Fresh<T> {
    /// Empty the value when `writes` is not what it was read at. `true` when
    /// it was emptied, which is when the caller reads again.
    pub fn renew(&mut self, writes: u64) -> bool {
        if self.read_at == Some(writes) {
            return false;
        }
        self.value = T::default();
        self.read_at = Some(writes);
        true
    }

    /// Whether a read started at `writes` still describes the database, for a
    /// task that answers after the counter may have moved.
    pub fn is_current(&self, writes: u64) -> bool {
        self.read_at == Some(writes)
    }
}

impl<T> std::ops::Deref for Fresh<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> std::ops::DerefMut for Fresh<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

/// One column of the project's claim on a template row: a creature's, a game
/// object's or an item's, by entry, as a plain value rather than the SQL
/// literal the store holds. A template can be claimed at several content
/// patches; the highest is the one the server loads, so it is the one read. A
/// row the project removes answers nothing.
pub fn claimed(edits: &Edits, table: &str, entry: u32, column: &str) -> Option<String> {
    edits
        .rows()
        .filter(|(claimed, key, row)| {
            *claimed == table && key.first() == Some(entry as u64) && row.life != Life::Delete
        })
        .filter_map(|(_, key, row)| {
            let patch: u64 = key
                .0
                .iter()
                .find(|(name, _)| name == "patch")
                .and_then(|(_, value)| value.parse().ok())
                .unwrap_or(0);
            Some((patch, row.columns.get(column)?))
        })
        .max_by_key(|(patch, _)| *patch)
        .map(|(_, value)| crate::tools::items::unquote(value))
}

/// The creatures that name a shared row through one `creature_template`
/// column (`vendor_id`, `trainer_id`, `spell_list_id`), as entry and name: the
/// ones the database answered, with the project's claims over them. A
/// template the project points elsewhere is left out, and one it points here,
/// created or edited, is added under the name the project gives it.
pub fn naming(from_database: &[(u32, String)], edits: &Edits, column: &str, id: u32) -> Vec<(u32, String)> {
    let table = vale_mangos::creature::TEMPLATE;
    let mut out: Vec<(u32, String)> = from_database
        .iter()
        .filter(|(entry, _)| match claimed(edits, table, *entry, column) {
            Some(value) => value.trim().parse::<u32>().ok() == Some(id),
            None => true,
        })
        .cloned()
        .collect();
    for (claimed_table, key, row) in edits.rows() {
        if claimed_table != table || row.life == Life::Delete || !row.columns.contains_key(column) {
            continue;
        }
        let Some(entry) = key.first().map(|entry| entry as u32) else {
            continue;
        };
        let names_it = claimed(edits, table, entry, column).and_then(|value| value.trim().parse::<u32>().ok()) == Some(id);
        if names_it && !out.iter().any(|(had, _)| *had == entry) {
            let name = claimed(edits, table, entry, "name")
                .or_else(|| from_database.iter().find(|(had, _)| *had == entry).map(|(_, name)| name.clone()))
                .unwrap_or_else(|| format!("entry {entry}"));
            out.push((entry, name));
        }
    }
    for (entry, name) in out.iter_mut() {
        if let Some(renamed) = claimed(edits, table, *entry, "name") {
            *name = renamed;
        }
    }
    out.sort_by_key(|(entry, _)| *entry);
    out
}

/// Move the counter when the editor is pointed at a different database: the
/// Server panel's `mangosd.conf`, or the environment it reads first. Every
/// cache of the old database is then read again from the new one.
pub fn watch_the_database(
    settings: Res<super::settings::ServerSettings>,
    session: Option<ResMut<EditSession>>,
    mut last: Local<Option<Option<vale_mangos::conn::Where>>>,
) {
    // `resolve` reads the configuration file, so it is asked only when the
    // settings have changed, and once at the start.
    if last.is_some() && !settings.is_changed() {
        return;
    }
    let now = settings.resolve().map(|(at, _)| at);
    let moved = last.as_ref().is_some_and(|was| *was != now);
    *last = Some(now);
    if moved {
        if let Some(mut session) = session {
            info!("server: the world database changed; every cache of it is read again");
            session.wrote_the_database();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_value_empties_when_the_counter_moves() {
        let mut names: Fresh<Vec<&str>> = Fresh::default();
        assert!(names.renew(0), "never read");
        names.push("Hamfort Gaga");
        assert!(!names.renew(0));
        assert_eq!(names.len(), 1);
        assert!(names.renew(1), "an apply or a restore moved the counter");
        assert!(names.is_empty());
        assert!(names.is_current(1));
        assert!(!names.is_current(0), "a read started before the move is not kept");
    }

    /// The creatures sharing a list are the database's with the project's
    /// `vendor_id` edits over them: one pointed away is gone, one pointed here
    /// or created naming it is added, and a rename is shown.
    #[test]
    fn a_shared_list_is_named_by_the_projects_templates_too() {
        use vale_mangos::creature::{template_key, TEMPLATE};
        use vale_mangos::row::RowEdit;
        let from_database = vec![(100, "Old Vendor".to_string()), (101, "Other Vendor".to_string())];
        let mut edits = Edits::default();
        edits.set(TEMPLATE, &template_key(100, 0), "vendor_id", Some("0".into()));
        edits.set(TEMPLATE, &template_key(101, 0), "name", Some("'Renamed'".into()));
        let mut created = RowEdit { life: Life::Insert, ..RowEdit::default() };
        created.columns.insert("vendor_id".into(), "7".into());
        created.columns.insert("name".into(), "'Hobart Stefa'".into());
        edits.set_row_line(TEMPLATE, &template_key(2_000_000, 10), Some(&created.to_line()));
        let users = naming(&from_database, &edits, "vendor_id", 7);
        assert_eq!(users, vec![(101, "Renamed".to_string()), (2_000_000, "Hobart Stefa".to_string())]);
    }

    /// Every function in this crate that starts a database read mentions
    /// `database_writes` (or renews a [`Fresh`]) before it does, which is
    /// where it decides whether what it holds is still true. A new read that
    /// keeps its answer without that check fails here. The check is on the
    /// source text, from the function's `fn` to the read.
    #[test]
    fn every_read_is_started_by_a_function_that_checks_the_counter() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("the source folder") {
                let path = entry.expect("an entry").path();
                match path.is_dir() {
                    true => stack.push(path),
                    false if path.extension().is_some_and(|e| e == "rs") => files.push(path),
                    false => {}
                }
            }
        }
        let mut unchecked = Vec::new();
        let mut reads = 0;
        for file in files {
            // `queue.rs` defines the read, and this file names it in the check.
            let skip = ["queue.rs", "fresh.rs"];
            if file.parent().is_some_and(|dir| dir.ends_with("server"))
                && file.file_name().is_some_and(|name| skip.iter().any(|s| name == *s))
            {
                continue;
            }
            let text = std::fs::read_to_string(&file).expect("a source file");
            for (at, _) in text.match_indices("queue::read(") {
                reads += 1;
                let before = &text[..at];
                let start = before
                    .match_indices("fn ")
                    .map(|(i, _)| i)
                    .filter(|&i| {
                        let line = &before[before[..i].rfind('\n').map_or(0, |n| n + 1)..i];
                        line.trim().is_empty() || line.trim() == "pub" || line.trim().starts_with("pub(")
                    })
                    .last()
                    .unwrap_or(0);
                let body = &before[start..];
                if !body.contains("database_writes") && !body.contains(".renew(") {
                    let name: String = body[3..].chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    let line = text[..at].lines().count();
                    unchecked.push(format!("{}:{line} in fn {name}", file.strip_prefix(&root).unwrap_or(&file).display()));
                }
            }
        }
        assert!(reads > 20, "the scan found {reads} reads; it is not reading the source");
        assert!(
            unchecked.is_empty(),
            "these start a database read without checking EditSession::database_writes:\n{}",
            unchecked.join("\n")
        );
    }
}
