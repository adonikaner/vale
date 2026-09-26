//! **The server's tiles, regenerated** for the tiles a project changed:
//! `maps\`, `vmaps\` and `mmaps\`, through the four vmangos tools. Two
//! callers: the dev-time buttons, which write the live `DataDir` so a
//! playtest walks on the edit after a restart (*Regenerate changed tiles* on
//! the Server panel, *Server files* on the map window, `--regenerate`); and
//! a publish, which writes a patch folder — see [`super::patch`].
//!
//! ## Which tiles
//!
//! A record holds, per tile the project carries, the hash of the ADT the
//! server's files were last built from — one record for the live `DataDir`
//! ([`LIVE_RECORD`]) and one for the patches ([`PATCH_RECORD`]). A run
//! compares the project against its record:
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
//! How far a change reaches is read off the tile itself: a tile whose
//! placements are the archives' own — `AdtFile::same_placements` against
//! the shipped copy, read past the overlay and past this project's own
//! archive — is a terrain edit and takes the `.map` and the navmesh; any
//! other, and a tile the archives do not have, takes the vmap half of its
//! map as well. See `vale_mangos::datadir`, which is the driver and where
//! the costs are.
//!
//! ## The tools read an archive, never the project folder
//!
//! The extractors read a `Data\`, so a tile reaches them through a
//! `Patch-<X>.MPQ` and nowhere else. The dev-time run rebuilds the project's
//! own archive in the real `Data\` first; a publish stages a copy of the
//! install with the patch's archive in it. Either way a save's edit is in the
//! project folder, which the tools cannot see, so neither is a step of Save.
//!
//! ## It is a write on the server queue
//!
//! Minutes, off the main thread, one at a time with the database writes,
//! finished on the main thread with the record written and the status line
//! set — see [`super::queue`]. The toast says which step is running.

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::adt::AdtFile;
use vale_edit::project::Project;
use vale_mangos::datadir::{self, Reach, Request, Tile, Tools};
use bevy::prelude::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where the project records what the server's tiles were built from —
/// **two records**, because two things build them. A patch records under
/// [`PATCH_RECORD`] the tiles its folder holds; the dev-time regenerate
/// records under [`LIVE_RECORD`] the tiles the live `DataDir` holds. One
/// record for both would let a dev regenerate hide a tile from the next
/// patch, or the reverse.
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
}

/// One tile to regenerate: where it is in the project, which server tile,
/// how far, and the hash the record will hold — `None` for a tile that has
/// left the project.
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

/// **Which tiles a publish regenerates**, against the record. `maps` is
/// `Map.dbc`'s `(id, directory)` list, for the id the server files a tile
/// under; a tile of a map the table does not name is skipped and named in
/// the second list.
pub fn dirty(
    session: &EditSession,
    assets: &GameAssets,
    maps: &[(u32, String)],
    record: &Record,
) -> (Vec<Dirty>, Vec<String>) {
    let mut out: Vec<Dirty> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for vpath in session.project.files() {
        let Some((map, x, y)) = parse_tile_vpath(&vpath) else {
            continue;
        };
        seen.push(vpath.clone());
        match classify(session, assets, maps, &map, x, y) {
            Ok(Some(one)) if record.tiles.get(&vpath) == one.hash.as_ref() => {}
            Ok(Some(one)) => out.push(one),
            Ok(None) => {}
            Err(why) => skipped.push(why),
        }
    }
    // Tiles the record has and the project no longer carries.
    for vpath in record.tiles.keys() {
        if seen.iter().any(|s| s.eq_ignore_ascii_case(vpath)) {
            continue;
        }
        let Some((map, x, y)) = parse_tile_vpath(vpath) else {
            continue;
        };
        match classify(session, assets, maps, &map, x, y) {
            Ok(Some(one)) => out.push(one),
            Ok(None) => {}
            Err(why) => skipped.push(why),
        }
    }
    (out, skipped)
}

