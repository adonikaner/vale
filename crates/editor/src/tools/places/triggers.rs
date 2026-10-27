//! Area triggers: the spheres and boxes of `AreaTrigger.dbc` on the open map,
//! drawn in the world, picked, dragged, made and removed, and what the server
//! does when a character is inside one.
//!
//! The volume is a client table. The client tests the player's position
//! against the current map's rows and sends the id of the first it is inside;
//! the server decides what that means from three tables of its own:
//!
//! ```text
//! areatrigger_template          the label, and the script the trigger runs:
//!                               a C++ script by name or areatrigger_scripts
//!                               rows by id, with a condition and a cooldown
//! areatrigger_teleport          send the character to a map and a position
//! areatrigger_tavern            the trigger is an inn: a character inside rests
//! areatrigger_involvedrelation  standing inside completes a quest's
//!                               exploration objective
//! areatrigger_bg_entrance       open a battleground's list for one side
//! ```
//!
//! A trigger with none of these does nothing on the server. Most such shipped
//! triggers say so in the template's label ("Unused"); a few are handled by
//! the server's own C++ by id, which no table records.
//!
//! What an edit to the client table is, including the rule that a new row goes
//! at the end of its map's block, is `vale_edit::dbc::places`. What the three
//! server tables hold is `vale_mangos::trigger`. This file is the pointer, the
//! drawing, and the reads and writes of the three tables' rows; the panel is
//! `crate::ui::tools::places::triggers`.
//!
//! ## What the pointer does
//!
//! ```text
//! click a trigger               select it: its centre, or anywhere inside it
//! drag a trigger                move it across the horizontal plane
//! Ctrl + drag                   move it up and down instead
//! New trigger armed + click     make a sphere on the ground there
//! Delete                        remove the selected trigger
//! Escape                        disarm, cancel a pick, then drop the selection
//! ```
//!
//! ## Picking a place on another map
//!
//! A teleport's target and a battleground entrance's exit are a map and a
//! position with a facing. A [`Pick`] sets one: press on the ground for the
//! position, drag before releasing to point the facing, release to write all
//! of it as one undo entry. When the place is on another map, the pick takes
//! the editor there first ([`Travel`], with the camera over the place the row
//! names now) and brings it back to the trigger afterwards, or on Escape.
//!
//! ## The server rows
//!
//! While the tool is open and a connection is set, the five tables are read
//! whole, one query each, and kept until an apply moves the database
//! (`EditSession::database_writes`). An edit is a row of the project's store on
//! `crate::subjects::services`' terms: a new row is a [`Life::Insert`] carrying
//! every column, a removed row the database holds is a [`Life::Delete`], and a
//! column edit is a value under the row's key. A template row's server columns
//! are edits under its build-5875 key; its volume is not here, since it
//! follows the client table on every save (`crate::server::rows`).
//!
//! ## Where an edit reaches the game
//!
//! The client reads the table from the project's copy on the next playtest. The
//! server reads `areatrigger_template` and `areatrigger_bg_entrance` at
//! startup only, and the teleport, inn and quest tables on their own
//! `.reload`.

use crate::tools::points::{self, Aim, MapPoint, PointTool, Points, HANDLE_PIXELS};
use crate::tools::Tool;
use crate::marks::{Look, Marks};
use crate::server::store::Gesture;
use crate::session::EditSession;
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use vale_edit::dbc::places::{self, Trigger};
use vale_edit::dbc::DbcFile;
use vale_mangos::row::{Assignment, Edits, Key, Life, RowEdit};
use vale_mangos::schema::Row;
use vale_mangos::trigger::{self, BgEntrance, Teleport};
use bevy::prelude::*;
use crate::server::fresh::FreshReads;
use std::collections::HashMap;

/// What a click on empty ground does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Armed {
    #[default]
    Nothing,
    /// Make a sphere trigger on the ground.
    NewTrigger,
}

/// Which place a [`Pick`] sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickFor {
    /// The teleport's target.
    Teleport,
    /// The battleground entrance's exit.
    BgExit,
}

impl PickFor {
    pub(crate) fn noun(self) -> &'static str {
        match self {
            PickFor::Teleport => "teleport target",
            PickFor::BgExit => "battleground exit",
        }
    }
}

/// A place being picked for a trigger's row, possibly on another map.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Pick {
    pub(crate) what: PickFor,
    pub(crate) trigger: u32,
    /// The map the place is on.
    pub(crate) map: u32,
    /// The map the trigger is on and where the camera stood, when the pick
    /// took the editor away from it.
    pub(crate) home: Option<(String, u32, Vec3)>,
    /// Where the button went down, while it is held to set the facing.
    pub(crate) pressed: Option<Vec3>,
    /// The facing, in radians about up from north; starts at the row's own.
    pub(crate) facing: f32,
}

/// A map change a pick asks for, made by [`travel`], which has the camera.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Travel {
    /// Open this map and look at this place on it. The height is kept, since
    /// a map that is one building has no ground for the camera to drop to.
    To { name: String, id: u32, at: Vec3 },
    /// Return to the pick's home.
    Home,
}

/// The five server tables as the database holds them, one entry per trigger.
#[derive(Debug, Clone, Default)]
pub(crate) struct Held {
    /// The template row the server uses for each trigger: its five server
    /// columns.
    pub(crate) templates: HashMap<u32, trigger::Template>,
    pub(crate) teleports: HashMap<u32, Teleport>,
    pub(crate) entrances: HashMap<u32, BgEntrance>,
    /// The inns, by trigger id, with the row's `name` and `patch_min`.
    pub(crate) taverns: HashMap<u32, (String, u32)>,
    /// The quest each quest trigger completes.
    pub(crate) quests: HashMap<u32, u32>,
}

/// What a trigger does on the server, with the project's edits over the
/// database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Does {
    pub(crate) teleport: bool,
    pub(crate) inn: bool,
    pub(crate) quest: bool,
    pub(crate) entrance: bool,
    pub(crate) script: bool,
}

/// The trigger tool's state.
#[derive(Resource, Debug)]
pub(crate) struct Triggers {
    /// The triggers on the open map, and what the pointer is doing with them.
    pub(crate) points: Points<Trigger>,
    /// The maps whose rows are not one block of the file; see
    /// `vale_edit::dbc::places::trigger_blocks`. Empty for the shipped file.
    pub(crate) blocks: Vec<u32>,
    pub(crate) armed: Armed,
    /// The radius a new trigger is made with, in yards.
    pub(crate) radius: f32,
    /// A place being picked; see [`Pick`].
    pub(crate) pick: Option<Pick>,
    /// A map change [`travel`] is to make.
    pub(crate) travel: Option<Travel>,
    /// The trigger to select once the list is rebuilt, after a pick returns.
    reselect: Option<u32>,
    /// The server's rows, read at once and kept until an apply moves
    /// `EditSession::database_writes`. See [`Self::held`].
    reads: FreshReads<(), Held>,
}

impl Default for Triggers {
    fn default() -> Triggers {
        Triggers {
            points: Points::default(),
            blocks: Vec::new(),
            armed: Armed::Nothing,
            radius: DEFAULT_RADIUS,
            pick: None,
            travel: None,
            reselect: None,
            reads: FreshReads::default(),
        }
    }
}

/// The radius a new trigger is made with, by default. The Deadmines entrance
/// is 7 yards and most instance portals are between 5 and 10.
pub(crate) const DEFAULT_RADIUS: f32 = 5.0;

