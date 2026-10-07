//! Area triggers on the server: `areatrigger_template`, the server's copy of
//! `AreaTrigger.dbc`, and the three tables that say what a trigger does.
//!
//! ## What the server reads, and from where
//!
//! ```text
//! areatrigger_template          the volume, one row per (id, build);
//!                               ObjectMgr::LoadAreaTriggers (ObjectMgr.cpp:7090)
//!                               takes, per id, the row with the highest build at
//!                               or below 5875. Read at startup; no reload
//! areatrigger_teleport          where the trigger sends a character: a map and a
//!                               position, a level and a message. One row per
//!                               (id, patch); LoadAreaTriggerTeleports takes the
//!                               highest patch at or below WowPatch
//! areatrigger_tavern            the trigger is an inn: a character inside rests
//! areatrigger_involvedrelation  the trigger completes a quest's exploration
//!                               objective
//! areatrigger_bg_entrance       the trigger opens a battleground's list, for
//!                               one side, and says where a character leaves
//!                               the battleground to. Read at startup
//!                               (ObjectMgr.cpp:7275); no reload
//! ```
//!
//! Teleports, inns and quest triggers each have a `.reload` (`Chat.cpp:811`),
//! and each loader clears its map first, so a removed row is gone after the
//! reload. Each of the loaders skips a row whose trigger is not in
//! `areatrigger_template`,
//! which is why a new trigger reaches the server through the template row
//! first: an edited `AreaTrigger.dbc` row becomes a template row on every save,
//! on `taxi_nodes`' terms (see `crate::taxi`).
//!
//! ## The template row
//!
//! ```text
//! id, build          the key; an edit is written at build 5875
//! name               the server's own label; not in the client's file
//! map_id             AreaTrigger field 1
//! x, y, z            fields 2, 3, 4
//! radius             field 5
//! box_x, box_y, box_z  fields 6, 7, 8: the box's whole length, width, height
//! box_orientation    field 9, radians
//! cooldown, condition_id, script_id, script_name   the server's own
//! ```
//!
//! Measured on the reference database: trigger 101's build-4449 row holds the
//! same box as the client's file, field for field, so the four box columns are
//! the file's fields in order and the yaw is in radians on both sides.
//!
//! The row has two writers, and they write different columns of the same dev
//! row at build 5875. The volume follows `AreaTrigger.dbc` on every save
//! ([`statements`], through the client tables' mirror). The five columns the
//! client's file does not have ([`TEMPLATE_COLUMNS_OF_THE_SERVER`]) are rows of
//! the project's store, written by the places subject ([`row_statements`]).
//! Both create the dev row the same way, with [`copy_forward`], an `INSERT
//! IGNORE` of the row the server uses, so whichever runs first makes it and
//! the other updates its own columns in place.
//!
//! When the server finds a trigger it runs, in order: the template's script
//! (`script_name`, a C++ script registered under that name, else `script_id`,
//! the rows of `areatrigger_scripts` under that id), gated by `condition_id`
//! and `cooldown` (`Map::StartAreaTriggerScript`, `Map.cpp:2688`); the quest
//! objective; the inn; the battleground entrance; the battleground's and the
//! zone's own C++ handlers; the teleport (`WorldSession::HandleAreaTriggerOpcode`,
//! `MiscHandler.cpp:620`).
//!
//! ## What the loaders skip
//!
//! `LoadAreaTriggerTeleports` skips a row whose target map has no
//! `map_template` row, and one whose target x, y and z are all 0.
//! `LoadQuestAreaTriggers` skips a row naming no quest. [`Teleport::check`] and
//! [`check_created`] make the checks a row alone can show; a target map that
//! does not exist is the form's to refuse.

use crate::row::{self, Assignment, Key, Life};
use crate::schema::{Column, Group, Kind};
use std::collections::HashMap;
use vale_edit::dbc::places::Trigger;

/// The four tables.
pub const TEMPLATE: &str = "areatrigger_template";
pub const TELEPORT: &str = "areatrigger_teleport";
pub const TAVERN: &str = "areatrigger_tavern";
pub const QUEST: &str = "areatrigger_involvedrelation";
pub const BG_ENTRANCE: &str = "areatrigger_bg_entrance";

