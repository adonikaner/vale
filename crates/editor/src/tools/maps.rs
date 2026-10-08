//! A new map: its `Map.dbc` row, a WDT with no tiles in the project, its
//! `map_template` row, and a first zone when one is asked for.
//!
//! ```text
//! Map.dbc                 the client's row: the id, the folder, the name, the
//!                         kind of place and the loading screen
//! World\Maps\<dir>\<dir>.wdt   the map's tile list, empty; the map window's
//!                         Create claims tiles in it
//! AreaTable.dbc           a zone on the new map, named after it, which the
//!                         map row and the server row name as its area
//! map_template            the server's row, at patch 0 (`vale_mangos::map`),
//!                         with a dungeon's parent and ghost entrance
//! ```
//!
//! The DBC rows are undo entries, as every table edit is; the server
//! row is a row of the project's store, on `crate::tools::services`' terms.
//! The WDT is a file written at once, since nothing reads it until a tile is
//! made, and an undo of the map row leaves it in the project unused.
//!
//! The same form edits an existing map (Map properties on the Map menu):
//! the rest of its `Map.dbc` row, its `map_template` row from the project
//! or the database, and whether its WDT makes it terrain or a single WMO.
//! See [`read_edited_map`] and [`save`]. `Map.dbc` has no flags column in
//! 1.12; a single-WMO map is the WDT's `MPHD` flag.
//!
//! The server needs more than its row before a character can stand on the
//! map: its extracted tiles. Server files on the map window, or a publish,
//! builds them from the project's archive, whose `Map.dbc` names the new map.
//! The server opens the map's grids at startup, so it needs a restart after
//! the row is applied.

use crate::session::{EditSession, Gesture};
use vale_assets::tables::map::{self as map_table, fields as mf};
use vale_client::assets::GameAssets;
use vale_edit::dbc::places::{self, MapSpec};
use vale_mangos::map::Template;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};

/// What the map window's New map form holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub directory: String,
    pub name: String,
    /// `Map.dbc` field 2, which is also `map_template.map_type`.
    pub instance_type: u32,
    pub max_players: u32,
    /// A `LoadingScreens` row, or `None` for the open map's.
    pub loading_screen: Option<u32>,
    /// Whether the form is showing the loading screen pictures to choose from.
    pub choosing_screen: bool,
    /// For a dungeon or raid reached through another, that dungeon; 0 for
    /// none. `map_template.parent`.
    pub parent: u32,
    /// For a dungeon or raid, where a dead character's ghost walks in from: a
    /// continent and a point on it, north and west in yards. `None` for none.
    pub ghost: Option<(u32, [f32; 2])>,
    /// Make a zone on the new map, named after it.
    pub zone: bool,
    /// Open the new map once it is made.
    pub open_it: bool,

    // What the form holds when it edits a map that exists. See
    // [`Form::editing`] and [`save`].
    /// The map this form edits, or `None` for a new map.
    pub editing: Option<u32>,
    /// Whether the edited map's rows have been read into the form.
    pub loaded: bool,
    /// `Map.dbc` field 3.
    pub pvp: bool,
    /// `Map.dbc` fields 13 and 14.
    pub min_level: u32,
    pub max_level: u32,
    /// `Map.dbc` field 19, which the server's row holds as `linked_zone`.
    pub area: u32,
    /// `Map.dbc` fields 20 and 29.
    pub descriptions: [String; 2],
    /// `map_template.reset_delay`: days between a raid's resets.
    pub reset_delay: u32,
    /// Whether the map is one WMO with no terrain tiles, rather than terrain:
    /// the WDT's `MPHD` flag. See `vale_edit::wdt::ONE_BUILDING`.
    pub single_wmo: bool,
    /// The WMO the map is, for a single-WMO map.
    pub wmo: String,
    /// …and what it was when the form was read.
    pub wmo_was: String,
    /// How many tiles the map had when the form was read. Only a map with no
    /// tiles can be made a single-WMO map.
    pub tiles: usize,
    /// The server's row, as read.
    pub server: ServerRow,
}

