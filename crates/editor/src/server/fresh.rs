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
//! `crate::subjects::quests::Quests::holder` and `Quests::item`.
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
//! * [`FreshTask`], one read on a task at a time: an answer to a read started
//!   before the counter moved is dropped, and the last failure is kept until
//!   it moves. [`FreshReads`] is the same with the answers kept by key. Both
//!   take the counter as an argument to start a read, and start nothing at a
//!   counter they have not been renewed to.
//! * The test `every_read_is_started_by_a_function_that_checks_the_counter`,
//!   which reads this crate's source and fails when a function that starts a
//!   database read (`queue::read(`) does not mention the counter first.

use crate::session::EditSession;
use vale_mangos::row::{Edits, Life};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::HashMap;
use std::hash::Hash;

/// A value read from the world database, and the
/// [`EditSession::database_writes`] it was read at. [`Self::renew`] empties it
/// when the database has been written since.
#[derive(Debug, Default)]
pub(crate) struct Fresh<T> {
    value: T,
    read_at: Option<u64>,
}

impl<T: Default> Fresh<T> {
    /// Empty the value when `writes` is not what it was read at. `true` when
    /// it was emptied, which is when the caller reads again.
    pub(crate) fn renew(&mut self, writes: u64) -> bool {
        if self.read_at == Some(writes) {
            return false;
        }
        self.value = T::default();
        self.read_at = Some(writes);
        true
    }

    /// Whether a read started at `writes` still describes the database, for a
    /// task that answers after the counter may have moved.
    #[cfg(test)]
    pub(crate) fn is_current(&self, writes: u64) -> bool {
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

/// A database read that answers `Result<A, String>`, run on a task.
pub(crate) type Read<A> = Task<Result<A, String>>;

/// One read of the world database at a time, and why the last one failed.
///
/// The caller renews it with [`EditSession::database_writes`] every frame
/// before reading anything out of it, and empties what it keeps from earlier
/// answers when [`Self::renew`] says the counter moved. A read started at an
/// earlier counter is let finish and its answer dropped, so one task is in
/// flight at a time.
pub(crate) struct FreshTask<A> {
    read_at: Option<u64>,
    /// The read in flight and the counter it was started at.
    running: Option<(u64, Read<A>)>,
    /// Why the last read answered nothing. A tool shows it in place of what
    /// it would have read, and starts no read while it is set.
    trouble: Option<String>,
}

impl<A> Default for FreshTask<A> {
    fn default() -> FreshTask<A> {
        FreshTask { read_at: None, running: None, trouble: None }
    }
}

impl<A> std::fmt::Debug for FreshTask<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FreshTask")
            .field("read_at", &self.read_at)
            .field("running", &self.running.as_ref().map(|(at, _)| at))
            .field("trouble", &self.trouble)
            .finish()
    }
}

impl<A> FreshTask<A> {
    /// Note the counter. `true` when it moved since the last call, which is
    /// when the caller empties what it holds; a failure against the database
    /// as it was is forgotten, so the read is tried again.
    pub(crate) fn renew(&mut self, writes: u64) -> bool {
        if self.read_at == Some(writes) {
            return false;
        }
        self.read_at = Some(writes);
        self.trouble = None;
        true
    }

    /// The answer of the read in flight, once it has one. A failure is kept as
    /// [`Self::trouble`] as well as returned. An answer to a read started
    /// before the counter moved is dropped and gives `None`.
    pub(crate) fn poll(&mut self) -> Option<Result<A, String>> {
        let (started, task) = self.running.as_mut()?;
        let started = *started;
        let done = block_on(future::poll_once(task))?;
        self.running = None;
        if Some(started) != self.read_at {
            return None;
        }
        self.trouble = done.as_ref().err().cloned();
        Some(done)
    }

    /// Whether a read is in flight.
    pub(crate) fn is_busy(&self) -> bool {
        self.running.is_some()
    }

    /// Why the last read answered nothing, when it answered nothing.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.trouble.as_ref()
    }

    /// Start a read at `writes`, unless one is in flight, the last one failed,
    /// or [`Self::renew`] has not been given `writes`. `read` builds the task,
    /// or says why it cannot (no database is set); that reason becomes
    /// [`Self::trouble`]. `true` when a read was started.
    pub(crate) fn start(&mut self, writes: u64, read: impl FnOnce() -> Result<Read<A>, String>) -> bool {
        if self.running.is_some() || self.trouble.is_some() || self.read_at != Some(writes) {
            return false;
        }
        match read() {
            Ok(task) => {
                self.running = Some((writes, task));
                true
            }
            Err(why) => {
                self.trouble = Some(why);
                false
            }
        }
    }
}

/// Answers of the world database by key, read one key at a time on a task,
/// and emptied when [`EditSession::database_writes`] moves. A [`FreshTask`]
/// whose answers are kept in a map: a loot set by its table and entry, a
/// vendor list, or one value under the key `()`.
pub(crate) struct FreshReads<K, V> {
    held: HashMap<K, V>,
    /// The key the read in flight is for.
    wanted: Option<K>,
    task: FreshTask<V>,
}

impl<K, V> Default for FreshReads<K, V> {
    fn default() -> FreshReads<K, V> {
        FreshReads { held: HashMap::new(), wanted: None, task: FreshTask::default() }
    }
}

impl<K: std::fmt::Debug, V> std::fmt::Debug for FreshReads<K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FreshReads")
            .field("held", &self.held.keys().collect::<Vec<_>>())
            .field("wanted", &self.wanted)
            .field("task", &self.task)
            .finish()
    }
}

