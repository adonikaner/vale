//! The server's tiles regenerated for the tiles a project changed: `maps\`,
//! `vmaps\` and `mmaps\`, through the four vmangos tools. There are two
//! callers. The dev-time buttons write the live `DataDir`, so a playtest walks
//! on the edit after a server restart: *Regenerate changed tiles* on the
//! Server panel, *Server files* on the map window, and `--regenerate`. A
//! publish writes a patch folder; see [`super::patch`].
//!
//! ## Which tiles
//!
//! A record holds, per tile the project carries, the hash of the ADT the
//! server's files were last built from. There is one record for the live
//! `DataDir` ([`LIVE_RECORD`]) and one for the patches ([`PATCH_RECORD`]). A
//! run compares the project against its record:
//!
//! ```text
//! a tile not in the record, or whose bytes changed     regenerated
//! a tile in the record the project no longer carries   regenerated from the
//!                                                       archives, which is
//!                                                       what the reverted
//!                                                       tile now is
//! a tile in the record with the same bytes             left alone
//! ```
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
//! run the tools       see `vale_mangos::datadir::regenerate`
//! write the record
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
use vale_assets::archive::{ChainPool, PooledChain};
use vale_mangos::datadir::{self, Outcome, Reach, Request, Tile, Tools};
use bevy::prelude::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where the project records what the server's tiles were built from. There
/// are two records because two things build them: a patch records under
/// [`PATCH_RECORD`] the tiles its folder holds, and the dev-time regenerate
/// records under [`LIVE_RECORD`] the tiles the live `DataDir` holds. One record
/// for both would let a dev regenerate hide a tile from the next patch, or the
/// reverse.
pub const PATCH_RECORD: &str = "publish\\server-tiles.txt";
pub const LIVE_RECORD: &str = "server\\regenerated.txt";

/// The record: `vpath -> FNV-1a of the ADT bytes`, sorted by path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    pub tiles: BTreeMap<String, u64>,
}

impl Record {
    pub fn read(project: &Project, vpath: &str) -> Record {
        let Some(bytes) = project.read(vpath) else {
            return Record::default();
        };
        Record::from_text(&String::from_utf8_lossy(&bytes))
    }

    pub fn from_text(text: &str) -> Record {
        let mut out = Record::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((vpath, hash)) = line.rsplit_once('\t') else {
                continue;
            };
            if let Ok(hash) = u64::from_str_radix(hash.trim(), 16) {
                out.tiles.insert(vpath.trim().to_string(), hash);
            }
        }
        out
    }

    pub fn to_text(&self, project: &str) -> String {
        let mut out = format!(
            "# {project} — the tiles the server's maps, vmaps and mmaps were last built\n\
             # from, as the FNV-1a 64 of each ADT's bytes. A publish regenerates every\n\
             # tile whose bytes differ, and every tile listed here that the project no\n\
             # longer carries.\n"
        );
        for (vpath, hash) in &self.tiles {
            out.push_str(&format!("{vpath}\t{hash:016x}\n"));
        }
        out
    }

    pub fn write(&self, project: &Project, vpath: &str) -> Result<(), String> {
        project
            .write(vpath, self.to_text(&project.name).as_bytes())
            .map(|_| ())
            .map_err(|e| format!("{vpath}: {e}"))
    }

    /// Record every tile of `done` as built from the bytes it was compared
    /// at, and forget a tile that has left the project.
    fn note(&mut self, done: &[Dirty]) {
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
    }
}

/// One tile to regenerate: where it is in the project, which server tile,
/// how far the change reaches, and the hash the record will hold. The hash is
/// `None` for a tile that has left the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirty {
    pub vpath: String,
    pub tile: Tile,
    pub reach: Reach,
    pub hash: Option<u64>,
}

/// `World\Maps\<Map>\<Map>_<x>_<y>.adt` to `(map name, x, y)`.
pub fn parse_tile_vpath(vpath: &str) -> Option<(String, u32, u32)> {
    let parts: Vec<&str> = vpath.split(['\\', '/']).collect();
    let [world, maps, map, file] = parts[..] else {
        return None;
    };
    if !world.eq_ignore_ascii_case("World") || !maps.eq_ignore_ascii_case("Maps") {
        return None;
    }
    let stem = file.strip_suffix(".adt").or_else(|| file.strip_suffix(".ADT"))?;
    let rest = stem.strip_prefix(map)?.strip_prefix('_')?;
    let (x, y) = rest.split_once('_')?;
    Some((map.to_string(), x.parse().ok()?, y.parse().ok()?))
}

