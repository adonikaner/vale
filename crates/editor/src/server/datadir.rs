//! The server's tiles regenerated for the tiles a project changed: `maps\`,
//! `vmaps\` and `mmaps\`, through the four vmangos tools. There are two
//! callers. The dev-time buttons write the live `DataDir`, so a playtest walks
//! on the edit after a server restart: *Regenerate changed tiles* on the
//! Server panel, *Server files* on the map window, and `--regenerate`. A
//! publish writes a patch folder; see [`super::publish::patch`].
//!
//! ## Which tiles
//!
//! A record holds, per tile, the hash of the ADT the server's files were last
//! built from. For the live `DataDir` the record is the `DataDir`'s own
//! [`Ledger`], which every project's runs write, so it holds the tiles other
//! projects built as well as this one's ([`ledger_record`]). For a patch it is
//! the project's [`PATCH_RECORD`]. A run compares the project against its
//! record:
//!
//! ```text
//! a tile not in the record, or whose bytes changed     regenerated
//! a tile in the record the project does not carry      regenerated from the
//!                                                       archives, which is
//!                                                       what the reverted
//!                                                       tile, or another
//!                                                       project's tile, is in
//!                                                       this project
//!   ... and the archives do not have it                its server files
//!                                                       marked stale
//!   ... and Map.dbc no longer lists its map            every file of the map
//!                                                       marked stale, found by
//!                                                       the map id the record
//!                                                       kept
//! a tile in the record with the same bytes             left alone
//! ```
//!
//! A run removes no server file; see `vale_mangos::datadir`'s *Nothing is
//! removed by a run*. Stale files are reviewed and removed in the *Server tile
//! files* window ([`crate::ui::shell::tilefiles`]).
//!
//! How far a change reaches is read off the tile itself. A tile whose
//! placements are the archives' own (`AdtFile::same_placements` against the
//! shipped copy, read past the overlay and past this project's own archive) is
//! a terrain edit and takes the `.map` and the navmesh. Any other tile, and a
//! tile the archives do not have, takes the vmap half of its map as well. See
//! `vale_mangos::datadir`, which is the driver and states the costs.
//!
//! ## The tools read an archive, never the project folder
//!
//! The extractors read a `Data\`, so a tile reaches them only through a
//! `Patch-<X>.MPQ`. The dev-time run rebuilds the project's own archive, in the
//! real `Data\` or for a staged copy of the install; a publish stages a copy of
//! the install with the patch's archive in it. A save writes the project
//! folder, which the tools cannot read, so neither step is part of Save.
//!
//! ## What runs on the main thread
//!
//! A dev-time run is a write on the server queue ([`super::queue`]): it runs on
//! a worker, one at a time with the database writes, and its finish runs on
//! the main thread and sets the status line. The press itself does only what
//! needs the session or is a lookup: it saves the open tiles, finds the tools
//! and the `DataDir`, and reads `Map.dbc`'s list of maps. Every step whose cost
//! grows with the number of tiles runs on the worker, in this order:
//!
//! ```text
//! check the tools     each extractor is run once for its usage text
//! compare             every tile read from the project and from the archives,
//!                     parsed and hashed; see [`dirty`]
//! build the archive   every file of the project packed into one MPQ
//! run the tools       see `vale_mangos::datadir::regenerate`; each write is
//!                     noted in the ledger as it is made
//! note the tiles      the ledger's tile lines, on success
//! ```
//!
//! The comparison and the archive build ran on the main thread, and the window
//! stopped drawing for a time that grew with the number of tiles.
//!
//! The comparison runs before the archive is built. A tile saved between the
//! two is in the archive with a hash the record does not hold, so the next run
//! regenerates it again instead of leaving it out.
//!
//! The toast's bar says which step is running.

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::adt::AdtFile;
use vale_edit::project::Project;
use vale_edit::TileKey;
use vale_assets::archive::{ChainPool, PooledChain};
use vale_mangos::datadir::ledger::Built;
use vale_mangos::datadir::{self, Ledger, Outcome, Reach, Request, Tile, Tools};
use bevy::prelude::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where the project records what its patches' server tiles were built from.
/// The live `DataDir`'s tiles are recorded in the `DataDir`'s [`Ledger`]
/// instead. Two records are kept because two things build tiles: one record
/// for both would let a dev regenerate hide a tile from the next patch, or the
/// reverse.
pub(crate) const PATCH_RECORD: &str = "publish\\server-tiles.txt";
/// Where a project recorded the live `DataDir`'s tiles before the `DataDir`
/// had a ledger. Read once, into the ledger, by [`import_live_record`], and
/// not written.
pub(crate) const LIVE_RECORD: &str = "server\\regenerated.txt";

/// The record: `vpath -> FNV-1a of the ADT bytes`, sorted by path, and the
/// `Map.dbc` id of each map those tiles are on.
///
/// The ids are kept because the server's file names are numbered by map id
/// and a tile's path names the map by directory. When a project removes a map
/// it made, `Map.dbc` no longer lists the directory, and the id kept here is
/// the only way to find the map's files on the server. A record written
/// before the ids were kept has none, and such a map's files are left, with a
/// line saying so.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) tiles: BTreeMap<String, u64>,
    /// Lower-cased map directory -> map id.
    pub(crate) maps: BTreeMap<String, u32>,
}

impl Record {
    pub(crate) fn read(project: &Project, vpath: &str) -> Record {
        let Some(bytes) = project.read(vpath) else {
            return Record::default();
        };
        Record::from_text(&String::from_utf8_lossy(&bytes))
    }

