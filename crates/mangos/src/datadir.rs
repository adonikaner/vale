//! The server's `DataDir`: the derived tiles it walks on, and driving the
//! four vmangos tools that build them from the client's archives.
//!
//! ## What is in it
//!
//! `mangosd.conf`'s `DataDir` holds the world data the server reads in place
//! of the archives, one file per tile. The vmangos extractors build these
//! files once, from the same ADTs the client draws:
//!
//! ```text
//! maps\MMMYYXX.map       heights, liquid, holes and area ids of one tile
//! vmaps\MMM.vmtree       the map's buildings and models as a bounding tree
//! vmaps\MMM_XX_YY.vmtile the buildings and models that stand on one tile
//! vmaps\<model>.vmo      the triangles of one model
//! mmaps\MMM.mmap         the map's navmesh parameters
//! mmaps\MMMYYXX.mmtile   one tile's navmesh, the surface a creature walks
//!                        on
//! 5875\dbc\*.dbc         48 client tables the server reads as files; see
//!                       [`dbc_dir`]
//! ```
//!
//! `MMM` is the map id, and `XX`, `YY` are the tile's own `_x_y` from its ADT
//! name. The `.map` and `.mmtile` names order them `YYXX` and the `.vmtile`
//! name orders them `_XX_YY`. The tools choose these names, and this module
//! uses them unchanged.
//!
//! The four tools and the arguments each takes, read from `contrib/` of a
//! vmangos checkout on 2026-09-22:
//!
//! ```text
//! mapextractor    -i <client root> -o <out> -e 1 -f <0|1>   every map, every tile
//! vmapextractor   -d <client root>\Data\                   every map, into .\Buildings
//! VMapAssembler   <Buildings> <vmaps>                       what dir_bin names, into <vmaps>
//! MoveMapGenerator <map> --tile x,y --silent --threads 1    one tile, from .\maps and .\vmaps
//!                                                           into .\mmaps
//! ```
//!
//! ## What the extractor patch adds to the tools
//!
//! The patch adds three things the shipped tools cannot do, each in two or
//! three hunks:
//!
//! * Reading a lettered patch archive. The map extractor opens five named
//!   archives and the vmap extractor scans `patch-1..99`; neither opens
//!   `Patch-A.MPQ`, which is where a published project is. With the patch,
//!   both open `patch-3..9` and `Patch-A..Z` after the rest, in the client's
//!   own order, so the last archive wins as it does in the client.
//! * Limiting a run to a map or a tile: `mapextractor -m <id> -t x,y` (both
//!   repeatable) and `vmapextractor -m <id>`. Without the patch every run
//!   covers every map, and a vmap extractor run covers the buildings of the
//!   whole world.
//! * Extracting a building when a tile names it, under `-m`, instead of first
//!   sweeping every archive's listing for `.wmo` files. That sweep made a
//!   per-map run cost as much as a whole-world run.
//!
//! [`Tools::check_patched`] runs each extractor's usage and looks for the
//! option, so an unpatched tool is refused with its path instead of run.
//!
//! ## What one tile costs, measured on this machine
//!
//! ```text
//! mapextractor, two tiles of Azeroth            0.3 s   Debug build
//! vmapextractor -m 0, all of Azeroth           74 s     Debug; 2,136 files
//! VMapAssembler over that                      61 s     Debug; 375 vmtiles, 1,874 vmo
//! MoveMapGenerator, Azeroth 32,48             222 s     Release; the Debug build
//!                                                       had not finished it in 15 min
//! ```
//!
//! A terrain-only edit therefore costs about a second of extraction plus one
//! navmesh build for the tile and one for each of its four neighbours. A
//! moved building adds two minutes for the map. The vmap step runs only for a
//! tile whose placements changed (see [`Reach`]), and once per map however
//! many tiles asked for it. The navmesh builds run [`PARALLEL`] at a time.
//! The tools setting must point at a Release build: the generator is Recast
//! and Detour, which are too slow to use unoptimised.
//!
//! ## The vmap files of a map are replaced together
//!
//! The assembler numbers a map's spawns in the order it meets them, and the
//! `.vmtree` indexes tiles by that number, so a `.vmtile` from one run does
//! not agree with a `.vmtree` from another. Measured: one run over unchanged
//! archives wrote a `000_03_57.vmtile` identical to the server's and a
//! `000_32_49.vmtile` that differs from it, for a tile with no changes. The
//! map's tree and all of its tiles are therefore copied together, and a
//! `.vmo` is copied only where its bytes differ from the file already there.
//!
//! ## A navmesh tile reads its neighbours' edges
//!
//! `TerrainBuilder::loadMap` opens the four orthogonal neighbours' `.map`
//! files and stitches their border strip into the tile being built, so a
//! changed tile changes the navmesh of the four tiles around it. Each of
//! those is rebuilt as well, when it has a `.map`. The generator writes the
//! map's `.mmap` header from every tile it finds under `maps\` and `vmaps\`,
//! so it is run from `DataDir` itself and not from a scratch folder: a
//! scratch folder holding one tile would produce a header sized for one tile.
//!
//! ## What a regenerated tile needs before the server uses it
//!
//! This module does not reload the running server. With `GridUnload = 0`, as
//! on the reference install, a grid stays loaded until restart once it is
//! loaded, and no GM command re-reads one. The GM command `.mmap unload`
//! drops a navmesh tile, and the next step onto that tile re-reads the file.
//! New `maps` and `vmaps` files need a server restart, and the report says
//! so.

