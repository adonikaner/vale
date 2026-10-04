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
//! map_template            the server's row, at patch 0 (`vale_mangos::map`)
//! ```
//!
//! The client table rows are undo entries, as every table edit is; the server
//! row is a row of the project's store, on `crate::tools::services`' terms.
//! The WDT is a file written at once, since nothing reads it until a tile is
//! made, and an undo of the map row leaves it in the project unused.
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
    /// Make a zone on the new map, named after it.
    pub zone: bool,
    /// Open the new map once it is made.
    pub open_it: bool,
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
            zone: true,
            open_it: true,
        }
    }
}

impl Form {
    /// Why the form cannot make a map yet, or `None` when it can. `maps` is
    /// the session's list of maps, whose directories a new one may not repeat.
    pub fn problem(&self, maps: &[(u32, String)]) -> Option<String> {
        if let Some(why) = map_table::directory_problem(&self.directory, maps.iter().map(|(_, dir)| dir.as_str())) {
            return Some(why);
        }
        if self.name.trim().is_empty() {
            return Some("the map needs a name".to_string());
        }
        if map_table::next_id(maps.iter().map(|(id, _)| *id)).is_none() {
            return Some(format!("map ids stop at {}", map_table::MAX_NEW_ID));
        }
        None
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
    let id = map_table::next_id(ids).ok_or_else(|| format!("map ids stop at {}", map_table::MAX_NEW_ID))?;
    let loading_screen = form.loading_screen.unwrap_or_else(|| {
        maps.row_of(session.map_id)
            .and_then(|record| maps.u32_at(record, mf::LOADING_SCREEN))
            .unwrap_or(0)
    });

    // The zone first, since the map row names it.
    let zone = match form.zone {
        true => {
            let record = super::tables::add_zone(session, id).ok_or("AreaTable.dbc took no new row")?;
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
        false => 0,
    };

    let spec = MapSpec {
        directory: form.directory.clone(),
        name: form.name.clone(),
        instance_type: form.instance_type,
        max_players: form.max_players,
        area: zone,
        loading_screen,
    };
    let done = places::new_map(&mut session.tables, &spec).map_err(|e| e.to_string())?;
    super::flightpaths::record(session, "Add map", done);

    let wdt = vale_edit::wdt::WdtFile::blank();
    session
        .project
        .write(&vale_edit::wdt::wdt_path(&form.directory), &wdt.write())
        .map_err(|e| format!("map {id} made, but its WDT was not written: {e}"))?;

    let mut template = Template::new(id, form.instance_type, &form.name);
    template.linked_zone = zone;
    template.player_limit = form.max_players;
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
    Ok(format!(
        "map {id} {} made{zone_line}; Create in the map window makes its first tiles",
        form.directory
    ))
}

/// Make [`EditSession::maps`] the open `Map.dbc`'s list, sorted by name as
/// `session::map_directories` sorts it. Nothing when the table is not open.
pub fn refresh_maps(session: &mut EditSession) {
    let Some(table) = session.table(places::MAPS) else {
        return;
    };
    let mut maps = places::map_directories(table);
    maps.sort_by(|a, b| a.1.to_ascii_lowercase().cmp(&b.1.to_ascii_lowercase()));
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
}
