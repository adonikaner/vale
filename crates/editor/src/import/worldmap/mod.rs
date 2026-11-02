//! The World Map part of the Zones workspace: the pictures the 1.12.1
//! client's world map shows, and the rows that place them. What a piece, an
//! overlay's file, a zone grid and a highlight sheet are is
//! `vale_edit::worldmap`.
//!
//! ```text
//! picture.rs   the chosen map's picture: its twelve sectors, and a new
//!              picture imported or drawn from the minimap into some or all of
//!              them, shown before it is written as the pieces
//! overlays.rs  the explored areas' art: a zone's WorldMapOverlay rows, each
//!              drawn on its picture, moved, given its areas and new art,
//!              added and removed
//! rows.rs      the rows behind a map: a new zone map, continent or single
//!              map, its rectangle and directory, a continent's place on the
//!              whole world's picture, and removing a map
//! grid.rs      what is read from a map's saved tiles: the area of every
//!              chunk, and from it a continent's zone grid, the highlight
//!              sheets and a rectangle fitted to what a map covers
//! ```
//!
//! ## The kinds of map
//!
//! The 1.12.1 client's world map has four kinds of picture, and the list
//! shows them in this order:
//!
//! ```text
//! World       the whole world, directory World, no WorldMapArea row
//! continent   a WorldMapArea row with area 0, and a WorldMapContinent row for
//!             its map that places it on the whole world's picture; one entry
//!             of the continent drop-down
//! zone        a row with an area, on a continent's map; one of its zones
//! single map  a row with an area, on a map with no continent row, as the
//!             three battlegrounds are: on that map the world map opens on it
//! ```
//!
//! On a continent's map the world map opens on the zone the character is in,
//! or on the continent when the zone has no row. On a map with no continent
//! row it opens on the first single map of that map, and on any other map on
//! the whole world.
//!
//! ## A new picture is shown before it is written
//!
//! See [`picture`]. Pictures and pieces are files written with
//! [`EditSession::save_bytes`], undone by Remove from project and not by
//! Ctrl+Z, as an imported texture is. Rows are table edits and are undone
//! with Ctrl+Z.

pub(crate) mod grid;
pub(crate) mod overlays;
pub(crate) mod picture;
pub(crate) mod rows;

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use crate::session::EditSession;
use crate::tables::zones::area;
use vale_assets::tables::worldmap::{area_fields as af, MapArea};
use vale_client::assets::GameAssets;
use vale_edit::dbc::DbcFile;
use vale_edit::worldmap as pieces;

/// The three tables the part reads and writes.
pub(crate) const AREAS: &str = "WorldMapArea";
pub(crate) const CONTINENTS: &str = "WorldMapContinent";
pub(crate) const OVERLAYS: &str = "WorldMapOverlay";

/// Which map: the world, or a `WorldMapArea` row by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Which {
    World,
    Row(u32),
}

/// What sort of map an entry is. See the module comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    World,
    Continent,
    Zone,
    Single,
}

impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::World => "The whole world",
            Kind::Continent => "Continent",
            Kind::Zone => "Zone",
            Kind::Single => "Single map",
        }
    }
}

/// One map in the list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Entry {
    pub(crate) which: Which,
    pub(crate) kind: Kind,
    /// The directory its pieces are in, and the start of their names.
    pub(crate) directory: String,
    /// What the list calls it: `AreaTable`'s name for a zone or a single
    /// map, the directory for a continent and for the world.
    pub(crate) name: String,
    /// Its `WorldMapArea` row. `None` for the world.
    pub(crate) area: Option<MapArea>,
    /// For a zone, its continent's row id.
    pub(crate) continent: Option<u32>,
}