use crate::conn::setting;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

/// `mangosd.conf`'s `DataDir`. A relative value is resolved against the
/// conf's own folder, as `mangosd` resolves it. An absent key reads as `.`,
/// which is vmangos' default.
pub fn from_conf(conf: impl AsRef<Path>) -> Option<PathBuf> {
    let conf = conf.as_ref();
    let raw = setting(conf, "DataDir").unwrap_or_else(|| ".".to_string());
    let raw = raw.trim().trim_matches('"').trim();
    let dir = PathBuf::from(raw);
    let dir = match dir.is_absolute() {
        true => dir,
        false => conf.parent()?.join(dir),
    };
    Some(dir)
}

/// The environment variable naming the folder that holds the four tools. It
/// takes precedence over the editor's own setting, for the same reason
/// `VALE_MANGOSD` does: a scripted run has to be reproducible.
pub const TOOLS_ENV: &str = "VALE_VMANGOS_TOOLS";

/// The paths of the four tools, once found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub map_extractor: PathBuf,
    pub vmap_extractor: PathBuf,
    pub assembler: PathBuf,
    pub mmap_generator: PathBuf,
    /// `offmesh.txt` and `config.json`, the generator's two optional inputs,
    /// when found beside the tools or in the checkout's `contrib\mmap\`. The
    /// generator runs without them and reports that they are missing; the
    /// shipped navmeshes were built with them.
    pub off_mesh: Option<PathBuf>,
    pub config: Option<PathBuf>,
}

/// The names each tool is built under, in the order tried: the CMake target
/// and the older lower-case name the same build tree writes beside it.
pub const NAMES: [(&str, &[&str]); 4] = [
    ("map extractor", &["MapExtractor.exe", "mapextractor.exe", "mapextractor"]),
    ("vmap extractor", &["VMapExtractor.exe", "vmapextractor.exe", "vmapextractor"]),
    ("vmap assembler", &["VMapAssembler.exe", "vmap_assembler.exe", "vmap_assembler"]),
    ("movemap generator", &["MoveMapGenerator.exe", "MoveMapGen.exe", "MoveMapGenerator"]),
];

impl Tools {
    /// Finds the four tools in one folder, or returns an error naming the
    /// ones that are missing.
    pub fn find(dir: impl AsRef<Path>) -> Result<Tools, String> {
        let dir = dir.as_ref();
        let mut found: Vec<PathBuf> = Vec::new();
        let mut missing: Vec<&str> = Vec::new();
        for (what, names) in NAMES {
            match names.iter().map(|name| dir.join(name)).find(|path| path.is_file()) {
                Some(path) => found.push(path),
                None => missing.push(what),
            }
        }
        if !missing.is_empty() {
            return Err(format!(
                "{} has no {}",
                dir.display(),
                missing.join(", ")
            ));
        }
        let mut tools = Tools {
            map_extractor: found[0].clone(),
            vmap_extractor: found[1].clone(),
            assembler: found[2].clone(),
            mmap_generator: found[3].clone(),
            off_mesh: None,
            config: None,
        };
        // Look beside the tools, then in `contrib\mmap\` one folder up and
        // two folders up. Those are the checkout's own copies when the tools
        // are in `core\bin\` or in `core\bin\Debug\`.
        let mut places: Vec<PathBuf> = vec![dir.to_path_buf()];
        for up in [dir.parent(), dir.parent().and_then(Path::parent)] {
            if let Some(up) = up {
                places.push(up.join("contrib").join("mmap"));
            }
        }
        for place in places {
            if tools.off_mesh.is_none() && place.join("offmesh.txt").is_file() {
                tools.off_mesh = Some(place.join("offmesh.txt"));
            }
            if tools.config.is_none() && place.join("config.json").is_file() {
                tools.config = Some(place.join("config.json"));
            }
        }
        Ok(tools)
    }

    /// Finds the four tools in the folder [`TOOLS_ENV`] names, or returns
    /// `None` when the variable is not set.
    pub fn from_env() -> Option<Result<Tools, String>> {
        let dir = std::env::var(TOOLS_ENV).ok()?;
        Some(Tools::find(dir))
    }

    /// Checks whether both extractors were built with the patch, by looking
    /// for the `-m` option in the usage text each one prints.
    ///
    /// The map extractor prints usage and exits on any argument that does
    /// not start with a dash. It ignores a dash option it does not know, and
    /// `-?` alone once ran a whole-world extraction into the editor's working
    /// directory, so it is called with `help`. The vmap extractor prints
    /// usage on `-?`. Both run in the temp folder, which has no `Data\`, so a
    /// tool that does not stop at the usage text has nothing to read.
    pub fn check_patched(&self) -> Result<(), String> {
        let usage = |exe: &Path, args: &[&str]| -> String {
            Command::new(exe)
                .args(args)
                .current_dir(std::env::temp_dir())
                .stdin(Stdio::null())
                .output()
                .map(|o| {
                    let mut text = String::from_utf8_lossy(&o.stdout).into_owned();
                    text.push_str(&String::from_utf8_lossy(&o.stderr));
                    text
                })
                .unwrap_or_default()
        };
        let mut unpatched: Vec<String> = Vec::new();
        if !usage(&self.map_extractor, &["help"]).contains("-m extract only this map") {
            unpatched.push(self.map_extractor.display().to_string());
        }
        if !usage(&self.vmap_extractor, &["-?"]).contains("-m <map id>") {
            unpatched.push(self.vmap_extractor.display().to_string());
        }
        match unpatched.is_empty() {
            true => Ok(()),
            false => Err(format!(
                "not built with the vmangos extractor patch: {}",
                unpatched.join(", ")
            )),
        }
    }
}