/// The size a sphere turned into a box starts at, in yards on each side.
pub(crate) const DEFAULT_BOX: f32 = 10.0;

/// How far from the camera a trigger is drawn and picked, in yards.
const MARKER_RANGE: f32 = 2600.0;

impl Triggers {
    /// The server's rows, once read.
    pub(crate) fn held(&self) -> Option<&Held> {
        self.reads.value()
    }

    /// Why the last read of the server's rows answered nothing, when it
    /// answered nothing.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.reads.trouble()
    }

    pub(crate) fn trigger(&self, id: u32) -> Option<&Trigger> {
        self.points.get(id)
    }

    /// Rebuild the list on the next frame, after an edit this frame.
    pub(crate) fn stale(&mut self) {
        self.points.stale();
    }

    /// The trigger's teleport as the project leaves it, whether that row is
    /// the database's, and what the project says is to become of it. `None`
    /// when neither side has one.
    pub(crate) fn teleport(&self, edits: &Edits, id: u32) -> Option<(Teleport, bool, Life)> {
        let held = self.held()?.teleports.get(&id);
        let key = teleport_key(held, id);
        let assignments = held.map(Teleport::assignments);
        let (row, life) = shown(edits, trigger::TELEPORT, &key, assignments.as_deref())?;
        Some((Teleport::from_row(&row)?, held.is_some(), life))
    }

    /// The teleport row's key, and the row as the database holds it: what an
    /// unedited column of the form shows, and what its revert arrow takes the
    /// column back to. The row is empty for a teleport the database does not
    /// hold.
    pub(crate) fn teleport_held(&self, id: u32) -> (Key, Row) {
        let held = self.held().and_then(|held| held.teleports.get(&id));
        let key = teleport_key(held, id);
        let row = held
            .and_then(|held| shown(&Edits::default(), trigger::TELEPORT, &key, Some(&held.assignments())))
            .map_or_else(Row::new, |(row, _)| row);
        (key, row)
    }

    /// The quest the trigger completes as the project leaves it, and whether
    /// the database holds the row.
    pub(crate) fn quest(&self, edits: &Edits, id: u32) -> Option<(u32, bool, Life)> {
        let held = self.held()?.quests.get(&id).copied();
        let assignments = held.map(quest_assignments);
        let (row, life) = shown(edits, trigger::QUEST, &trigger::quest_key(id), assignments.as_deref())?;
        use vale_mangos::schema::RowValue;
        Some((row.integer("quest")? as u32, held.is_some(), life))
    }

    /// Whether the trigger is an inn as the project leaves it, and whether the
    /// database holds the row.
    pub(crate) fn inn(&self, edits: &Edits, id: u32) -> Option<(bool, bool)> {
        let held = self.held()?.taverns.get(&id);
        let assignments = held.map(|(name, patch)| tavern_assignments(name, *patch));
        let shown = shown(edits, trigger::TAVERN, &trigger::tavern_key(id), assignments.as_deref());
        Some((shown.is_some_and(|(_, life)| life != Life::Delete), held.is_some()))
    }

    /// The template row's server columns as the project leaves them, and
    /// whether the database holds a template row for the trigger at all.
    pub(crate) fn template(&self, edits: &Edits, id: u32) -> Option<(trigger::Template, bool)> {
        let held = self.held()?.templates.get(&id);
        let assignments = held.map(trigger::Template::assignments);
        let key = trigger::key(id);
        match shown(edits, trigger::TEMPLATE, &key, assignments.as_deref()) {
            Some((row, _)) => {
                let mut template = trigger::Template::from_row(&row)?;
                template.build = held.map_or(trigger::BUILD, |held| held.build);
                Some((template, held.is_some()))
            }
            // No row in the database: the project's edits alone, if any.
            None => {
                let claim = edits.row(trigger::TEMPLATE, &key)?;
                let mut row = Row::new();
                row.insert("id".to_string(), Some(id.to_string()));
                for (column, value) in &claim.columns {
                    row.insert(column.clone(), Some(vale_mangos::sql::unquote(value)));
                }
                Some((trigger::Template::from_row(&row)?, false))
            }
        }
    }

    /// The battleground entrance as the project leaves it, whether the
    /// database holds the row, and what the project says is to become of it.
    pub(crate) fn entrance(&self, edits: &Edits, id: u32) -> Option<(BgEntrance, bool, Life)> {
        let held = self.held()?.entrances.get(&id);
        let assignments = held.map(BgEntrance::assignments);
        let (row, life) = shown(edits, trigger::BG_ENTRANCE, &trigger::bg_entrance_key(id), assignments.as_deref())?;
        Some((BgEntrance::from_row(&row)?, held.is_some(), life))
    }

    /// The C++ script names the database's template rows use, sorted, as
    /// suggestions for the script name field.
    pub(crate) fn script_names(&self) -> Vec<String> {
        let Some(held) = self.held() else {
            return Vec::new();
        };
        let mut names: Vec<String> = held
            .templates
            .values()
            .map(|template| template.script_name.clone())
            .filter(|name| !name.is_empty())
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// What the trigger does, for the colour it is drawn in.
    pub(crate) fn does(&self, edits: &Edits, id: u32) -> Does {
        let live = |found: Option<Life>| found.is_some_and(|life| life != Life::Delete);
        let script = self
            .template(edits, id)
            .is_some_and(|(template, _)| template.script_id != 0 || !template.script_name.is_empty());
        Does {
            teleport: live(self.teleport(edits, id).map(|(_, _, life)| life)),
            inn: self.inn(edits, id).is_some_and(|(inn, _)| inn),
            quest: live(self.quest(edits, id).map(|(_, _, life)| life)),
            entrance: live(self.entrance(edits, id).map(|(_, _, life)| life)),
            script,
        }
    }

    /// Start picking a place on `map` for one of the trigger's rows. `now_at`
    /// is the place the row names now, which the camera is put over when the
    /// pick changes map, and `facing` its facing. Answers why it cannot start.
    pub(crate) fn start_pick(
        &mut self,
        session: &EditSession,
        what: PickFor,
        trigger: u32,
        map: u32,
        now_at: [f32; 3],
        facing: f32,
    ) -> Result<(), String> {
        let home = match map == session.map_id {
            true => None,
            false => {
                let Some((id, name)) = session.maps.iter().find(|(id, _)| *id == map).cloned() else {
                    return Err(format!("map {map} is not in Map.dbc, so it cannot be opened"));
                };
                self.travel = Some(Travel::To {
                    name,
                    id,
                    at: Vec3::from(now_at),
                });
                // The camera's place is filled in by `travel`.
                Some((session.map.clone(), session.map_id, Vec3::ZERO))
            }
        };
        self.armed = Armed::Nothing;
        self.pick = Some(Pick {
            what,
            trigger,
            map,
            home,
            pressed: None,
            facing,
        });
        Ok(())
    }

    /// Stop picking, and go back to the trigger's map when the pick left it.
    pub(crate) fn end_pick(&mut self) {
        if let Some(pick) = self.pick.take() {
            if pick.home.is_some() {
                self.travel = Some(Travel::Home);
                self.reselect = Some(pick.trigger);
                // Kept until `travel` has used it.
                self.pick = Some(Pick { pressed: None, ..pick });
            }
        }
    }
}

/// The key a trigger's teleport is stored under: the database row's, or patch
/// 0 for a row the project creates.
fn teleport_key(held: Option<&Teleport>, id: u32) -> Key {
    held.map(Teleport::key).unwrap_or_else(|| trigger::teleport_key(id, 0))
}