/// What the panel asked for, carried out by [`answer`] after it is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ask {
    /// Choose a picture file and fit it to the chosen sectors, or to the whole
    /// map when none is chosen.
    Import,
    /// Draw the chosen sectors, or the whole map, from the minimap.
    Draw,
    /// Write the new picture's changed pieces.
    Write,
    /// Remove the project's pieces of the chosen sectors, or of the whole map.
    Remove,
    /// Choose a picture as the new art of an overlay, by row id.
    OverlayArt(u32),
    /// Choose a picture as the art of a new overlay.
    NewOverlay,
    /// Write the new overlay art.
    WriteOverlay,
    /// Make the map in the New map form.
    Create,
    /// Read the chosen map's tiles, then do what is in [`WorldMaps::after_scan`].
    Scan,
}

/// What to do once the chosen map's tiles are read. See [`grid`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AfterScan {
    /// Set the chosen row's rectangle round what it covers.
    FitRect,
    /// Write the chosen continent's zone grid.
    WriteGrid,
    /// Write the chosen zone's or continent's highlight sheet.
    WriteHighlight,
}

/// The World Map part's state.
#[derive(Resource, Default)]
pub(crate) struct WorldMaps {
    pub(crate) chosen: Option<Which>,
    /// What the search box holds.
    pub(crate) query: String,
    pub(crate) picture: picture::Picture,
    pub(crate) overlays: overlays::Overlays,
    /// Whether a zone's or a single map's picture shows its explored areas
    /// rather than its sectors.
    pub(crate) explored: bool,
    /// Whether a continent's or the whole world's picture shows its zones or
    /// continents to choose and move, rather than its sectors.
    pub(crate) placing: bool,
    /// The zone (its `WorldMapArea` row id) or the continent (its map id)
    /// chosen on that picture.
    pub(crate) placed: Option<u32>,
    /// The open New map form.
    pub(crate) create: Option<rows::NewMap>,
    pub(crate) scans: grid::Scans,
    pub(crate) after_scan: Option<AfterScan>,
    pub(crate) ask: Option<Ask>,
    /// The folder the last picture was chosen from, where the file dialog
    /// opens next.
    pub(crate) picture_dir: Option<PathBuf>,
    /// What the last action did, for the panel.
    pub(crate) said: Option<String>,
    /// The list and the [`EditSession::table_revision`] it was built at.
    listed: Option<(u64, Vec<Entry>)>,
    /// The directories the project holds pieces of, lower case, and the
    /// [`EditSession::files_revision`] they were listed at.
    owned: Option<(u64, Vec<String>)>,
}

impl WorldMaps {
    /// The list of maps, built again when a table changes. Opens the tables
    /// it reads.
    pub(crate) fn entries(&mut self, session: &mut EditSession, assets: &GameAssets) -> &[Entry] {
        let current = self.listed.as_ref().is_some_and(|(at, _)| *at == session.table_revision);
        if !current {
            for table in [AREAS, CONTINENTS, OVERLAYS, area::TABLE, vale_edit::dbc::places::MAPS] {
                session.open_table(assets, table);
            }
            let entries = list(session);
            self.listed = Some((session.table_revision, entries));
        }
        &self.listed.as_ref().expect("built above").1
    }

    /// The chosen map's entry, from the list as it was last built.
    pub(crate) fn chosen_entry(&self) -> Option<&Entry> {
        let chosen = self.chosen?;
        self.listed.as_ref()?.1.iter().find(|entry| entry.which == chosen)
    }

    /// An entry by its row id.
    pub(crate) fn entry_of(&self, which: Which) -> Option<&Entry> {
        self.listed.as_ref()?.1.iter().find(|entry| entry.which == which)
    }

    /// Choose a map. A new picture, new overlay art and a drawing for another
    /// map are dropped, and the New map form is closed.
    pub(crate) fn choose(&mut self, which: Which) {
        self.create = None;
        if self.chosen == Some(which) {
            return;
        }
        self.chosen = Some(which);
        self.picture.leave();
        self.overlays.leave();
        self.placed = None;
        self.after_scan = None;
        self.said = None;
    }