/// One tile, identified by the numbers in the server's file names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tile {
    pub map: u32,
    pub x: u32,
    pub y: u32,
}

impl Tile {
    pub fn new(map: u32, x: u32, y: u32) -> Tile {
        Tile { map, x, y }
    }

    /// `maps\0004832.map` for Azeroth 32,48.
    pub fn map_file(&self) -> String {
        format!("{:03}{:02}{:02}.map", self.map, self.y, self.x)
    }

    /// `vmaps\000_32_48.vmtile`.
    pub fn vmtile_file(&self) -> String {
        format!("{:03}_{:02}_{:02}.vmtile", self.map, self.x, self.y)
    }

    /// `mmaps\0004832.mmtile`.
    pub fn mmtile_file(&self) -> String {
        format!("{:03}{:02}{:02}.mmtile", self.map, self.y, self.x)
    }

    /// The orthogonal neighbours whose navmesh reads this tile's edge: four,
    /// or fewer at the edge of the 64x64 grid.
    pub fn neighbours(&self) -> Vec<Tile> {
        let mut out = Vec::new();
        for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let x = self.x as i32 + dx;
            let y = self.y as i32 + dy;
            if (0..64).contains(&x) && (0..64).contains(&y) {
                out.push(Tile::new(self.map, x as u32, y as u32));
            }
        }
        out
    }
}

/// `vmaps\000.vmtree`.
pub fn vmtree_file(map: u32) -> String {
    format!("{map:03}.vmtree")
}

/// `mmaps\000.mmap`.
pub fn mmap_file(map: u32) -> String {
    format!("{map:03}.mmap")
}

/// Which of the derived files a tile's change affects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reach {
    /// Heights, liquid, holes, area ids: the `.map` and the `.mmtile`.
    Terrain,
    /// A model or a building placed, moved or removed: the vmap files as
    /// well, which are regenerated for the whole map.
    Placements,
}

/// What to regenerate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The client install, holding `Data\`. The map extractor takes this and
    /// appends `Data`; the vmap extractor takes `Data\` itself.
    pub client_root: PathBuf,
    /// The tiles, each with the [`Reach`] of its change.
    pub tiles: Vec<(Tile, Reach)>,
    /// `mapextractor -f`: store heights as `int16` where the range allows,
    /// which halves the file. [`heights_as_int`] reads it from an existing
    /// `.map`, so a regenerated tile is stored the same way as its
    /// neighbours.
    pub heights_as_int: bool,
    /// Folders added to `PATH` while the tools run. The assembler and the
    /// generator load DLLs (`ACE.dll`, `tbb*.dll`) that sit beside
    /// `mangosd.exe`, not beside the tools. The tools' own folder is always
    /// added. A generator that cannot find a DLL exits with `0xc0000135` and
    /// no output.
    pub dll_dirs: Vec<PathBuf>,
}

/// Whether the `.map` files already in `data_dir` store heights as
/// integers. Reads the `MHGT` header's flag from the first file found for
/// the map, or from any file when the map has none. Returns `true` when
/// there are no files, which is the extractor's own recommendation and the
/// reference install's setting.
pub fn heights_as_int(data_dir: &Path, map: u32) -> bool {
    let maps = data_dir.join("maps");
    let Ok(entries) = std::fs::read_dir(&maps) else {
        return true;
    };
    let prefix = format!("{map:03}");
    let mut fallback: Option<bool> = None;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".map") {
            continue;
        }
        let Some(flag) = map_heights_flag(&entry.path()) else {
            continue;
        };
        if name.starts_with(&prefix) {
            return flag;
        }
        fallback.get_or_insert(flag);
    }
    fallback.unwrap_or(true)
}

