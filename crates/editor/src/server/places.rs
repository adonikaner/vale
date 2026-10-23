//! What a project changes about conditions, gossip, maps, area triggers and
//! graveyards on the server, as SQL, applied when a person asks.
//!
//! Five row subjects share this module, one per [`Group`]: each has its own
//! block on the Server panel, its own SQL and revert files, its own reloads and
//! its own place in `super::stack`'s order. They share the row rules, which are
//! the same for all thirteen tables.
//!
//! ## Thirteen tables, written as keyed rows
//!
//! ```text
//! conditions                    the server's reusable yes-or-no tests
//! npc_text                      what a gossip text says: up to eight lines
//! points_of_interest            a place a gossip option marks on the map
//! gossip_menu                   one text of a gossip menu
//! gossip_menu_option            one option of a gossip menu
//! map_template                  a new map's server row
//! areatrigger_template          a trigger's label, script, condition and
//!                               cooldown: the five columns the client's file
//!                               does not have, edited and never created
//! areatrigger_teleport          where a trigger sends a character
//! areatrigger_tavern            a trigger that is an inn
//! areatrigger_involvedrelation  a trigger that completes an exploration objective
//! areatrigger_bg_entrance       a trigger that opens a battleground's list
//! game_graveyard_zone           which safe place serves which zone
//! world_safe_locs_facing        which way a spirit faces at a safe place
//! ```
//!
//! They are keyed rows in the project's store, on [`super::services`]' terms: a
//! created row is a `DELETE` and an `INSERT` naming every column, an edited one
//! an `UPDATE`, a removed one a `DELETE`, and the statements that put each row
//! back are read from the database immediately before anything runs. The
//! columns and keys are `vale_mangos::trigger`, `vale_mangos::graveyard` and
//! `vale_mangos::map`. A created row is refused, with the loader's reason, when
//! the server would skip it for a reason the row alone shows.
//!
//! The triggers' volumes and the safe places are client tables, and reach the
//! server another way: `AreaTrigger.dbc` as `areatrigger_template` rows
//! written by [`super::rows`], `WorldSafeLocs.dbc` as a file copied into
//! `DataDir\5875\dbc` by [`super::dbcs`]. A template row's volume and its
//! server columns are written to the same dev row at build 5875 by the two
//! writers; see `vale_mangos::trigger`.
//!
//! ## What makes an applied row live
//!
//! Ten of the thirteen tables have a `.reload` whose loader clears its map
//! first, so an apply asks a running playtest for all ten, removals included.
//! `areatrigger_template`, `areatrigger_bg_entrance` and
//! `world_safe_locs_facing` are read at startup only. A new `map_template` row
//! is read on its reload, but the server opens a map's grids at startup, so a
//! new map needs a restart before anything can stand on it.
//!
//! ```text
//! sql\<group>.sql         what this project does to each group's tables:
//!                         conditions, gossip, maps, triggers, graveyards
//! sql\<group>-revert.sql  what puts those rows back
//! server\rows.txt         the store all of them are written from, shared
//! ```

use super::stack::Subject;
use super::subject::{Row, RowSubject};
use vale_mangos::conn::Db;
use vale_mangos::row;
use vale_mangos::{condition, gossip, graveyard, map, trigger};

/// One of the five subjects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Group {
    /// The `conditions` table.
    Conditions,
    /// Gossip menus, their options and the `npc_text` rows they show.
    Gossip,
    /// A new map's `map_template` row.
    Maps,
    /// What area triggers do: the template's server columns, teleports, inns,
    /// quest objectives and battleground entrances.
    Triggers,
    /// Which safe place serves which zone, and which way a spirit faces there.
    Graveyards,
}

impl Group {
    /// The five, in `super::stack`'s order. Conditions first, since every other
    /// group names them; maps before triggers, since a teleport's target map is
    /// checked against `map_template`.
    #[cfg(test)]
    pub(crate) const ALL: [Group; 5] = [Group::Conditions, Group::Gossip, Group::Maps, Group::Triggers, Group::Graveyards];