fn quest_assignments(quest: u32) -> Vec<Assignment> {
    vec![Assignment { column: "quest", value: quest.to_string() }]
}

fn tavern_assignments(name: &str, patch_min: u32) -> Vec<Assignment> {
    vec![
        Assignment { column: "name", value: vale_mangos::sql::text(name) },
        Assignment { column: "patch_min", value: patch_min.to_string() },
    ]
}

/// A row as the project leaves it, as the database would return it: the
/// database's columns, `held`, with the project's edits over them, or the
/// columns of a row the project creates. A row the project removes comes back
/// with [`Life::Delete`] and the database's columns. `None` when neither side
/// has the row.
pub(crate) fn shown(edits: &Edits, table: &str, key: &Key, held: Option<&[Assignment]>) -> Option<(Row, Life)> {
    let claim = edits.row(table, key);
    let mut row = Row::new();
    for (column, value) in &key.0 {
        row.insert(column.clone(), Some(value.clone()));
    }
    let mut put = |column: &str, literal: &str| {
        row.insert(column.to_string(), Some(vale_mangos::sql::unquote(literal)));
    };
    // A row the project creates is shown as its creation even when the
    // database also holds it, which it does once the creation is applied.
    let held = held.filter(|_| !claim.is_some_and(|claim| claim.life == Life::Insert));
    let life = match (held, claim) {
        (Some(held), claim) => {
            for change in held {
                put(change.column, &change.value);
            }
            match claim {
                Some(claim) if claim.life == Life::Delete => Life::Delete,
                Some(claim) => {
                    for (column, value) in &claim.columns {
                        put(column, value);
                    }
                    Life::Update
                }
                None => Life::Update,
            }
        }
        (None, Some(claim)) if claim.life == Life::Insert => {
            // A column the creation lacks is shown at its default, so the row
            // can be seen and completed; `missing` names it, and the plan
            // refuses the row until it is written.
            for column in columns_of(table).iter().filter(|column| column.editable()) {
                let blank = match column.kind {
                    vale_mangos::schema::Kind::Text => "''",
                    _ => "0",
                };
                put(column.name, blank);
            }
            for (column, value) in &claim.columns {
                put(column, value);
            }
            Life::Insert
        }
        (None, _) => return None,
    };
    Some((row, life))
}

/// The columns of one of the tables `crate::server::places` writes.
fn columns_of(table: &str) -> &'static [vale_mangos::schema::Column] {
    crate::server::places::table_named(table).map_or(&[], |table| table.columns)
}

/// The columns a row the project creates does not carry, which the plan
/// refuses it for. Empty for a row that is not a creation.
pub(crate) fn missing(edits: &Edits, table: &str, key: &Key) -> Vec<&'static str> {
    let Some(claim) = edits.row(table, key).filter(|claim| claim.life == Life::Insert) else {
        return Vec::new();
    };
    columns_of(table)
        .iter()
        .filter(|column| column.editable() && !claim.columns.contains_key(column.name))
        .map(|column| column.name)
        .collect()
}

/// Whether the database's row under a key is the database's own rather than a
/// creation of this project's that has been applied. Only the first is kept
/// or removed with a claim of its own; the second is the project's to take
/// back.
fn the_databases_own(session: &EditSession, table: &str, key: &Key, held: bool) -> bool {
    held && session
        .rows.edits
        .row(table, key)
        .is_none_or(|claim| claim.life != Life::Insert)
}

/// Create a row, or keep one the database holds that was marked for removal.
pub(crate) fn create_row(
    session: &mut EditSession,
    table: &str,
    key: &Key,
    held: bool,
    columns: &[Assignment],
    gesture: Gesture<'_>,
) {
    match the_databases_own(session, table, key, held) {
        true => session.set_server_row(table, key, None, Some(gesture)),
        false => {
            let row = crate::subjects::services::creation(columns);
            session.set_server_row(table, key, Some(&row), Some(gesture));
        }
    }
}

/// Remove a row: a `Delete` claim for one the database holds, and the claim
/// taken back for one the project creates, applied or not. A taken-back
/// creation that was applied is put back by the next apply.
pub(crate) fn remove_row(session: &mut EditSession, table: &str, key: &Key, held: bool, gesture: Gesture<'_>) {
    match the_databases_own(session, table, key, held) {
        true => {
            let row = RowEdit {
                life: Life::Delete,
                ..RowEdit::default()
            };
            session.set_server_row(table, key, Some(&row), Some(gesture));
        }
        false => session.set_server_row(table, key, None, Some(gesture)),
    }
}

/// Give the trigger a teleport to a place, or keep the database's that was
/// marked for removal.
pub(crate) fn add_teleport(session: &mut EditSession, triggers: &Triggers, id: u32, map: u32, at: [f32; 3], now: f64) {
    let held = triggers.held().and_then(|held| held.teleports.get(&id));
    let key = teleport_key(held, id);
    let label = format!("{} {}", trigger::TELEPORT, key.text());
    let gesture = Gesture { label: "Add teleport", subject: &label, now };
    let columns = Teleport::new(id, map, at).assignments();
    create_row(session, trigger::TELEPORT, &key, held.is_some(), &columns, gesture);
}

pub(crate) fn remove_teleport(session: &mut EditSession, triggers: &Triggers, id: u32, now: f64) {
    let held = triggers.held().and_then(|held| held.teleports.get(&id));
    let key = teleport_key(held, id);
    let label = format!("{} {}", trigger::TELEPORT, key.text());
    let gesture = Gesture { label: "Remove teleport", subject: &label, now };
    remove_row(session, trigger::TELEPORT, &key, held.is_some(), gesture);
}

/// Set one column of the trigger's teleport. `value` is a SQL literal.
pub(crate) fn set_teleport(session: &mut EditSession, triggers: &Triggers, id: u32, column: &'static str, value: String, now: f64) {
    let held = triggers.held().and_then(|held| held.teleports.get(&id));
    let key = teleport_key(held, id);
    let label = format!("{} {} {column}", trigger::TELEPORT, key.text());
    let gesture = Gesture { label: "Edit teleport", subject: &label, now };
    let assignments = held.map(Teleport::assignments);
    crate::subjects::services::write_column(session, trigger::TELEPORT, &key, assignments.as_deref(), column, value, gesture);
}

/// Send the trigger's teleport to a place with a facing, as one undo entry.
pub(crate) fn set_target(session: &mut EditSession, triggers: &Triggers, id: u32, map: u32, at: Vec3, facing: f32, now: f64) {
    let subject = format!("{} {id} target", trigger::TELEPORT);
    session.as_one("Edit teleport", &subject, |session| {
        let f = vale_mangos::sql::float;
        set_teleport(session, triggers, id, "target_map", map.to_string(), now);
        set_teleport(session, triggers, id, "target_position_x", f(at.x), now);
        set_teleport(session, triggers, id, "target_position_y", f(at.y), now);
        set_teleport(session, triggers, id, "target_position_z", f(at.z), now);
        set_teleport(session, triggers, id, "target_orientation", f(facing), now);
    });
}

/// Give the trigger a battleground entrance returning to a place, or keep the
/// database's that was marked for removal.
pub(crate) fn add_entrance(session: &mut EditSession, triggers: &Triggers, id: u32, map: u32, at: [f32; 3], now: f64) {
    let held = triggers.held().and_then(|held| held.entrances.get(&id)).is_some();
    let key = trigger::bg_entrance_key(id);
    let label = format!("{} {}", trigger::BG_ENTRANCE, key.text());
    let gesture = Gesture { label: "Add battleground entrance", subject: &label, now };
    create_row(session, trigger::BG_ENTRANCE, &key, held, &BgEntrance::new(id, map, at).assignments(), gesture);
}