    pub(crate) fn from_text(text: &str) -> Record {
        let mut out = Record::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').map(str::trim).collect();
            match fields.as_slice() {
                ["map", directory, id] => {
                    if let Ok(id) = id.parse() {
                        out.maps.insert(directory.to_ascii_lowercase(), id);
                    }
                }
                [vpath, hash] => {
                    if let Ok(hash) = u64::from_str_radix(hash, 16) {
                        out.tiles.insert(vpath.to_string(), hash);
                    }
                }
                _ => {}
            }
        }
        out
    }

    pub(crate) fn to_text(&self, project: &str) -> String {
        let mut out = format!(
            "# {project} — the tiles the server's maps, vmaps and mmaps were last built\n\
             # from, as the FNV-1a 64 of each ADT's bytes, and the Map.dbc id of each map\n\
             # they are on. A publish regenerates every tile whose bytes differ, and every\n\
             # tile listed here that the project no longer carries.\n"
        );
        for (directory, id) in &self.maps {
            out.push_str(&format!("map\t{directory}\t{id}\n"));
        }
        for (vpath, hash) in &self.tiles {
            out.push_str(&format!("{vpath}\t{hash:016x}\n"));
        }
        out
    }

    pub(crate) fn write(&self, project: &Project, vpath: &str) -> Result<(), String> {
        project
            .write(vpath, self.to_text(&project.name).as_bytes())
            .map(|_| ())
            .map_err(|e| format!("{vpath}: {e}"))
    }

    /// Record every tile of `done` as built from the bytes it was compared
    /// at, and forget a tile that has left the project. Then hold the id of
    /// every map the record's tiles are on: from `maps`, `Map.dbc`'s
    /// `(id, directory)` list, where it lists the map, and as before where it
    /// does not. A map's id is forgotten with its last tile.
    pub(crate) fn note(&mut self, done: &[Dirty], maps: &[(u32, String)]) {
        for d in done {
            match d.hash {
                Some(hash) => {
                    self.tiles.insert(d.vpath.clone(), hash);
                }
                None => {
                    self.tiles.remove(&d.vpath);
                }
            }
        }
        let mut held: Vec<String> = self
            .tiles
            .keys()
            .filter_map(|vpath| TileKey::from_vpath(vpath).map(|key| key.map.to_ascii_lowercase()))
            .collect();
        held.sort();
        held.dedup();
        self.maps.retain(|directory, _| held.contains(directory));
        for directory in held {
            if let Some((id, _)) = maps.iter().find(|(_, dir)| dir.eq_ignore_ascii_case(&directory)) {
                self.maps.insert(directory, *id);
            }
        }
    }

    /// The id of the map in `directory`, as this record last saw it.
    fn map_id(&self, directory: &str) -> Option<u32> {
        self.maps.get(&directory.to_ascii_lowercase()).copied()
    }
}

/// One tile to regenerate: where it is in the project, which server tile,
/// how far the change reaches, and the hash the record will hold. The hash is
/// `None` for a tile that has left the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dirty {
    pub(crate) vpath: String,
    pub(crate) tile: Tile,
    pub(crate) reach: Reach,
    pub(crate) hash: Option<u64>,
    /// The tile's map is no longer in `Map.dbc`: every file of the map is
    /// removed and no tool runs for it.
    pub(crate) map_removed: bool,
}

/// The [`Request`] fields a list of tiles gives: the tiles the tools run
/// for, with their reach; the ones the project carries; and the maps whose
/// files are removed. The caller fills in the rest.
pub(crate) fn split(dirty: &[Dirty]) -> (Vec<(Tile, Reach)>, Vec<Tile>, Vec<u32>) {
    let built = dirty.iter().filter(|d| !d.map_removed);
    let tiles = built.clone().map(|d| (d.tile, d.reach)).collect();
    let carried = built.filter(|d| d.hash.is_some()).map(|d| d.tile).collect();
    let mut removed_maps: Vec<u32> = dirty.iter().filter(|d| d.map_removed).map(|d| d.tile.map).collect();
    removed_maps.sort_unstable();
    removed_maps.dedup();
    (tiles, carried, removed_maps)
}

/// What the comparison of tiles reads: the project folder, the archive chain,
/// and `Map.dbc`'s `(id, directory)` list. It owns all three and is `Send`, so
/// [`dirty`] runs on a worker as well as on the main thread.
///
/// The archives are read through a chain borrowed from the client's pool
/// (`GameAssets::chains`), as the tile loaders read them, because a worker
/// cannot hold `GameAssets`.
#[derive(Clone)]
pub(crate) struct TileReader {
    project: Project,
    chains: Arc<ChainPool>,
    /// The folder the pool is keyed on: `GameAssets::gamedata_dir` as it is
    /// written. A borrow under any other spelling empties the pool, and the
    /// tile loaders would open their chains again.
    gamedata_dir: String,
    maps: Vec<(u32, String)>,
}

impl TileReader {
    /// Reads `Map.dbc` through `assets`, which is one small file.
    pub(crate) fn new(project: &Project, assets: &GameAssets) -> TileReader {
        TileReader {
            project: project.clone(),
            chains: assets.chains(),
            gamedata_dir: assets.gamedata_dir.clone(),
            maps: crate::places::map_directories(assets),
        }
    }