/// The tables a project edits as rows of its own store, in the order a plan
/// writes them. The template is here for its five server columns only.
pub const TABLES: [&str; 5] = [TEMPLATE, TELEPORT, TAVERN, QUEST, BG_ENTRANCE];

/// The three of them with a `.reload`, in the order the server is asked.
pub const RELOADS: [&str; 3] = [TELEPORT, TAVERN, QUEST];

/// The static name for one of [`TABLES`] read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// The build a template edit is written at. `LoadAreaTriggers` takes the
/// highest build at or below `SUPPORTED_CLIENT_BUILD`, 5875, so this row wins.
pub const BUILD: u32 = 5875;

/// The largest id `areatrigger_template.id`, a `smallint unsigned`, holds.
pub const MAX_ID: u32 = vale_edit::dbc::places::MAX_TRIGGER_ID;

const ID: &str = "id";
const BUILD_COLUMN: &str = "build";

/// Every column of `areatrigger_template`, in table order.
pub const TEMPLATE_COLUMNS: [&str; 16] = [
    ID,
    BUILD_COLUMN,
    "name",
    "map_id",
    "x",
    "y",
    "z",
    "radius",
    "box_x",
    "box_y",
    "box_z",
    "box_orientation",
    "cooldown",
    "condition_id",
    "script_id",
    "script_name",
];

/// Whether the server's table can hold this trigger.
pub fn fits(id: u32) -> bool {
    (1..=MAX_ID).contains(&id)
}