/// **The tiles asked for by name**, whatever the record says: the map
/// window's selection. A tile the project carries is regenerated from it,
/// and one it does not from the archives, which puts back what a reverted
/// tile had. Answers what was skipped and why beside the list.
pub fn dirty_for(
    session: &EditSession,
    assets: &GameAssets,
    maps: &[(u32, String)],
    chosen: &[(u32, u32)],
) -> (Vec<Dirty>, Vec<String>) {
    let mut out: Vec<Dirty> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for &(x, y) in chosen {
        match classify(session, assets, maps, &session.map, x, y) {
            Ok(Some(one)) => out.push(one),
            Ok(None) => skipped.push(format!(
                "{}: neither this project nor the archives have it",
                vale_assets::adt_path(&session.map, x, y)
            )),
            Err(why) => skipped.push(why),
        }
    }
    (out, skipped)
}

/// **One tile, as a publish would regenerate it**: from the project when it
/// carries the tile, with how far the change reaches; from the archives when
/// it does not, whole. `None` when neither has it; `Err` when the map is not
/// in `Map.dbc`.
fn classify(
    session: &EditSession,
    assets: &GameAssets,
    maps: &[(u32, String)],
    map: &str,
    x: u32,
    y: u32,
) -> Result<Option<Dirty>, String> {
    let vpath = vale_assets::adt_path(map, x, y);
    let id = maps
        .iter()
        .find(|(_, dir)| dir.eq_ignore_ascii_case(map))
        .map(|(id, _)| *id)
        .ok_or_else(|| format!("{vpath}: {map} is not a map in Map.dbc"))?;
    let own: Vec<String> = session
        .project
        .published()
        .into_iter()
        .map(|archive| archive.name)
        .collect();
    let shipped = assets.read_past_overlay_skipping(&vpath, &own).ok();
    let Some(bytes) = session.project.read(&vpath) else {
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

/// FNV-1a 64, the hash `vale_edit::project` records archives by.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// **Where the tools are**, from the environment or the panel — `None` when
/// neither says, with the sentence to show.
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

/// **Which step the tools are on, and how far along**: the line the bar's
/// hover and the spinner's hover show, and the fraction the bar fills to.
/// Written by the worker, read by the panels. Empty between runs.
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
    /// a bar at nothing, for the steps before the tools start.
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

/// **The archive the tools read, when it is not to be put in `Data\`.**
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
    /// data: linked so the staging function has a file to link, then
    /// removed, so the tools read the install less this project's older
    /// archives.
    empty: bool,
    /// The staging folder, under the project's `publish\`.
    into: PathBuf,
}

impl Staging {
    /// **Build the project's archive where the tools can be given it**: under
    /// `publish\.stage\archive\`, under the letter it would take in `Data\`.
    /// Saves first, as writing the archive into `Data\` would.
    fn prepare(session: &mut EditSession, assets: &GameAssets) -> Result<Staging, String> {
        session.save_all();
        let data_dir = std::path::absolute(&assets.gamedata_dir).map_err(|e| e.to_string())?;
        let into = session
            .project
            .path_for(super::patch::STAGE)
            .ok_or_else(|| "the staging folder leaves the project".to_string())?;
        let _ = std::fs::remove_dir_all(&into);
        let as_name = vale_edit::project::next_patch_name(&data_dir)
            .unwrap_or_else(|| "Patch-Z.MPQ".to_string());
        let archive = into.join("archive").join(&as_name);
        let count = session
            .project
            .publish(&archive)
            .map_err(|e| format!("{}: {e}", archive.display()))?;
        let empty = count == 0;
        if empty {
            if let Some(parent) = archive.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&archive, b"").map_err(|e| e.to_string())?;
        }
        let own: Vec<String> = session.project.published().into_iter().map(|a| a.name).collect();
        Ok(Staging {
            data_dir,
            own,
            archive,
            as_name,
            empty,
            into,
        })
    }
}