/// The `.map` header is `GridMapFileHeader`'s ten words: magic, version,
/// then `(offset, size)` for the area, height, liquid and holes blocks. The
/// height block starts with `MHGT` and a flags word whose bit 1 is
/// `MAP_HEIGHT_AS_INT16` and bit 2 `MAP_HEIGHT_AS_INT8` (`GridMapDefines.h`).
fn map_heights_flag(path: &Path) -> Option<bool> {
    let bytes = std::fs::read(path).ok()?;
    let word = |at: usize| -> Option<u32> {
        bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if bytes.get(0..4) != Some(b"MAPS") {
        return None;
    }
    let height_offset = word(16)? as usize;
    if bytes.get(height_offset..height_offset + 4) != Some(b"MHGT") {
        return None;
    }
    let flags = word(height_offset + 4)?;
    // A flat tile stores no heights at all (`MAP_HEIGHT_NO_HEIGHT`), so its
    // flags do not show how the other tiles store theirs.
    if flags & 0b1 != 0 {
        return None;
    }
    Some(flags & 0b110 != 0)
}

/// What a run did, for the status line and the record.
#[derive(Debug, Default, Clone)]
pub struct Outcome {
    /// `.map` files written into `DataDir\maps\`, by name.
    pub maps: Vec<String>,
    /// `.map` files removed from `DataDir\maps\`, for tiles the archives no
    /// longer contain.
    pub maps_removed: Vec<String>,
    /// `.vmtree` and `.vmtile` files written, by name.
    pub vmaps: Vec<String>,
    /// `.vmo` model files written, because their bytes differed, by name.
    pub models: Vec<String>,
    /// `.mmtile` files written.
    pub mmaps: Vec<String>,
    /// Navmesh tiles requested and not produced, each with the generator's
    /// last output lines.
    pub mmaps_failed: Vec<(String, String)>,
    /// Each tool's wall time, in the order run.
    pub timings: Vec<(String, f32)>,
    /// The tools' own output, kept for the log.
    pub log: Vec<String>,
}

impl Outcome {
    /// One line for a status bar.
    pub fn line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.maps.is_empty() {
            parts.push(format!("{} .map", self.maps.len()));
        }
        if !self.maps_removed.is_empty() {
            parts.push(format!("{} .map removed", self.maps_removed.len()));
        }
        if !self.vmaps.is_empty() {
            parts.push(format!("{} vmap files, {} models", self.vmaps.len(), self.models.len()));
        }
        if !self.mmaps.is_empty() {
            parts.push(format!("{} .mmtile", self.mmaps.len()));
        }
        if !self.mmaps_failed.is_empty() {
            parts.push(format!("{} navmesh tile(s) FAILED", self.mmaps_failed.len()));
        }
        let seconds: f32 = self.timings.iter().map(|(_, s)| s).sum();
        match parts.is_empty() {
            true => "no server tile to regenerate".to_string(),
            false => format!(
                "server tiles: {} in {seconds:.0} s — restart the server to walk on them",
                parts.join(", ")
            ),
        }
    }
}

