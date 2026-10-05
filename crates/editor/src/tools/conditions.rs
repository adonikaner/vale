//! The condition window's state: the `conditions` table, read whole, with the
//! project's edits over it, and what an edit to a condition is.
//!
//! A condition is opened by its entry from any form whose column names one:
//! the form posts the entry with [`ask_to_open`] and the window takes it on
//! the next frame, so a form needs nothing of this module's state. The window
//! shows one condition at a time, with a trail back through the conditions an
//! AND, an OR or a NOT led it to.
//!
//! The table is read whole when the window first opens, a few thousand rows,
//! and kept until an apply moves the database
//! (`EditSession::database_writes`). An edit is a row of the project's store
//! on `crate::tools::services`' terms. What the table refuses is
//! `vale_mangos::condition`: a combining condition must name lower entries
//! than itself, and two rows may not test the same thing.

use crate::session::{EditSession, Gesture};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::HashMap;
use vale_mangos::condition::{self, Condition};
use vale_mangos::row::{Edits, Life};

/// Where a form posts the entry it wants the window to open, in egui's frame
/// memory; see [`ask_to_open`].
pub fn request_id() -> bevy_egui::egui::Id {
    bevy_egui::egui::Id::new("vale-open-condition")
}

/// Ask the condition window to open `entry`, or a new condition for 0. Any
/// form can call this with the `egui::Context` it draws on.
pub fn ask_to_open(ctx: &bevy_egui::egui::Context, entry: u32) {
    ctx.data_mut(|data| data.insert_temp(request_id(), entry));
}

/// One condition as the window draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub condition: Condition,
    /// The row as the database holds it, or `None` for one the project
    /// creates.
    pub in_database: Option<Condition>,
    pub life: Life,
}

/// The window's state.
#[derive(Resource, Default)]
pub struct Conditions {
    pub open: bool,
    /// The condition shown, or `None` for the page that makes a new one.
    pub showing: Option<u32>,
    /// The conditions the window came from, newest last, for Back.
    pub trail: Vec<u32>,
    /// The entry typed into the Open field.
    pub typed: u32,
    /// The type a new condition is made with.
    pub new_type: i32,
    held: Option<HashMap<u32, Condition>>,
    loaded_for: Option<u64>,
    reading: Option<Task<Result<Vec<Condition>, String>>>,
    /// Why the last read answered nothing, when it answered nothing.
    pub trouble: Option<String>,
}

impl Conditions {
    /// Show a condition, keeping the one shown now on the trail.
    pub fn show(&mut self, entry: Option<u32>) {
        if let Some(now) = self.showing.filter(|now| Some(*now) != entry) {
            self.trail.push(now);
        }
        self.showing = entry;
        self.open = true;
    }

    /// Go back to the condition shown before.
    pub fn back(&mut self) {
        if let Some(previous) = self.trail.pop() {
            self.showing = Some(previous);
        }
    }

    /// Whether the table has been read.
    pub fn read(&self) -> bool {
        self.held.is_some()
    }

    /// One condition, the database's with the project's edits over it, or the
    /// project's creation. `None` when neither has it, or the table is not
    /// read.
    pub fn shown(&self, edits: &Edits, entry: u32) -> Option<Shown> {
        let held = self.held.as_ref()?.get(&entry);
        let assignments = held.map(Condition::assignments);
        let (row, life) = super::triggers::shown(edits, condition::TABLE, &condition::key(entry), assignments.as_deref())?;
        Some(Shown {
            condition: Condition::from_row(&row)?,
            in_database: held.filter(|_| life != Life::Insert).cloned(),
            life,
        })
    }

    /// Every condition as the project leaves it, in entry order, removals
    /// left out.
    pub fn all(&self, edits: &Edits) -> Vec<Condition> {
        let Some(held) = self.held.as_ref() else {
            return Vec::new();
        };
        let mut entries: Vec<u32> = held.keys().copied().collect();
        entries.extend(
            edits
                .rows()
                .filter(|(table, _, row)| *table == condition::TABLE && row.life == Life::Insert)
                .filter_map(|(_, key, _)| key.first().map(|entry| entry as u32)),
        );
        entries.sort_unstable();
        entries.dedup();
        entries
            .into_iter()
            .filter_map(|entry| self.shown(edits, entry))
            .filter(|shown| shown.life != Life::Delete)
            .map(|shown| shown.condition)
            .collect()
    }

    /// The conditions that name `entry` as a child.
    pub fn users(&self, edits: &Edits, entry: u32) -> Vec<u32> {
        self.all(edits)
            .into_iter()
            .filter(|other| other.children().contains(&entry))
            .map(|other| other.entry)
            .collect()
    }