/// What the comparison of tiles reads: the project folder, the archive chain,
/// and `Map.dbc`'s `(id, directory)` list. It owns all three and is `Send`, so
/// [`dirty`] runs on a worker as well as on the main thread.
///
/// The archives are read through a chain borrowed from the client's pool
/// (`GameAssets::chains`), as the tile loaders read them, because a worker
/// cannot hold `GameAssets`.
#[derive(Clone)]
pub struct TileReader {
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
    pub fn new(project: &Project, assets: &GameAssets) -> TileReader {
        TileReader {
            project: project.clone(),
            chains: assets.chains(),
            gamedata_dir: assets.gamedata_dir.clone(),
            maps: crate::session::map_directories(assets),
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
            .maps
            .iter()
            .find(|(_, dir)| dir.eq_ignore_ascii_case(map))
            .map(|(id, _)| *id)
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
            hash: Some(fnv1a(&bytes)),
        }))
    }
}

/// Which tiles a run regenerates, against `record`. A tile of a map that
/// `Map.dbc` does not name is skipped and named in the second list.
///
/// Every tile is read from the project and from the archives, parsed and
/// hashed, so the cost grows with the number of tiles the project carries.
/// `progress` is called before each tile with the number compared and the
/// total.
pub fn dirty(
    reader: &TileReader,
    record: &Record,
    progress: &mut dyn FnMut(usize, usize),
) -> (Vec<Dirty>, Vec<String>) {
    let carried: Vec<String> = reader
        .project
        .files()
        .into_iter()
        .filter(|vpath| parse_tile_vpath(vpath).is_some())
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
        let Some((map, x, y)) = parse_tile_vpath(vpath) else {
            continue;
        };
        let left_the_project = done >= carried.len();
        match reader.classify(&mut chain, &own, &map, x, y) {
            Ok(Some(one))
                if !left_the_project && record.tiles.get(vpath) == one.hash.as_ref() => {}
            Ok(Some(one)) => out.push(one),
            Ok(None) => {}
            Err(why) => skipped.push(why),
        }
    }
    (out, skipped)
}