/// Regenerates the requested tiles into `data_dir`, running the tools in
/// `scratch`, which is emptied first. `progress` is called as each step
/// starts, with the step's name, the number of steps finished, and the total
/// number of steps: one per map for the `.map` files, two per map that needs
/// the vmap files, and one per navmesh tile. The number of navmesh tiles is
/// an estimate until the navmesh step starts, because the `.map` step may
/// write a neighbour's `.map`.
///
/// The steps run in dependency order: every map's `.map` files first, then
/// the vmap files for each map with a placement change, then one navmesh
/// build per tile and per neighbour. A failing tool stops the run and the
/// error carries its last output lines. Files copied before the failure are
/// kept, because each file is complete and correct on its own.
pub fn regenerate(
    tools: &Tools,
    data_dir: &Path,
    request: &Request,
    scratch: &Path,
    mut progress: impl FnMut(&str, usize, usize),
) -> Result<Outcome, String> {
    let mut out = Outcome::default();
    if request.tiles.is_empty() {
        return Ok(out);
    }
    let _ = std::fs::remove_dir_all(scratch);
    std::fs::create_dir_all(scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    for sub in ["maps", "vmaps", "mmaps"] {
        std::fs::create_dir_all(data_dir.join(sub))
            .map_err(|e| format!("{}: {e}", data_dir.join(sub).display()))?;
    }
    let mut by_map: BTreeMap<u32, Vec<(Tile, Reach)>> = BTreeMap::new();
    for (tile, reach) in &request.tiles {
        by_map.entry(tile.map).or_default().push((*tile, *reach));
    }
    let data_arg = request.client_root.join("Data");

    // The step count for the progress bar. The navmesh tiles are estimated
    // as every requested tile plus every neighbour that has a `.map` or is
    // one of the requested tiles.
    let placement_maps = by_map
        .values()
        .filter(|tiles| tiles.iter().any(|(_, reach)| *reach == Reach::Placements))
        .count();
    let mut estimated: Vec<Tile> = Vec::new();
    for (tile, _) in &request.tiles {
        for candidate in std::iter::once(*tile).chain(tile.neighbours()) {
            let asked = request.tiles.iter().any(|(t, _)| *t == candidate);
            if !estimated.contains(&candidate)
                && (asked || data_dir.join("maps").join(candidate.map_file()).is_file())
            {
                estimated.push(candidate);
            }
        }
    }
    let mut total = by_map.len() + 2 * placement_maps + estimated.len();
    let mut step = 0usize;

    // 1. The .map files: one extractor run per map, covering all of that
    // map's requested tiles.
    for (map, tiles) in &by_map {
        progress(&format!("extracting {} .map file(s) of map {map}", tiles.len()), step, total);
        step += 1;
        let out_dir = scratch.join(format!("maps-{map:03}"));
        std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
        let mut command = Command::new(&tools.map_extractor);
        command
            .arg("-e")
            .arg("1")
            .arg("-f")
            .arg(match request.heights_as_int {
                true => "1",
                false => "0",
            })
            .arg("-m")
            .arg(map.to_string())
            .arg("-i")
            .arg(&request.client_root)
            .arg("-o")
            .arg(&out_dir);
        for (tile, _) in tiles {
            command.arg("-t").arg(format!("{},{}", tile.x, tile.y));
        }
        let started = Instant::now();
        run(&mut command, &out_dir, tools, &request.dll_dirs, &mut out.log)?;
        out.timings.push((format!("mapextractor map {map}"), started.elapsed().as_secs_f32()));
        for (tile, _) in tiles {
            let name = tile.map_file();
            let produced = out_dir.join("maps").join(&name);
            let target = data_dir.join("maps").join(&name);
            match produced.is_file() {
                true => {
                    copy_replacing(&produced, &target)?;
                    out.maps.push(name);
                }
                // The archives no longer contain the tile: the project
                // created it and has since reverted it. The server's file
                // for it is removed.
                false => {
                    if target.is_file() {
                        std::fs::remove_file(&target).map_err(|e| format!("{}: {e}", target.display()))?;
                        out.maps_removed.push(name);
                    }
                }
            }
        }
    }

    // 2. The vmap files, once per map that has a placement change.
    for (map, tiles) in &by_map {
        if !tiles.iter().any(|(_, reach)| *reach == Reach::Placements) {
            continue;
        }
        progress(&format!("extracting the buildings and models of map {map}"), step, total);
        step += 1;
        let work = scratch.join(format!("vmap-{map:03}"));
        std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
        let mut command = Command::new(&tools.vmap_extractor);
        command.arg("-d").arg(&data_arg).arg("-m").arg(map.to_string());
        let started = Instant::now();
        run(&mut command, &work, tools, &request.dll_dirs, &mut out.log)?;
        out.timings.push((format!("vmapextractor map {map}"), started.elapsed().as_secs_f32()));

        progress(&format!("assembling the vmaps of map {map}"), step, total);
        step += 1;
        let buildings = work.join("Buildings");
        let vmaps = work.join("vmaps");
        std::fs::create_dir_all(&vmaps).map_err(|e| e.to_string())?;
        let mut command = Command::new(&tools.assembler);
        command.arg(&buildings).arg(&vmaps);
        let started = Instant::now();
        run(&mut command, &work, tools, &request.dll_dirs, &mut out.log)?;
        out.timings.push((format!("VMapAssembler map {map}"), started.elapsed().as_secs_f32()));

        // Copy the tree and every tile of the map together (see the module
        // documentation), and a model only where its bytes differ.
        let prefix = format!("{map:03}");
        let mut produced_tiles: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&vmaps).map_err(|e| e.to_string())?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let target = data_dir.join("vmaps").join(&name);
            if name == vmtree_file(*map) || (name.starts_with(&prefix) && name.ends_with(".vmtile")) {
                copy_replacing(&path, &target)?;
                out.vmaps.push(name.clone());
                if name.ends_with(".vmtile") {
                    produced_tiles.push(name);
                }
            } else if name.ends_with(".vmo") && !same_bytes(&path, &target) {
                copy_replacing(&path, &target)?;
                out.models.push(name);
            }
        }
        if !produced_tiles.iter().any(|name| name.starts_with(&prefix)) && !out.vmaps.iter().any(|n| *n == vmtree_file(*map)) {
            return Err(format!(
                "the assembler wrote no vmaps for map {map} into {}",
                vmaps.display()
            ));
        }
        // A tile of this map that the server had and this run did not
        // produce no longer has any buildings. Its old file would name spawns
        // the new tree does not have, so it is removed.
        for entry in std::fs::read_dir(data_dir.join("vmaps")).map_err(|e| e.to_string())?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&format!("{prefix}_")) && name.ends_with(".vmtile") && !produced_tiles.contains(&name) {
                std::fs::remove_file(entry.path()).map_err(|e| format!("{name}: {e}"))?;
                out.vmaps.push(format!("{name} removed"));
            }
        }
    }

    // 3. The navmesh: one build per requested tile and per neighbour that
    // has a .map.
    let mut wanted: Vec<Tile> = Vec::new();
    for (tile, _) in &request.tiles {
        for candidate in std::iter::once(*tile).chain(tile.neighbours()) {
            if !wanted.contains(&candidate) && data_dir.join("maps").join(candidate.map_file()).is_file() {
                wanted.push(candidate);
            }
        }
    }
    // The builds run as separate processes, up to [`PARALLEL`] at once.
    // `buildSingleTile` runs one worker whatever `--threads` says, and one
    // tile takes minutes. Each process rewrites the map's `.mmap` header from
    // the same tile list, so the header has the same bytes whichever process
    // finishes last.
    for tile in &wanted {
        let target = data_dir.join("mmaps").join(tile.mmtile_file());
        // The generator skips a tile whose file exists and is current, so
        // the old file is removed first.
        if target.is_file() {
            std::fs::remove_file(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        }
    }
    let builds = wanted.len();
    // Replace the estimate with the actual number of builds.
    total = step + builds;
    let mut queue = wanted.into_iter();
    let mut running: Vec<(Tile, std::process::Child, Instant)> = Vec::new();
    let mut done = 0usize;
    loop {
        while running.len() < parallel() {
            let Some(tile) = queue.next() else {
                break;
            };
            progress(
                &format!(
                    "building the navmesh of map {} tile {},{} ({} of {builds})",
                    tile.map,
                    tile.x,
                    tile.y,
                    done + running.len() + 1
                ),
                step + done + running.len(),
                total,
            );
            let mut command = Command::new(&tools.mmap_generator);
            command
                .arg(tile.map.to_string())
                .arg("--tile")
                .arg(format!("{},{}", tile.x, tile.y))
                .arg("--silent")
                .arg("--threads")
                .arg("1");
            if let Some(off_mesh) = &tools.off_mesh {
                command.arg("--offMeshInput").arg(off_mesh);
            }
            if let Some(config) = &tools.config {
                command.arg("--configInputPath").arg(config);
            }
            out.log.push(format!("> {}", describe(&command)));
            let child = prepare(&mut command, data_dir, tools, &request.dll_dirs)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("{}: {e}", tools.mmap_generator.display()))?;
            running.push((tile, child, Instant::now()));
        }
        if running.is_empty() {
            break;
        }
        // Wait until any one of the running builds exits.
        let mut finished: Option<usize> = None;
        while finished.is_none() {
            for (index, (_, child, _)) in running.iter_mut().enumerate() {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        finished = Some(index);
                        break;
                    }
                    Ok(None) => {}
                    Err(_) => {
                        finished = Some(index);
                        break;
                    }
                }
            }
            if finished.is_none() {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
        let (tile, child, started) = running.remove(finished.expect("set above"));
        done += 1;
        let output = child
            .wait_with_output()
            .map_err(|e| format!("{}: {e}", tools.mmap_generator.display()))?;
        let (ok, tail) = collect(&output, &mut out.log);
        out.timings.push((
            format!("MoveMapGenerator {} {},{}", tile.map, tile.x, tile.y),
            started.elapsed().as_secs_f32(),
        ));
        let target = data_dir.join("mmaps").join(tile.mmtile_file());
        match (ok, target.is_file()) {
            (true, true) => out.mmaps.push(tile.mmtile_file()),
            (true, false) => out.mmaps_failed.push((
                tile.mmtile_file(),
                format!("the generator finished and wrote no tile: {tail}"),
            )),
            (false, _) => out.mmaps_failed.push((tile.mmtile_file(), tail)),
        }
    }
    // The generator writes its own log into its working directory, which is
    // `data_dir`; remove it.
    let _ = std::fs::remove_file(data_dir.join("MoveMapGen.log"));
    Ok(out)
}