/// The edited map's `map_template` row, and where it came from.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ServerRow {
    /// Not looked for yet.
    #[default]
    Unread,
    /// Being read from the database.
    Reading,
    /// Found: the row the server uses, with the project's edits to it.
    /// `created` is a row this project creates.
    Found {
        template: Template,
        key: vale_mangos::row::Key,
        created: bool,
    },
    /// The database has no row for the map.
    Missing,
    /// The database could not be read.
    Unreachable(String),
}

impl Default for Form {
    fn default() -> Form {
        Form {
            directory: String::new(),
            name: String::new(),
            instance_type: 0,
            max_players: 0,
            loading_screen: None,
            choosing_screen: false,
            parent: 0,
            ghost: None,
            zone: true,
            open_it: true,
            editing: None,
            loaded: false,
            pvp: false,
            min_level: 0,
            max_level: 0,
            area: 0,
            descriptions: [String::new(), String::new()],
            reset_delay: 0,
            single_wmo: false,
            wmo: String::new(),
            wmo_was: String::new(),
            tiles: 0,
            server: ServerRow::Unread,
        }
    }
}

impl Form {
    /// A form that edits map `id`, whose folder is `directory`. Its fields
    /// are read in by [`read_edited_map`].
    pub fn editing(id: u32, directory: &str) -> Form {
        Form {
            directory: directory.to_string(),
            editing: Some(id),
            zone: false,
            open_it: false,
            ..Form::default()
        }
    }

    /// Why the form cannot make a map yet, or `None` when it can. `maps` is
    /// the session's list of maps, whose directories a new one may not repeat.
    pub fn problem(&self, maps: &[(u32, String)]) -> Option<String> {
        if let Some(id) = self.editing {
            return self.edit_problem(id);
        }
        if let Some(why) = map_table::directory_problem(&self.directory, maps.iter().map(|(_, dir)| dir.as_str())) {
            return Some(why);
        }
        if self.name.trim().is_empty() {
            return Some("the map needs a name".to_string());
        }
        if map_table::next_id(maps.iter().map(|(id, _)| *id)).is_none() {
            return Some(format!("the next map id would exceed the limit of {}", map_table::MAX_NEW_ID));
        }
        if let Some(why) = self.row_problem() {
            return Some(why);
        }
        self.template(0, 0).check().into_iter().next()
    }

    /// Why the levels or the layout cannot be written, for either mode.
    fn row_problem(&self) -> Option<String> {
        if self.min_level > 0 && self.max_level > 0 && self.min_level > self.max_level {
            return Some("the minimum level is above the maximum level".to_string());
        }
        let wmo = self.wmo.trim();
        if self.single_wmo && wmo.is_empty() {
            return Some("enter the path of the WMO the map is".to_string());
        }
        if self.single_wmo && !wmo.to_ascii_lowercase().ends_with(".wmo") {
            return Some("the WMO path must end in .wmo".to_string());
        }
        if self.single_wmo && self.wmo_was.is_empty() && self.tiles > 0 {
            return Some(format!(
                "the map has {} tiles; only a map with no tiles can be a single-WMO map",
                self.tiles
            ));
        }
        None
    }

    /// Why the edits cannot be saved yet, or `None` when they can.
    fn edit_problem(&self, id: u32) -> Option<String> {
        if !self.loaded {
            return Some("the map's rows are still being read".to_string());
        }
        if self.name.trim().is_empty() {
            return Some("the map needs a name".to_string());
        }
        if let Some(why) = self.row_problem() {
            return Some(why);
        }
        match &self.server {
            ServerRow::Found { template, .. } => self.edited_template(template).check().into_iter().next(),
            _ => self.template(id, self.area).check().into_iter().next(),
        }
    }

    /// The server's row as these fields would leave it: `found` with the
    /// columns the form shows replaced. The script name is kept.
    pub fn edited_template(&self, found: &Template) -> Template {
        let mut template = found.clone();
        template.map_type = self.instance_type;
        template.player_limit = self.max_players;
        template.linked_zone = self.area;
        template.name = self.name.trim().to_string();
        template.reset_delay = self.reset_delay;
        match self.is_instance() {
            true => {
                template.parent = self.parent;
                (template.ghost_map, template.ghost_at) = match self.ghost {
                    Some((map, at)) => (map as i32, at),
                    None => (-1, [0.0, 0.0]),
                };
            }
            false => {
                template.parent = 0;
                template.ghost_map = -1;
                template.ghost_at = [0.0, 0.0];
            }
        }
        template
    }