pub(crate) fn remove_entrance(session: &mut EditSession, triggers: &Triggers, id: u32, now: f64) {
    let held = triggers.held().and_then(|held| held.entrances.get(&id)).is_some();
    let key = trigger::bg_entrance_key(id);
    let label = format!("{} {}", trigger::BG_ENTRANCE, key.text());
    let gesture = Gesture { label: "Remove battleground entrance", subject: &label, now };
    remove_row(session, trigger::BG_ENTRANCE, &key, held, gesture);
}

/// Set one column of the trigger's battleground entrance. `value` is a SQL
/// literal.
pub(crate) fn set_entrance(session: &mut EditSession, triggers: &Triggers, id: u32, column: &'static str, value: String, now: f64) {
    let held = triggers.held().and_then(|held| held.entrances.get(&id));
    let key = trigger::bg_entrance_key(id);
    let label = format!("{} {} {column}", trigger::BG_ENTRANCE, key.text());
    let gesture = Gesture { label: "Edit battleground entrance", subject: &label, now };
    let assignments = held.map(BgEntrance::assignments);
    crate::subjects::services::write_column(session, trigger::BG_ENTRANCE, &key, assignments.as_deref(), column, value, gesture);
}

/// Send the entrance's exit to a place with a facing, as one undo entry.
pub(crate) fn set_exit(session: &mut EditSession, triggers: &Triggers, id: u32, map: u32, at: Vec3, facing: f32, now: f64) {
    let subject = format!("{} {id} exit", trigger::BG_ENTRANCE);
    session.as_one("Edit battleground entrance", &subject, |session| {
        let f = vale_mangos::sql::float;
        set_entrance(session, triggers, id, "exit_map", map.to_string(), now);
        set_entrance(session, triggers, id, "exit_position_x", f(at.x), now);
        set_entrance(session, triggers, id, "exit_position_y", f(at.y), now);
        set_entrance(session, triggers, id, "exit_position_z", f(at.z), now);
        set_entrance(session, triggers, id, "exit_orientation", f(facing), now);
    });
}

/// Set one of the template row's five server columns. `value` is a SQL
/// literal. A value typed back to the database's clears the edit.
pub(crate) fn set_template(session: &mut EditSession, triggers: &Triggers, id: u32, column: &'static str, value: String, now: f64) {
    let held = triggers.held().and_then(|held| held.templates.get(&id));
    let key = trigger::key(id);
    let label = format!("{} {} {column}", trigger::TEMPLATE, key.text());
    let gesture = Gesture { label: "Edit area trigger script", subject: &label, now };
    match held {
        Some(held) => {
            let assignments = held.assignments();
            crate::subjects::services::write_column(session, trigger::TEMPLATE, &key, Some(&assignments), column, value, gesture);
        }
        None => session.set_server_edit(trigger::TEMPLATE, &key, column, Some(value), Some(gesture)),
    }
}

/// Write what a finished pick chose. Answers the status line.
pub(crate) fn finish_pick(session: &mut EditSession, triggers: &Triggers, pick: &Pick, at: Vec3, now: f64) -> String {
    match pick.what {
        PickFor::Teleport => set_target(session, triggers, pick.trigger, pick.map, at, pick.facing, now),
        PickFor::BgExit => set_exit(session, triggers, pick.trigger, pick.map, at, pick.facing, now),
    }
    format!(
        "trigger {}'s {}: {:.1}, {:.1}, {:.1} on map {}, facing {:.0}\u{b0}",
        pick.trigger,
        pick.what.noun(),
        at.x,
        at.y,
        at.z,
        pick.map,
        pick.facing.to_degrees().rem_euclid(360.0)
    )
}

/// The facing from one place toward another across the ground, in radians
/// about up from north (+x toward +y), which is the server's convention.
pub(crate) fn facing_toward(from: Vec3, to: Vec3) -> Option<f32> {
    let d = (to - from).truncate();
    (d.length() > 0.5).then(|| d.y.atan2(d.x).rem_euclid(std::f32::consts::TAU))
}

/// Make the trigger an inn, or stop it being one.
pub(crate) fn set_inn(session: &mut EditSession, triggers: &Triggers, id: u32, inn: bool, now: f64) {
    let held = triggers.held().and_then(|held| held.taverns.get(&id)).is_some();
    let key = trigger::tavern_key(id);
    let label = format!("{} {}", trigger::TAVERN, key.text());
    match inn {
        true => {
            let gesture = Gesture { label: "Add inn", subject: &label, now };
            create_row(session, trigger::TAVERN, &key, held, &tavern_assignments("", 0), gesture);
        }
        false => {
            let gesture = Gesture { label: "Remove inn", subject: &label, now };
            remove_row(session, trigger::TAVERN, &key, held, gesture);
        }
    }
}

/// Make standing in the trigger complete `quest`'s exploration objective, or,
/// with 0, nothing's.
pub(crate) fn set_quest(session: &mut EditSession, triggers: &Triggers, id: u32, quest: u32, now: f64) {
    let held = triggers.held().and_then(|held| held.quests.get(&id)).copied();
    let key = trigger::quest_key(id);
    let label = format!("{} {}", trigger::QUEST, key.text());
    let gesture = Gesture { label: "Edit quest trigger", subject: &label, now };
    if quest == 0 {
        remove_row(session, trigger::QUEST, &key, held.is_some(), gesture);
        return;
    }
    let claimed = session.rows.edits.row(trigger::QUEST, &key).map(|row| row.life);
    match (held, claimed) {
        (Some(_), Some(Life::Delete)) | (None, None | Some(Life::Delete)) => {
            create_row(session, trigger::QUEST, &key, false, &quest_assignments(quest), gesture);
        }
        (held, _) => {
            let assignments = held.map(quest_assignments);
            crate::subjects::services::write_column(
                session,
                trigger::QUEST,
                &key,
                assignments.as_deref(),
                "quest",
                quest.to_string(),
                gesture,
            );
        }
    }
}

/// Write the trigger's volume fields as one gesture.
pub(crate) fn set_volume(session: &mut EditSession, trigger: &Trigger, fields: &[(usize, u32)], label: &str, now: f64) {
    crate::tables::fields::set_fields(
        session,
        places::TRIGGERS,
        trigger.record,
        fields,
        label,
        &format!("AreaTrigger {} volume", trigger.id),
        now,
    );
}

/// Turn a sphere into a box of [`DEFAULT_BOX`] yards, or a box into the
/// sphere that holds its smallest side.
pub(crate) fn set_sphere(session: &mut EditSession, trigger: &Trigger, sphere: bool, now: f64) {
    use vale_assets::tables::areatrigger::fields as tf;
    let fields = match sphere {
        true => {
            let radius = trigger.extent.iter().copied().fold(f32::MAX, f32::min).max(1.0) * 0.5;
            [
                (tf::RADIUS, radius.to_bits()),
                (tf::BOX_LENGTH, 0),
                (tf::BOX_WIDTH, 0),
                (tf::BOX_HEIGHT, 0),
                (tf::BOX_YAW, 0),
            ]
        }
        false => {
            let side = match trigger.radius > 0.0 {
                true => trigger.radius * 2.0,
                false => DEFAULT_BOX,
            };
            [
                (tf::RADIUS, 0),
                (tf::BOX_LENGTH, side.to_bits()),
                (tf::BOX_WIDTH, side.to_bits()),
                (tf::BOX_HEIGHT, side.to_bits()),
                (tf::BOX_YAW, 0),
            ]
        }
    };
    set_volume(session, trigger, &fields, "Change trigger shape", now);
}

