//! What a project changes about a creature's behaviour, as SQL, applied when
//! a person asks, and live on a reload of each table written.
//!
//! ## How events, spell lists, texts and scripts are written
//!
//! `creature_ai_events`, `creature_spells` and `broadcast_text` are keyed rows
//! in the project's store, on [`super::loot`]'s terms: a created row is a `DELETE` and an
//! `INSERT`, an edited one an `UPDATE`, a removed one a `DELETE`. The eleven
//! `*_scripts` tables have no key, so a script is the unit and lives in the
//! session's second store (`ServerRows::scripts`), written whole on
//! [`super::creatures`]' path terms: `DELETE` under the id, then one `INSERT`
//! per row. `vale_mangos::scripts`' module comment is where that was
//! decided.
//!
//! ## Live on a reload
//!
//! `.reload creature_ai_events` re-reads `creature_ai_scripts` and then the
//! events; `.reload creature_spells` re-reads the lists; five of the script
//! tables reload under their own names and the other six are read at start
//! (`vale_mangos::scripts::Table::reload`). A creature already in the
//! world keeps the events and the list it spawned with until it respawns.
//! `broadcast_text` has no reload and is read at start, so an applied text is
//! said after the server restarts.
//!
//! ```text
//! sql\behaviour.sql          what this project does to the behaviour tables
//! sql\behaviour-revert.sql   what puts those rows back
//! server\rows.txt            the store the events, lists and texts are written from
//! server\scripts.txt         the store the scripts are written from
//! ```

use super::stack::Subject;
use super::subject::{Row, RowSubject, Stores, Whole};
use vale_mangos::row;
use vale_mangos::{broadcast, creaturespells, eventai, scripts};

/// What this project does to the server's behaviour tables, as SQL.
pub(crate) const SQL_VPATH: &str = "sql\\behaviour.sql";

/// What puts it back.
pub(crate) const REVERT_VPATH: &str = "sql\\behaviour-revert.sql";

/// Where the project keeps the scripts it changes, a store of their own for
/// the reason `super::creatures::PATHS_VPATH` is.
pub(crate) const SCRIPTS_VPATH: &str = "server\\scripts.txt";

/// The tables the row store may hold for this subject.
pub(crate) const ROW_TABLES: &[row::Table] = &[eventai::EVENTS, creaturespells::LISTS, broadcast::TEXTS];

/// A creature's behaviour: its events, spell lists and texts as keyed rows,
/// and its scripts written whole.
pub(crate) struct Behaviour;

impl RowSubject for Behaviour {
    fn subject(&self) -> Subject {
        Subject::Behaviour
    }