    /// Whether the project holds any piece of the map in `directory`.
    pub(crate) fn owns(&mut self, session: &EditSession, directory: &str) -> bool {
        let current = self.owned.as_ref().is_some_and(|(at, _)| *at == session.files_revision);
        if !current {
            let mut found: Vec<String> = session
                .project
                .files()
                .iter()
                .filter_map(|vpath| {
                    let lower = vpath.replace('/', "\\").to_ascii_lowercase();
                    let rest = lower.strip_prefix("interface\\worldmap\\")?;
                    let (folder, file) = rest.split_once('\\')?;
                    let number = file.strip_prefix(folder)?.strip_suffix(".blp")?;
                    number.parse::<u32>().ok().filter(|n| (1..=pieces::PIECES).contains(n))?;
                    Some(folder.to_string())
                })
                .collect();
            found.sort();
            found.dedup();
            self.owned = Some((session.files_revision, found));
        }
        let lower = directory.to_ascii_lowercase();
        self.owned.as_ref().expect("listed above").1.contains(&lower)
    }
}

/// A `WorldMapArea` row as `MapArea`, from the editable table.
pub(crate) fn row_area(table: &DbcFile, record: usize) -> Option<MapArea> {
    let float = |field| table.f32_at(record, field);
    Some(MapArea {
        id: table.u32_at(record, 0)?,
        map: table.u32_at(record, af::MAP_ID)?,
        area: table.u32_at(record, af::AREA_ID)?,
        directory: table.string_at(record, af::DIRECTORY).unwrap_or_default(),
        left: float(af::LOC_LEFT)?,
        right: float(af::LOC_RIGHT)?,
        top: float(af::LOC_TOP)?,
        bottom: float(af::LOC_BOTTOM)?,
    })
}

/// `AreaTable`'s name of `id`, or `None` for a row it does not have or an
/// empty name.
pub(crate) fn area_name(session: &EditSession, id: u32) -> Option<String> {
    let table = session.table(area::TABLE)?;
    table.row_of(id).and_then(|record| table.string_at(record, area::NAME)).filter(|name| !name.is_empty())
}

/// The list of maps from the session's open tables: the world, each
/// continent followed by its zones, then the single maps.
fn list(session: &EditSession) -> Vec<Entry> {
    let mut entries = vec![Entry {
        which: Which::World,
        kind: Kind::World,
        directory: pieces::WORLD.to_string(),
        name: pieces::WORLD.to_string(),
        area: None,
        continent: None,
    }];
    let Some(table) = session.table(AREAS) else {
        return entries;
    };
    let rows: Vec<MapArea> = (0..table.record_count()).filter_map(|record| row_area(table, record)).collect();
    let mut continents: Vec<&MapArea> = rows.iter().filter(|row| row.area == 0).collect();
    continents.sort_by_key(|row| row.id);
    let named = |row: &MapArea| area_name(session, row.area).unwrap_or_else(|| row.directory.clone());
    for continent in &continents {
        entries.push(Entry {
            which: Which::Row(continent.id),
            kind: Kind::Continent,
            directory: continent.directory.clone(),
            name: continent.directory.clone(),
            area: Some((*continent).clone()),
            continent: None,
        });
        let mut zones: Vec<Entry> = rows
            .iter()
            .filter(|row| row.area != 0 && row.map == continent.map)
            .map(|row| Entry {
                which: Which::Row(row.id),
                kind: Kind::Zone,
                directory: row.directory.clone(),
                name: named(row),
                area: Some(row.clone()),
                continent: Some(continent.id),
            })
            .collect();
        zones.sort_by_key(|entry| entry.name.to_lowercase());
        entries.extend(zones);
    }
    let mut singles: Vec<Entry> = rows
        .iter()
        .filter(|row| row.area != 0 && !continents.iter().any(|continent| continent.map == row.map))
        .map(|row| Entry {
            which: Which::Row(row.id),
            kind: Kind::Single,
            directory: row.directory.clone(),
            name: named(row),
            area: Some(row.clone()),
            continent: None,
        })
        .collect();
    singles.sort_by_key(|entry| entry.name.to_lowercase());
    entries.extend(singles);
    entries
}