/// How many navmesh builds run at once: half the machine's threads, at
/// least one and at most [`PARALLEL`].
pub const PARALLEL: usize = 6;

fn parallel() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get() / 2)
        .unwrap_or(1)
        .clamp(1, PARALLEL)
}

/// The command as one line, for the log.
fn describe(command: &Command) -> String {
    format!(
        "{} {}",
        command.get_program().to_string_lossy(),
        command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// Sets a command to run in `cwd` with a null stdin, so it cannot wait for
/// input, and with the tools' own folder and `dll_dirs` on `PATH`, for the
/// DLLs a Debug build loads from beside it.
fn prepare<'a>(command: &'a mut Command, cwd: &Path, tools: &Tools, dll_dirs: &[PathBuf]) -> &'a mut Command {
    let mut path = std::env::var_os("PATH").unwrap_or_default();
    let ahead = tools.mmap_generator.parent().into_iter().chain(dll_dirs.iter().map(PathBuf::as_path));
    for dir in ahead {
        let mut joined = dir.as_os_str().to_os_string();
        joined.push(";");
        joined.push(&path);
        path = joined;
    }
    command.current_dir(cwd).env("PATH", path).stdin(Stdio::null())
}

/// Appends the last lines of a tool's output to the log, and returns whether
/// the tool succeeded along with those lines joined. A tool draws its
/// progress bar by writing carriage returns over one line, so both `\n` and
/// `\r` count as line breaks.
fn collect(output: &std::process::Output, log: &mut Vec<String>) -> (bool, String) {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    let lines: Vec<&str> = text
        .split(['\n', '\r'])
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    let tail: Vec<String> = lines.iter().rev().take(6).rev().map(|s| s.to_string()).collect();
    log.extend(tail.iter().cloned());
    let ok = output.status.success();
    let said = match ok {
        true => tail.join(" | "),
        false => format!("exited with {}: {}", output.status, tail.join(" | ")),
    };
    (ok, said)
}

/// Runs one tool to completion in `cwd`. On failure, the error carries the
/// last lines of its output.
fn run(
    command: &mut Command,
    cwd: &Path,
    tools: &Tools,
    dll_dirs: &[PathBuf],
    log: &mut Vec<String>,
) -> Result<(), String> {
    let exe = command.get_program().to_string_lossy().into_owned();
    log.push(format!("> {}", describe(command)));
    let output = prepare(command, cwd, tools, dll_dirs)
        .output()
        .map_err(|e| format!("{exe}: {e}"))?;
    match collect(&output, log) {
        (true, _) => Ok(()),
        (false, said) => Err(format!("{said}
  in: {}", describe(command))),
    }
}

/// Whether two files hold the same bytes; `false` when either is absent.
fn same_bytes(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(ma), Ok(mb)) if ma.len() == mb.len() => {
            matches!((std::fs::read(a), std::fs::read(b)), (Ok(x), Ok(y)) if x == y)
        }
        _ => false,
    }
}

/// Copies a file over another in one step, so a reader sees either the old
/// bytes or the new ones and never a mixture.
fn copy_replacing(from: &Path, to: &Path) -> Result<(), String> {
    let bytes = std::fs::read(from).map_err(|e| format!("{}: {e}", from.display()))?;
    vale_edit::project::write_replacing(to, &bytes).map_err(|e| format!("{}: {e}", to.display()))
}