    fn name(&self) -> &'static str {
        "behaviour"
    }

    fn label(&self) -> &'static str {
        "Behaviour"
    }

    fn word(&self) -> &'static str {
        "behaviour"
    }

    fn noun(&self) -> &'static str {
        "behaviour"
    }

    fn tables(&self) -> &'static [row::Table] {
        ROW_TABLES
    }

    fn table_list(&self) -> &'static str {
        "creature_ai_events, creature_spells, broadcast_text, and the eleven *_scripts tables"
    }

    /// An event, a spell list, a text or a script.
    fn owns(&self, table: &str) -> bool {
        row::table_named(ROW_TABLES, table).is_some() || scripts::table_named(table).is_some()
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "creature behaviour: events,\n-- scripts and spell lists"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then reload what it names:\n\
         --   .reload creature_ai_events    (re-reads creature_ai_scripts first)\n\
         --   .reload creature_spells\n\
         --   .reload gossip_scripts, generic_scripts, event_scripts,\n\
         --           quest_start_scripts, creature_spells_scripts, as written\n\
         -- The other script tables and broadcast_text are read at start, so restart\n\
         -- the server for them."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "Live on `.reload creature_ai_events`, which re-reads creature_ai_scripts \
         first, and `.reload creature_spells`, which an apply sends when it is made \
         with the panels open over a playtest; applied at any other time, it is \
         live after a restart. A creature already in the world keeps the events and the list it \
         spawned with until it respawns. Five of the script tables reload under \
         their own names; the other six and broadcast_text are read at start, so a \
         change to one of those needs a restart."
    }

    /// Every table with a reload, since the put-back before an Apply may have
    /// written any of them: the events, the lists, and the script tables
    /// `vale_mangos::scripts::Table::reload` names.
    fn reloads(&self) -> Vec<&'static str> {
        let mut out = vec!["creature_ai_events", "creature_spells"];
        for table in scripts::ALL {
            if let Some(reload) = table.reload {
                if !out.contains(&reload) {
                    out.push(reload);
                }
            }
        }
        out
    }

    /// The scripts, each written whole under its id, after the keyed rows.
    fn wholes(&self, stores: &Stores<'_>, _rows: &[Row], _refused: &mut Vec<String>) -> Vec<Whole> {
        stores.scripts.iter().map(Whole::Script).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::subject::{self, Plan};
    use vale_mangos::row::{Edits, Life, RowEdit};
    use vale_mangos::scripts::Script;

    fn plan_from(edits: &Edits, scripts: &scripts::Scripts) -> Plan {
        let paths = vale_mangos::path::Paths::default();
        subject::plan_from(&Behaviour, &Stores { edits, paths: &paths, scripts })
    }

    fn owns(table: &str) -> bool {
        Behaviour.owns(table)
    }

    /// An event edit is one `UPDATE`, a created event names every column, and
    /// a script is written whole after the rows.
    #[test]
    fn the_plan_holds_rows_and_scripts() {
        let mut edits = Edits::default();
        edits.set(eventai::TABLE, &eventai::key(6801), "event_chance", Some("50".into()));
        let event = eventai::Event::new(6806, 68);
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in event.assignments() {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(eventai::TABLE, &event.key(), Some(&row.to_line()));
        // Another subject's row is not this plan's.
        edits.set(vale_mangos::item::TEMPLATE, &vale_mangos::item::template_key(19019, 10), "Quality", Some("4".into()));
        let mut store = scripts::Scripts::default();
        let mut talk = scripts::ScriptRow::new(0);
        talk.dataint[0] = 1234;
        store.set(&Script { table: scripts::CREATURE_AI, id: 6806, rows: vec![talk] });

        let plan = plan_from(&edits, &store);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.rows.len(), 2);
        assert_eq!(plan.wholes.len(), 1);
        let sql = plan.statements();
        assert_eq!(sql[0], "UPDATE `creature_ai_events` SET `event_chance` = 50 WHERE `id` = 6801;");
        assert!(sql[1].starts_with("DELETE FROM `creature_ai_events` WHERE `id` = 6806;"));
        assert!(sql[2].starts_with("INSERT INTO `creature_ai_events`"));
        assert_eq!(sql[3], "DELETE FROM `creature_ai_scripts` WHERE `id` = 6806;");
        assert!(sql[4].starts_with("INSERT INTO `creature_ai_scripts`"));
        assert_eq!(plan.line(), "2 row(s): 1 new, 1 edited, 1 script(s) replaced");
        let empty = plan_from(&Edits::default(), &scripts::Scripts::default());
        assert!(empty.is_empty());
    }

    /// An event, a spell list and a text made for a creature this project
    /// creates, as the behaviour tool makes them, are written whole and
    /// refused nowhere: not here, and not by the creature writer, which walks
    /// the same store. An edit to a text the database holds is an `UPDATE`.
    #[test]
    fn a_new_creatures_event_and_list_are_written() {
        let entry = vale_mangos::creature::RESERVED_ENTRY_BASE;
        let mut edits = Edits::default();
        let event = eventai::Event::new(eventai::next_id(entry, &[]), entry);
        let list = creaturespells::List::new(entry, "a new list");
        let text = broadcast::Text::new(90_001, "Stand fast!");
        for (table, key, assignments) in [
            (eventai::TABLE, event.key(), event.assignments()),
            (creaturespells::TABLE, list.key(), list.assignments()),
            (broadcast::TABLE, text.key(), text.assignments()),
        ] {
            let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
            for change in assignments {
                row.columns.insert(change.column.to_string(), change.value);
            }
            edits.set_row_line(table, &key, Some(&row.to_line()));
        }
        edits.set(broadcast::TABLE, &broadcast::key(1234), "male_text", Some("'Hold the line'".into()));
        let plan = plan_from(&edits, &scripts::Scripts::default());
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(plan.rows.len(), 4);
        let sql = plan.statements();
        assert!(sql.iter().any(|s| s.starts_with("INSERT INTO `broadcast_text`") && s.contains("'Stand fast!'")), "{sql:?}");
        assert!(sql.contains(&"UPDATE `broadcast_text` SET `male_text` = 'Hold the line' WHERE `entry` = 1234;".to_string()), "{sql:?}");
        assert!(owns(broadcast::TABLE));
        let creatures = subject::plan_of(&super::super::creatures::Creatures, &edits);
        assert!(creatures.refused.is_empty(), "{:?}", creatures.refused);
        assert!(creatures.rows.is_empty());
    }

    /// A created row missing a column is refused by name, and a key column
    /// is not written.
    #[test]
    fn a_partial_creation_and_a_key_column_are_refused() {
        let mut edits = Edits::default();
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        row.columns.insert("event_type".into(), "4".into());
        edits.set_row_line(eventai::TABLE, &eventai::key(6807), Some(&row.to_line()));
        edits.set(creaturespells::TABLE, &creaturespells::key(680), "entry", Some("681".into()));
        let plan = plan_from(&edits, &scripts::Scripts::default());
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 2);
        assert!(plan.refused.iter().any(|why| why.contains("created without")), "{:?}", plan.refused);
        assert!(plan.refused.iter().any(|why| why.contains("part of the key")), "{:?}", plan.refused);
    }

    /// The reloads an apply asks for are the ones the server has.
    #[test]
    fn the_reloads_are_the_servers_own() {
        let reloads = Behaviour.reloads();
        assert_eq!(&reloads[..2], &["creature_ai_events", "creature_spells"]);
        assert!(reloads.contains(&"gossip_scripts"));
        assert!(!reloads.contains(&"quest_end_scripts"));
        assert!(owns(scripts::GENERIC));
        assert!(owns(eventai::TABLE));
        assert!(!owns(vale_mangos::loot::CREATURE));
    }
}