/// Make a sphere trigger standing on the ground at `ground`. Answers the
/// status line.
pub(crate) fn new_trigger(session: &mut EditSession, triggers: &mut Triggers, ground: Vec3) -> String {
    let radius = triggers.radius.max(0.5);
    let at = [ground.x, ground.y, ground.z + radius * 0.5];
    match places::new_trigger(&mut session.tables, triggers.points.map, at, radius) {
        Ok(done) => {
            let made = done.made;
            super::flightpaths::record(session, "Add area trigger", done);
            triggers.stale();
            triggers.points.selected = made;
            format!("trigger {} made, radius {radius} yd", made.unwrap_or(0))
        }
        Err(e) => e.to_string(),
    }
}

/// Remove a trigger, and the server rows that say what it does, as one undo
/// entry. Answers the status line.
pub(crate) fn remove_trigger(session: &mut EditSession, triggers: &mut Triggers, id: u32, now: f64) -> String {
    let done = match places::remove_trigger(&mut session.tables, id) {
        Ok(done) => done,
        Err(e) => return e.to_string(),
    };
    let subject = format!("AreaTrigger {id} removed");
    session.history.begin("Remove area trigger");
    for (table, row) in done.rows {
        session.history.record_row(&table, row);
    }
    session.history.end();
    session.table_edited(places::TRIGGERS);
    // The rows naming it on the server go with it, on one entry of their own:
    // a server row and a table row are separate kinds of undo entry.
    session.as_one("Remove area trigger", &subject, |session| {
        let edits = session.rows.edits.clone();
        if triggers.teleport(&edits, id).is_some_and(|(_, _, life)| life != Life::Delete) {
            remove_teleport(session, triggers, id, now);
        }
        if triggers.inn(&edits, id).is_some_and(|(inn, _)| inn) {
            set_inn(session, triggers, id, false, now);
        }
        if triggers.quest(&edits, id).is_some_and(|(_, _, life)| life != Life::Delete) {
            set_quest(session, triggers, id, 0, now);
        }
        if triggers.entrance(&edits, id).is_some_and(|(_, _, life)| life != Life::Delete) {
            remove_entrance(session, triggers, id, now);
        }
        // The template's server columns are edits of a row the client
        // table's mirror stops writing, so they are taken back.
        let key = trigger::key(id);
        if edits.touches(trigger::TEMPLATE, &key) {
            let gesture = Gesture { label: "Remove area trigger", subject: &subject, now };
            session.set_server_row(trigger::TEMPLATE, &key, None, Some(gesture));
        }
    });
    triggers.points.selected = None;
    triggers.points.confirm_remove = None;
    triggers.stale();
    format!("trigger {id} removed")
}

pub(crate) struct TriggerToolPlugin;

impl Plugin for TriggerToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Triggers>()
            .add_systems(
                Update,
                (
                    travel,
                    points::collect::<Triggers>,
                    scripted,
                    read_the_rows,
                    points::aim::<Triggers>,
                    press,
                    pick_facing,
                    points::drag::<Triggers>,
                    keys,
                )
                    .chain()
                    .after(crate::pick::aim),
            )
            .add_systems(Update, draw.after(keys));
    }
}

fn active(tool: &Tool, state: &crate::playtest::Playtest) -> bool {
    *tool == Tool::Triggers && state.editing()
}

impl MapPoint for Trigger {
    fn id(&self) -> u32 {
        self.id
    }

    fn position(&self) -> Vec3 {
        Vec3::from(self.at)
    }
}

impl PointTool for Triggers {
    type Point = Trigger;
    const TOOL: Tool = Tool::Triggers;
    const TABLE: &'static str = places::TRIGGERS;
    const RANGE: f32 = MARKER_RANGE;
    const PICK_PIXELS: f32 = HANDLE_PIXELS;
    const NOUN: &'static str = "trigger";

    fn points(&self) -> &Points<Trigger> {
        &self.points
    }

    fn points_mut(&mut self) -> &mut Points<Trigger> {
        &mut self.points
    }

    fn rebuild(&mut self, table: &DbcFile, map: u32) {
        self.points.list = places::triggers(table).into_iter().filter(|trigger| trigger.map == map).collect();
        self.blocks = places::trigger_blocks(table);
        if let Some(id) = self.reselect.filter(|id| self.points.get(*id).is_some()) {
            self.points.selected = Some(id);
            self.reselect = None;
        }
    }

    fn set_position(session: &mut EditSession, trigger: &Trigger, to: Vec3, now: f64) {
        use vale_assets::tables::areatrigger::fields as tf;
        set_volume(
            session,
            trigger,
            &[(tf::X, to.x.to_bits()), (tf::Y, to.y.to_bits()), (tf::Z, to.z.to_bits())],
            "Move area trigger",
            now,
        );
    }

    /// The nearest volume the pointer's ray passes through.
    fn aim_inside(&self, aim: &Aim, eye: Vec3) -> Option<u32> {
        self.points
            .list
            .iter()
            .filter(|trigger| self.points.near(trigger, eye, MARKER_RANGE))
            .filter_map(|trigger| Some((ray_meets(trigger, aim.origin, aim.direction)?, trigger.id)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    fn remove(&mut self, session: &mut EditSession, id: u32, now: f64) -> String {
        remove_trigger(session, self, id, now)
    }
}

/// `--trigger <id>`: select one trigger of the open map and fly to it, once
/// the list is built.
fn scripted(
    mut triggers: ResMut<Triggers>,
    args: Res<crate::args::Args>,
    mut fly: MessageWriter<crate::camera::FlyTo>,
    mut done: Local<bool>,
) {
    let Some(id) = args.tools.trigger.filter(|_| !*done) else {
        return;
    };
    if !triggers.points.is_built() {
        return;
    }
    *done = true;
    match triggers.trigger(id).copied() {
        Some(trigger) => {
            triggers.points.selected = Some(id);
            fly.write(crate::camera::FlyTo::at(Vec3::from(trigger.at)));
            info!("--trigger: {id} on map {}", trigger.map);
        }
        None => warn!("--trigger {id}: no trigger by that id on this map"),
    }
}

/// Make the map change a pick asked for: to the place's map with the camera
/// over the place, or back to the trigger's map and camera.
fn travel(
    mut triggers: ResMut<Triggers>,
    mut session: Option<ResMut<EditSession>>,
    mut camera: ResMut<crate::camera::EditorCamera>,
) {
    let Some(asked) = triggers.travel.take() else {
        return;
    };
    let Some(session) = session.as_mut() else { return };
    match asked {
        Travel::To { name, id, at } => {
            if let Some(Pick { home: Some(home), .. }) = triggers.pick.as_mut() {
                home.2 = camera.target;
            }
            session.switch_map(name, id, &mut camera);
            // A row that names no place yet is all zeros; the camera then
            // drops to whatever ground is there.
            match at == Vec3::ZERO {
                true => camera.go_to(Vec2::ZERO),
                false => {
                    camera.target = at;
                    camera.wants_the_ground = false;
                }
            }
        }
        Travel::Home => {
            let Some(Pick { home: Some((name, id, at)), .. }) = triggers.pick.take() else {
                return;
            };
            session.switch_map(name, id, &mut camera);
            camera.target = at;
            camera.wants_the_ground = false;
        }
    }
}

/// Read the five server tables while the tool is open, and forget them when
/// an apply has moved the database.
fn read_the_rows(
    mut triggers: ResMut<Triggers>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !active(&tool, &state) {
        return;
    }
    let Some(session) = session else { return };
    let writes = session.database_writes;
    triggers.reads.renew(writes);
    match triggers.reads.poll() {
        Some(Ok(())) => {
            if let Some(held) = triggers.held() {
                info!(
                    "triggers: {} template row(s), {} teleport(s), {} inn(s), {} quest trigger(s), {} battleground entrance(s)",
                    held.templates.len(),
                    held.teleports.len(),
                    held.taverns.len(),
                    held.quests.len(),
                    held.entrances.len()
                );
            }
        }
        Some(Err(e)) => warn!("triggers: {e}"),
        None => {}
    }
    let patch = crate::server::settings::server_patch(&settings);
    triggers.reads.want(writes, (), || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            use vale_mangos::schema::RowValue;
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let templates = db
                .rows(&trigger::templates_query())?
                .iter()
                .filter_map(trigger::Template::from_row)
                .map(|template| (template.id, template))
                .collect();
            let entrances = db
                .rows(&trigger::bg_entrances_query())?
                .iter()
                .filter_map(BgEntrance::from_row)
                .map(|entrance| (entrance.id, entrance))
                .collect();
            let teleports = db
                .rows(&trigger::teleports_query(patch))?
                .iter()
                .filter_map(Teleport::from_row)
                .map(|teleport| (teleport.id, teleport))
                .collect();
            let taverns = db
                .rows(&trigger::taverns_query(patch))?
                .iter()
                .filter_map(|row| {
                    let name = row.text("name").unwrap_or("").to_string();
                    let patch_min = row.integer("patch_min").unwrap_or(0) as u32;
                    Some((row.integer("id")? as u32, (name, patch_min)))
                })
                .collect();
            let quests = db
                .rows(&trigger::quests_query())?
                .iter()
                .filter_map(|row| Some((row.integer("id")? as u32, row.integer("quest")? as u32)))
                .collect();
            Ok(Held { templates, teleports, entrances, taverns, quests })
        }))
    });
}

