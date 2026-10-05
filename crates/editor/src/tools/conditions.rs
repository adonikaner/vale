//! The condition window's state: the `conditions` table, read whole, with the
//! project's edits over it, and what an edit to a condition is.
//!
//! ## How a form reaches the window
//!
//! A form column that names a condition is drawn by `ui::conditions::cell`,
//! and the form holds none of this module's state. Three messages pass
//! through egui's frame memory instead:
//!
//! ```text
//! Request    form -> window   open an entry, the new page or the list, and
//!                             which column to answer (`Asker`)
//! answer     window -> form   an entry made or chosen for that column, which
//!                             the form writes as it writes a typed number
//! Board      window -> forms  what each entry a form showed tests, for the
//!                             line beside its number
//! ```
//!
//! The window shows one condition at a time, with a trail back through the
//! conditions an AND, an OR or a NOT led it to.
//!
//! ## Editing a condition as a tree
//!
//! The window's Tests view edits a [`Draft`]: the condition read as a
//! `vale_mangos::condition::Node` tree, changed in memory, and written only
//! by [`Conditions::save`]. Save never changes a row that exists. It names
//! the rows that already test each part of the tree and creates the rest
//! (`vale_mangos::condition::build`), as one undo entry, and the column the
//! window answers is set to the result. A row of `conditions` is commonly
//! named by many loot rows, quests and other conditions, and writing a new
//! row keeps an edit made for one column from changing the others. The Rows
//! view still edits one row where it is.
//!
//! ## When the table is read
//!
//! The table is read whole, a few thousand rows, when the window opens or a
//! form shows a column that holds a condition, and kept until an apply moves
//! the database (`EditSession::database_writes`). An edit is a row of the
//! project's store, written as `crate::tools::services` writes one. The rules
//! the table enforces are in `vale_mangos::condition`: a combining condition
//! must name lower entries than itself, and two rows may not test the same
//! thing.

use crate::session::{EditSession, Gesture};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use bevy_egui::egui;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use vale_mangos::condition::{self, Condition, Node};
use vale_mangos::row::{Edits, Life};

/// Where a form posts its [`Request`], in egui's frame memory.
pub fn request_id() -> egui::Id {
    egui::Id::new("vale-open-condition")
}

/// What a form asks of the window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Request {
    /// The condition to show, or 0 for the page that makes one.
    pub entry: u32,
    /// Show the list of every condition instead.
    pub list: bool,
    /// The column a made or chosen condition is written into, or `None`.
    pub asker: Option<Asker>,
}

/// The form column the window answers into.
#[derive(Debug, Clone, PartialEq)]
pub struct Asker {
    /// Where the answer is posted; the form's cell reads it back from there.
    pub reply: egui::Id,
    /// The table and column, as the window names it.
    pub column: String,
    /// The entry the column holds.
    pub holds: u32,
}

/// Ask the condition window for something. Any form can call this with the
/// `egui::Context` it draws on; the window takes it on its next draw.
pub fn ask(ctx: &egui::Context, request: Request) {
    ctx.data_mut(|data| data.insert_temp(request_id(), request));
}

/// Post `entry` to the column that asked, and remember that the column holds
/// it now. Nothing when no column asked.
pub fn answer(ctx: &egui::Context, conditions: &mut Conditions, entry: u32) {
    if let Some(asker) = conditions.asker.as_mut() {
        ctx.data_mut(|data| data.insert_temp(asker.reply, entry));
        asker.holds = entry;
    }
}

/// Take the entry the window answered into the column whose reply id is
/// `reply`, once.
pub fn take_answer(ctx: &egui::Context, reply: egui::Id) -> Option<u32> {
    ctx.data_mut(|data| data.remove_temp::<u32>(reply))
}

/// Where the forms list the entries they showed since the window last drew.
fn asked_id() -> egui::Id {
    egui::Id::new("vale-conditions-asked")
}

/// Where the window posts the [`Board`].
fn board_id() -> egui::Id {
    egui::Id::new("vale-conditions-board")
}

/// Note that a form shows `entry`, so the window says what it tests on its
/// next draw.
pub fn ask_about(ctx: &egui::Context, entry: u32) {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<HashSet<u32>>(asked_id()).insert(entry);
    });
}

/// The entries the forms showed since the last call, taken.
pub fn take_asked(ctx: &egui::Context) -> HashSet<u32> {
    ctx.data_mut(|data| data.remove_temp::<HashSet<u32>>(asked_id())).unwrap_or_default()
}

/// What one entry tests, as a form shows it beside the number.
#[derive(Debug, Clone, PartialEq)]
pub enum Told {
    /// The condition in one line, and why the server would skip it.
    Tests { line: String, faults: Vec<String> },
    /// Neither the table nor the project holds the entry.
    Missing,
    /// The project removes it.
    Removed,
}