/// `map`'s directory in `Map.dbc`, which its tiles and its minimap tiles are
/// filed under.
pub(crate) fn map_directory(session: &mut EditSession, assets: &GameAssets, map: u32) -> Option<String> {
    use vale_edit::dbc::places::{map_directories, MAPS};
    session.open_table(assets, MAPS);
    map_directories(session.table(MAPS)?).into_iter().find(|(id, _)| *id == map).map(|(_, directory)| directory)
}

/// Ask the system's file dialog for one picture, from the folder the last
/// one came from.
fn choose_picture(maps: &mut WorldMaps) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter("Picture", &["png", "tga", "blp"]);
    if let Some(dir) = &maps.picture_dir {
        dialog = dialog.set_directory(dir);
    }
    let path = dialog.pick_file()?;
    maps.picture_dir = path.parent().map(Path::to_path_buf);
    Some(path)
}

/// Carry out what the panel asked for.
fn answer(mut maps: ResMut<WorldMaps>, session: Option<ResMut<EditSession>>, assets: Res<GameAssets>, time: Res<Time>) {
    let Some(ask) = maps.ask.take() else { return };
    let Some(mut session) = session else { return };
    let maps = &mut *maps;
    let said = match ask {
        Ask::Import => {
            let Some(entry) = maps.chosen_entry().cloned() else { return };
            let Some(path) = choose_picture(maps) else { return };
            match maps.picture.read_file(&entry, &session, &assets, &path) {
                Ok(()) => format!("read {}", path.display()),
                Err(why) => why,
            }
        }
        Ask::Draw => {
            let Some(entry) = maps.chosen_entry().cloned() else { return };
            match maps.picture.draw(&entry, &mut session, &assets) {
                Ok(()) => return,
                Err(why) => why,
            }
        }
        Ask::Write => {
            let Some(entry) = maps.chosen_entry().cloned() else { return };
            maps.picture.write(&entry, &mut session)
        }
        Ask::Remove => {
            let Some(entry) = maps.chosen_entry().cloned() else { return };
            maps.picture.remove(&entry, &mut session)
        }
        Ask::OverlayArt(id) => {
            let Some(path) = choose_picture(maps) else { return };
            match maps.overlays.read_art(overlays::Target::Row(id), &path) {
                Ok(()) => format!("read {}", path.display()),
                Err(why) => why,
            }
        }
        Ask::NewOverlay => {
            let Some(path) = choose_picture(maps) else { return };
            match maps.overlays.read_art(overlays::Target::New, &path) {
                Ok(()) => format!("read {}; place it, then Write art", path.display()),
                Err(why) => why,
            }
        }
        Ask::WriteOverlay => {
            let Some(entry) = maps.chosen_entry().cloned() else { return };
            maps.overlays.write(&entry, &mut session)
        }
        Ask::Create => {
            let Some(form) = maps.create.take() else { return };
            match rows::create(&form, &mut session, &assets) {
                Ok((id, said)) => {
                    maps.listed = None;
                    maps.entries(&mut session, &assets);
                    maps.choose(Which::Row(id));
                    // A new row's rectangle is fitted to what the map holds,
                    // once its tiles are read.
                    maps.after_scan = Some(AfterScan::FitRect);
                    maps.ask = Some(Ask::Scan);
                    said
                }
                Err(why) => {
                    maps.create = Some(form);
                    why
                }
            }
        }
        Ask::Scan => {
            let Some(entry) = maps.chosen_entry().cloned() else { return };
            let Some(map) = entry.area.as_ref().map(|area| area.map) else { return };
            match maps.scans.start(map, &mut session, &assets) {
                Ok(true) => return,
                Ok(false) => grid::after(maps, &entry, &mut session, time.elapsed_secs_f64()),
                Err(why) => why,
            }
        }
    };
    session.status = said.clone();
    maps.said = Some(said);
}