    /// The group a table belongs to, or `None` for a table of no group.
    #[cfg(test)]
    pub(crate) fn of(table: &str) -> Option<Group> {
        Group::ALL
            .into_iter()
            .find(|group| row::table_named(group.tables(), table).is_some())
    }
}

/// The thirteen tables, by group. [`RowSubject::tables`] names each group's.
pub(crate) const TABLES: &[row::Table] = &[
    condition::TABLES[0],
    gossip::TABLES[0],
    gossip::TABLES[1],
    gossip::TABLES[2],
    gossip::TABLES[3],
    map::TABLES[0],
    trigger::TABLES[0],
    trigger::TABLES[1],
    trigger::TABLES[2],
    trigger::TABLES[3],
    trigger::TABLES[4],
    graveyard::TABLES[0],
    graveyard::TABLES[1],
];

/// One of the thirteen tables, by name, or `None`.
pub(crate) fn table_named(name: &str) -> Option<&'static row::Table> {
    row::table_named(TABLES, name)
}

impl RowSubject for Group {
    fn subject(&self) -> Subject {
        match self {
            Group::Conditions => Subject::Conditions,
            Group::Gossip => Subject::Gossip,
            Group::Maps => Subject::Maps,
            Group::Triggers => Subject::Triggers,
            Group::Graveyards => Subject::Graveyards,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Group::Conditions => "conditions",
            Group::Gossip => "gossip menus",
            Group::Maps => "maps",
            Group::Triggers => "area triggers",
            Group::Graveyards => "graveyards",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Group::Conditions => "Conditions",
            Group::Gossip => "Gossip",
            Group::Maps => "Maps",
            Group::Triggers => "Area triggers",
            Group::Graveyards => "Graveyards",
        }
    }