    /// A chain for the comparison, borrowed once for all its tiles. `None`
    /// when the archives do not open, in which case no tile is found in them,
    /// as a failed read of one tile already means.
    fn borrow(&self) -> Option<PooledChain<'_>> {
        self.chains.take(&self.gamedata_dir).ok()
    }

    /// This project's own archives in `Data\`, which the comparison reads
    /// past: a comparison against a chain that includes them would compare an
    /// edit with itself.
    fn own(&self) -> Vec<String> {
        self.project
            .published()
            .into_iter()
            .map(|archive| archive.name)
            .collect()
    }

    /// `Map.dbc`'s `(id, directory)` list, for [`Record::note`].
    pub(crate) fn maps(&self) -> &[(u32, String)] {
        &self.maps
    }

    /// The tiles the map in `directory` has in this project: its WDT read
    /// from the project, or from the archives when the project has none.
    /// `None` when neither has a WDT that parses.
    fn wdt_tiles(&self, chain: &mut Option<PooledChain<'_>>, directory: &str) -> Option<Vec<(u32, u32)>> {
        let path = vale_edit::wdt::wdt_path(directory);
        let bytes = self
            .project
            .read(&path)
            .or_else(|| chain.as_mut().and_then(|chain| chain.read(&path).ok()))?;
        let wdt = vale_assets::world::wdt::Wdt::parse(&bytes).ok()?;
        Some(wdt.existing_tiles())
    }

    /// Files in `data_dir` that nothing in this project uses, each with why:
    ///
    /// ```text
    /// every file of a map id Map.dbc does not list
    /// a .map or .mmtile of a terrain map, for a tile its WDT does not have
    /// a model file no .vmtile, .vmtree or game object names
    /// ```
    ///
    /// A `.vmtile` outside the WDT is not listed: the assembler writes one
    /// for every tile a model's bounds reach, and the map's tree indexes it.
    /// A map whose WDT cannot be read is left out, as is a map with no
    /// terrain tiles, whose navmesh tiles follow its buildings. Nothing is
    /// removed here.
    pub(crate) fn orphans(&self, data_dir: &Path) -> Result<Vec<(String, String)>, String> {
        let mut ids: Vec<u32> = Vec::new();
        for sub in ["maps", "vmaps", "mmaps"] {
            let Ok(entries) = std::fs::read_dir(data_dir.join(sub)) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let id = name.get(0..3).filter(|d| d.bytes().all(|b| b.is_ascii_digit()));
                let tile_file = name.ends_with(".map")
                    || name.ends_with(".mmtile")
                    || name.ends_with(".mmap")
                    || name.ends_with(".vmtile")
                    || name.ends_with(".vmtree");
                if let (Some(id), true) = (id.and_then(|d| d.parse().ok()), tile_file) {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
        }
        ids.sort_unstable();
        let mut chain = self.borrow();
        let mut out: Vec<(String, String)> = Vec::new();
        for id in ids {
            let Some((_, directory)) = self.maps.iter().find(|(m, _)| *m == id) else {
                for (sub, name) in datadir::map_files(data_dir, id) {
                    out.push((datadir::ledger::key(sub, &name), format!("map {id} is not in Map.dbc")));
                }
                continue;
            };
            let Some(tiles) = self.wdt_tiles(&mut chain, directory) else {
                continue;
            };
            if tiles.is_empty() {
                continue;
            }
            for (sub, name) in datadir::map_files(data_dir, id) {
                let stem = name.strip_suffix(".map").or_else(|| name.strip_suffix(".mmtile"));
                let Some(stem) = stem.filter(|s| s.len() == 7) else {
                    continue;
                };
                // `MMMYYXX`: the row, then the column.
                let (Ok(y), Ok(x)) = (stem[3..5].parse::<u32>(), stem[5..7].parse::<u32>()) else {
                    continue;
                };
                if !tiles.contains(&(x, y)) {
                    out.push((
                        datadir::ledger::key(sub, &name),
                        format!("{directory} has no tile {x},{y} in this project or the archives"),
                    ));
                }
            }
        }
        for name in datadir::unused_models(&data_dir.join("vmaps"))? {
            out.push((
                datadir::ledger::key("vmaps", &name),
                "no .vmtile, .vmtree or game object names this model".to_string(),
            ));
        }
        Ok(out)
    }

    /// The `Map.dbc` id of the map in `directory`.
    fn map_id(&self, directory: &str) -> Option<u32> {
        self.maps
            .iter()
            .find(|(_, dir)| dir.eq_ignore_ascii_case(directory))
            .map(|(id, _)| *id)
    }

    /// One tile, as a run would regenerate it: from the project when it
    /// carries the tile, with how far the change reaches; from the archives
    /// when it does not, whole. `None` when neither has it; `Err` when the map
    /// is not in `Map.dbc`.
    fn classify(
        &self,
        chain: &mut Option<PooledChain<'_>>,
        own: &[String],
        map: &str,
        x: u32,
        y: u32,
    ) -> Result<Option<Dirty>, String> {
        let vpath = vale_assets::adt_path(map, x, y);
        let id = self
            .map_id(map)
            .ok_or_else(|| format!("{vpath}: {map} is not a map in Map.dbc"))?;
        let shipped = chain
            .as_mut()
            .and_then(|chain| chain.read_past_overlay_skipping(&vpath, own).ok());
        let Some(bytes) = self.project.read(&vpath) else {
            // Not in the project: the archives' own, or nothing.
            return Ok(shipped.map(|_| Dirty {
                vpath,
                tile: Tile::new(id, x, y),
                reach: Reach::Placements,
                hash: None,
                map_removed: false,
            }));
        };
        let reach = match shipped {
            Some(shipped) => match (AdtFile::parse(&shipped), AdtFile::parse(&bytes)) {
                (Ok(shipped), Ok(edited)) if shipped.same_placements(&edited) => Reach::Terrain,
                _ => Reach::Placements,
            },
            // Not in the archives: a tile the project made.
            None => Reach::Placements,
        };
        Ok(Some(Dirty {
            vpath,
            tile: Tile::new(id, x, y),
            reach,
            hash: Some(vale_edit::project::fnv1a(&bytes)),
            map_removed: false,
        }))
    }
}