/// Builds a staged client install for the tools to read in place of the real
/// one: every archive of `data_dir` hard-linked under `<into>\Data\`, except
/// the names in `skip`, plus `archive` under `as_name`. Returns the install
/// root, which is what [`Request::client_root`] takes.
///
/// A hard link takes no space and reads as the file itself, so the five
/// gigabytes of archives are never copied. A hard link requires the same
/// volume, so the staging folder goes under the project, which is beside
/// `Data\`, and not in the temp folder. Removing the staging folder
/// afterwards removes the links and leaves the archives.
pub fn stage_client(
    data_dir: &Path,
    skip: &[String],
    archive: &Path,
    as_name: &str,
    into: &Path,
) -> Result<PathBuf, String> {
    let staged = into.join("Data");
    let _ = std::fs::remove_dir_all(into);
    std::fs::create_dir_all(&staged).map_err(|e| format!("{}: {e}", staged.display()))?;
    for entry in std::fs::read_dir(data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.to_ascii_lowercase().ends_with(".mpq") || !entry.path().is_file() {
            continue;
        }
        if skip.iter().any(|s| s.eq_ignore_ascii_case(&name)) || name.eq_ignore_ascii_case(as_name) {
            continue;
        }
        link(&entry.path(), &staged.join(&name))?;
    }
    link(archive, &staged.join(as_name))?;
    // Return an absolute path: the tools run in working directories of
    // their own, and a project under `.\Edit\` has a relative path.
    std::path::absolute(into).map_err(|e| format!("{}: {e}", into.display()))
}

/// Builds a staged `DataDir` for the tools to write into in place of the
/// real one: `maps\` and `vmaps\` hard-linked file by file, and `mmaps\`
/// holding copies of the `.mmap` headers only. The generator reads the
/// neighbours' `.map` files and the map's `.vmtree`, `.vmtile` and model
/// files through the links, finds every tile for the `.mmap` header it
/// rewrites, and writes new files beside them. A file the run replaces is
/// renamed over, which replaces the link and leaves the real file unchanged.
/// The headers are copied and not linked because the generator opens them
/// for writing in place, which would change the real file through a link.
pub fn stage_data_dir(data_dir: &Path, into: &Path) -> Result<PathBuf, String> {
    let _ = std::fs::remove_dir_all(into);
    for sub in ["maps", "vmaps"] {
        let from = data_dir.join(sub);
        let to = into.join(sub);
        std::fs::create_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
        let Ok(entries) = std::fs::read_dir(&from) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_file() {
                link(&entry.path(), &to.join(entry.file_name()))?;
            }
        }
    }
    let mmaps = into.join("mmaps");
    std::fs::create_dir_all(&mmaps).map_err(|e| format!("{}: {e}", mmaps.display()))?;
    if let Ok(entries) = std::fs::read_dir(data_dir.join("mmaps")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".mmap") {
                std::fs::copy(entry.path(), mmaps.join(&name)).map_err(|e| format!("{name}: {e}"))?;
            }
        }
    }
    std::path::absolute(into).map_err(|e| format!("{}: {e}", into.display()))
}

/// Creates a hard link, or a copy when the hard link fails, as it does
/// across volumes.
fn link(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::hard_link(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("{} -> {}: {e}", from.display(), to.display()))
}

/// The client build vmangos is built for. It names the folder under
/// `DataDir` the server reads its DBC files from; see [`dbc_dir`].
pub const CLIENT_BUILD: u32 = 5875;

/// The folder the server reads its DBC files from: `DataDir\5875\dbc\`.
///
/// `World::SetInitialWorldSettings` passes `DataDir` plus the build number
/// and a separator to `LoadDBCStores` (`World.cpp:1396`), which appends
/// `dbc/` (`DBCStores.cpp:187`). A file copied to `DataDir\dbc\` is not read.
pub fn dbc_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(CLIENT_BUILD.to_string()).join("dbc")
}

/// The 48 client tables the server reads as files, by bare name, as
/// `DBCStores.cpp:189` of the checkout loads them. An edited table reaches
/// the server by being copied into [`dbc_dir`]. Every other DBC the client
/// has is either a SQL table on the server or not read by the server.
pub const SERVER_DBCS: [&str; 48] = [
    "AuctionHouse", "BankBagSlotPrices", "CharSections", "CharacterFacialHairStyles", "ChatChannels",
    "ChrClasses", "ChrRaces", "CinematicSequences", "CreatureDisplayInfo", "CreatureDisplayInfoExtra",
    "CreatureFamily", "CreatureModelData", "CreatureType", "DurabilityCosts", "DurabilityQuality",
    "Emotes", "EmotesText", "GameObjectDisplayInfo", "ItemBagFamily", "ItemDisplayInfo",
    "ItemRandomProperties", "ItemSet", "Lock", "NamesProfanity", "NamesReserved", "QuestSort", "SkillLine",
    "SkillRaceClassInfo", "SkillTiers", "SpellCastTimes", "SpellCategory", "SpellDuration",
    "SpellFocusObject", "SpellItemEnchantment", "SpellRadius", "SpellRange", "SpellShapeshiftForm",
    "SpellVisual", "StableSlotPrices", "Talent", "TalentTab", "TaxiPath", "TaxiPathNode",
    "TransportAnimation", "WMOAreaTable", "WorldMapArea", "WorldMapOverlay", "WorldSafeLocs",
];