impl<K: Eq + Hash + Clone, V> FreshReads<K, V> {
    /// Note the counter, and empty every answer when it moved. `true` when it
    /// did.
    pub(crate) fn renew(&mut self, writes: u64) -> bool {
        if !self.task.renew(writes) {
            return false;
        }
        self.held.clear();
        true
    }

    /// Keep the answer of the read in flight, once it has one, and say which
    /// key it was for; a failure is returned as the reason. See
    /// [`FreshTask::poll`].
    pub(crate) fn poll(&mut self) -> Option<Result<K, String>> {
        let done = self.task.poll();
        if self.task.is_busy() {
            return None;
        }
        let key = self.wanted.take();
        match (done?, key) {
            (Ok(value), Some(key)) => {
                self.held.insert(key.clone(), value);
                Some(Ok(key))
            }
            (Ok(_), None) => None,
            (Err(why), _) => Some(Err(why)),
        }
    }

    /// Start reading `key` at `writes`, unless it is held, a read is in
    /// flight, or the last one failed. See [`FreshTask::start`].
    pub(crate) fn want(&mut self, writes: u64, key: K, read: impl FnOnce() -> Result<Read<V>, String>) {
        if self.held.contains_key(&key) {
            return;
        }
        if self.task.start(writes, read) {
            self.wanted = Some(key);
        }
    }

    /// The answer held for `key`.
    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        self.held.get(key)
    }

    /// Whether `key` has been read.
    pub(crate) fn is_read(&self, key: &K) -> bool {
        self.held.contains_key(key)
    }

    /// Whether `key` is being read now.
    #[cfg(test)]
    pub(crate) fn is_reading(&self, key: &K) -> bool {
        self.task.is_busy() && self.wanted.as_ref() == Some(key)
    }

    /// Whether a read is in flight.
    pub(crate) fn is_busy(&self) -> bool {
        self.task.is_busy()
    }

    /// Why the last read answered nothing, when it answered nothing.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.task.trouble()
    }

    /// Hold an answer without reading it, for a test.
    #[cfg(test)]
    pub(crate) fn insert(&mut self, key: K, value: V) {
        self.held.insert(key, value);
    }
}

impl<V> FreshReads<(), V> {
    /// The one answer, for a reader that reads everything at once.
    pub(crate) fn value(&self) -> Option<&V> {
        self.held.get(&())
    }
}

/// One column of the project's claim on a template row: a creature's, a game
/// object's or an item's, by entry, as a plain value rather than the SQL
/// literal the store holds. A template can be claimed at several content
/// patches; the highest is the one the server loads, so it is the one read. A
/// row the project removes answers nothing.
pub(crate) fn claimed(edits: &Edits, table: &str, entry: u32, column: &str) -> Option<String> {
    edits
        .rows()
        .filter(|(claimed, key, row)| {
            *claimed == table && key.first() == Some(entry as u64) && row.life != Life::Delete
        })
        .filter_map(|(_, key, row)| {
            let patch = key.patch().unwrap_or(0);
            Some((patch, row.columns.get(column)?))
        })
        .max_by_key(|(patch, _)| *patch)
        .map(|(_, value)| vale_mangos::sql::unquote(value))
}

/// The creatures that name a shared row through one `creature_template`
/// column (`vendor_id`, `trainer_id`, `spell_list_id`), as entry and name: the
/// ones the database answered, with the project's claims over them. A
/// template the project points elsewhere is left out, and one it points here,
/// created or edited, is added under the name the project gives it.
pub(crate) fn naming(from_database: &[(u32, String)], edits: &Edits, column: &str, id: u32) -> Vec<(u32, String)> {
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
pub(crate) fn watch_the_database(
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

    /// A read started before the counter moved is let finish and its answer
    /// dropped; a failure is kept until the counter moves, and no read starts
    /// while it is kept.
    #[test]
    fn a_keyed_read_drops_an_answer_from_before_a_write() {
        let pool = bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default);
        let answer = |name: &'static str| move || Ok(pool.spawn(async move { Ok::<_, String>(name) }));
        // The read runs on another thread: poll until it has answered.
        let settle = |reads: &mut FreshReads<u32, &'static str>| loop {
            let done = reads.poll();
            if !reads.is_busy() {
                return done;
            }
            std::thread::yield_now();
        };
        let mut reads: FreshReads<u32, &'static str> = FreshReads::default();
        reads.renew(0);
        reads.want(0, 7, answer("Hamfort Gaga"));
        assert!(reads.is_reading(&7));
        assert_eq!(settle(&mut reads), Some(Ok(7)));
        assert_eq!(reads.get(&7), Some(&"Hamfort Gaga"));
        reads.renew(1);
        assert!(!reads.is_read(&7), "an apply moved the counter");
        reads.want(1, 7, answer("Old name"));
        reads.renew(2);
        assert_eq!(settle(&mut reads), None, "started at 1, answered at 2");
        assert!(!reads.is_read(&7));
        reads.want(1, 7, answer("stale"));
        assert!(!reads.is_busy(), "the counter is at 2");
        reads.want(2, 7, || Err("no database".to_string()));
        assert_eq!(reads.trouble().map(String::as_str), Some("no database"));
        reads.want(2, 8, answer("never"));
        assert!(!reads.is_busy(), "a failure stops the next read");
        reads.renew(3);
        assert!(reads.trouble().is_none());
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
