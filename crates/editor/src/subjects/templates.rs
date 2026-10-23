//! The Creatures and Game Objects workspaces: every template row of one spawn
//! subject that the server would load, as a list, with the open row's form in
//! the middle of the screen. One mechanism over [`SpawnSubject`].
//!
//! ## A second way to a spawn tool's rows
//!
//! A spawn tool reaches a template through a spawn: a click on the ground, or
//! the Place picker's search, which lists fifty matches. A workspace reaches
//! the same rows as a list, with no map involved, so a creature with no spawn
//! anywhere is found as easily as one in Goldshire.
//!
//! The rows are the tool's own. The list is [`SpawnTool::listed`]; the open
//! row is read into [`SpawnTool::template`] by the tool's own
//! [`crate::tools::spawns::spawn::fetch_rows`]; the form is the template
//! window's form; and a column edit, a new row, a copy, a discard and a
//! renumber are the tool's own calls with the same undo entries. A row edited
//! here is the row the template window shows, and the subject's block of the
//! bar's Server… applies both. This file holds what the list adds: the read,
//! and `--creature-data` and `--object-data`. The row type's search text is the
//! subject's `Listed` impl, and the list mechanics are [`super::listing`].

use super::listing::ListRead;
use crate::session::EditSession;
use crate::tools::spawns::spawn::{SpawnSubject, SpawnTool};
use bevy::prelude::*;
use std::marker::PhantomData;
use vale_mangos::schema::RowValue;

/// Reads one subject's list while its workspace is open, and applies the
/// flag that opens a row in it.
pub(crate) struct TemplatesPlugin<S>(PhantomData<fn() -> S>);

impl<S> Default for TemplatesPlugin<S> {
    fn default() -> TemplatesPlugin<S> {
        TemplatesPlugin(PhantomData)
    }
}

impl<S: SpawnSubject> Plugin for TemplatesPlugin<S> {
    fn build(&self, app: &mut App) {
        // Before the tool's own row read, so the row a click opened is asked
        // for on the same frame.
        app.add_systems(
            Update,
            (
                read_the_list::<S>.before(crate::tools::spawns::spawn::fetch_rows::<S>),
                on_the_command_line::<S>,
            ),
        );
    }
}

/// Read the template table whole, on a task, while the workspace is open and
/// the database has been written since the last read.
fn read_the_list<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    chosen: Res<crate::tools::Tool>,
    popout: Res<crate::popout::Popout>,
    state: Res<crate::playtest::Playtest>,
) {
    // A finished read is collected under any tool, so switching away and
    // back does not lose it.
    if let Some(highest) = tool.listed.collect() {
        tool.max_entry = tool.max_entry.max(highest);
    }
    if !state.editing() || !popout.shows(*chosen, S::WORKSPACE) {
        return;
    }
    let Some(session) = session else { return };
    tool.listed.follow(&session.rows.edits, session.rows.revision);
    if !tool.listed.wants_read(session.database_writes) {
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        tool.listed.unreadable(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = crate::server::settings::server_patch(&settings);
    tool.listed.reading_with(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let rows = db.rows(&S::all_templates_query(patch))?;
        // The highest entry at any patch, not only among the winning rows:
        // a new row numbered from those alone could collide with a row at a
        // patch this server does not load.
        let highest = db.row(S::MAX_ENTRY_QUERY)?.and_then(|row| row.integer("entry")).map(|entry| entry as u32);
        Ok(ListRead { rows: rows.iter().filter_map(S::read_known).collect(), highest })
    }));
}

/// `--creature-data <entry>` or `--object-data <entry>`: open the row, once.
/// The list is read afterwards and finds it there.
fn on_the_command_line<S: SpawnSubject>(
    args: Res<crate::args::Args>,
    mut tool: ResMut<SpawnTool<S>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    if let Some(entry) = S::workspace_row(&args) {
        tool.open_listed(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::super::listing::{fold_moves, Listed, Listing};
    use vale_mangos::row::{Edits, Life, RowEdit};

    mod creatures {
        use super::*;
        use crate::tools::spawns::creatures::Known;
        use vale_mangos::creature as table;

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
            let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
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

    mod game_objects {
        use super::*;
        use crate::tools::spawns::gameobjects::Known;
        use vale_mangos::gameobject as table;

        fn chest(entry: u32, name: &str) -> Known {
            Known {
                entry,
                read_entry: entry,
                claim: Life::Update,
                patch: 0,
                name: name.to_string(),
                kind: table::TYPE_CHEST,
                display_id: 10,
                size: 1.0,
                faction: 0,
                flags: 0,
                data: [0; 3],
            }
        }

        /// The type's word is searchable, so every chest is one search away.
        #[test]
        fn a_search_matches_the_type() {
            let mut door = chest(2, "Stormwind Gate");
            door.kind = 0;
            let mut list = Listing::holding(vec![chest(1, "Copper Vein"), door]);
            let edits = Edits::default();
            list.query = "chest".to_string();
            assert_eq!(list.matches(&edits, 0), &[0]);
            list.query = "gate".to_string();
            assert_eq!(list.matches(&edits, 0), &[1]);
        }

        /// An edited type is what the list shows and searches.
        #[test]
        fn the_projects_type_is_the_listed_type() {
            let mut edits = Edits::default();
            let key = table::template_key(1, 0);
            edits.set(table::TEMPLATE, &key, "type", Some("0".to_string()));
            let shown = chest(1, "Copper Vein").with_edits(&edits);
            assert_eq!(shown.kind, 0);
            assert!(!shown.haystack().contains("chest"));
        }
    }
}