/// What the window last told the forms.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Board {
    /// Why the table could not be read, when it could not.
    pub trouble: Option<String>,
    /// Whether the table has been read.
    pub read: bool,
    pub told: HashMap<u32, Told>,
}

/// Post the board for the forms to read.
pub fn post_board(ctx: &egui::Context, board: Board) {
    ctx.data_mut(|data| data.insert_temp(board_id(), Arc::new(board)));
}

/// The board the window last posted, or an empty one.
pub fn board(ctx: &egui::Context) -> Arc<Board> {
    ctx.data(|data| data.get_temp::<Arc<Board>>(board_id())).unwrap_or_default()
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

/// A condition being edited as a tree. See the module doc.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// The entry the tree was read from, or 0 for a new condition.
    pub from: u32,
    pub tree: Node,
    /// The tree as read, which Revert goes back to.
    pub read: Node,
    /// The tests whose values are unfolded, by path.
    pub open: HashSet<Vec<usize>>,
}

impl Draft {
    /// The condition `from` read as a tree whose root is always a group, so
    /// that a test can be added beside a condition that is one test. A group
    /// of one member is written as that member, so the rows do not change.
    pub fn read(from: u32, lookup: impl Fn(u32) -> Option<Condition>) -> Draft {
        let tree = match from {
            0 => Node::empty(),
            entry => match condition::tree(entry, lookup) {
                group @ Node::Group { .. } => group,
                one => Node::Group { any: false, flags: 0, members: vec![one] },
            },
        };
        Draft { from, read: tree.clone(), tree, open: HashSet::new() }
    }