/// The template columns a trigger feeds, in table order, with the value each
/// takes.
fn values(trigger: &Trigger) -> [(&'static str, String); 9] {
    let f = crate::sql::float;
    [
        ("map_id", trigger.map.to_string()),
        ("x", f(trigger.at[0])),
        ("y", f(trigger.at[1])),
        ("z", f(trigger.at[2])),
        ("radius", f(trigger.radius)),
        ("box_x", f(trigger.extent[0])),
        ("box_y", f(trigger.extent[1])),
        ("box_z", f(trigger.extent[2])),
        ("box_orientation", f(trigger.yaw)),
    ]
}

/// The dev row's key.
pub fn key(id: u32) -> Key {
    Key::two((ID, u64::from(id)), (BUILD_COLUMN, u64::from(BUILD)))
}

/// What changed between the trigger the archives ship and the one this project
/// edited: one assignment per column whose value differs. A trigger the shipped
/// file does not have is new, and every column is a change.
pub fn changes(shipped: Option<&Trigger>, edited: &Trigger) -> Vec<Assignment> {
    let before = shipped.map(values);
    values(edited)
        .into_iter()
        .enumerate()
        .filter(|(n, (_, after))| before.as_ref().map(|b| &b[*n].1) != Some(after))
        .map(|(_, (column, value))| Assignment { column, value })
        .collect()
}

/// Copy the row the server is using for this trigger to a dev row, if it has
/// no dev row already. The sub-select is `LoadAreaTriggers`' own.
pub fn copy_forward(id: u32) -> String {
    let selected: Vec<String> = TEMPLATE_COLUMNS
        .iter()
        .map(|&column| match column {
            BUILD_COLUMN => BUILD.to_string(),
            name => crate::sql::name(name),
        })
        .collect();
    format!(
        "INSERT IGNORE INTO {table} SELECT {selected} FROM {table} t1 \
         WHERE t1.{id_col} = {id} \
         AND t1.{build_col} = (SELECT MAX(t2.{build_col}) FROM {table} t2 \
         WHERE t2.{id_col} = {id} AND t2.{build_col} <= {build});",
        table = crate::sql::name(TEMPLATE),
        selected = selected.join(", "),
        id_col = crate::sql::name(ID),
        build_col = crate::sql::name(BUILD_COLUMN),
        build = BUILD,
    )
}

/// A dev row from the trigger, for an id the table has never had. The
/// server's own columns take their defaults: no name, no cooldown, no
/// condition and no script. `INSERT IGNORE`, so it does nothing when
/// [`copy_forward`] made the row.
pub fn insert_new(trigger: &Trigger) -> String {
    let mut names = vec![crate::sql::name(ID), crate::sql::name(BUILD_COLUMN)];
    let mut literals = vec![trigger.id.to_string(), BUILD.to_string()];
    for (column, value) in values(trigger) {
        names.push(crate::sql::name(column));
        literals.push(value);
    }
    format!(
        "INSERT IGNORE INTO {} ({}) VALUES ({});",
        crate::sql::name(TEMPLATE),
        names.join(", "),
        literals.join(", ")
    )
}

/// Everything a save emits for one edited trigger, in order. Empty when
/// nothing changed or the id cannot be stored.
pub fn statements(shipped: Option<&Trigger>, edited: &Trigger) -> Vec<String> {
    if !fits(edited.id) {
        return Vec::new();
    }
    let changes = changes(shipped, edited);
    let Some(update) = row::update(TEMPLATE, &key(edited.id), &changes) else {
        return Vec::new();
    };
    vec![copy_forward(edited.id), insert_new(edited), update]
}

/// The dev row of one trigger, for [`undo`] to read before an apply.
pub fn dev_row_query(id: u32) -> String {
    format!("SELECT * FROM {} WHERE {};", crate::sql::name(TEMPLATE), key(id).where_clause())
}

/// The statement that puts one trigger's dev row back to what it holds now:
/// `DELETE` when there is no dev row yet, and an `UPDATE` of the changed
/// columns when there is one.
pub fn undo(id: u32, changes: &[Assignment], now: Option<&HashMap<String, Option<String>>>) -> Option<String> {
    if changes.is_empty() || !fits(id) {
        return None;
    }
    match now {
        None => Some(row::delete(TEMPLATE, &key(id))),
        Some(now) => row::undo(TEMPLATE, &key(id), changes, Some(now)),
    }
}

// ---------------------------------------------------------------------------
// The three tables that say what a trigger does
// ---------------------------------------------------------------------------

const REF_MAP: Kind = Kind::Ref("Map");
const REF_QUEST: Kind = Kind::Ref(crate::quest::TEMPLATE);

/// `areatrigger_teleport`, in table order.
pub const TELEPORT_COLUMNS: [Column; 11] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the AreaTrigger.dbc id" },
    Column { name: "patch", kind: Kind::Key, group: Group::Identity, about: "the content patch the row belongs to; the server reads the highest at or below its WowPatch" },
    Column { name: "name", kind: Kind::Text, group: Group::Identity, about: "a label for editors; not shown by the server" },
    Column { name: "message", kind: Kind::Text, group: Group::Requirements, about: "the message a character below required_level receives" },
    Column { name: "required_level", kind: Kind::Unsigned, group: Group::Requirements, about: "the level a character needs to be teleported" },
    Column { name: "required_condition", kind: Kind::Unsigned, group: Group::Requirements, about: "a row of `conditions` the character must meet, or 0" },
    Column { name: "target_map", kind: REF_MAP, group: Group::Place, about: "Map.dbc id of the map the character is teleported to" },
    Column { name: "target_position_x", kind: Kind::Float, group: Group::Place, about: "target x: north, in yards" },
    Column { name: "target_position_y", kind: Kind::Float, group: Group::Place, about: "target y: west, in yards" },
    Column { name: "target_position_z", kind: Kind::Float, group: Group::Place, about: "target z: height, in yards" },
    Column { name: "target_orientation", kind: Kind::Float, group: Group::Place, about: "the facing on arrival, in radians" },
];

/// `areatrigger_template`'s key and the five columns the client's file does
/// not have, which the places subject writes.
pub const TEMPLATE_COLUMNS_OF_THE_SERVER: [Column; 7] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the AreaTrigger.dbc id" },
    Column { name: "build", kind: Kind::Key, group: Group::Identity, about: "the client build the row belongs to; edits are written at 5875" },
    Column { name: "name", kind: Kind::Text, group: Group::Identity, about: "a label for editors; not shown by the server" },
    Column { name: "cooldown", kind: Kind::Unsigned, group: Group::Requirements, about: "seconds before the script can run again on this map, or 0" },
    Column { name: "condition_id", kind: Kind::Unsigned, group: Group::Requirements, about: "a row of `conditions` the character must meet for the script to run, or 0" },
    Column { name: "script_id", kind: Kind::Unsigned, group: Group::Behaviour, about: "the id of the `areatrigger_scripts` rows to run, or 0" },
    Column { name: "script_name", kind: Kind::Text, group: Group::Behaviour, about: "the name of a C++ script the server registers, or empty; it runs instead of script_id" },
];