    /// Whether the map is a dungeon or a raid, which alone have a parent and a
    /// ghost entrance.
    pub fn is_instance(&self) -> bool {
        matches!(self.instance_type, 1 | 2)
    }

    /// The `map_template` row the form makes for map `id`, with `zone` as its
    /// area.
    pub fn template(&self, id: u32, zone: u32) -> Template {
        let mut template = Template::new(id, self.instance_type, &self.name);
        template.linked_zone = zone;
        template.player_limit = self.max_players;
        template.reset_delay = self.reset_delay;
        if self.is_instance() {
            template.parent = self.parent;
            if let Some((map, at)) = self.ghost {
                template.ghost_map = map as i32;
                template.ghost_at = at;
            }
        }
        template
    }
}

/// Make the map the form describes. Answers the status line.
pub fn make(
    session: &mut EditSession,
    assets: &GameAssets,
    form: &Form,
    camera: &mut crate::camera::EditorCamera,
    now: f64,
) -> Result<String, String> {
    if let Some(why) = form.problem(&session.maps) {
        return Err(why);
    }
    for table in [places::MAPS, super::tables::area::TABLE] {
        if !session.open_table(assets, table) {
            return Err(format!("{table}.dbc could not be opened"));
        }
    }
    let maps = session.table(places::MAPS).ok_or("Map.dbc is not open")?;
    let ids: Vec<u32> = places::map_directories(maps).into_iter().map(|(id, _)| id).collect();
    let id = map_table::next_id(ids).ok_or_else(|| format!("the next map id would exceed the limit of {}", map_table::MAX_NEW_ID))?;
    let loading_screen = form.loading_screen.unwrap_or_else(|| {
        maps.row_of(session.map_id)
            .and_then(|record| maps.u32_at(record, mf::LOADING_SCREEN))
            .unwrap_or(0)
    });

    // The zone first, since the map row names it.
    let zone = match form.zone {
        true => {
            let record = super::tables::add_zone(session, id).ok_or("AreaTable.dbc did not add a row")?;
            let area = session.table(super::tables::area::TABLE).ok_or("AreaTable.dbc is not open")?;
            let zone = area.u32_at(record, 0).unwrap_or(0);
            super::tables::set_text(
                session,
                super::tables::area::TABLE,
                record,
                super::tables::area::NAME,
                &form.name,
                "Name zone",
                now,
            );
            zone
        }
        // A zone that is there already, or none.
        false => form.area,
    };

    // A single-WMO map's building is read before anything is written, so a
    // path that is not a WMO makes nothing.
    let building = match form.single_wmo {
        true => {
            let path = form.wmo.trim();
            Some((path, super::tiles::wmo_bounds(assets, path)?))
        }
        false => None,
    };

    let spec = MapSpec {
        directory: form.directory.clone(),
        name: form.name.clone(),
        instance_type: form.instance_type,
        max_players: form.max_players,
        area: zone,
        loading_screen,
        pvp: form.pvp,
        min_level: form.min_level,
        max_level: form.max_level,
        descriptions: form.descriptions.clone(),
    };
    let done = places::new_map(&mut session.tables, &spec).map_err(|e| e.to_string())?;
    super::flightpaths::record(session, "Add map", done);

    let mut wdt = vale_edit::wdt::WdtFile::blank();
    if let Some(building) = building {
        wdt.set_building(Some(building));
    }
    session
        .project
        .write(&vale_edit::wdt::wdt_path(&form.directory), &wdt.write())
        .map_err(|e| format!("map {id} made, but its WDT was not written: {e}"))?;

    let template = form.template(id, zone);
    let key = template.key();
    let label = format!("{} {}", vale_mangos::map::TEMPLATE, key.text());
    let row = super::services::creation(&template.assignments());
    session.set_server_row(
        vale_mangos::map::TEMPLATE,
        &key,
        Some(&row),
        Some(Gesture { label: "Add map", subject: &label, now }),
    );

    refresh_maps(session);
    if form.open_it {
        session.switch_map(form.directory.clone(), id, camera);
        camera.go_to(Vec2::ZERO);
    }
    let zone_line = match zone {
        0 => String::new(),
        zone => format!(", zone {zone}"),
    };
    info!("new map {id} {}{zone_line}", form.directory);
    Ok(match form.single_wmo {
        true => format!("map {id} {} made{zone_line}, a single-WMO map", form.directory),
        false => format!(
            "map {id} {} made{zone_line}; use Create in the map window to make its first tiles",
            form.directory
        ),
    })
}