    /// Whether the tree differs from what was read.
    pub fn changed(&self) -> bool {
        self.tree != self.read
    }
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
    /// The list of every condition is shown instead of one condition.
    pub listing: bool,
    /// The list's search: an entry, a value, or part of a type's name.
    pub search: String,
    /// The list's type, or `None` for every type.
    pub search_type: Option<i32>,
    /// The form column a made or chosen condition is written into.
    pub asker: Option<Asker>,
    /// A form showed a condition on the last frame, so the table is read
    /// whether or not the window is open.
    pub wanted: bool,
    /// The condition being edited as a tree, in the Tests view.
    pub draft: Option<Draft>,
    /// The Rows view, which edits the shown row's columns, is shown instead
    /// of the Tests view.
    pub rows_view: bool,
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
        self.listing = false;
        self.open = true;
    }

    /// Show the list of every condition.
    pub fn list(&mut self) {
        self.listing = true;
        self.open = true;
    }

    /// Take a form's request.
    pub fn take(&mut self, request: Request) {
        self.asker = request.asker;
        match request.list {
            true => self.list(),
            false => self.show((request.entry != 0).then_some(request.entry)),
        }
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

    /// Whether `entry` is a condition the project leaves in place.
    pub fn exists(&self, edits: &Edits, entry: u32) -> bool {
        self.shown(edits, entry).is_some_and(|shown| shown.life != Life::Delete)
    }

    /// The row an entry names as the project leaves it, or `None` for one
    /// neither holds or the project removes.
    pub fn row(&self, edits: &Edits, entry: u32) -> Option<Condition> {
        self.shown(edits, entry).filter(|shown| shown.life != Life::Delete).map(|shown| shown.condition)
    }

    /// Make sure the draft is the one read from `from` (0 for a new
    /// condition). A draft read from another entry is dropped, changes and
    /// all.
    pub fn draft_for(&mut self, edits: &Edits, from: u32) {
        if self.draft.as_ref().is_some_and(|draft| draft.from == from) || !self.read() {
            return;
        }
        let draft = Draft::read(from, |entry| self.row(edits, entry));
        self.draft = Some(draft);
    }

    /// What Save would write: the entry the draft is, and the rows it would
    /// create. An error says why the draft cannot be written.
    pub fn plan(&self, edits: &Edits) -> Result<condition::Built, String> {
        let draft = self.draft.as_ref().ok_or("there is no condition to save")?;
        let next = self.next_entry(edits).ok_or("the conditions are not read yet")?;
        condition::build(&draft.tree, &self.all(edits), next)
    }

    /// Write the draft: create the rows [`Self::plan`] names, as one undo
    /// entry, and drop the draft. Answers the entry the draft is.
    pub fn save(&mut self, session: &mut EditSession, now: f64) -> Result<u32, String> {
        let built = self.plan(&session.server_edits)?;
        let label = format!("{} {}", condition::TABLE, built.root);
        session.as_one("Edit condition", &label, |session| {
            for row in &built.created {
                let made = super::services::creation(&row.assignments());
                let gesture = Gesture { label: "Edit condition", subject: &label, now };
                session.set_server_row(condition::TABLE, &row.key(), Some(&made), Some(gesture));
            }
        });
        self.draft = None;
        Ok(built.root)
    }

    /// [`Self::faults`] without the search for a repeated test, which reads
    /// every row: the row's own faults and a child that does not exist. For
    /// the line a form draws beside a column, every frame.
    pub fn row_faults(&self, edits: &Edits, shown: &Condition) -> Vec<String> {
        let mut out = shown.check();
        out.extend(condition::check_tree(shown, |entry| self.exists(edits, entry)));
        out
    }

    /// The conditions the list shows, in entry order. `search` is a number,
    /// which matches an entry or any of the four values, or text, which
    /// matches part of a type's name; empty matches every row. `kind`
    /// narrows to one type.
    pub fn matching(&self, edits: &Edits, search: &str, kind: Option<i32>) -> Vec<Condition> {
        let search = search.trim();
        let number: Option<i64> = search.parse().ok();
        let words = search.to_ascii_lowercase();
        self.all(edits)
            .into_iter()
            .filter(|row| kind.is_none_or(|kind| row.kind == kind))
            .filter(|row| match (search.is_empty(), number) {
                (true, _) => true,
                (false, Some(n)) => i64::from(row.entry) == n || row.values.iter().any(|value| i64::from(*value) == n),
                (false, None) => condition::type_of(row.kind).is_some_and(|known| known.name.to_ascii_lowercase().contains(&words)),
            })
            .collect()
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

/// Read the table while the window is open or a form shows a condition, and
/// forget it when an apply has moved the database.
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
    if !(conditions.open || conditions.wanted) || conditions.held.is_some() || conditions.trouble.is_some() {
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
        // An AND over both takes the next entry above them, and `users` lists
        // it as a user of each child.
        let (and, _) = conditions.create(&mut session, -1, [40, 41, 0, 0], 0, 2.0).unwrap();
        assert_eq!(and, 42);
        assert_eq!(conditions.users(&session.server_edits, 41), vec![42]);
        assert!(conditions.faults(&session.server_edits, &conditions.shown(&session.server_edits, 42).unwrap().condition).is_empty());
        let _ = std::fs::remove_dir_all(&install);
    }

    /// The list matches an entry or a value by number, and a type by part of
    /// its name; a type narrows it.
    #[test]
    fn the_list_matches_entries_values_and_type_names() {
        let conditions = read(vec![
            Condition { entry: 5, kind: 8, values: [783, 0, 0, 0], flags: 0 },
            Condition { entry: 6, kind: 2, values: [2589, 5, 0, 0], flags: 0 },
            Condition { entry: 783, kind: -1, values: [5, 6, 0, 0], flags: 0 },
        ]);
        let edits = Edits::default();
        let entries = |search: &str, kind: Option<i32>| -> Vec<u32> {
            conditions.matching(&edits, search, kind).iter().map(|row| row.entry).collect()
        };
        assert_eq!(entries("", None), vec![5, 6, 783]);
        assert_eq!(entries(" 783 ", None), vec![5, 783]);
        assert_eq!(entries("5", None), vec![5, 6, 783]);
        assert_eq!(entries("QUEST", None), vec![5]);
        assert_eq!(entries("", Some(2)), vec![6]);
        assert_eq!(entries("5", Some(-1)), vec![783]);
    }

    /// The window answers the column that opened it, once, and a request
    /// from no column stops the answering.
    #[test]
    fn a_made_or_chosen_condition_is_answered_to_the_column_that_asked() {
        let ctx = egui::Context::default();
        let reply = egui::Id::new("test-column");
        let mut conditions = Conditions::default();
        let asker = Asker { reply, column: "npc_vendor 1 2 condition_id".to_string(), holds: 0 };
        conditions.take(Request { entry: 0, list: true, asker: Some(asker) });
        assert!(conditions.open && conditions.listing);
        answer(&ctx, &mut conditions, 41);
        assert_eq!(take_answer(&ctx, reply), Some(41));
        assert_eq!(take_answer(&ctx, reply), None);
        assert_eq!(conditions.asker.as_ref().map(|asker| asker.holds), Some(41));

        conditions.take(Request { entry: 41, ..Request::default() });
        assert_eq!((conditions.showing, conditions.listing), (Some(41), false));
        answer(&ctx, &mut conditions, 42);
        assert_eq!(take_answer(&ctx, reply), None);
    }

    /// The entries the forms show are collected until the window takes them.
    #[test]
    fn the_entries_forms_show_are_taken_once() {
        let ctx = egui::Context::default();
        ask_about(&ctx, 80);
        ask_about(&ctx, 80);
        ask_about(&ctx, 1);
        assert_eq!(take_asked(&ctx), HashSet::from([1, 80]));
        assert!(take_asked(&ctx).is_empty());
    }
}