/// **The regeneration of every tile changed since the last**, as one write on
/// the server queue — what a publish runs. `None` when there is nothing to
/// do; `Err` with the reason when there is no way to do it.
fn work(
    session: &EditSession,
    assets: &GameAssets,
    server: &super::settings::ServerSettings,
    step: &Step,
    staging: Option<Staging>,
) -> Result<Option<(String, super::queue::Work)>, String> {
    let maps = crate::session::map_directories(assets);
    let record = Record::read(&session.project, LIVE_RECORD);
    let (dirty, skipped) = dirty(session, assets, &maps, &record);
    for line in &skipped {
        warn!("server tiles: {line}");
    }
    work_over(session, server, assets, step, dirty, staging)
}

/// **…and of the tiles named**, whatever the record says — the map window's
/// selection. The archive is what the tools read, so the caller rebuilds it
/// first; see `ui::mapview`.
fn work_for(
    session: &EditSession,
    assets: &GameAssets,
    server: &super::settings::ServerSettings,
    step: &Step,
    chosen: &[(u32, u32)],
    staging: Option<Staging>,
) -> Result<Option<(String, super::queue::Work)>, String> {
    let maps = crate::session::map_directories(assets);
    let (dirty, skipped) = dirty_for(session, assets, &maps, chosen);
    for line in &skipped {
        warn!("server tiles: {line}");
    }
    work_over(session, server, assets, step, dirty, staging)
}

/// The write itself, over a list already decided.
///
/// Everything the worker needs is gathered here: the tools checked for the
/// patch, the two folders, and a copy of the project handle for the record.
/// `staging` is where the archive is, when it is not in `Data\`; the worker
/// stages the install before the tools run and removes the staging after.
/// The finish writes the record for every tile that was regenerated and puts
/// the outcome on the status line.
fn work_over(
    session: &EditSession,
    server: &super::settings::ServerSettings,
    assets: &GameAssets,
    step: &Step,
    dirty: Vec<Dirty>,
    staging: Option<Staging>,
) -> Result<Option<(String, super::queue::Work)>, String> {
    if dirty.is_empty() {
        return Ok(None);
    }
    let tools = tools(server)?;
    tools.check_patched()?;
    let data_dir = data_dir(server)?;
    let real_root = match staging.is_some() {
        true => None,
        false => Some(client_root(assets)?),
    };
    let first_map = dirty[0].tile.map;
    // The server's own folder is where its runtime DLLs are, and the Release
    // assembler and generator want them.
    let dll_dirs: Vec<PathBuf> = server
        .conf_path()
        .and_then(|conf| conf.parent().map(Path::to_path_buf))
        .into_iter()
        .collect();
    let tiles: Vec<(Tile, Reach)> = dirty.iter().map(|d| (d.tile, d.reach)).collect();
    let heights_as_int = datadir::heights_as_int(&data_dir, first_map);
    let scratch = std::env::temp_dir().join(format!("vale-server-tiles-{}", std::process::id()));
    let project = session.project.clone();
    let progress = step.clone();
    let label = format!(
        "regenerating {} server tile(s)",
        dirty.len()
    );
    let terrain_only = dirty.iter().filter(|d| d.reach == Reach::Terrain).count();
    info!(
        "server tiles: {} to regenerate ({terrain_only} terrain-only) into {}",
        dirty.len(),
        data_dir.display()
    );
    for d in &dirty {
        info!("  {} -> map {} tile {},{} ({:?})", d.vpath, d.tile.map, d.tile.x, d.tile.y, d.reach);
    }
    let work: super::queue::Work = Box::new(move || {
        let client_root = match (&staging, real_root) {
            (Some(stage), _) => {
                progress.set("staging a copy of the install for the tools", 0, 0);
                datadir::stage_client(
                    &stage.data_dir,
                    &stage.own,
                    &stage.archive,
                    &stage.as_name,
                    &stage.into.join("client"),
                )
                .map(|root| {
                    if stage.empty {
                        let _ = std::fs::remove_file(root.join("Data").join(&stage.as_name));
                    }
                    root
                })
            }
            (None, Some(root)) => Ok(root),
            (None, None) => Err("no install to read".to_string()),
        };
        let outcome = client_root.and_then(|client_root| {
            let request = Request {
                client_root,
                tiles,
                heights_as_int,
                dll_dirs,
            };
            datadir::regenerate(&tools, &data_dir, &request, &scratch, |line, done, total| {
                progress.set(line, done, total)
            })
        });
        let _ = std::fs::remove_dir_all(&scratch);
        if let Some(stage) = &staging {
            let _ = std::fs::remove_dir_all(&stage.into);
        }
        progress.clear();
        // The record: every tile the run reached. On a failure part way, the
        // tiles before the failing tool are copied and the ones after are
        // not, and nothing says which — so a failed run records nothing and
        // the next publish does it all again.
        let recorded: Result<Record, String> = match &outcome {
            Ok(_) => {
                let mut record = Record::read(&project, LIVE_RECORD);
                for d in &dirty {
                    match d.hash {
                        Some(hash) => {
                            record.tiles.insert(d.vpath.clone(), hash);
                        }
                        None => {
                            record.tiles.remove(&d.vpath);
                        }
                    }
                }
                record.write(&project, LIVE_RECORD).map(|_| record)
            }
            Err(e) => Err(e.clone()),
        };
        Box::new(move |session: &mut EditSession, _: &mut super::reload::Reloads| {
            match (&outcome, &recorded) {
                (Ok(done), Ok(_)) => {
                    for line in &done.log {
                        info!("server tiles: {line}");
                    }
                    for (name, why) in &done.mmaps_failed {
                        warn!("server tiles: {name}: {why}");
                    }
                    for (what, seconds) in &done.timings {
                        info!("server tiles: {what}: {seconds:.1} s");
                    }
                    session.status = done.line();
                }
                (Ok(done), Err(e)) => {
                    session.status = format!(
                        "{} — but {LIVE_RECORD} could not be written ({e}), so the next publish \
                         regenerates them again",
                        done.line()
                    );
                }
                (Err(e), _) => {
                    warn!("server tiles: {e}");
                    session.status = format!("server tiles: {e}");
                }
            }
        })
    });
    Ok(Some((label, work)))
}