/// How far down the ray it first meets a trigger's volume, or `None` when it
/// misses or the volume is behind the eye.
pub(crate) fn ray_meets(trigger: &Trigger, origin: Vec3, direction: Vec3) -> Option<f32> {
    let centre = Vec3::from(trigger.at);
    match trigger.is_sphere() {
        true => crate::pick::ray_meets_sphere(origin, direction, centre, trigger.radius),
        false => {
            let half = Vec3::from(trigger.extent) * 0.5;
            crate::pick::ray_enters_turned_box(origin, direction, centre, half, trigger.yaw)
        }
    }
}

/// A box trigger's eight corners in the world's axes: the four of the bottom
/// face, then the four of the top in the same order.
pub(crate) fn corners(trigger: &Trigger) -> [Vec3; 8] {
    let centre = Vec3::from(trigger.at);
    let half = Vec3::from(trigger.extent) * 0.5;
    let (s, c) = trigger.yaw.sin_cos();
    let turn = |x: f32, y: f32, z: f32| centre + Vec3::new(x * c - y * s, x * s + y * c, z);
    let mut out = [Vec3::ZERO; 8];
    for (n, (sx, sy)) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].into_iter().enumerate() {
        out[n] = turn(sx * half.x, sy * half.y, -half.z);
        out[n + 4] = turn(sx * half.x, sy * half.y, half.z);
    }
    out
}

/// A press: select and start a drag, or do what is armed.
#[allow(clippy::too_many_arguments)]
fn press(
    mut triggers: ResMut<Triggers>,
    buttons: Res<ButtonInput<MouseButton>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cursor: Res<crate::pick::Cursor>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active(&tool, &state) || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let Some(session) = session.as_mut() else { return };
    // A pick takes the press: it is the place, and the drag that follows sets
    // the facing. A pick still waiting for its map to open takes nothing.
    let travelling = triggers.travel.is_some();
    if let Some(pick) = triggers.pick.as_mut() {
        if !travelling && pick.map == session.map_id {
            pick.pressed = cursor.surface;
        }
        return;
    }
    match (triggers.armed, triggers.points.hovered) {
        (Armed::NewTrigger, _) => {
            if let Some(at) = cursor.surface {
                triggers.armed = Armed::Nothing;
                let line = new_trigger(session, &mut triggers, at);
                session.status = line;
            }
        }
        (Armed::Nothing, Some(id)) => triggers.points.grab(id, &windows),
        (Armed::Nothing, None) => {}
    }
}

/// A pick's facing: while the button is held after the press that set the
/// place, the facing follows the pointer; on release the pick is written.
fn pick_facing(
    mut triggers: ResMut<Triggers>,
    buttons: Res<ButtonInput<MouseButton>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    time: Res<Time>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active(&tool, &state) {
        return;
    }
    let Some(pressed) = triggers.pick.as_ref().and_then(|pick| pick.pressed) else {
        return;
    };
    let Some(session) = session.as_mut() else { return };
    match buttons.pressed(MouseButton::Left) {
        true => {
            let hit = crate::pick::ray(&windows, &camera)
                .and_then(|(origin, direction)| crate::pick::meets_plane(origin, direction, pressed.z));
            if let (Some(hit), Some(pick)) = (hit, triggers.pick.as_mut()) {
                if let Some(facing) = facing_toward(pressed, hit) {
                    pick.facing = facing;
                }
            }
        }
        false => {
            let Some(pick) = triggers.pick.clone() else { return };
            let line = finish_pick(session, &triggers, &pick, pressed, time.elapsed_secs_f64());
            session.status = line;
            triggers.end_pick();
        }
    }
}

/// Escape and Delete.
fn keys(
    mut triggers: ResMut<Triggers>,
    keys: Res<ButtonInput<KeyCode>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    time: Res<Time>,
    mut session: Option<ResMut<EditSession>>,
) {
    if !active(&tool, &state) || wants.wants_keyboard_input() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        if triggers.pick.is_some() && triggers.travel.is_none() {
            triggers.end_pick();
            if let Some(session) = session.as_mut() {
                session.status = "pick cancelled".to_string();
            }
        } else if triggers.armed != Armed::Nothing {
            triggers.armed = Armed::Nothing;
        } else if triggers.points.confirm_remove.is_some() {
            triggers.points.confirm_remove = None;
        } else {
            triggers.points.selected = None;
        }
    }
    if !keys.just_pressed(KeyCode::Delete) {
        return;
    }
    if let Some(session) = session.as_mut() {
        points::delete_selected(&mut *triggers, session, time.elapsed_secs_f64());
    }
}

/// The colour a trigger is drawn in: by what it does on the server.
pub(crate) fn colour_of(does: Does) -> Color {
    match does {
        Does { teleport: true, .. } => Color::srgb(0.75, 0.4, 1.0),
        Does { inn: true, .. } => Color::srgb(0.35, 0.9, 0.45),
        Does { quest: true, .. } => Color::srgb(1.0, 0.85, 0.25),
        Does { entrance: true, .. } => Color::srgb(1.0, 0.45, 0.35),
        Does { script: true, .. } => Color::srgb(0.95, 0.6, 0.85),
        _ => Color::srgb(0.35, 0.75, 0.95),
    }
}