/// Whether a DBC, by bare name, is one the server reads as a file.
pub fn server_reads_dbc(name: &str) -> bool {
    SERVER_DBCS.iter().any(|known| known.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dbc_folder_is_under_the_build_number() {
        assert_eq!(
            dbc_dir(Path::new("C:/MaNGOS")),
            Path::new("C:/MaNGOS").join("5875").join("dbc")
        );
    }

    /// The three file names the tools write for Azeroth 32,48, checked
    /// against the files in a server install: `maps\0004832.map`,
    /// `vmaps\000_32_48.vmtile`, `mmaps\0004832.mmtile`.
    #[test]
    fn a_tile_is_filed_under_the_tools_own_names() {
        let tile = Tile::new(0, 32, 48);
        assert_eq!(tile.map_file(), "0004832.map");
        assert_eq!(tile.vmtile_file(), "000_32_48.vmtile");
        assert_eq!(tile.mmtile_file(), "0004832.mmtile");
        assert_eq!(vmtree_file(0), "000.vmtree");
        assert_eq!(mmap_file(1), "001.mmap");
        // Azeroth 3,57, the one asymmetric tile the server holds.
        assert_eq!(Tile::new(0, 3, 57).vmtile_file(), "000_03_57.vmtile");
        assert_eq!(Tile::new(0, 3, 57).map_file(), "0005703.map");
    }

    /// A tile's navmesh reads its four orthogonal neighbours' edges, and a
    /// tile on the grid's edge has fewer.
    #[test]
    fn the_neighbours_are_the_four_on_the_grid() {
        let mut around = Tile::new(0, 32, 48).neighbours();
        around.sort();
        assert_eq!(
            around,
            vec![Tile::new(0, 31, 48), Tile::new(0, 32, 47), Tile::new(0, 32, 49), Tile::new(0, 33, 48)]
        );
        assert_eq!(Tile::new(0, 0, 0).neighbours().len(), 2);
        assert_eq!(Tile::new(0, 63, 5).neighbours().len(), 3);
    }

    /// `DataDir` is resolved relative to the conf, and read in the quoted
    /// form vmangos writes.
    #[test]
    fn the_data_dir_is_relative_to_the_conf() {
        let dir = std::env::temp_dir().join(format!("vale-datadir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("mangosd.conf");
        std::fs::write(&conf, "DataDir = \"./data\"\n").unwrap();
        assert_eq!(from_conf(&conf), Some(dir.join("./data")));
        std::fs::write(&conf, "# DataDir = x\nDataDir = \"C:/MaNGOS\"\n").unwrap();
        assert_eq!(from_conf(&conf), Some(PathBuf::from("C:/MaNGOS")));
        std::fs::write(&conf, "LogsDir = \"\"\n").unwrap();
        assert_eq!(from_conf(&conf), Some(dir.join(".")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The height flag is read from the `MHGT` header, and a folder with no
    /// `.map` returns the extractor's own recommendation.
    #[test]
    fn the_height_flag_is_read_off_an_existing_map() {
        let dir = std::env::temp_dir().join(format!("vale-heights-{}", std::process::id()));
        let maps = dir.join("maps");
        std::fs::create_dir_all(&maps).unwrap();
        assert!(heights_as_int(&dir, 0));
        // Ten header words, then an MHGT block at offset 40 with flags.
        let file = |flags: u32| -> Vec<u8> {
            let mut b = Vec::new();
            b.extend_from_slice(b"MAPSz1.4");
            for word in [0u32, 0, 40, 8, 0, 0, 0, 0] {
                b.extend_from_slice(&word.to_le_bytes());
            }
            b.extend_from_slice(b"MHGT");
            b.extend_from_slice(&flags.to_le_bytes());
            b
        };
        std::fs::write(maps.join("0004832.map"), file(0)).unwrap();
        assert!(!heights_as_int(&dir, 0), "floats");
        assert!(!heights_as_int(&dir, 1), "another map falls back to any");
        std::fs::write(maps.join("0014832.map"), file(2)).unwrap();
        assert!(heights_as_int(&dir, 1), "int16");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder missing a tool is refused with the tool's name in the error,
    /// and the generator's two inputs are optional.
    #[test]
    fn the_tools_are_found_by_name_or_refused_by_name() {
        let dir = std::env::temp_dir().join(format!("vale-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = Tools::find(&dir).unwrap_err();
        assert!(missing.contains("map extractor") && missing.contains("movemap generator"), "{missing}");
        for name in ["mapextractor.exe", "vmapextractor.exe", "vmap_assembler.exe", "MoveMapGen.exe"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let tools = Tools::find(&dir).unwrap();
        assert_eq!(tools.assembler, dir.join("vmap_assembler.exe"));
        assert!(tools.off_mesh.is_none() && tools.config.is_none());
        std::fs::write(dir.join("offmesh.txt"), b"").unwrap();
        assert_eq!(Tools::find(&dir).unwrap().off_mesh, Some(dir.join("offmesh.txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_server_reads_forty_eight_tables_as_files() {
        assert_eq!(SERVER_DBCS.len(), 48);
        assert!(server_reads_dbc("SpellCastTimes"));
        assert!(server_reads_dbc("lock"));
        assert!(!server_reads_dbc("Spell"), "spell_template is a SQL table");
        assert!(!server_reads_dbc("AreaTable"), "area_template is a SQL table");
    }
}