/// `areatrigger_bg_entrance`, in table order.
pub const BG_ENTRANCE_COLUMNS: [Column; 9] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the AreaTrigger.dbc id" },
    Column { name: "name", kind: Kind::Text, group: Group::Identity, about: "a label for editors; not shown by the server" },
    Column { name: "team", kind: Kind::Choice(&TEAMS), group: Group::Requirements, about: "the side the entrance serves: 469 Alliance, 67 Horde" },
    Column { name: "bg_template", kind: Kind::Unsigned, group: Group::Identity, about: "the battleground_template id whose queue list opens" },
    Column { name: "exit_map", kind: REF_MAP, group: Group::Place, about: "the map a character leaving the battleground returns to" },
    Column { name: "exit_position_x", kind: Kind::Float, group: Group::Place, about: "exit x: north, in yards" },
    Column { name: "exit_position_y", kind: Kind::Float, group: Group::Place, about: "exit y: west, in yards" },
    Column { name: "exit_position_z", kind: Kind::Float, group: Group::Place, about: "exit z: height, in yards" },
    Column { name: "exit_orientation", kind: Kind::Float, group: Group::Place, about: "the facing on return, in radians" },
];

/// The two sides an entrance serves, as faction template ids.
pub const TEAMS: [crate::schema::Value; 2] = [
    crate::schema::Value { value: 469, name: "Alliance" },
    crate::schema::Value { value: 67, name: "Horde" },
];

/// `areatrigger_tavern`, in table order.
pub const TAVERN_COLUMNS: [Column; 3] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the AreaTrigger.dbc id" },
    Column { name: "name", kind: Kind::Text, group: Group::Identity, about: "a label for editors; not shown by the server" },
    Column { name: "patch_min", kind: Kind::Unsigned, group: Group::Identity, about: "the first content patch the inn exists in" },
];

/// `areatrigger_involvedrelation`, in table order.
pub const QUEST_COLUMNS: [Column; 2] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the AreaTrigger.dbc id" },
    Column { name: "quest", kind: REF_QUEST, group: Group::Objectives, about: "the quest whose exploration objective is completed by entering the trigger" },
];

/// Every column of one of [`TABLES`]. For the template, the key and the five
/// columns the client's file does not have.
pub fn columns_of(table: &str) -> &'static [Column] {
    match table {
        TEMPLATE => &TEMPLATE_COLUMNS_OF_THE_SERVER,
        TELEPORT => &TELEPORT_COLUMNS,
        TAVERN => &TAVERN_COLUMNS,
        QUEST => &QUEST_COLUMNS,
        BG_ENTRANCE => &BG_ENTRANCE_COLUMNS,
        _ => &[],
    }
}

/// One column, by name, of one of [`TABLES`].
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    columns_of(table).iter().find(|column| column.name == name)
}

/// The key of a battleground entrance row.
pub fn bg_entrance_key(id: u32) -> Key {
    Key::one(ID, u64::from(id))
}

/// The five server columns of one template row, as the server uses it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Template {
    pub id: u32,
    /// The build of the row the server uses.
    pub build: u32,
    pub name: String,
    pub cooldown: u32,
    pub condition_id: u32,
    pub script_id: u32,
    pub script_name: String,
}

impl Template {
    /// The five columns, as a store row names them.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "name", value: crate::sql::text(&self.name) },
            Assignment { column: "cooldown", value: self.cooldown.to_string() },
            Assignment { column: "condition_id", value: self.condition_id.to_string() },
            Assignment { column: "script_id", value: self.script_id.to_string() },
            Assignment { column: "script_name", value: crate::sql::text(&self.script_name) },
        ]
    }

    /// A row as the database returned it.
    pub fn from_row(row: &crate::schema::Row) -> Option<Template> {
        use crate::schema::RowValue;
        let int = |column: &str| row.text(column).and_then(|value| value.parse::<u32>().ok());
        Some(Template {
            id: int("id")?,
            build: int("build").unwrap_or(BUILD),
            name: row.text("name").unwrap_or_default().to_string(),
            cooldown: int("cooldown").unwrap_or(0),
            condition_id: int("condition_id").unwrap_or(0),
            script_id: int("script_id").unwrap_or(0),
            script_name: row.text("script_name").unwrap_or_default().to_string(),
        })
    }
}

/// One battleground entrance, as the server uses it.
#[derive(Debug, Clone, PartialEq)]
pub struct BgEntrance {
    pub id: u32,
    pub name: String,
    pub team: u32,
    pub bg_template: u32,
    pub exit_map: u32,
    pub exit: [f32; 3],
    pub orientation: f32,
}