/// Collect finished work: a minimap drawing, and a read of a map's tiles,
/// after which what was waiting for it is done.
fn collect(mut maps: ResMut<WorldMaps>, session: Option<ResMut<EditSession>>, time: Res<Time>) {
    let maps = &mut *maps;
    if let Some(said) = maps.picture.collect() {
        maps.said = Some(said);
    }
    let Some(mut session) = session else { return };
    if maps.scans.collect(&session) && maps.after_scan.is_some() {
        if let Some(entry) = maps.chosen_entry().cloned() {
            let said = grid::after(maps, &entry, &mut session, time.elapsed_secs_f64());
            session.status = said.clone();
            maps.said = Some(said);
        }
    }
}

/// `--world-map` and the flags with it: choose the map, read a picture or
/// start a drawing, then write the new picture once it is ready. See
/// `crate::args::import::Import`.
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut maps: ResMut<WorldMaps>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    mut step: Local<u8>,
) {
    let Some(mut session) = session else { return };
    let maps = &mut *maps;
    if let Some(new) = args.import.world_map_new.as_deref().filter(|_| *step == 0) {
        *step = 2;
        maps.entries(&mut session, &assets);
        let parts: Vec<&str> = new.split(':').collect();
        let (kind, id) = match parts.as_slice() {
            ["zone", id, _] => (Kind::Zone, id.parse().ok()),
            ["continent", id, _] => (Kind::Continent, id.parse().ok()),
            ["single", id, _] => (Kind::Single, id.parse().ok()),
            _ => (Kind::World, None),
        };
        let form = rows::NewMap {
            kind,
            map: if kind == Kind::Zone { None } else { id },
            area: if kind == Kind::Zone { id } else { None },
            directory: parts.get(2).map_or_else(String::new, |d| d.to_string()),
        };
        info!("--world-map-new: {form:?}");
        maps.create = Some(form);
        maps.ask = Some(Ask::Create);
        return;
    }
    let Some(directory) = args.import.world_map.clone() else { return };
    match *step {
        0 => {
            *step = 1;
            let found = maps
                .entries(&mut session, &assets)
                .iter()
                .find(|entry| entry.directory.eq_ignore_ascii_case(&directory))
                .cloned();
            let Some(entry) = found else {
                warn!("--world-map {directory}: no map has its pieces in that directory");
                *step = 2;
                return;
            };
            maps.choose(entry.which);
            maps.explored = args.import.world_map_explored;
            maps.placing = args.import.world_map_places.is_some();
            maps.placed = args.import.world_map_places.filter(|&id| id != 0);
            maps.picture.selected = args
                .import
                .world_map_sectors
                .iter()
                .filter(|n| (1..=pieces::PIECES).contains(*n))
                .fold(0, |mask, n| mask | (1 << (n - 1)));
            info!("--world-map: {directory}");
            let started = match (&args.import.world_map_image, args.import.world_map_draw) {
                (Some(path), _) => maps.picture.read_file(&entry, &session, &assets, path),
                (None, true) => maps.picture.draw(&entry, &mut session, &assets),
                (None, false) => Ok(()),
            };
            if let Err(why) = started {
                warn!("--world-map {directory}: {why}");
            }
        }
        1 => {
            if maps.picture.drawing.is_some() {
                return;
            }
            *step = 2;
            if let Some(said) = &maps.said {
                info!("--world-map: {said}");
            }
            if args.import.world_map_write && maps.picture.pending.is_some() {
                let Some(entry) = maps.chosen_entry().cloned() else { return };
                let said = maps.picture.write(&entry, &mut session);
                info!("--world-map-write: {said}");
            }
        }
        _ => {}
    }
}

pub(crate) struct WorldMapPlugin;

impl Plugin for WorldMapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldMaps>().add_systems(Update, (on_the_command_line, answer, collect).chain());
    }
}