/// Which tiles a run regenerates, against `record`. A tile the project
/// carries on a map that `Map.dbc` does not name is skipped and named in the
/// second list. A tile the record holds and the project no longer carries is
/// regenerated from the archives; when the archives do not have it either,
/// its server files are removed, and when `Map.dbc` no longer names its map,
/// the whole map's are ([`Dirty::map_removed`]).
///
/// Every tile is read from the project and from the archives, parsed and
/// hashed, so the cost grows with the number of tiles the project carries.
/// `progress` is called before each tile with the number compared and the
/// total.
pub(crate) fn dirty(
    reader: &TileReader,
    record: &Record,
    progress: &mut dyn FnMut(usize, usize),
) -> (Vec<Dirty>, Vec<String>) {
    let carried: Vec<String> = reader
        .project
        .files()
        .into_iter()
        .filter(|vpath| TileKey::from_vpath(vpath).is_some())
        .collect();
    // Tiles the record has and the project no longer carries.
    let gone: Vec<String> = record
        .tiles
        .keys()
        .filter(|vpath| !carried.iter().any(|c| c.eq_ignore_ascii_case(vpath)))
        .cloned()
        .collect();
    let total = carried.len() + gone.len();
    let own = reader.own();
    let mut chain = reader.borrow();
    let mut out: Vec<Dirty> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (done, vpath) in carried.iter().chain(&gone).enumerate() {
        progress(done, total);
        let Some(TileKey { map, x, y }) = TileKey::from_vpath(vpath) else {
            continue;
        };
        let left_the_project = done >= carried.len();
        match reader.classify(&mut chain, &own, &map, x, y) {
            Ok(Some(one))
                if !left_the_project && record.tiles.get(vpath) == one.hash.as_ref() => {}
            Ok(Some(one)) => out.push(one),
            Ok(None) if !left_the_project => {}
            Err(why) if !left_the_project => skipped.push(why),
            // The tiles below have left the project. Each keeps the record's
            // own spelling of its path, so that the record forgets it.
            //
            // Neither the project nor the archives have the tile: the project
            // made it and has removed it. The run removes its server files.
            Ok(None) => {
                if let Some(id) = reader.map_id(&map) {
                    out.push(removed(vpath, Tile::new(id, x, y), false));
                }
            }
            // `Map.dbc` no longer lists the map: the project made the map and
            // has removed it. Its files are found by the id the record kept.
            Err(why) => match record.map_id(&map) {
                Some(id) => out.push(removed(vpath, Tile::new(id, x, y), true)),
                None => skipped.push(format!(
                    "{why}, and the record holds no id for it, so its server files were left"
                )),
            },
        }
    }
    (out, skipped)
}

/// A tile whose server files are removed: on a map that is still in
/// `Map.dbc`, through a run of the tools for the map, or with the whole map
/// when `map_removed`.
fn removed(vpath: &str, tile: Tile, map_removed: bool) -> Dirty {
    Dirty {
        vpath: vpath.to_string(),
        tile,
        reach: Reach::Placements,
        hash: None,
        map_removed,
    }
}

/// The tiles of `map` asked for by name, whatever the record says: the map
/// window's selection. A tile the project carries is regenerated from it, and
/// one it does not from the archives, which puts back what a reverted tile
/// had. Answers what was skipped and why beside the list.
pub(crate) fn dirty_for(
    reader: &TileReader,
    map: &str,
    chosen: &[(u32, u32)],
    progress: &mut dyn FnMut(usize, usize),
) -> (Vec<Dirty>, Vec<String>) {
    let own = reader.own();
    let mut chain = reader.borrow();
    let mut out: Vec<Dirty> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (done, &(x, y)) in chosen.iter().enumerate() {
        progress(done, chosen.len());
        match reader.classify(&mut chain, &own, map, x, y) {
            Ok(Some(one)) => out.push(one),
            Ok(None) => skipped.push(format!(
                "{}: neither this project nor the archives have it",
                vale_assets::adt_path(map, x, y)
            )),
            Err(why) => skipped.push(why),
        }
    }
    (out, skipped)
}

/// Where the tools are, from the environment or the panel, or the sentence to
/// show when neither says.
pub(crate) fn tools(server: &super::settings::ServerSettings) -> Result<Tools, String> {
    if let Some(found) = Tools::from_env() {
        return found;
    }
    let dir = server.tools.trim();
    if dir.is_empty() {
        return Err(format!(
            "no vmangos tools folder: set it on the Server panel, or set {}",
            datadir::TOOLS_ENV
        ));
    }
    Tools::find(dir)
}

/// Where the server's `DataDir` is, through the conf the panel or the
/// environment names.
pub(crate) fn data_dir(server: &super::settings::ServerSettings) -> Result<PathBuf, String> {
    let conf = server
        .conf_path()
        .ok_or_else(|| "no mangosd.conf: set it on the Server panel, or set VALE_MANGOSD".to_string())?;
    if !conf.is_file() {
        return Err(format!("{} is not a file", conf.display()));
    }
    datadir::from_conf(&conf).ok_or_else(|| format!("{}: no DataDir", conf.display()))
}

/// The client install the extractors read: the folder `Data\` is in.
fn client_root(assets: &GameAssets) -> Result<PathBuf, String> {
    // Absolute, because the folder is `Data` as often as not, whose parent
    // is nothing, and the tools are run from folders of their own.
    let data = std::path::absolute(&assets.gamedata_dir)
        .map_err(|e| format!("{}: {e}", assets.gamedata_dir))?;
    let named_data = data
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Data"));
    match (named_data, data.parent()) {
        (true, Some(root)) => Ok(root.to_path_buf()),
        _ => Err(format!(
            "the archives are read from {}, which is not a folder called Data — the vmangos \
             extractors take the install folder and open Data\\ under it",
            data.display()
        )),
    }
}