/// The database read of an edited map's server row: which map, and the
/// row the server uses with its key, or `None` for no row.
type RowRead = (u32, Task<Result<Option<(Template, vale_mangos::row::Key)>, String>>);

/// Fill a form that edits a map from what the map is now: its `Map.dbc` row,
/// its WDT, and its `map_template` row from the project or the database.
pub fn read_edited_map(
    mut tiles: ResMut<super::tiles::Tiles>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    settings: Res<crate::server::settings::ServerSettings>,
    mut reading: Local<Option<RowRead>>,
) {
    let Some(mut session) = session else { return };
    let editing = tiles.new_map.as_ref().and_then(|form| form.editing);

    // A database read finishes into the form it was started for, and is
    // dropped if that form has gone.
    if let Some((id, task)) = reading.as_mut() {
        let id = *id;
        if let Some(done) = block_on(future::poll_once(task)) {
            *reading = None;
            if let Some(form) = tiles.new_map.as_mut().filter(|form| form.editing == Some(id)) {
                form.server = match done {
                    Ok(Some((template, key))) => {
                        let template = with_project_edits(&session, template, &key);
                        fill_server_fields(form, &template);
                        ServerRow::Found {
                            template,
                            key,
                            created: false,
                        }
                    }
                    Ok(None) => ServerRow::Missing,
                    Err(why) => ServerRow::Unreachable(why),
                };
            }
        }
    }

    let Some(id) = editing else { return };
    let Some(form) = tiles.new_map.as_mut() else { return };
    if !form.loaded {
        if !session.open_table(&assets, places::MAPS) {
            form.server = ServerRow::Unreachable("Map.dbc could not be opened".to_string());
            return;
        }
        let Some(maps) = session.table(places::MAPS) else { return };
        let Some(record) = maps.row_of(id) else {
            session.status = format!("map {id} has no Map.dbc row");
            tiles.new_map = None;
            return;
        };
        let number = |field: usize| maps.u32_at(record, field).unwrap_or(0);
        let text = |field: usize| maps.string_at(record, field).unwrap_or_default();
        form.name = text(mf::NAME);
        form.instance_type = number(mf::INSTANCE_TYPE);
        form.pvp = number(mf::PVP) != 0;
        form.min_level = number(mf::MIN_LEVEL);
        form.max_level = number(mf::MAX_LEVEL);
        form.max_players = number(mf::MAX_PLAYERS);
        form.area = number(mf::AREA);
        form.descriptions = [text(mf::DESCRIPTION_0), text(mf::DESCRIPTION_1)];
        form.loading_screen = Some(number(mf::LOADING_SCREEN));
        if let Ok(wdt) = super::tiles::load_wdt_of(&session, &assets, &form.directory) {
            form.wmo = wdt.building().unwrap_or_default();
            form.wmo_was = form.wmo.clone();
            form.single_wmo = !form.wmo.is_empty();
            form.tiles = wdt.tile_count();
        }
        form.loaded = true;
    }

    if form.server == ServerRow::Unread {
        // A row this project creates is the project's, whatever the database
        // holds.
        let created_key = vale_mangos::map::key(id, 0);
        let created = session
            .server_edits
            .row(vale_mangos::map::TEMPLATE, &created_key)
            .filter(|row| row.life == vale_mangos::row::Life::Insert)
            .cloned();
        if let Some(row) = created {
            let mut template = Template::new(id, form.instance_type, &form.name);
            for (column, literal) in &row.columns {
                set_from_literal(&mut template, column, literal);
            }
            fill_server_fields(form, &template);
            form.server = ServerRow::Found {
                template,
                key: created_key,
                created: true,
            };
            return;
        }
        let Some((at, _)) = settings.resolve() else {
            form.server = ServerRow::Unreachable("no server database is set".to_string());
            return;
        };
        let patch = super::creatures::server_patch(&settings);
        form.server = ServerRow::Reading;
        *reading = Some((
            id,
            crate::server::queue::read(async move {
                use vale_mangos::schema::RowValue;
                let mut db = vale_mangos::conn::Db::open(&at)?;
                let rows = db.rows(&vale_mangos::map::winning_row_query(id, patch))?;
                let Some(row) = rows.first() else {
                    return Ok(None);
                };
                let int = |column: &str| row.integer(column).unwrap_or(0);
                let float = |column: &str| row.number(column).unwrap_or(0.0) as f32;
                let template = Template {
                    entry: id,
                    parent: int("parent") as u32,
                    map_type: int("map_type") as u32,
                    linked_zone: int("linked_zone") as u32,
                    player_limit: int("player_limit") as u32,
                    reset_delay: int("reset_delay") as u32,
                    ghost_map: int("ghost_entrance_map") as i32,
                    ghost_at: [float("ghost_entrance_x"), float("ghost_entrance_y")],
                    name: row.text("map_name").unwrap_or_default().to_string(),
                };
                Ok(Some((template, vale_mangos::map::key(id, int("patch") as u32))))
            }),
        ));
    }
}

