//! The Creatures workspace: every `creature_template` row the server would
//! load, as a list, with the open row's form in the middle of the screen.
//!
//! ## A second way to the creature tool's rows
//!
//! The creature tool reaches a template through a spawn: a click on the
//! ground, or the Place picker's search, which lists fifty matches. This
//! workspace reaches the same rows as a list, with no map involved, so a
//! creature with no spawn anywhere is found as easily as one in Goldshire.
//!
//! The rows are the creature tool's own. The list is `Creatures::listed`; the
//! open row is read into `Creatures::template` by the tool's own
//! [`super::creatures::fetch_rows`]; the form is the template window's form;
//! and a column edit, a new row, a copy, a discard and a renumber are the
//! tool's own calls with the same undo entries. A row edited here is the row
//! the template window shows, and the Creatures block of the bar's Server…
//! applies both. This file holds what the list adds: the row type's search
//! text, the read, and `--creature-data`. The list mechanics are
//! [`super::listing`], shared with the Game Objects workspace.

use super::listing::{ListRead, Listed};
use super::Tool;
use crate::session::EditSession;
use crate::tools::creatures::{self, Creatures, Known};
use crate::tools::items::unquote;
use vale_mangos::creature::{self as table, RowValue};
use vale_mangos::row::{Edits, Key, RowEdit};
use bevy::prelude::*;

impl Listed for Known {
    const TABLE: &'static str = table::TEMPLATE;

    fn entry(&self) -> u32 {
        self.entry
    }

    fn read_entry(&self) -> u32 {
        self.read_entry
    }

    fn patch(&self) -> u32 {
        self.patch
    }

    fn set_entry(&mut self, entry: u32) {
        self.entry = entry;
    }

    fn key(&self) -> Key {
        Known::key(self)
    }

    fn label(&self) -> String {
        Known::label(self)
    }

    /// The name and the subname, so `guard` and `<trade supplies>` both
    /// find a row.
    fn haystack(&self) -> String {
        match self.subname.as_deref() {
            Some(subname) => format!("{} <{subname}>", self.name).to_lowercase(),
            None => self.name.to_lowercase(),
        }
    }

    fn with_edits(&self, edits: &Edits) -> Self {
        let key = Known::key(self);
        if !edits.touches(table::TEMPLATE, &key) {
            return self.clone();
        }
        let get = |column: &str| edits.get(table::TEMPLATE, &key, column);
        let number = |column: &str| get(column).and_then(|value| value.trim().parse::<f64>().ok());
        let mut out = self.clone();
        out.claim = edits.life(table::TEMPLATE, &key);
        if let Some(name) = get("name") {
            out.name = unquote(name);
        }
        if let Some(subname) = get("subname") {
            out.subname = Some(unquote(subname));
        }
        if let Some(id) = number("display_id1") {
            out.display_id = id as u32;
        }
        if let Some(scale) = number("display_scale1") {
            out.display_scale = scale as f32;
        }
        if let Some(level) = number("level_min") {
            out.level.0 = level as u32;
        }
        if let Some(level) = number("level_max") {
            out.level.1 = level as u32;
        }
        if let Some(faction) = number("faction") {
            out.faction = faction as u32;
        }
        if let Some(flags) = number("npc_flags") {
            out.npc_flags = flags as u32;
        }
        if let Some(rank) = number("rank") {
            out.rank = rank as u32;
        }
        out
    }

    fn from_claim(key: &Key, row: &RowEdit) -> Option<Self> {
        Known::from_store(key, row)
    }
}

/// Reads the list while the workspace is open, and applies
/// `--creature-data`.
pub struct CreatureDataPlugin;

impl Plugin for CreatureDataPlugin {
    fn build(&self, app: &mut App) {
        // Before the tool's own row read, so the row a click opened is asked
        // for on the same frame.
        app.add_systems(
            Update,
            (
                read_the_list.before(creatures::fetch_rows),
                on_the_command_line,
            ),
        );
    }
}

/// Read `creature_template` whole, on a task, while the workspace is open and
/// the database has been written since the last read.
fn read_the_list(
    mut creatures: ResMut<Creatures>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    popout: Res<crate::popout::Popout>,
    state: Res<crate::playtest::Playtest>,
) {
    // A finished read is collected under any tool, so switching away and
    // back does not lose it.
    if let Some(highest) = creatures.listed.collect() {
        creatures.max_entry = creatures.max_entry.max(highest);
    }
    if !state.editing() || !popout.shows(*tool, Tool::CreatureData) {
        return;
    }
    let Some(session) = session else { return };
    creatures
        .listed
        .follow(&session.server_edits, session.server_edit_revision);
    if !creatures.listed.wants_read(session.database_writes) {
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        creatures
            .listed
            .unreadable(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = creatures::server_patch(&settings);
    creatures.listed.reading_with(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let rows = db.rows(&table::all_templates_query(patch))?;
        // The highest entry at any patch, not only among the winning rows:
        // a new row numbered from those alone could collide with a row at a
        // patch this server does not load.
        let highest = db
            .row(table::MAX_ENTRY_QUERY)?
            .and_then(|row| row.integer("entry"))
            .map(|entry| entry as u32);
        Ok(ListRead {
            rows: rows.iter().filter_map(creatures::read_known).collect(),
            highest,
        })
    }));
}