/// Which step a run is on, and how far along: the line the bar's hover and the
/// spinner's hover show, and the fraction the bar fills to. Written by the
/// worker and read by the panels. Empty between runs.
#[derive(Resource, Default, Clone)]
pub(crate) struct Step(pub(crate) Arc<Mutex<(String, f32)>>);

impl Step {
    /// The step's name, or empty when nothing is running.
    pub(crate) fn line(&self) -> String {
        self.0.lock().map(|s| s.0.clone()).unwrap_or_default()
    }

    /// The line and the fraction together, for a bar, or `None` when nothing
    /// is running.
    pub(crate) fn summary(&self) -> Option<(String, f32)> {
        let held = self.0.lock().ok()?;
        (!held.0.is_empty()).then(|| held.clone())
    }

    /// Say which step, and how many of how many are done. A total of zero is
    /// a bar at nothing, for the steps that report no count.
    pub(crate) fn set(&self, line: &str, done: usize, total: usize) {
        if let Ok(mut held) = self.0.lock() {
            let fraction = match total {
                0 => 0.0,
                n => (done as f32 / n as f32).clamp(0.0, 1.0),
            };
            *held = (line.to_string(), fraction);
        }
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut held) = self.0.lock() {
            *held = (String::new(), 0.0);
        }
    }
}

/// The archive the tools read, when it is not to be put in `Data\`.
///
/// The extractors open an install's `Data\` and nothing else, so a project
/// whose archive is not copied there is staged the way a publish stages its
/// patch: the archives hard-linked under a folder of the project's own, this
/// project's archive beside them under the letter it would take, and the
/// folder removed when the run ends. See `vale_mangos::datadir::stage_client`.
struct Staging {
    /// The real `Data\`.
    data_dir: PathBuf,
    /// This project's own archives in it, to leave out of the copy.
    own: Vec<String>,
    /// Where the archive is built, and the name it is linked under.
    archive: PathBuf,
    as_name: String,
    /// Whether the archive is an empty stand-in, for a project with no game
    /// data. It is linked so the staging function has a file to link, then
    /// removed, so the tools read the install without this project's older
    /// archives.
    empty: bool,
    /// The staging folder, under the project's `publish\`.
    into: PathBuf,
}

impl Staging {
    /// Build the project's archive under `publish\.stage\archive\`, under the
    /// letter it would take in `Data\`. The caller has saved the open tiles.
    fn build(project: &Project, gamedata_dir: &str) -> Result<Staging, String> {
        let data_dir = std::path::absolute(gamedata_dir).map_err(|e| e.to_string())?;
        let into = project
            .path_for(super::publish::patch::STAGE)
            .ok_or_else(|| "the staging folder is outside the project folder".to_string())?;
        let _ = std::fs::remove_dir_all(&into);
        let as_name = vale_edit::project::next_patch_name(&data_dir)
            .unwrap_or_else(|| "Patch-Z.MPQ".to_string());
        let archive = into.join("archive").join(&as_name);
        let count = project
            .publish(&archive)
            .map_err(|e| format!("{}: {e}", archive.display()))?;
        let empty = count == 0;
        if empty {
            if let Some(parent) = archive.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&archive, b"").map_err(|e| e.to_string())?;
        }
        let own: Vec<String> = project.published().into_iter().map(|a| a.name).collect();
        Ok(Staging {
            data_dir,
            own,
            archive,
            as_name,
            empty,
            into,
        })
    }

    /// Link the install and the archive under the staging folder, and answer
    /// the install folder the tools are given.
    fn stage(&self) -> Result<PathBuf, String> {
        let root = datadir::stage_client(
            &self.data_dir,
            &self.own,
            &self.archive,
            &self.as_name,
            &self.into.join("client"),
        )?;
        if self.empty {
            let _ = std::fs::remove_file(root.join("Data").join(&self.as_name));
        }
        Ok(root)
    }
}

/// The record a dev-time run compares against: every tile the `DataDir`'s
/// ledger says a project built, whichever project, with the id of its map.
/// While the ledger holds no tile of `project`, the project's own
/// [`LIVE_RECORD`] is read as well, for a project that regenerated before the
/// ledger existed.
pub(crate) fn ledger_record(ledger: &Ledger, project: &Project, maps: &[(u32, String)]) -> Record {
    let mut out = Record::default();
    for (tile, built) in &ledger.tiles {
        out.tiles.insert(built.vpath.clone(), built.hash);
        if let Some(key) = TileKey::from_vpath(&built.vpath) {
            out.maps.insert(key.map.to_ascii_lowercase(), tile.map);
        }
    }
    if !ledger.tiles.values().any(|built| built.project == project.name) {
        let old = Record::read(project, LIVE_RECORD);
        for (vpath, hash) in old.tiles {
            out.tiles.entry(vpath).or_insert(hash);
        }
        for (directory, id) in old.maps {
            out.maps.entry(directory).or_insert(id);
        }
        for (id, directory) in maps {
            if out.tiles.keys().any(|v| TileKey::from_vpath(v).is_some_and(|k| k.map.eq_ignore_ascii_case(directory))) {
                out.maps.entry(directory.to_ascii_lowercase()).or_insert(*id);
            }
        }
    }
    out
}