/// **Regenerate, on its own**: the Server panel's button, the map window's,
/// and `--regenerate`. `chosen` is the tiles to do whatever the record says,
/// or `None` for every tile changed since the last time.
///
/// **The archive is rebuilt first.** The extractors read `Data\`, so a tile
/// reaches them through the project's own archive and through nothing else;
/// rebuilding it is seconds. With *Copy the client archive into Data* on it
/// is written into the real `Data\`, replacing only this project's own;
/// otherwise it is built under the project's `publish\.stage\` and the tools
/// are given a staged copy of the install with it beside the real archives —
/// see [`Staging`]. Not while a playtest is running, for the reason Publish is
/// not — the running game has the chain open.
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
    let staging = match server.copy_archive {
        true => {
            let data = assets.gamedata_dir.clone();
            if session.publish_archive(&data).is_none()
                && !session.status.starts_with("nothing to publish")
            {
                return session.status.clone();
            }
            None
        }
        false => match Staging::prepare(session, assets) {
            Ok(staging) => Some(staging),
            Err(e) => return format!("server tiles: {e}"),
        },
    };
    let work = match chosen {
        Some(chosen) => work_for(session, assets, server, step, chosen, staging),
        None => work(session, assets, server, step, staging),
    };
    match work {
        Ok(Some((label, work))) => {
            queue.push(label.clone(), work);
            format!("{label} \u{2014} the bar says which step is running")
        }
        Ok(None) => match chosen {
            Some(_) => "none of the selection can be regenerated".to_string(),
            None => "no server tile has changed since the last regeneration".to_string(),
        },
        Err(e) => format!("server tiles: {e}"),
    }
}

/// **`--regenerate`**: the Server panel's *Regenerate changed tiles*, from a
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
}