    /// The entry a new condition takes: one above every entry the table holds
    /// and the project creates, which is also above any child it names.
    pub fn next_entry(&self, edits: &Edits) -> Option<u32> {
        let held = self.held.as_ref()?.keys().copied().max().unwrap_or(0);
        let created = edits
            .rows()
            .filter(|(table, _, row)| *table == condition::TABLE && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .unwrap_or(0) as u32;
        Some(held.max(created) + 1)
    }

    /// Make a condition of `kind` with `values`, or answer the condition that
    /// already tests exactly that, which the table would refuse a second of.
    /// Answers the entry, and whether it is new; `None` while the table is
    /// not read.
    pub fn create(&mut self, session: &mut EditSession, kind: i32, values: [i32; 4], flags: u8, now: f64) -> Option<(u32, bool)> {
        let entry = self.next_entry(&session.server_edits)?;
        let made = Condition { entry, kind, values, flags };
        let all = self.all(&session.server_edits);
        if let Some(same) = condition::same_test(&made, &all) {
            return Some((same.entry, false));
        }
        let key = made.key();
        let label = format!("{} {}", condition::TABLE, key.text());
        let row = super::services::creation(&made.assignments());
        session.set_server_row(condition::TABLE, &key, Some(&row), Some(Gesture { label: "Add condition", subject: &label, now }));
        Some((entry, true))
    }

    /// Set one column of a condition. `value` is a SQL literal.
    pub fn set(&mut self, session: &mut EditSession, shown: &Shown, column: &'static str, value: String, now: f64) {
        let key = shown.condition.key();
        let label = format!("{} {} {column}", condition::TABLE, key.text());
        let gesture = Gesture { label: "Edit condition", subject: &label, now };
        let held = shown.in_database.as_ref().map(Condition::assignments);
        super::services::write_column(session, condition::TABLE, &key, held.as_deref(), column, value, gesture);
    }

    /// Remove a condition: a `Delete` claim for one the database holds, and
    /// the creation taken back for one the project makes.
    pub fn remove(&mut self, session: &mut EditSession, shown: &Shown, now: f64) {
        let key = shown.condition.key();
        let label = format!("{} {}", condition::TABLE, key.text());
        let gesture = Gesture { label: "Remove condition", subject: &label, now };
        super::triggers::remove_row(session, condition::TABLE, &key, shown.in_database.is_some(), gesture);
    }

    /// Why the server would skip a condition: its own faults, a child that
    /// does not exist, and a row that already tests the same thing.
    pub fn faults(&self, edits: &Edits, shown: &Condition) -> Vec<String> {
        let all = self.all(edits);
        let mut out = shown.check();
        out.extend(condition::check_tree(shown, |entry| all.iter().any(|other| other.entry == entry)));
        if let Some(same) = condition::same_test(shown, &all) {
            out.push(format!("condition {} tests exactly the same, and the table holds only one such row", same.entry));
        }
        out
    }
}

pub struct ConditionToolPlugin;

impl Plugin for ConditionToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Conditions>().add_systems(Update, (on_the_command_line, read_the_rows).chain());
    }
}

/// `--condition <id>`: the window open on one condition from the first frame.
fn on_the_command_line(args: Res<crate::Args>, mut conditions: ResMut<Conditions>, mut done: Local<bool>) {
    if !*done {
        *done = true;
        if let Some(entry) = args.condition {
            conditions.show(Some(entry));
        }
    }
}

/// Read the table while the window is open, and forget it when an apply has
/// moved the database.
fn read_the_rows(
    mut conditions: ResMut<Conditions>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    if let Some(task) = conditions.reading.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            conditions.reading = None;
            match done {
                Ok(rows) => {
                    info!("conditions: {} row(s)", rows.len());
                    conditions.held = Some(rows.into_iter().map(|row| (row.entry, row)).collect());
                    conditions.trouble = None;
                }
                Err(e) => {
                    warn!("conditions: {e}");
                    conditions.trouble = Some(e);
                }
            }
        }
        return;
    }
    let Some(session) = session else { return };
    if conditions.loaded_for != Some(session.database_writes) {
        conditions.loaded_for = Some(session.database_writes);
        conditions.held = None;
        conditions.trouble = None;
    }
    if !conditions.open || conditions.held.is_some() || conditions.trouble.is_some() {
        return;
    }
    let Some((at, _source)) = settings.resolve() else {
        conditions.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    conditions.reading = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        Ok(db.rows(&condition::all_query())?.iter().filter_map(Condition::from_row).collect())
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(rows: Vec<Condition>) -> Conditions {
        Conditions {
            held: Some(rows.into_iter().map(|row| (row.entry, row)).collect()),
            ..Conditions::default()
        }
    }

    /// A new condition takes an entry above every other, and a repeated test
    /// answers the row that already makes it.
    #[test]
    fn a_new_condition_is_numbered_above_the_rest_and_not_repeated() {
        let install = std::env::temp_dir().join(format!("vale-conditions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        let mut conditions = read(vec![Condition { entry: 40, kind: 8, values: [100, 0, 0, 0], flags: 0 }]);
        assert_eq!(conditions.create(&mut session, 8, [100, 0, 0, 0], 0, 1.0), Some((40, false)));
        assert_eq!(conditions.create(&mut session, 8, [101, 0, 0, 0], 0, 1.0), Some((41, true)));
        assert_eq!(conditions.next_entry(&session.server_edits), Some(42));
        let shown = conditions.shown(&session.server_edits, 41).unwrap();
        assert_eq!((shown.life, shown.condition.values[0]), (Life::Insert, 101));
        // An AND over both is made above them, and names them as users see.
        let (and, _) = conditions.create(&mut session, -1, [40, 41, 0, 0], 0, 2.0).unwrap();
        assert_eq!(and, 42);
        assert_eq!(conditions.users(&session.server_edits, 41), vec![42]);
        assert!(conditions.faults(&session.server_edits, &conditions.shown(&session.server_edits, 42).unwrap().condition).is_empty());
        let _ = std::fs::remove_dir_all(&install);
    }
}