/// Copy the project's [`LIVE_RECORD`] into the ledger as tiles the project
/// built, while the ledger holds none of the project's. A tile whose map id
/// is not known is left out.
fn import_live_record(ledger: &mut Ledger, project: &Project, maps: &[(u32, String)]) -> Result<(), String> {
    if ledger.tiles.values().any(|built| built.project == project.name) {
        return Ok(());
    }
    let old = Record::read(project, LIVE_RECORD);
    for (vpath, hash) in &old.tiles {
        let Some(key) = TileKey::from_vpath(vpath) else {
            continue;
        };
        let id = maps
            .iter()
            .find(|(_, dir)| dir.eq_ignore_ascii_case(&key.map))
            .map(|(id, _)| *id)
            .or_else(|| old.maps.get(&key.map.to_ascii_lowercase()).copied());
        let Some(id) = id else {
            continue;
        };
        let tile = Tile::new(id, key.x, key.y);
        if ledger.tiles.contains_key(&tile) {
            continue;
        }
        let built = Built {
            project: project.name.clone(),
            vpath: vpath.clone(),
            hash: *hash,
        };
        ledger.note_tile(tile, Some(built))?;
    }
    Ok(())
}

/// Which tiles a dev-time run regenerates.
enum Which {
    /// Every tile changed since the last run, against the ledger.
    Changed,
    /// The tiles named on `map`, whatever the record says: the map window's
    /// selection.
    Named { map: String, tiles: Vec<(u32, u32)> },
}

/// Where a dev-time run builds the project's archive.
enum Archive {
    /// Into the real `Data\`, replacing this project's own archive there
    /// (*Copy the client archive into Data* on). The tools read the install
    /// at `client_root`.
    IntoData { client_root: PathBuf },
    /// Under the project's `publish\.stage\`, and the tools read a staged
    /// copy of the install. See [`Staging`].
    Staged,
}

/// Everything a dev-time run needs on the worker, gathered on the main thread
/// by [`Run::gather`]. It holds no resource.
struct Run {
    which: Which,
    archive: Archive,
    reader: TileReader,
    tools: Tools,
    data_dir: PathBuf,
    /// The server's own folder, where its runtime DLLs are. The Release
    /// assembler and generator need them.
    dll_dirs: Vec<PathBuf>,
    scratch: PathBuf,
}

/// What a dev-time run reports to the main thread.
enum Ran {
    /// There was nothing to regenerate; the sentence says why.
    Nothing(String),
    /// The tools ran.
    Done {
        outcome: Outcome,
        /// The error from writing the `DataDir`'s ledger, if it could not be
        /// written.
        recorded: Result<(), String>,
        /// This project's older archives in `Data\` that could not be
        /// removed, with why.
        left: Vec<(String, String)>,
    },
}

impl Run {
    /// Save the open tiles and gather what the worker needs. Each step is a
    /// save or a lookup, so this returns within the frame; a missing tool
    /// folder or conf is refused here, before anything is queued.
    fn gather(
        session: &mut EditSession,
        assets: &GameAssets,
        server: &super::settings::ServerSettings,
        chosen: Option<&[(u32, u32)]>,
    ) -> Result<Run, String> {
        let tools = tools(server)?;
        let data_dir = data_dir(server)?;
        let archive = match server.copy_archive {
            true => Archive::IntoData {
                client_root: client_root(assets)?,
            },
            false => Archive::Staged,
        };
        // The archive is built from the project folder, so the edits on
        // screen are written there first: the tiles, and the tables, since
        // the extractors find a new map only in the archive's `Map.dbc`.
        session.save_all();
        session.save_all_tables();
        repair_project_wdts(session);
        let which = match chosen {
            Some(tiles) => Which::Named {
                map: session.map.clone(),
                tiles: tiles.to_vec(),
            },
            None => Which::Changed,
        };
        let dll_dirs: Vec<PathBuf> = server
            .conf_path()
            .and_then(|conf| conf.parent().map(Path::to_path_buf))
            .into_iter()
            .collect();
        Ok(Run {
            which,
            archive,
            reader: TileReader::new(&session.project, assets),
            tools,
            data_dir,
            dll_dirs,
            scratch: std::env::temp_dir()
                .join(format!("vale-server-tiles-{}", std::process::id())),
        })
    }

    /// The run itself, on the worker. The staging folder and the scratch
    /// folder are removed whether the tools succeed or not.
    fn go(self, step: &Step) -> Result<Ran, String> {
        step.set("checking the map tools", 0, 0);
        self.tools.check_patched()?;

        let project = &self.reader.project;
        let mut ledger = Ledger::open(&self.data_dir)?;
        import_live_record(&mut ledger, project, &self.reader.maps)?;
        let record = ledger_record(&ledger, project, &self.reader.maps);
        let mut compared =
            |done, total| step.set("comparing the project's tiles with the archives", done, total);
        let (dirty, skipped) = match &self.which {
            Which::Changed => dirty(&self.reader, &record, &mut compared),
            Which::Named { map, tiles } => dirty_for(&self.reader, map, tiles, &mut compared),
        };
        for line in &skipped {
            warn!("server tiles: {line}");
        }
        if dirty.is_empty() {
            return Ok(Ran::Nothing(match self.which {
                Which::Changed => "no server tile has changed since the last regeneration",
                Which::Named { .. } => "no selected tile can be regenerated",
            }
            .to_string()));
        }
        let terrain_only = dirty.iter().filter(|d| d.reach == Reach::Terrain).count();
        info!(
            "server tiles: {} to regenerate ({terrain_only} terrain-only) into {}",
            dirty.len(),
            self.data_dir.display()
        );
        for d in &dirty {
            info!("  {} -> map {} tile {},{} ({:?})", d.vpath, d.tile.map, d.tile.x, d.tile.y, d.reach);
        }

        step.set("building the project's archive", 0, 0);
        let (client_root, staging, left) = match self.archive {
            Archive::IntoData { client_root } => {
                let report = project
                    .publish_into(&self.reader.gamedata_dir)
                    .map_err(|e| format!("publish: {e}"))?;
                for (name, why) in &report.left {
                    warn!("publish: {name} could not be removed: {why}");
                }
                (Ok(client_root), None, report.left)
            }
            Archive::Staged => {
                let staging = Staging::build(project, &self.reader.gamedata_dir)?;
                step.set("staging a copy of the client install for the map tools", 0, 0);
                (staging.stage(), Some(staging), Vec::new())
            }
        };

        let (tiles, carried, removed_maps) = split(&dirty);
        let first_map = tiles.first().map_or(dirty[0].tile.map, |(tile, _)| tile.map);
        let request = client_root.map(|client_root| Request {
            client_root,
            tiles,
            carried,
            removed_maps,
            project: project.name.clone(),
            heights_as_int: datadir::heights_as_int(&self.data_dir, first_map),
            dll_dirs: self.dll_dirs,
        });
        let outcome = request.and_then(|request| {
            datadir::regenerate(
                &self.tools,
                &self.data_dir,
                &request,
                &self.scratch,
                Some(&mut ledger),
                |line, done, total| step.set(line, done, total),
            )
        });
        let _ = std::fs::remove_dir_all(&self.scratch);
        if let Some(staging) = &staging {
            let _ = std::fs::remove_dir_all(&staging.into);
        }
        // A run that fails part way has copied the tiles before the failing
        // tool and not the ones after, and nothing says which. So a failed
        // run notes no tile, and the next run does all of them again. The
        // ledger's file lines were written as each file was, and are kept.
        let noted = match &outcome {
            Ok(_) => note_tiles(&mut ledger, project, &dirty),
            Err(_) => Ok(()),
        };
        let compacted = ledger.compact();
        let outcome = outcome?;
        Ok(Ran::Done {
            outcome,
            recorded: noted.and(compacted),
            left,
        })
    }
}