/// Draw the triggers near the camera, and what a click would make.
///
/// A trigger is a translucent volume with solid edges, so the ground hides the
/// part of it that is under the ground and a trigger buried in a hillside
/// reads as buried. The selected and hovered one is [`Look::Ghosted`]: its
/// hidden part is drawn faint as well. The rest are [`Look::Solid`].
#[allow(clippy::too_many_arguments)]
fn draw(
    mut marks: ResMut<Marks>,
    triggers: Res<Triggers>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    cursor: Res<crate::pick::Cursor>,
    session: Option<Res<EditSession>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !active(&tool, &state) {
        return;
    }
    let (Some(eye), Some(session)) = (points::eye(&camera), session) else {
        return;
    };
    let bevy = |p: Vec3| axes::to_bevy(p.to_array());
    for trigger in &triggers.points.list {
        let at = Vec3::from(trigger.at);
        if !triggers.points.near(trigger, eye, MARKER_RANGE) {
            continue;
        }
        let highlight = triggers.points.highlight(trigger.id);
        let colour = highlight.unwrap_or_else(|| colour_of(triggers.does(&session.rows.edits, trigger.id)));
        let marker = marks.visible_radius(bevy(at), super::flightpaths::MARKER_RADIUS * 0.5);
        marks.sphere(bevy(at), marker, colour, Look::Ghosted);
        let (look, fill) = match highlight.is_some() {
            true => (Look::Ghosted, VOLUME_AIMED),
            false => (Look::Solid, VOLUME),
        };
        if trigger.is_sphere() {
            marks.sphere(bevy(at), trigger.radius, colour.with_alpha(fill), look);
            // The equator, so the sphere's size reads where the fill is faint.
            marks.wide_ring(bevy(at), Vec3::Y, trigger.radius, 1.2, colour, look);
            continue;
        }
        let extent = Vec3::from(trigger.extent);
        // The box's own axes in Bevy's: WoW x is Bevy -z, WoW y is Bevy -x,
        // and the yaw about WoW's up is a turn about Bevy's y. See `axes`.
        let pose = Transform {
            translation: bevy(at),
            rotation: Quat::from_rotation_y(trigger.yaw),
            scale: Vec3::new(extent.y, extent.z, extent.x),
        };
        marks.cuboid(pose, colour.with_alpha(fill), look);
        let c = corners(trigger);
        for (a, b) in BOX_EDGES {
            marks.line(bevy(c[a]), bevy(c[b]), colour, look);
        }
    }
    // The selected trigger's teleport target and battleground exit, when they
    // are on this map, with the way a character faces there.
    if let Some(id) = triggers.points.selected {
        let teleport = triggers
            .teleport(&session.rows.edits, id)
            .filter(|(teleport, _, life)| *life != Life::Delete && teleport.target_map == triggers.points.map)
            .map(|(teleport, _, _)| (Vec3::from(teleport.target), teleport.orientation, PickFor::Teleport));
        let exit = triggers
            .entrance(&session.rows.edits, id)
            .filter(|(entrance, _, life)| *life != Life::Delete && entrance.exit_map == triggers.points.map)
            .map(|(entrance, _, _)| (Vec3::from(entrance.exit), entrance.orientation, PickFor::BgExit));
        let from = triggers.trigger(id).map(|trigger| Vec3::from(trigger.at));
        for (at, facing, what) in [teleport, exit].into_iter().flatten() {
            let colour = match what {
                PickFor::Teleport => colour_of(Does { teleport: true, ..Does::default() }),
                PickFor::BgExit => colour_of(Does { entrance: true, ..Does::default() }),
            };
            place_mark(&mut marks, at, facing, colour);
            if let Some(from) = from {
                marks.line(bevy(from), bevy(at), colour.with_alpha(0.5), Look::Ghosted);
            }
        }
    }
    let Some(pointer) = cursor.surface else { return };
    if let Some(pick) = triggers.pick.as_ref().filter(|pick| pick.map == triggers.points.map) {
        let at = pick.pressed.unwrap_or(pointer);
        place_mark(&mut marks, at, pick.facing, Color::WHITE);
        return;
    }
    if triggers.armed == Armed::NewTrigger {
        let radius = triggers.radius.max(0.5);
        let at = pointer + Vec3::Z * radius * 0.5;
        let colour = colour_of(Does::default()).with_alpha(VOLUME_AIMED);
        marks.sphere(bevy(at), radius, colour, Look::Ghosted);
    }
}

/// How opaque a trigger's volume is drawn, and the selected or hovered one's.
/// Faint, so the ground and the models inside it stay readable.
const VOLUME: f32 = 0.14;
const VOLUME_AIMED: f32 = 0.24;

/// A place a character is sent to: a marker standing on it and an arrow the
/// way the character faces.
fn place_mark(marks: &mut Marks, at: Vec3, facing: f32, colour: Color) {
    let bevy = |p: Vec3| axes::to_bevy(p.to_array());
    let radius = marks.visible_radius(bevy(at), super::flightpaths::MARKER_RADIUS);
    marks.sphere(bevy(at), radius, colour, Look::Ghosted);
    marks.line(bevy(at), bevy(at + Vec3::Z * radius * 4.0), colour, Look::Ghosted);
    let ahead = at + Vec3::new(facing.cos(), facing.sin(), 0.0) * radius * 4.0;
    let thickness = marks.line_radius(bevy(at)) * 1.6;
    marks.arrow(bevy(at), bevy(ahead), thickness, colour, Look::Ghosted);
}