/// The server row's own columns, which `Map.dbc` does not hold, into the
/// form.
fn fill_server_fields(form: &mut Form, template: &Template) {
    form.parent = template.parent;
    form.reset_delay = template.reset_delay;
    form.ghost = (template.ghost_map >= 0).then_some((template.ghost_map as u32, template.ghost_at));
}

/// `template` with the project's edits to its columns written over it.
fn with_project_edits(session: &EditSession, mut template: Template, key: &vale_mangos::row::Key) -> Template {
    for column in vale_mangos::map::COLUMNS.iter().filter(|c| c.editable()) {
        if let Some(literal) = session.server_edits.get(vale_mangos::map::TEMPLATE, key, column.name) {
            set_from_literal(&mut template, column.name, literal);
        }
    }
    template
}

/// Set one column of a row from its SQL literal. A literal that does not
/// parse leaves the column as it was.
fn set_from_literal(template: &mut Template, column: &str, literal: &str) {
    let int = || literal.trim().parse::<i64>().ok();
    let float = || literal.trim().parse::<f32>().ok();
    match column {
        "parent" => template.parent = int().map_or(template.parent, |v| v as u32),
        "map_type" => template.map_type = int().map_or(template.map_type, |v| v as u32),
        "linked_zone" => template.linked_zone = int().map_or(template.linked_zone, |v| v as u32),
        "player_limit" => template.player_limit = int().map_or(template.player_limit, |v| v as u32),
        "reset_delay" => template.reset_delay = int().map_or(template.reset_delay, |v| v as u32),
        "ghost_entrance_map" => template.ghost_map = int().map_or(template.ghost_map, |v| v as i32),
        "ghost_entrance_x" => template.ghost_at[0] = float().unwrap_or(template.ghost_at[0]),
        "ghost_entrance_y" => template.ghost_at[1] = float().unwrap_or(template.ghost_at[1]),
        "map_name" => template.name = unquote(literal),
        _ => {}
    }
}