/// Note in the ledger what each tile of `done` is now built from: the
/// project's ADT, or the archives or nothing.
fn note_tiles(ledger: &mut Ledger, project: &Project, done: &[Dirty]) -> Result<(), String> {
    for d in done {
        let built = d.hash.map(|hash| Built {
            project: project.name.clone(),
            vpath: d.vpath.clone(),
            hash,
        });
        ledger.note_tile(d.tile, built)?;
    }
    Ok(())
}

/// Put a run's result on the status line and in the log. Runs on the main
/// thread.
fn finish(session: &mut EditSession, ran: Result<Ran, String>) {
    match ran {
        Ok(Ran::Nothing(line)) => session.status = line,
        Ok(Ran::Done {
            outcome,
            recorded,
            left,
        }) => {
            for line in &outcome.log {
                info!("server tiles: {line}");
            }
            for (name, why) in &outcome.mmaps_failed {
                warn!("server tiles: {name}: {why}");
            }
            for (what, seconds) in &outcome.timings {
                info!("server tiles: {what}: {seconds:.1} s");
            }
            let mut line = outcome.line();
            for (key, why) in &outcome.stale {
                info!("server tiles: stale, left in place: {key}: {why}");
            }
            if let Err(e) = recorded {
                line.push_str(&format!(
                    " — the DataDir's ledger ({}\\tiles.txt) could not be written ({e}), so the \
                     next regeneration repeats these tiles",
                    datadir::ledger::FOLDER
                ));
            }
            for (name, why) in &left {
                line.push_str(&format!(
                    "; {name} could not be removed ({why}) and still holds the project's older \
                     files — close the program that has it open and regenerate again"
                ));
            }
            session.status = line;
        }
        Err(e) => {
            warn!("server tiles: {e}");
            session.status = format!("server tiles: {e}");
        }
    }
}

/// Regenerate the server's tiles: the Server panel's button, the map
/// window's, and `--regenerate`. `chosen` is the tiles to do whatever the
/// record says, or `None` for every tile changed since the last time. Returns
/// the status line for the press; the run's own result replaces it when the
/// run finishes.
///
/// The project's archive is rebuilt as part of the run, because the extractors
/// read `Data\` and a tile reaches them only through that archive. With *Copy
/// the client archive into Data* on it is written into the real `Data\`,
/// replacing only this project's own. Otherwise it is built under the
/// project's `publish\.stage\` and the tools are given a staged copy of the
/// install with it beside the real archives; see [`Staging`]. The Server
/// panel's button is disabled while a playtest is running, as Publish is,
/// because the running game has the archive chain open. The map window's
/// button is not.
///
/// Everything whose cost grows with the number of tiles runs on the server
/// queue's worker. See the module comment's *What runs on the main thread*.
pub(crate) fn regenerate(
    session: &mut EditSession,
    assets: &GameAssets,
    server: &super::settings::ServerSettings,
    queue: &mut super::queue::ServerQueue,
    step: &Step,
    chosen: Option<&[(u32, u32)]>,
) -> String {
    if queue.busy() {
        return "a server write is still running".to_string();
    }
    if chosen.is_some_and(|tiles| tiles.is_empty()) {
        return "no selected tile can be regenerated".to_string();
    }
    let run = match Run::gather(session, assets, server, chosen) {
        Ok(run) => run,
        Err(e) => return format!("server tiles: {e}"),
    };
    let label = match chosen {
        Some(tiles) => format!("regenerating {} server tile(s)", tiles.len()),
        None => "regenerating changed server tiles".to_string(),
    };
    let progress = step.clone();
    queue.push(
        label.clone(),
        Box::new(move || {
            let ran = run.go(&progress);
            progress.clear();
            Box::new(move |session: &mut EditSession, _: &mut super::reload::Reloads| {
                finish(session, ran)
            })
        }),
    );
    format!("{label} \u{2014} the progress bar shows the current step")
}