/// `--creature-data <entry>`: open the row, once. The list is read afterwards
/// and finds it there.
fn on_the_command_line(
    args: Res<crate::Args>,
    mut creatures: ResMut<Creatures>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    if let Some(entry) = args.creature_data {
        creatures.open_listed(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::super::listing::{fold_moves, Listing};
    use super::*;
    use vale_mangos::row::Life;

    fn guard(entry: u32) -> Known {
        Known {
            entry,
            read_entry: entry,
            patch: 0,
            name: "Stormwind City Guard".to_string(),
            subname: None,
            display_id: 3167,
            display_scale: 1.0,
            level: (55, 55),
            faction: 11,
            npc_flags: 0,
            rank: 0,
            claim: Life::Update,
        }
    }

    fn list(rows: Vec<Known>) -> Listing<Known> {
        Listing::holding(rows)
    }

    /// A number finds the row with that entry and text finds a name, in any
    /// case.
    #[test]
    fn a_search_matches_an_entry_or_a_name() {
        let mut rows = vec![guard(68), guard(1423)];
        rows[1].name = "Stormwind Guard".to_string();
        rows[1].subname = Some("Trade Supplies".to_string());
        let mut list = list(rows);
        let edits = Edits::default();
        list.query = "68".to_string();
        assert_eq!(list.matches(&edits, 0), &[0]);
        list.query = "CITY".to_string();
        assert_eq!(list.matches(&edits, 0), &[0]);
        list.query = "stormwind".to_string();
        assert_eq!(list.matches(&edits, 0), &[0, 1]);
        list.query = "trade".to_string();
        assert_eq!(list.matches(&edits, 0), &[1]);
        list.query = String::new();
        assert_eq!(list.matches(&edits, 0).len(), 2);
    }

    /// A row renamed by the project is found by its new name, and its old
    /// name no longer finds it.
    #[test]
    fn a_search_reads_the_projects_name() {
        let mut list = list(vec![guard(68)]);
        let mut edits = Edits::default();
        let key = table::template_key(68, 0);
        edits.set(table::TEMPLATE, &key, "name", Some("'Captain'".to_string()));
        list.query = "captain".to_string();
        assert_eq!(list.matches(&edits, 1), &[0]);
        list.query = "guard".to_string();
        assert!(list.matches(&edits, 1).is_empty());
        let shown = list.shown(0, &edits).expect("a row");
        assert_eq!(shown.name, "Captain");
    }

    /// The open row is the subject the form edits, named as the template
    /// window names it.
    #[test]
    fn the_open_row_is_the_forms_subject() {
        let mut list = list(vec![guard(68)]);
        let edits = Edits::default();
        assert!(list.subject(&edits).is_none());
        list.open = Some(68);
        let subject = list.subject(&edits).expect("the open row");
        assert_eq!((subject.entry, subject.patch), (68, 0));
        assert_eq!(subject.claim, Life::Update);
        assert!(list.holds(68) && !list.holds(69));
    }

    /// A row the project moves is listed at its new entry, and an undone move
    /// takes it back.
    #[test]
    fn a_moved_row_is_listed_at_its_new_entry() {
        let mut rows = vec![guard(68)];
        let mut edits = Edits::default();
        let from = table::template_key(68, 0);
        let to = table::template_key(2_000_000, 0);
        edits.rekey(table::TEMPLATE, &from, &to, Some(&from));
        fold_moves(&mut rows, &edits);
        assert_eq!((rows[0].entry, rows[0].read_entry), (2_000_000, 68));
        fold_moves(&mut rows, &Edits::default());
        assert_eq!(rows[0].entry, 68);
    }

    /// A template the project creates is listed after the database's rows,
    /// and once an apply puts it in the table that was read it is listed
    /// once.
    #[test]
    fn a_created_row_is_listed_once() {
        let mut list = list(vec![guard(68)]);
        let mut edits = Edits::default();
        let key = table::template_key(2_000_000, 0);
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in table::new_template("Made") {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(table::TEMPLATE, &key, Some(&row.to_line()));
        list.follow(&edits, 1);
        assert_eq!(list.count(), 2);
        assert_eq!(list.at(1).map(|known| known.name.as_str()), Some("Made"));
        list.all.push(guard(2_000_000));
        list.follow(&edits, 2);
        assert_eq!(list.count(), 2);
    }
}