/// The tiles of `map` asked for by name, whatever the record says: the map
/// window's selection. A tile the project carries is regenerated from it, and
/// one it does not from the archives, which puts back what a reverted tile
/// had. Answers what was skipped and why beside the list.
pub fn dirty_for(
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

/// FNV-1a 64, the hash `vale_edit::project` records archives by.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Where the tools are, from the environment or the panel, or the sentence to
/// show when neither says.
pub fn tools(server: &super::settings::ServerSettings) -> Result<Tools, String> {
    if let Some(found) = Tools::from_env() {
        return found;
    }
    let dir = server.tools.trim();
    if dir.is_empty() {
        return Err(format!(
            "no vmangos tools: set the folder on the Server panel, or {}",
            datadir::TOOLS_ENV
        ));
    }
    Tools::find(dir)
}

/// Where the server's `DataDir` is, through the conf the panel or the
/// environment names.
pub fn data_dir(server: &super::settings::ServerSettings) -> Result<PathBuf, String> {
    let conf = server
        .conf_path()
        .ok_or_else(|| "no mangosd.conf: set it on the Server panel, or VALE_MANGOSD".to_string())?;
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
pub struct Step(pub Arc<Mutex<(String, f32)>>);

impl Step {
    /// The step's name, or empty when nothing is running.
    pub fn line(&self) -> String {
        self.0.lock().map(|s| s.0.clone()).unwrap_or_default()
    }

    /// 0 to 1.
    pub fn fraction(&self) -> f32 {
        self.0.lock().map(|s| s.1).unwrap_or(0.0)
    }

    /// The line and the fraction together, for a bar, or `None` when nothing
    /// is running.
    pub fn summary(&self) -> Option<(String, f32)> {
        let held = self.0.lock().ok()?;
        (!held.0.is_empty()).then(|| held.clone())
    }

    /// Say which step, and how many of how many are done. A total of zero is
    /// a bar at nothing, for the steps that report no count.
    pub fn set(&self, line: &str, done: usize, total: usize) {
        if let Ok(mut held) = self.0.lock() {
            let fraction = match total {
                0 => 0.0,
                n => (done as f32 / n as f32).clamp(0.0, 1.0),
            };
            *held = (line.to_string(), fraction);
        }
    }

    pub fn clear(&self) {
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
            .path_for(super::patch::STAGE)
            .ok_or_else(|| "the staging folder leaves the project".to_string())?;
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

/// Which tiles a dev-time run regenerates.
enum Which {
    /// Every tile changed since the last run, against [`LIVE_RECORD`].
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
        /// The error from writing [`LIVE_RECORD`], if it could not be written.
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
        // screen are written there first.
        session.save_all();
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
        let record = Record::read(project, LIVE_RECORD);
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
                Which::Named { .. } => "none of the selection can be regenerated",
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
                step.set("staging a copy of the install for the tools", 0, 0);
                (staging.stage(), Some(staging), Vec::new())
            }
        };

        let request = client_root.map(|client_root| Request {
            client_root,
            tiles: dirty.iter().map(|d| (d.tile, d.reach)).collect(),
            heights_as_int: datadir::heights_as_int(&self.data_dir, dirty[0].tile.map),
            dll_dirs: self.dll_dirs,
        });
        let outcome = request.and_then(|request| {
            datadir::regenerate(&self.tools, &self.data_dir, &request, &self.scratch, |line, done, total| {
                step.set(line, done, total)
            })
        });
        let _ = std::fs::remove_dir_all(&self.scratch);
        if let Some(staging) = &staging {
            let _ = std::fs::remove_dir_all(&staging.into);
        }
        // A run that fails part way has copied the tiles before the failing
        // tool and not the ones after, and nothing says which. So a failed
        // run records nothing, and the next run does all of them again.
        let outcome = outcome?;
        let mut record = Record::read(project, LIVE_RECORD);
        record.note(&dirty);
        Ok(Ran::Done {
            outcome,
            recorded: record.write(project, LIVE_RECORD),
            left,
        })
    }
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
            if let Err(e) = recorded {
                line.push_str(&format!(
                    " — but {LIVE_RECORD} could not be written ({e}), so the next \
                     regeneration does them again"
                ));
            }
            for (name, why) in &left {
                line.push_str(&format!(
                    "; {name} could not be removed ({why}) and still holds the project's older \
                     files — close what has it open and regenerate again"
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
pub fn regenerate(
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
        return "none of the selection can be regenerated".to_string();
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
    format!("{label} \u{2014} the bar says which step is running")
}

/// `--regenerate`: the Server panel's *Regenerate changed tiles*, from a
/// command line. Fires once, on the first frame there is a session.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    server: Res<super::settings::ServerSettings>,
    mut queue: ResMut<super::queue::ServerQueue>,
    step: Res<Step>,
    mut done: Local<bool>,
) {
    if *done || !args.regenerate {
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

    #[test]
    fn a_tile_path_is_read_back_to_its_map_and_numbers() {
        assert_eq!(
            parse_tile_vpath(r"World\Maps\Azeroth\Azeroth_32_48.adt"),
            Some(("Azeroth".to_string(), 32, 48))
        );
        assert_eq!(
            parse_tile_vpath("World/Maps/Kalimdor/Kalimdor_3_57.adt"),
            Some(("Kalimdor".to_string(), 3, 57))
        );
        assert_eq!(parse_tile_vpath(r"World\Maps\Azeroth\Azeroth.wdt"), None);
        assert_eq!(parse_tile_vpath(r"DBFilesClient\Spell.dbc"), None);
        assert_eq!(parse_tile_vpath(r"World\Maps\Azeroth\Other_32_48.adt"), None);
    }

    /// The record round-trips, and a line that is not a path and a hash is
    /// not an entry.
    #[test]
    fn the_record_reads_back_what_it_wrote() {
        let mut record = Record::default();
        record.tiles.insert(r"World\Maps\Azeroth\Azeroth_32_48.adt".into(), 0xdead_beef);
        record.tiles.insert(r"World\Maps\Azeroth\Azeroth_32_49.adt".into(), 1);
        let text = record.to_text("p");
        assert_eq!(Record::from_text(&text), record);
        assert_eq!(Record::from_text("# nothing\nbad line\n"), Record::default());
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