/// Give every map WDT the project carries the zero-length `MWMO` a terrain map
/// ships with, when it lacks one, and write it back. A new map's WDT was
/// written without it before, and vmangos' extractors skip such a map.
pub(super) fn repair_project_wdts(session: &mut EditSession) {
    let maps: Vec<String> = session.maps.iter().map(|(_, name)| name.clone()).collect();
    for map in maps {
        let path = vale_edit::wdt::wdt_path(&map);
        let Some(bytes) = session.project.read(&path) else {
            continue;
        };
        let Ok(mut wdt) = vale_edit::wdt::WdtFile::parse(&bytes) else {
            continue;
        };
        if !wdt.repair_terrain_shape() {
            continue;
        }
        match session.project.write(&path, &wdt.write()) {
            Ok(_) => info!("server tiles: {path} given the empty MWMO the server's extractors need"),
            Err(e) => warn!("server tiles: {path}: {e}"),
        }
    }
}

/// `--regenerate`: the Server panel's *Regenerate changed tiles*, from a
/// command line. Fires once, on the first frame there is a session.
pub(crate) fn on_the_command_line(
    args: Res<crate::args::Args>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    server: Res<super::settings::ServerSettings>,
    mut queue: ResMut<super::queue::ServerQueue>,
    step: Res<Step>,
    mut done: Local<bool>,
) {
    if *done || !args.server.regenerate {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    let said = regenerate(&mut session, &assets, &server, &mut queue, &step, None);
    info!("--regenerate: {said}");
    session.status = said;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record round-trips, and a line that is not a path and a hash is
    /// not an entry.
    #[test]
    fn the_record_reads_back_what_it_wrote() {
        let mut record = Record::default();
        record.tiles.insert(r"World\Maps\Azeroth\Azeroth_32_48.adt".into(), 0xdead_beef);
        record.tiles.insert(r"World\Maps\Azeroth\Azeroth_32_49.adt".into(), 1);
        record.maps.insert("azeroth".into(), 0);
        let text = record.to_text("p");
        assert_eq!(Record::from_text(&text), record);
        assert_eq!(Record::from_text("# nothing\nbad line\n"), Record::default());
    }

    fn one(vpath: &str, map: u32, hash: Option<u64>, map_removed: bool) -> Dirty {
        Dirty {
            vpath: vpath.into(),
            tile: Tile::new(map, 1, 2),
            reach: Reach::Placements,
            hash,
            map_removed,
        }
    }

    /// A noted record holds the id of every map its tiles are on, taken from
    /// `Map.dbc` where it lists the map and kept where it does not, and
    /// forgets a map with its last tile.
    #[test]
    fn the_record_keeps_the_id_of_each_map() {
        let maps = vec![(0, "Azeroth".to_string())];
        let mut record = Record::default();
        record.tiles.insert(r"World\Maps\Azeroth\Azeroth_32_48.adt".into(), 1);
        record.tiles.insert(r"World\Maps\Mine\Mine_30_30.adt".into(), 2);
        record.maps.insert("mine".into(), 534);
        record.note(&[], &maps);
        assert_eq!(record.map_id("AZEROTH"), Some(0), "taken from Map.dbc");
        assert_eq!(record.map_id("Mine"), Some(534), "kept, though Map.dbc does not list it");

        record.note(&[one(r"World\Maps\Mine\Mine_30_30.adt", 534, None, true)], &maps);
        assert_eq!(record.map_id("Mine"), None, "forgotten with its last tile");
        assert_eq!(record.tiles.len(), 1);
    }

    /// A tile on a removed map goes to the removed maps, once per map, and
    /// not to the tiles the tools run for.
    #[test]
    fn a_removed_map_is_requested_once_and_not_built() {
        let dirty = vec![
            one("a", 0, Some(1), false),
            one("b", 0, None, false),
            one("c", 534, None, true),
            one("d", 534, None, true),
        ];
        let (tiles, carried, removed_maps) = split(&dirty);
        assert_eq!(tiles.len(), 2);
        assert_eq!(carried, vec![Tile::new(0, 1, 2)]);
        assert_eq!(removed_maps, vec![534]);
    }

    /// The live record is every tile the ledger holds, whichever project
    /// built it. A project's own regenerated.txt is read while the ledger
    /// holds none of its tiles, and is copied in once.
    #[test]
    fn the_live_record_is_the_ledger() {
        let root = std::env::temp_dir().join(format!("vale-live-record-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data_dir = root.join("DataDir");
        let project = Project::at(root.join("p")).unwrap();
        let maps = vec![(0, "Azeroth".to_string())];
        let old = r"World\Maps\Azeroth\Azeroth_32_48.adt";
        let other = r"World\Maps\Azeroth\Azeroth_33_48.adt";
        let mut record = Record::default();
        record.tiles.insert(old.into(), 7);
        record.write(&project, LIVE_RECORD).unwrap();

        let mut ledger = Ledger::open(&data_dir).unwrap();
        let built = Built {
            project: "q".into(),
            vpath: other.into(),
            hash: 9,
        };
        ledger.note_tile(Tile::new(0, 33, 48), Some(built)).unwrap();
        let view = ledger_record(&ledger, &project, &maps);
        assert_eq!(view.tiles.get(other), Some(&9), "another project's tile");
        assert_eq!(view.tiles.get(old), Some(&7), "the project's old record");

        import_live_record(&mut ledger, &project, &maps).unwrap();
        assert_eq!(ledger.tiles[&Tile::new(0, 32, 48)].project, "p");
        let reopened = Ledger::open(&data_dir).unwrap();
        assert_eq!(reopened.tiles.len(), 2, "written to the DataDir's ledger");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The comparison and the run are moved to the server queue's worker, so
    /// both have to be `Send`. A field added that is not fails to compile
    /// here rather than at the queue.
    #[test]
    fn what_the_worker_is_given_is_send() {
        fn send<T: Send>() {}
        send::<TileReader>();
        send::<Run>();
        send::<Result<Ran, String>>();
    }
}