    fn word(&self) -> &'static str {
        match self {
            Group::Conditions => "conditions",
            Group::Gossip => "gossip",
            Group::Maps => "maps",
            Group::Triggers => "triggers",
            Group::Graveyards => "graveyards",
        }
    }

    fn noun(&self) -> &'static str {
        match self {
            Group::Conditions => "condition",
            Group::Gossip => "gossip",
            Group::Maps => "map",
            Group::Triggers => "area trigger",
            Group::Graveyards => "graveyard",
        }
    }

    /// Its tables, in the order a plan writes them.
    fn tables(&self) -> &'static [row::Table] {
        match self {
            Group::Conditions => condition::TABLES,
            Group::Gossip => gossip::TABLES,
            Group::Maps => map::TABLES,
            Group::Triggers => trigger::TABLES,
            Group::Graveyards => graveyard::TABLES,
        }
    }

    fn table_list(&self) -> &'static str {
        match self {
            Group::Conditions => "conditions",
            Group::Gossip => "npc_text, points_of_interest, gossip_menu, gossip_menu_option",
            Group::Maps => "map_template",
            Group::Triggers => {
                "areatrigger_template (label, script, condition, cooldown), areatrigger_teleport, \
                 areatrigger_tavern, areatrigger_involvedrelation, areatrigger_bg_entrance"
            }
            Group::Graveyards => "game_graveyard_zone, world_safe_locs_facing",
        }
    }

    fn sql_vpath(&self) -> &'static str {
        match self {
            Group::Conditions => "sql\\conditions.sql",
            Group::Gossip => "sql\\gossip.sql",
            Group::Maps => "sql\\maps.sql",
            Group::Triggers => "sql\\triggers.sql",
            Group::Graveyards => "sql\\graveyards.sql",
        }
    }

    fn revert_vpath(&self) -> &'static str {
        match self {
            Group::Conditions => "sql\\conditions-revert.sql",
            Group::Gossip => "sql\\gossip-revert.sql",
            Group::Maps => "sql\\maps-revert.sql",
            Group::Triggers => "sql\\triggers-revert.sql",
            Group::Graveyards => "sql\\graveyards-revert.sql",
        }
    }

    fn sql_about(&self) -> &'static str {
        self.name()
    }

    fn sql_note(&self) -> String {
        let reloads: String = self.reloads().iter().map(|table| format!("--   .reload {table}\n")).collect();
        format!(
            "-- Nothing applies this by itself. Apply it from the editor's Server panel,\n\
             -- or run it by hand and then reload:\n\
             {reloads}\
             -- A table with no reload is read at server start."
        )
    }

    fn going_live(&self) -> &'static str {
        match self {
            Group::Conditions => {
                "Live on `.reload conditions`, which an apply sends when it is made with the \
                 panels open over a playtest. Applied at any other time, it is live after a \
                 restart."
            }
            Group::Gossip => {
                "Live on `.reload npc_text`, `.reload points_of_interest`, `.reload gossip_menu` \
                 and `.reload gossip_menu_option`, which an apply sends when it is made with the \
                 panels open over a playtest \u{2014} removals included. A text a menu shows is \
                 broadcast_text, applied by Behaviour above."
            }
            Group::Maps => {
                "Read on `.reload map_template`, which an apply sends when it is made with the \
                 panels open over a playtest, but the server opens a map's grids at startup, \
                 so a new map needs a restart before a character can enter it. The map's \
                 Map.dbc row is a client table, applied by Client tables above, and its \
                 terrain reaches the server through Server files on the map window."
            }
            Group::Triggers => {
                "Live on `.reload areatrigger_teleport`, `.reload areatrigger_tavern` and \
                 `.reload areatrigger_involvedrelation`, which an apply sends when it is made \
                 with the panels open over a playtest \u{2014} removals included. Applied at any \
                 other time, it is live after a restart. The template's columns and \
                 areatrigger_bg_entrance have no reload and need a restart. A trigger's \
                 volume is a client table, applied by Client tables above."
            }
            Group::Graveyards => {
                "Live on `.reload game_graveyard_zone`, which an apply sends when it is made \
                 with the panels open over a playtest \u{2014} removals included. \
                 world_safe_locs_facing has no reload and needs a restart. A graveyard's \
                 position is a WorldSafeLocs.dbc row, applied by Client tables above."
            }
        }
    }

    /// The reload commands that make an apply live, in the order they are sent.
    fn reloads(&self) -> Vec<&'static str> {
        match self {
            Group::Conditions => vec![condition::TABLE],
            Group::Gossip => vec![gossip::NPC_TEXT, gossip::POI, gossip::MENU, gossip::OPTION],
            Group::Maps => vec![map::TEMPLATE],
            Group::Triggers => trigger::RELOADS.to_vec(),
            Group::Graveyards => vec![graveyard::ZONE],
        }
    }

    /// Why the server would skip a created row, from the row alone. Text
    /// columns are compared unquoted.
    fn faults(&self, table: &'static row::Table, row: &vale_mangos::schema::Row) -> Vec<String> {
        let mut out = trigger::check_created(table.name, row);
        out.extend(graveyard::check_created(table.name, row));
        out.extend(map::check_created(table.name, row));
        out.extend(condition::check_created(table.name, row));
        out.extend(gossip::check_created(table.name, row));
        out
    }

    /// A template row's edit is written on its dev row; see
    /// `vale_mangos::trigger::row_statements`.
    fn statements(&self, row: &Row) -> Vec<String> {
        match row.table.name == trigger::TEMPLATE {
            true => trigger::row_statements(row.table.name, &row.key, row.life, &row.changes),
            false => row.statements(),
        }
    }

    /// A template row with no dev row yet is made by this apply from the row
    /// the server uses, so its undo puts the server columns back to that row's
    /// values. The dev row stays, holding the same values, because the client
    /// tables' mirror may also be writing it.
    fn held(&self, db: &mut Db, row: &Row) -> Result<Option<vale_mangos::schema::Row>, String> {
        let held = db.row(&row::row_query(row.table.name, &row.at))?;
        if row.table.name != trigger::TEMPLATE || held.is_some() {
            return Ok(held);
        }
        match row.key.first() {
            Some(id) => db.row(&trigger::winning_template_query(id as u32)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::subject::{self, Plan};
    use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};

    fn plan_from(edits: &Edits, group: Group) -> Plan {
        subject::plan_of(group.subject().row_subject(), edits)
    }

    fn owns(table: &str) -> bool {
        table_named(table).is_some()
    }

    fn created(table: &str, key: &Key, changes: Vec<Assignment>, edits: &mut Edits) {
        let mut row = RowEdit {
            life: Life::Insert,
            ..RowEdit::default()
        };
        for change in changes {
            row.columns.insert(change.column.to_string(), change.value);
        }
        edits.set_row_line(table, key, Some(&row.to_line()));
    }

    /// A new map, a teleport into it and a graveyard link each go to their
    /// own group's plan.
    #[test]
    fn a_map_its_entrance_and_its_graveyard_are_three_subjects() {
        let mut edits = Edits::default();
        let link = graveyard::Link::new(900, 5000);
        created(graveyard::ZONE, &link.key(), link.assignments(), &mut edits);
        let teleport = trigger::Teleport::new(4500, 534, [10.0, 20.0, 30.0]);
        created(trigger::TELEPORT, &teleport.key(), teleport.assignments(), &mut edits);
        let template = map::Template::new(534, 1, "The Islands");
        created(map::TEMPLATE, &template.key(), template.assignments(), &mut edits);
        for (group, table) in [
            (Group::Maps, "map_template"),
            (Group::Triggers, "areatrigger_teleport"),
            (Group::Graveyards, "game_graveyard_zone"),
        ] {
            let plan = plan_from(&edits, group);
            assert!(plan.refused.is_empty(), "{:?}", plan.refused);
            let sql = plan.statements();
            assert_eq!(sql.len(), 2, "{group:?}");
            assert!(sql[1].starts_with(&format!("INSERT INTO `{table}`")), "{}", sql[1]);
        }
    }

    /// A created teleport with no target, and a link for a faction the loader
    /// does not accept, are refused with the loader's reason.
    #[test]
    fn a_row_the_server_would_skip_is_refused() {
        let mut edits = Edits::default();
        let teleport = trigger::Teleport::new(4500, 534, [0.0; 3]);
        created(trigger::TELEPORT, &teleport.key(), teleport.assignments(), &mut edits);
        let link = graveyard::Link { faction: 5, ..graveyard::Link::new(900, 5000) };
        created(graveyard::ZONE, &link.key(), link.assignments(), &mut edits);
        for group in [Group::Triggers, Group::Graveyards] {
            let plan = plan_from(&edits, group);
            assert!(plan.rows.is_empty());
            assert_eq!(plan.refused.len(), 1, "{:?}", plan.refused);
        }
    }

    #[test]
    fn the_subjects_own_the_thirteen_tables_and_no_other() {
        for table in TABLES {
            let name = table.name;
            assert!(owns(name));
            assert!(!table.columns.is_empty(), "{name}");
            let group = Group::of(name).unwrap();
            assert!(row::table_named(group.tables(), name).is_some(), "{name} is not in {group:?}'s slice");
        }
        let count: usize = Group::ALL.iter().map(|group| group.tables().len()).sum();
        assert_eq!(count, TABLES.len());
        assert!(!owns(vale_mangos::creature::TEMPLATE));
    }

    /// A template edit sets the server columns on the dev row, which it makes
    /// first; a created or removed template row is refused.
    #[test]
    fn a_template_row_is_edited_on_its_dev_row_and_never_created() {
        let mut edits = Edits::default();
        let key = trigger::key(78);
        edits.set(trigger::TEMPLATE, &key, "cooldown", Some("30".to_string()));
        let plan = plan_from(&edits, Group::Triggers);
        assert_eq!(plan.rows.len(), 1);
        let statements = Group::Triggers.statements(&plan.rows[0]);
        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("INSERT IGNORE INTO `areatrigger_template`"));
        assert!(statements[1].contains("`cooldown` = 30"), "{}", statements[1]);
        assert!(statements[1].contains("`build` = 5875"), "{}", statements[1]);

        let mut edits = Edits::default();
        let template = trigger::Template { id: 9000, ..trigger::Template::default() };
        created(trigger::TEMPLATE, &trigger::key(9000), template.assignments(), &mut edits);
        let plan = plan_from(&edits, Group::Triggers);
        assert!(plan.rows.is_empty());
        assert_eq!(plan.refused.len(), 1);
    }
}