/// The twelve edges of a box, as pairs of [`corners`].
const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (1, 2),
    (2, 3),
    (3, 0),
    (4, 5),
    (5, 6),
    (6, 7),
    (7, 4),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(at: [f32; 3], radius: f32) -> Trigger {
        Trigger { id: 1, record: 0, map: 0, at, radius, extent: [0.0; 3], yaw: 0.0 }
    }

    fn cuboid(at: [f32; 3], extent: [f32; 3], yaw: f32) -> Trigger {
        Trigger { id: 2, record: 0, map: 0, at, radius: 0.0, extent, yaw }
    }

    #[test]
    fn a_ray_meets_a_sphere_at_its_near_side() {
        let trigger = sphere([10.0, 0.0, 0.0], 2.0);
        let hit = ray_meets(&trigger, Vec3::ZERO, Vec3::X).unwrap();
        assert!((hit - 8.0).abs() < 1e-4);
        assert!(ray_meets(&trigger, Vec3::new(0.0, 3.0, 0.0), Vec3::X).is_none());
        assert!(ray_meets(&trigger, Vec3::ZERO, -Vec3::X).is_none());
    }

    #[test]
    fn a_turned_box_is_met_in_its_own_frame() {
        // Long along its own x, turned a quarter: long along the world's y.
        let trigger = cuboid([0.0, 0.0, 0.0], [20.0, 2.0, 2.0], std::f32::consts::FRAC_PI_2);
        let from_side = Vec3::new(-10.0, 8.0, 0.0);
        assert!(ray_meets(&trigger, from_side, Vec3::X).is_some());
        let unturned = cuboid([0.0, 0.0, 0.0], [20.0, 2.0, 2.0], 0.0);
        assert!(ray_meets(&unturned, from_side, Vec3::X).is_none());
    }

    #[test]
    fn a_box_has_its_corners_at_half_its_sides() {
        let trigger = cuboid([100.0, 50.0, 10.0], [4.0, 2.0, 6.0], 0.0);
        let c = corners(&trigger);
        assert_eq!(c[0], Vec3::new(98.0, 49.0, 7.0));
        assert_eq!(c[6], Vec3::new(102.0, 51.0, 13.0));
    }

    /// A pick on another map asks to go there and remembers home; ending it
    /// asks to go back and to select the trigger again. A pick on this map
    /// does neither.
    #[test]
    fn a_pick_on_another_map_goes_there_and_comes_back() {
        let install = std::env::temp_dir().join(format!("vale-trigger-pick-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        session.maps = vec![(0, "Azeroth".to_string()), (36, "DeadminesInstance".to_string())];
        let mut triggers = Triggers::default();

        triggers.start_pick(&session, PickFor::Teleport, 78, 0, [1.0, 2.0, 3.0], 0.5).unwrap();
        assert!(triggers.travel.is_none());
        assert!(triggers.pick.as_ref().unwrap().home.is_none());
        triggers.end_pick();
        assert!(triggers.pick.is_none() && triggers.travel.is_none());

        triggers.start_pick(&session, PickFor::Teleport, 78, 36, [-16.0, -383.0, 61.0], 1.5).unwrap();
        assert_eq!(
            triggers.travel,
            Some(Travel::To { name: "DeadminesInstance".to_string(), id: 36, at: Vec3::new(-16.0, -383.0, 61.0) })
        );
        let pick = triggers.pick.clone().unwrap();
        assert_eq!((pick.map, pick.facing), (36, 1.5));
        assert_eq!(pick.home.as_ref().map(|home| home.1), Some(0));
        triggers.travel = None;
        triggers.end_pick();
        assert_eq!(triggers.travel, Some(Travel::Home));
        assert_eq!(triggers.reselect, Some(78));

        assert!(triggers.start_pick(&session, PickFor::BgExit, 78, 999, [0.0; 3], 0.0).is_err());
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A created teleport keeps every column through a pick and an edit, and
    /// still does once the database holds the row too (after an apply): a
    /// value typed back to the database's is written into the creation rather
    /// than clearing the column.
    #[test]
    fn a_created_teleport_keeps_every_column() {
        let install = std::env::temp_dir().join(format!("vale-trigger-columns-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        let mut triggers = Triggers::default();
        triggers.reads.insert((), Held::default());
        let key = trigger::teleport_key(4300, 0);
        add_teleport(&mut session, &triggers, 4300, 0, [-880.5, -1219.4, 53.5], 1.0);
        set_target(&mut session, &triggers, 4300, 0, Vec3::new(-770.37, -1100.2, 19.568), 5.79, 2.0);
        set_teleport(&mut session, &triggers, 4300, "required_level", "15".to_string(), 3.0);
        assert!(missing(&session.rows.edits, trigger::TELEPORT, &key).is_empty());

        // Applied: the database now holds the same row.
        let (applied, _, _) = triggers.teleport(&session.rows.edits, 4300).unwrap();
        let mut held = triggers.held().cloned().unwrap();
        held.teleports.insert(4300, applied.clone());
        triggers.reads.insert((), held);
        let y = vale_mangos::sql::float(applied.target[1]);
        set_teleport(&mut session, &triggers, 4300, "target_position_y", y, 4.0);
        assert!(missing(&session.rows.edits, trigger::TELEPORT, &key).is_empty());
        assert_eq!(session.rows.edits.row(trigger::TELEPORT, &key).unwrap().life, Life::Insert);
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A creation missing a column is still shown, at the column's default,
    /// and `missing` names the column.
    #[test]
    fn a_creation_missing_a_column_is_shown_and_named() {
        let mut triggers = Triggers::default();
        triggers.reads.insert((), Held::default());
        let mut edits = Edits::default();
        let key = trigger::teleport_key(4300, 0);
        let mut row = crate::subjects::services::creation(&Teleport::new(4300, 0, [1.0, 2.0, 3.0]).assignments());
        row.columns.remove("target_position_y");
        edits.set_row_line(trigger::TELEPORT, &key, Some(&row.to_line()));
        let (teleport, _, life) = triggers.teleport(&edits, 4300).unwrap();
        assert_eq!((teleport.target, life), ([1.0, 0.0, 3.0], Life::Insert));
        assert_eq!(missing(&edits, trigger::TELEPORT, &key), vec!["target_position_y"]);
    }

    #[test]
    fn a_facing_is_measured_from_north_toward_west() {
        let from = Vec3::ZERO;
        assert!((facing_toward(from, Vec3::new(10.0, 0.0, 0.0)).unwrap()).abs() < 1e-5);
        let west = facing_toward(from, Vec3::new(0.0, 10.0, 3.0)).unwrap();
        assert!((west - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        let east = facing_toward(from, Vec3::new(0.0, -10.0, 0.0)).unwrap();
        assert!((east - 3.0 * std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!(facing_toward(from, Vec3::new(0.1, 0.1, 5.0)).is_none(), "too short to point");
    }

    #[test]
    fn a_template_shows_the_projects_columns_over_the_database() {
        let mut triggers = Triggers::default();
        let mut held = Held::default();
        held.templates.insert(
            1125,
            trigger::Template {
                id: 1125,
                build: 4222,
                name: "Stormwind City - Cathedral of Light Entrance".to_string(),
                cooldown: 30,
                script_id: 1125,
                ..trigger::Template::default()
            },
        );
        triggers.reads.insert((), held);
        let mut edits = Edits::default();
        let (template, in_database) = triggers.template(&edits, 1125).unwrap();
        assert!(in_database);
        assert_eq!((template.cooldown, template.build), (30, 4222));
        edits.set(trigger::TEMPLATE, &trigger::key(1125), "cooldown", Some("5".to_string()));
        assert_eq!(triggers.template(&edits, 1125).unwrap().0.cooldown, 5);
        // A trigger the database has no template row for shows the project's
        // edits alone.
        assert!(triggers.template(&edits, 9000).is_none());
        edits.set(trigger::TEMPLATE, &trigger::key(9000), "name", Some("'New'".to_string()));
        let (template, in_database) = triggers.template(&edits, 9000).unwrap();
        assert_eq!((template.name.as_str(), in_database), ("New", false));
        assert!(triggers.does(&edits, 1125).script);
    }

    #[test]
    fn a_row_shows_the_projects_edits_over_the_database() {
        let mut edits = Edits::default();
        let key = trigger::teleport_key(78, 0);
        let held = Teleport::new(78, 36, [-16.0, -383.0, 61.0]).assignments();
        let (row, life) = shown(&edits, trigger::TELEPORT, &key, Some(&held)).unwrap();
        assert_eq!(life, Life::Update);
        assert_eq!(Teleport::from_row(&row).unwrap().target_map, 36);
        edits.set(trigger::TELEPORT, &key, "target_map", Some("0".to_string()));
        let (row, _) = shown(&edits, trigger::TELEPORT, &key, Some(&held)).unwrap();
        assert_eq!(Teleport::from_row(&row).unwrap().target_map, 0);
        // A row neither side has is not shown; one the project creates is.
        let other = trigger::teleport_key(9000, 0);
        assert!(shown(&edits, trigger::TELEPORT, &other, None).is_none());
        let created = crate::subjects::services::creation(&Teleport::new(9000, 1, [1.0, 2.0, 3.0]).assignments());
        edits.set_row_line(trigger::TELEPORT, &other, Some(&created.to_line()));
        let (row, life) = shown(&edits, trigger::TELEPORT, &other, None).unwrap();
        assert_eq!(life, Life::Insert);
        assert_eq!(Teleport::from_row(&row).unwrap().target, [1.0, 2.0, 3.0]);
    }
}
