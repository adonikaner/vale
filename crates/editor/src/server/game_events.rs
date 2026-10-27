//! What a project changes about what the world holds while an event is on,
//! as SQL, applied when a person asks, and live after a restart.
//!
//! ## Five tables, written as keyed rows
//!
//! The game event tables are keyed rows in the project's store, on
//! [`super::addons`]' terms. The keys are `vale_mangos::game_event::event_key`,
//! `member_key`, `change_key` and `quest_key`. A spawn member's key is the
//! whole row, so creating one writes only its key. A created row is refused
//! with the loader's reason when the row alone shows the server would skip
//! it: event 0, an event of length 0. Whether the event and the spawn exist
//! is the inspector's to mark.
//!
//! ## Live after a restart
//!
//! `GameEventMgr::LoadFromDB` runs at startup only, and there is no
//! `.reload` for any of the five tables.
//!
//! ```text
//! sql\game-events.sql          what this project does to the five tables
//! sql\game-events-revert.sql   what puts those rows back
//! server\rows.txt              the store all five are written from, shared
//! ```

use super::stack::Subject;
use super::subject::RowSubject;
use vale_mangos::game_event;
use vale_mangos::row;

/// What this project does to the five tables, as SQL.
pub(crate) const SQL_VPATH: &str = "sql\\game-events.sql";

/// What puts it back.
pub(crate) const REVERT_VPATH: &str = "sql\\game-events-revert.sql";

/// The five tables, an event before the rows that name it.
pub(crate) const TABLES: &[row::Table] = game_event::TABLES;

/// What the world holds while an event is on.
pub(crate) struct GameEvents;

impl RowSubject for GameEvents {
    fn subject(&self) -> Subject {
        Subject::GameEvents
    }

    fn name(&self) -> &'static str {
        "game events"
    }

    fn label(&self) -> &'static str {
        "Game events"
    }

    fn word(&self) -> &'static str {
        "gameevents"
    }

    fn noun(&self) -> &'static str {
        "game event row"
    }

    fn tables(&self) -> &'static [row::Table] {
        TABLES
    }

    fn table_list(&self) -> &'static str {
        "game_event, game_event_creature, game_event_gameobject, game_event_creature_data, \
         game_event_quest"
    }

    fn sql_vpath(&self) -> &'static str {
        SQL_VPATH
    }

    fn revert_vpath(&self) -> &'static str {
        REVERT_VPATH
    }

    fn sql_about(&self) -> &'static str {
        "its game events and what each puts in the world"
    }

    fn sql_note(&self) -> String {
        "-- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
         -- or run it by hand and then restart the server: the game event tables are\n\
         -- read at startup and have no .reload."
            .to_string()
    }

    fn going_live(&self) -> &'static str {
        "Live after a restart. The server reads the game event tables at startup and none \
         has a .reload, so an applied event changes nothing until the server starts again."
    }

    /// None: the events are read at startup only.
    fn reloads(&self) -> Vec<&'static str> {
        Vec::new()
    }

    fn restart_note(&self) -> &'static str {
        "restart the server to load them; the game event tables have no reload"
    }

    /// The loader's own checks of a created event, member, change or quest.
    fn faults(&self, table: &'static row::Table, row: &vale_mangos::schema::Row) -> Vec<String> {
        use vale_mangos::schema::RowValue;
        match table.name {
            game_event::EVENT => game_event::Event::from_row(row).map(|event| event.check()).unwrap_or_default(),
            _ => row.integer("event").and_then(game_event::event_fault).into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::subject::{self, Plan};
    use vale_mangos::game_event::{Change, Event};
    use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};

    fn plan_from(edits: &Edits) -> Plan {
        subject::plan_of(&GameEvents, edits)
    }

    fn created(table: &str, key: &Key, changes: Vec<Assignment>, edits: &mut Edits) {
        let mut row = RowEdit { life: Life::Insert, ..RowEdit::default() };
        for change in changes {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(table, key, Some(&row.to_line()));
    }

    /// An event is written before the rows that name it; a spawn member is
    /// its key alone, negated for a spawn the event removes.
    #[test]
    fn an_event_is_written_before_its_members() {
        let mut edits = Edits::default();
        created(game_event::CREATURE, &game_event::member_key(79819, -200), Vec::new(), &mut edits);
        let event = Event::new(200, "Guard Change");
        created(game_event::EVENT, &event.key(), event.assignments(), &mut edits);
        let change = Change { display_id: 3167, ..Change::new(19272, 200) };
        created(game_event::CREATURE_DATA, &change.key(), change.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        let sql = plan.statements();
        assert!(sql[1].starts_with("INSERT INTO `game_event`"), "{}", sql[1]);
        assert!(sql[2].contains("`guid` = 79819 AND `event` = -200"), "{}", sql[2]);
        assert!(sql[3].starts_with("INSERT INTO `game_event_creature`") && sql[3].contains("-200"), "{}", sql[3]);
        assert!(sql[5].starts_with("INSERT INTO `game_event_creature_data`"), "{}", sql[5]);
        assert!(GameEvents.reloads().is_empty());
    }

    /// Event 0 and an event of length 0 are refused with the loader's reason.
    #[test]
    fn a_row_the_server_skips_is_refused() {
        let mut edits = Edits::default();
        created(game_event::GAMEOBJECT, &game_event::member_key(5000, 0), Vec::new(), &mut edits);
        let event = Event { length: 0, ..Event::new(201, "x") };
        created(game_event::EVENT, &event.key(), event.assignments(), &mut edits);
        let plan = plan_from(&edits);
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 2, "{:?}", plan.refused);
        assert!(plan.refused.iter().any(|why| why.contains("length is 0")), "{:?}", plan.refused);
        assert!(plan.refused.iter().any(|why| why.contains("event 0 is reserved")), "{:?}", plan.refused);
        for table in TABLES {
            assert!(GameEvents.owns(table.name));
        }
    }
}