impl BgEntrance {
    /// A new entrance from trigger `id`, for the Alliance, returning to a place.
    pub fn new(id: u32, exit_map: u32, exit: [f32; 3]) -> BgEntrance {
        BgEntrance {
            id,
            name: String::new(),
            team: 469,
            bg_template: 2,
            exit_map,
            exit,
            orientation: 0.0,
        }
    }

    pub fn key(&self) -> Key {
        bg_entrance_key(self.id)
    }

    /// Every column but the key, as a created row names them.
    pub fn assignments(&self) -> Vec<Assignment> {
        let f = crate::sql::float;
        vec![
            Assignment { column: "name", value: crate::sql::text(&self.name) },
            Assignment { column: "team", value: self.team.to_string() },
            Assignment { column: "bg_template", value: self.bg_template.to_string() },
            Assignment { column: "exit_map", value: self.exit_map.to_string() },
            Assignment { column: "exit_position_x", value: f(self.exit[0]) },
            Assignment { column: "exit_position_y", value: f(self.exit[1]) },
            Assignment { column: "exit_position_z", value: f(self.exit[2]) },
            Assignment { column: "exit_orientation", value: f(self.orientation) },
        ]
    }

    /// A row as the database returned it.
    pub fn from_row(row: &crate::schema::Row) -> Option<BgEntrance> {
        use crate::schema::RowValue;
        let int = |column: &str| row.text(column)?.parse::<u32>().ok();
        let float = |column: &str| row.text(column)?.parse::<f32>().ok();
        Some(BgEntrance {
            id: int("id")?,
            name: row.text("name").unwrap_or_default().to_string(),
            team: int("team").unwrap_or(0),
            bg_template: int("bg_template").unwrap_or(0),
            exit_map: int("exit_map").unwrap_or(0),
            exit: [
                float("exit_position_x").unwrap_or(0.0),
                float("exit_position_y").unwrap_or(0.0),
                float("exit_position_z").unwrap_or(0.0),
            ],
            orientation: float("exit_orientation").unwrap_or(0.0),
        })
    }

    /// Why the server would skip this row, from the row alone:
    /// `LoadBattleGroundEntranceTriggers` refuses a team that is not one of
    /// [`TEAMS`].
    pub fn check(&self) -> Vec<String> {
        match TEAMS.iter().any(|team| team.value == self.team) {
            true => Vec::new(),
            false => vec![format!("team {} is not 469 (Alliance) or 67 (Horde)", self.team)],
        }
    }
}

/// The key of a teleport row.
pub fn teleport_key(id: u32, patch: u32) -> Key {
    Key::two((ID, u64::from(id)), ("patch", u64::from(patch)))
}

/// The key of a tavern row.
pub fn tavern_key(id: u32) -> Key {
    Key::one(ID, u64::from(id))
}

/// The key of a quest row.
pub fn quest_key(id: u32) -> Key {
    Key::one(ID, u64::from(id))
}

/// One teleport, as the server uses it.
#[derive(Debug, Clone, PartialEq)]
pub struct Teleport {
    pub id: u32,
    pub patch: u32,
    pub name: String,
    pub message: String,
    pub required_level: u32,
    pub required_condition: u32,
    pub target_map: u32,
    pub target: [f32; 3],
    pub orientation: f32,
}

impl Teleport {
    /// A new teleport from trigger `id` to a place, at patch 0, open to every
    /// level.
    pub fn new(id: u32, target_map: u32, target: [f32; 3]) -> Teleport {
        Teleport {
            id,
            patch: 0,
            name: String::new(),
            message: String::new(),
            required_level: 0,
            required_condition: 0,
            target_map,
            target,
            orientation: 0.0,
        }
    }

    pub fn key(&self) -> Key {
        teleport_key(self.id, self.patch)
    }

    /// Every column but the key, as a created row names them.
    pub fn assignments(&self) -> Vec<Assignment> {
        let f = crate::sql::float;
        vec![
            Assignment { column: "name", value: crate::sql::text(&self.name) },
            Assignment { column: "message", value: crate::sql::text(&self.message) },
            Assignment { column: "required_level", value: self.required_level.to_string() },
            Assignment { column: "required_condition", value: self.required_condition.to_string() },
            Assignment { column: "target_map", value: self.target_map.to_string() },
            Assignment { column: "target_position_x", value: f(self.target[0]) },
            Assignment { column: "target_position_y", value: f(self.target[1]) },
            Assignment { column: "target_position_z", value: f(self.target[2]) },
            Assignment { column: "target_orientation", value: f(self.orientation) },
        ]
    }