/// The text a SQL string literal holds: `vale_mangos::sql::text` undone.
fn unquote(literal: &str) -> String {
    let inner = literal.trim();
    let inner = inner
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .unwrap_or(inner);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// **Write the edited map's fields**: its `Map.dbc` row and its server row
/// as one undo entry, and its WDT's layout when that changed. Answers the
/// status line.
pub fn save(
    session: &mut EditSession,
    assets: &GameAssets,
    form: &Form,
    now: f64,
) -> Result<String, String> {
    let id = form.editing.ok_or("the form makes a new map")?;
    if let Some(why) = form.problem(&session.maps) {
        return Err(why);
    }
    if !session.open_table(assets, places::MAPS) {
        return Err("Map.dbc could not be opened".to_string());
    }
    let record = session
        .table(places::MAPS)
        .and_then(|maps| maps.row_of(id))
        .ok_or_else(|| format!("map {id} has no Map.dbc row"))?;
    let label = "Edit map";
    let subject = format!("map {id} properties");
    let mut said = Vec::new();

    // The numbers, then the strings, on one gesture.
    let current_screen = session
        .table(places::MAPS)
        .and_then(|maps| maps.u32_at(record, mf::LOADING_SCREEN))
        .unwrap_or(0);
    super::tables::set_fields(
        session,
        places::MAPS,
        record,
        &[
            (mf::INSTANCE_TYPE, form.instance_type),
            (mf::PVP, u32::from(form.pvp)),
            (mf::MIN_LEVEL, form.min_level),
            (mf::MAX_LEVEL, form.max_level),
            (mf::MAX_PLAYERS, form.max_players),
            (mf::AREA, form.area),
            (mf::LOADING_SCREEN, form.loading_screen.unwrap_or(current_screen)),
        ],
        label,
        &subject,
        now,
    );
    for (field, text) in [
        (mf::NAME, form.name.trim()),
        (mf::DESCRIPTION_0, form.descriptions[0].as_str()),
        (mf::DESCRIPTION_1, form.descriptions[1].as_str()),
    ] {
        let Some(table) = session.tables.get_mut(places::MAPS) else { break };
        let Some(edit) = vale_edit::dbc::cell::set_text(table, record, field, text) else {
            continue;
        };
        if !edit.moves() {
            continue;
        }
        session.history.begin_gesture(label, subject.clone(), now);
        session.history.record_cell(places::MAPS, edit);
        session.history.end();
        session.table_edited(places::MAPS);
    }
    said.push(format!("map {id} saved"));

    // The server's row, column by column where the form changes it.
    match &form.server {
        ServerRow::Found { template, key, .. } => {
            let edited = form.edited_template(template);
            let mut changed = 0;
            for (before, after) in template.assignments().into_iter().zip(edited.assignments()) {
                // The script is not on the form, and a new map's row is
                // written with an empty one.
                if after.column == "script_name" || before.value == after.value {
                    continue;
                }
                session.set_server_edit(
                    vale_mangos::map::TEMPLATE,
                    key,
                    after.column,
                    Some(after.value),
                    Some(Gesture { label, subject: &subject, now }),
                );
                changed += 1;
            }
            if changed > 0 {
                said.push(format!(
                    "{changed} map_template column{} changed; apply maps on the Server panel",
                    if changed == 1 { "" } else { "s" }
                ));
            }
        }
        ServerRow::Missing => said.push("the database has no map_template row for it; the server row was not changed".to_string()),
        ServerRow::Unreachable(why) => said.push(format!("the server row was not changed: {why}")),
        ServerRow::Unread | ServerRow::Reading => {
            said.push("the server row was still being read and was not changed".to_string())
        }
    }

    // The layout, in the WDT. Not on the undo history: it is a file write,
    // as every change to a WDT is.
    let wmo = match form.single_wmo {
        true => form.wmo.trim(),
        false => "",
    };
    if wmo != form.wmo_was {
        if session.map_id != id {
            said.push("the layout was not changed: open the map to change it".to_string());
        } else {
            let building = (!wmo.is_empty()).then_some(wmo);
            said.push(super::tiles::set_layout(session, assets, building)?);
        }
    }
    Ok(said.join("; "))
}

/// Make [`EditSession::maps`] the open `Map.dbc`'s list, in
/// `session::order_maps`'s order. Nothing when the table is not open.
pub fn refresh_maps(session: &mut EditSession) {
    let Some(table) = session.table(places::MAPS) else {
        return;
    };
    let mut maps = places::map_directories(table);
    crate::session::order_maps(&mut maps);
    if maps != session.maps {
        session.maps = maps;
    }
}

/// Keep [`EditSession::maps`] in step with the open `Map.dbc` whenever a
/// table changes, so an undone or removed map leaves the list and a map menu
/// never offers two maps under one id.
pub fn follow_map_table(session: Option<ResMut<EditSession>>, mut seen: Local<Option<u64>>) {
    let Some(mut session) = session else { return };
    if *seen == Some(session.table_revision) {
        return;
    }
    *seen = Some(session.table_revision);
    refresh_maps(&mut session);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps() -> Vec<(u32, String)> {
        vec![(0, "Azeroth".to_string()), (1, "Kalimdor".to_string())]
    }

    #[test]
    fn a_form_needs_a_free_directory_and_a_name() {
        let mut form = Form::default();
        assert!(form.problem(&maps()).is_some());
        form.directory = "kalimdor".to_string();
        form.name = "Somewhere".to_string();
        assert!(form.problem(&maps()).is_some(), "directories compare without case");
        form.directory = "Somewhere".to_string();
        assert_eq!(form.problem(&maps()), None);
        form.name = "  ".to_string();
        assert!(form.problem(&maps()).is_some());
    }

    /// A dungeon's row carries the form's parent and ghost entrance; a world
    /// map's does not, and a ghost entrance off the grid stops the form.
    #[test]
    fn a_dungeon_takes_its_parent_and_ghost_entrance() {
        let mut form = Form {
            directory: "Somewhere".to_string(),
            name: "Somewhere".to_string(),
            parent: 33,
            ghost: Some((0, [-11207.8, 1681.15])),
            ..Form::default()
        };
        let world = form.template(534, 0);
        assert_eq!((world.parent, world.ghost_map), (0, -1));
        form.instance_type = 1;
        let dungeon = form.template(534, 0);
        assert_eq!((dungeon.parent, dungeon.ghost_map, dungeon.ghost_at), (33, 0, [-11207.8, 1681.15]));
        assert_eq!(form.problem(&maps()), None);
        form.ghost = Some((0, [40000.0, 0.0]));
        assert!(form.problem(&maps()).is_some());
    }

    #[test]
    fn a_text_literal_reads_back_as_its_text() {
        for text in ["Molten Core", "Onyxia's Lair", "a\\b\tc"] {
            assert_eq!(unquote(&vale_mangos::sql::text(text)), text);
        }
    }

    #[test]
    fn a_literal_sets_its_column() {
        let mut template = Template::new(409, 2, "Molten Core");
        set_from_literal(&mut template, "reset_delay", "7");
        set_from_literal(&mut template, "ghost_entrance_map", "0");
        set_from_literal(&mut template, "ghost_entrance_x", "-7510.56");
        set_from_literal(&mut template, "map_name", r"'Onyxia\'s Lair'");
        set_from_literal(&mut template, "parent", "not a number");
        assert_eq!(template.reset_delay, 7);
        assert_eq!(template.ghost_map, 0);
        assert!((template.ghost_at[0] + 7510.56).abs() < 1e-2);
        assert_eq!(template.name, "Onyxia's Lair");
        assert_eq!(template.parent, 0, "a literal that does not parse changes nothing");
    }

    /// The form's fields replace the server row's own, a world map loses a
    /// dungeon's columns, and an unchanged form changes no column.
    #[test]
    fn an_edited_row_takes_the_forms_fields() {
        let mut found = Template::new(409, 2, "Molten Core");
        found.ghost_map = 0;
        found.ghost_at = [-7510.0, -1036.0];
        found.reset_delay = 7;
        let mut form = Form {
            name: "Molten Core".to_string(),
            instance_type: 2,
            ghost: Some((0, [-7510.0, -1036.0])),
            reset_delay: 7,
            ..Form::editing(409, "MoltenCore")
        };
        assert_eq!(form.edited_template(&found), found);
        form.instance_type = 0;
        let world = form.edited_template(&found);
        assert_eq!((world.map_type, world.ghost_map, world.parent), (0, -1, 0));
    }

    #[test]
    fn only_a_map_with_no_tiles_becomes_a_single_wmo() {
        let mut form = Form {
            name: "Somewhere".to_string(),
            loaded: true,
            tiles: 3,
            single_wmo: true,
            ..Form::editing(600, "Somewhere")
        };
        assert!(form.problem(&maps()).is_some(), "no path");
        form.wmo = r"World\wmo\Dungeon\a.wmo".to_string();
        assert!(form.problem(&maps()).is_some(), "the map has tiles");
        form.tiles = 0;
        assert_eq!(form.problem(&maps()), None);
        form.wmo = "a.m2".to_string();
        assert!(form.problem(&maps()).is_some(), "not a WMO");
    }
}