    /// A row as the database returned it.
    pub fn from_row(row: &crate::schema::Row) -> Option<Teleport> {
        use crate::schema::RowValue;
        let int = |column: &str| row.text(column)?.parse::<u32>().ok();
        let float = |column: &str| row.text(column)?.parse::<f32>().ok();
        Some(Teleport {
            id: int("id")?,
            patch: int("patch").unwrap_or(0),
            name: row.text("name").unwrap_or_default().to_string(),
            message: row.text("message").unwrap_or_default().to_string(),
            required_level: int("required_level").unwrap_or(0),
            required_condition: int("required_condition").unwrap_or(0),
            target_map: int("target_map")?,
            target: [
                float("target_position_x")?,
                float("target_position_y")?,
                float("target_position_z")?,
            ],
            orientation: float("target_orientation").unwrap_or(0.0),
        })
    }

    /// Why the server would skip this row, from the row alone.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.target == [0.0, 0.0, 0.0] {
            out.push("the target x, y and z are all 0, which the server reads as no target".to_string());
        }
        out
    }
}

/// Why the server would skip a created row of one of [`TABLES`], from its
/// columns alone. `row` holds the key and every column as SQL literals.
pub fn check_created(table: &str, row: &crate::schema::Row) -> Vec<String> {
    use crate::schema::RowValue;
    match table {
        TELEPORT => Teleport::from_row(row).map(|teleport| teleport.check()).unwrap_or_else(|| {
            vec!["the row does not name a target map and position".to_string()]
        }),
        QUEST if row.text("quest").and_then(|quest| quest.parse::<u32>().ok()).unwrap_or(0) == 0 => {
            vec!["the row names no quest".to_string()]
        }
        BG_ENTRANCE => BgEntrance::from_row(row).map(|entrance| entrance.check()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The teleport rows the server uses, one per trigger: the highest patch at or
/// below `wow_patch`, which is `LoadAreaTriggerTeleports`' own selection.
pub fn teleports_query(wow_patch: u32) -> String {
    format!(
        "SELECT * FROM `{TELEPORT}` t1 WHERE `patch` = (SELECT MAX(`patch`) FROM `{TELEPORT}` t2 \
         WHERE t1.`id` = t2.`id` AND t2.`patch` <= {wow_patch}) ORDER BY `id`"
    )
}

/// The inns the server uses at `wow_patch`.
pub fn taverns_query(wow_patch: u32) -> String {
    format!("SELECT * FROM `{TAVERN}` WHERE `patch_min` <= {wow_patch} ORDER BY `id`")
}

/// Every quest trigger.
pub fn quests_query() -> String {
    format!("SELECT * FROM `{QUEST}` ORDER BY `id`")
}

/// Every battleground entrance.
pub fn bg_entrances_query() -> String {
    format!("SELECT * FROM `{BG_ENTRANCE}` ORDER BY `id`")
}

/// The template rows the server uses, one per trigger: the highest build at or
/// below 5875, which is `LoadAreaTriggers`' own selection.
pub fn templates_query() -> String {
    format!(
        "SELECT * FROM `{TEMPLATE}` t1 WHERE `build` = (SELECT MAX(`build`) FROM `{TEMPLATE}` t2 \
         WHERE t1.`id` = t2.`id` AND t2.`build` <= {BUILD}) ORDER BY `id`"
    )
}

/// The template row the server uses for one trigger, for an undo when the
/// project's apply is what makes the dev row.
pub fn winning_template_query(id: u32) -> String {
    format!(
        "SELECT * FROM `{TEMPLATE}` WHERE `id` = {id} AND `build` <= {BUILD} ORDER BY `build` DESC LIMIT 1"
    )
}

/// The row a key names, which an undo is taken from.
pub fn row_query(table: &str, key: &Key) -> String {
    format!("SELECT * FROM {} WHERE {} LIMIT 1", crate::sql::name(table), key.where_clause())
}

/// The statements one row of [`TABLES`] becomes: an `UPDATE` for an edit, a
/// `DELETE` and an `INSERT` for a created row, a `DELETE` for a removed one.
/// A template row is only ever edited: its dev row is made first with
/// [`copy_forward`], and the `UPDATE` sets the server columns on it.
pub fn row_statements(table: &str, key: &Key, life: Life, changes: &[Assignment]) -> Vec<String> {
    if table == TEMPLATE {
        let (Some(id), Life::Update) = (key.first(), life) else {
            return Vec::new();
        };
        let Some(update) = row::update(table, key, changes) else {
            return Vec::new();
        };
        return vec![copy_forward(id as u32), update];
    }
    match life {
        Life::Update => row::update(table, key, changes).into_iter().collect(),
        Life::Insert => match row::insert(table, key, changes) {
            Some(statement) => vec![row::delete(table, key), statement],
            None => Vec::new(),
        },
        Life::Delete => vec![row::delete(table, key)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(id: u32) -> Trigger {
        Trigger {
            id,
            record: 0,
            map: 0,
            at: [-8761.85, 848.557, 87.8052],
            radius: 0.0,
            extent: [4.972, 9.694, 7.444],
            yaw: 0.6632,
        }
    }

    #[test]
    fn a_moved_trigger_changes_only_its_position_at_build_5875() {
        let shipped = trigger(101);
        assert!(statements(Some(&shipped), &shipped).is_empty());
        let mut edited = shipped;
        edited.at[2] += 2.0;
        let changed: Vec<&str> = changes(Some(&shipped), &edited).iter().map(|c| c.column).collect();
        assert_eq!(changed, ["z"]);
        let sql = statements(Some(&shipped), &edited);
        assert_eq!(sql.len(), 3);
        assert!(sql[0].starts_with("INSERT IGNORE INTO `areatrigger_template` SELECT `id`, 5875, `name`"));
        assert!(sql[2].ends_with("WHERE `id` = 101 AND `build` = 5875;"), "{}", sql[2]);
    }

    #[test]
    fn a_new_trigger_names_every_column_the_file_feeds() {
        let insert = insert_new(&trigger(4500));
        for column in ["map_id", "x", "radius", "box_x", "box_orientation"] {
            assert!(insert.contains(&format!("`{column}`")), "{column}: {insert}");
        }
        assert_eq!(changes(None, &trigger(4500)).len(), 9);
        assert!(statements(None, &trigger(70_000)).is_empty());
    }

    #[test]
    fn the_undo_deletes_a_dev_row_this_project_made() {
        let mut edited = trigger(101);
        edited.radius = 5.0;
        let changes = changes(Some(&trigger(101)), &edited);
        assert_eq!(
            undo(101, &changes, None).unwrap(),
            "DELETE FROM `areatrigger_template` WHERE `id` = 101 AND `build` = 5875;"
        );
    }

    #[test]
    fn a_teleport_with_no_target_is_refused() {
        let teleport = Teleport::new(78, 36, [0.0; 3]);
        assert_eq!(teleport.check().len(), 1);
        let mut row = crate::schema::Row::new();
        row.insert("id".into(), Some("78".into()));
        row.insert("patch".into(), Some("0".into()));
        for change in Teleport::new(78, 36, [-14.57, -385.47, 62.45]).assignments() {
            row.insert(change.column.to_string(), Some(change.value.trim_matches('\'').to_string()));
        }
        assert!(check_created(TELEPORT, &row).is_empty());
        let mut quest = crate::schema::Row::new();
        quest.insert("quest".into(), Some("0".into()));
        assert_eq!(check_created(QUEST, &quest).len(), 1);
    }

    #[test]
    fn every_column_is_the_tables_own_in_order() {
        let names = |columns: &[Column]| columns.iter().map(|c| c.name).collect::<Vec<_>>();
        assert_eq!(
            names(&TELEPORT_COLUMNS),
            [
                "id", "patch", "name", "message", "required_level", "required_condition", "target_map",
                "target_position_x", "target_position_y", "target_position_z", "target_orientation"
            ]
        );
        assert_eq!(names(&TAVERN_COLUMNS), ["id", "name", "patch_min"]);
        assert_eq!(names(&QUEST_COLUMNS), ["id", "quest"]);
        assert!(teleports_query(10).contains("t2.`patch` <= 10"));
    }
}
