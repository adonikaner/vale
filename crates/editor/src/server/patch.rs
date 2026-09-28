//! A *patch* is the folder a publish writes: every file a server and its
//! players need, in one place, with a note saying where each file goes.
//!
//! ## What a publish writes, and what it leaves alone
//!
//! A patch is distributed, not applied. The person publishing hands the files
//! to a server and to players. A publish does not write to this machine's
//! `DataDir` or database, and writes its client archive into this install's
//! `Data\` only when the project's *Copy the client archive into Data* switch
//! is on. The development loop uses other routes: a
//! save applies rows, and the "Server files" button on the map window and
//! "Regenerate changed tiles" on the Server panel write the live server's
//! folders. A publish is for finished work.
//!
//! ```text
//! Edit\<project>\publish\<name>\
//!   README.txt                      what each file is and where it goes
//!   manifest.txt                    every file, with its size and hash
//!   client\Patch-<X>.MPQ            -> the client's Data\
//!   server\5875\dbc\*.dbc           -> DataDir\5875\dbc\
//!   server\maps\MMMYYXX.map         -> DataDir\maps\
//!   server\vmaps\*                  -> DataDir\vmaps\
//!   server\mmaps\*                  -> DataDir\mmaps\
//!   server\sql\<stamp>_world.sql    -> mysql <world database> < it
//! ```
//!
//! `<name>` is the typed name, or the UTC stamp when none is typed. Publishing
//! under an existing name rewrites that folder.
//!
//! `server\5875\dbc\` holds only the tables the server reads as files. A client
//! table the project edits that the server reads as rows is not copied there:
//! `Spell.dbc` is `spell_template` and `TaxiNodes.dbc` is `taxi_nodes`. The
//! README's `server\5875\dbc\` section names each such table and points it at
//! `server\sql\`, where its edits are rows of the migration.
//!
//! ## Every patch holds the whole project
//!
//! The archive holds the whole project, the migration holds all of its rows
//! (`vale_mangos::migration`), and the server files are every tile the
//! project carries, plus the neighbours whose navmesh reads a changed tile's
//! edge. A patch therefore applies to a server whether or not that server
//! received the previous patch.
//!
//! The tile work is not repeated each time. The navmesh takes minutes per
//! tile, so a publish starts from the previous patch's `server\` folder and
//! regenerates only the tiles changed since (`publish\server-tiles.txt` holds
//! a hash per tile as of the last publish). A tile that left the project is
//! regenerated from the archives, which puts the shipped file into the patch.
//! The server already has that file, so this does no harm.
//!
//! ## The tools read a staged copy of the install
//!
//! The extractors read `Data\`, and the generator reads and writes a
//! `DataDir`. Neither is given the real one. `publish\.stage\client` holds the
//! archives as hard links, with the new patch beside them under its letter.
//! `publish\.stage\server` holds `maps\` and `vmaps\` as hard links and a copy
//! of the `.mmap` headers. See `vale_mangos::datadir::stage_client` and
//! `stage_data_dir`. The tools see what an install with the patch in it would
//! give them, and their output is copied into the patch folder. The staging is
//! removed afterwards; removing a hard link leaves the original file.

use super::datadir::{self as tiles, Record, Step};
use super::release;
use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::project::{self, Project};
use vale_mangos::datadir::{self, Outcome, Request};
use bevy::prelude::*;
use std::path::{Path, PathBuf};

/// Where the patches go under the project.
pub const DIR: &str = "publish";

/// Where the staging folders go: beside the patches, under one name that
/// begins with a dot, which a walk of the folder such as [`patches`] skips.
pub const STAGE: &str = "publish\\.stage";

/// The name of the last patch written, for the tiles the next one starts
/// from.
pub const LAST_VPATH: &str = "publish\\last-patch.txt";

/// Whether text may name a patch: a folder name, not hidden.
pub fn is_a_name(name: &str) -> bool {
    project::is_a_name(name) && !name.trim().starts_with('.')
}

/// The name a publish takes when none is typed: the UTC stamp.
pub fn default_name() -> String {
    vale_mangos::migration::stamp(None)
}

/// Every patch folder the project holds, newest last by name.
pub fn patches(project: &Project) -> Vec<String> {
    let Some(dir) = project.path_for(DIR) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| is_a_name(name))
        .collect();
    out.sort();
    out
}

/// The last patch written, if its folder is still there.
pub fn last(project: &Project) -> Option<String> {
    let name = String::from_utf8_lossy(&project.read(LAST_VPATH)?).trim().to_string();
    (is_a_name(&name) && project.path_for(&format!("{DIR}\\{name}"))?.is_dir()).then_some(name)
}

/// What the synchronous half of a publish wrote, for the README and the
/// status line.
#[derive(Debug, Default, Clone)]
pub struct Head {
    pub name: String,
    pub folder: PathBuf,
    /// The archive's file name, or `None` when the project has no game data.
    pub archive: Option<String>,
    pub archive_files: usize,
    pub dbcs: Vec<String>,
    pub migration: Option<String>,
    pub migration_line: String,
    /// The patch the tiles were started from, when there was one.
    pub from: Option<String>,
    pub tiles_to_do: usize,
}

/// Publishes a patch under `name`. The archive, the DBCs and the migration
/// are written during this call. When any tile changed, the tiles are queued
/// on the server queue, and the README and manifest are written when that
/// work finishes; when no tile needs the tools, they are written during this
/// call. Returns the text for the status line.
pub fn publish(
    session: &mut EditSession,
    assets: &GameAssets,
    server: &super::settings::ServerSettings,
    queue: &mut super::queue::ServerQueue,
    step: &Step,
    name: &str,
) -> Result<String, String> {
    let name = name.trim();
    if !is_a_name(name) {
        return Err(format!("{name:?} is not a name a patch may have"));
    }
    if queue.busy() {
        return Err("a server write is still running".to_string());
    }
    let project = session.project.clone();
    let folder = project
        .path_for(&format!("{DIR}\\{name}"))
        .ok_or_else(|| format!("{name}: leaves the project"))?;

    // Save everything the project holds, so the patch matches what is on
    // screen.
    session.save_all();
    session.save_all_tables();
    super::creatures::save(session);
    super::gameobjects::save(session);
    super::items::save(session);
    super::quests::save(session);
    super::loot::save(session);
    super::services::save(session);
    super::behaviour::save(session);

    let mut head = Head {
        name: name.to_string(),
        folder: folder.clone(),
        ..Head::default()
    };
    // The client half, under the patch letter this machine's install would
    // give it. The README says what to do when that letter is already taken
    // in the install it is copied into.
    let client = folder.join("client");
    let _ = std::fs::remove_dir_all(&client);
    let data_dir = std::path::absolute(&assets.gamedata_dir).map_err(|e| e.to_string())?;
    // With "Copy the client archive into Data" on, the archive is also
    // written into `Data\`, replacing this project's earlier archive, and the
    // patch carries a copy under the letter it took there. With it off, the
    // patch's copy takes the letter it would have taken and `Data\` is not
    // changed.
    let (letter, count) = match server.copy_archive {
        true => {
            let data = assets.gamedata_dir.clone();
            match session.publish_archive(&data) {
                Some((at, count)) => {
                    let archive_name = at
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    std::fs::create_dir_all(&client).map_err(|e| format!("{}: {e}", client.display()))?;
                    std::fs::copy(&at, client.join(&archive_name))
                        .map_err(|e| format!("{}: {e}", at.display()))?;
                    (archive_name, count)
                }
                None if session.status.starts_with("nothing to publish") => (String::new(), 0),
                None => return Err(session.status.clone()),
            }
        }
        false => {
            let letter = project::next_patch_name(&data_dir).unwrap_or_else(|| "Patch-Z.MPQ".to_string());
            let archive = client.join(&letter);
            let count = project.publish(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
            (letter, count)
        }
    };
    if count > 0 {
        head.archive = Some(letter.clone());
        head.archive_files = count;
    } else {
        let _ = std::fs::remove_dir_all(&client);
    }

    // The server files that need no tool: the DBCs and the migration.
    let server_dir = folder.join("server");
    let _ = std::fs::remove_dir_all(vale_mangos::datadir::dbc_dir(&server_dir));
    let _ = std::fs::remove_dir_all(server_dir.join("sql"));
    head.dbcs = release::copy_server_dbcs(&project, &server_dir)?;
    let written = release::write_migration(session, assets, &server_dir.join("sql"))?;
    release::report_unknown(&written.contents);
    head.migration = written
        .file
        .as_ref()
        .and_then(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()));
    head.migration_line = written.contents.line();

    // The tiles: the previous patch's, plus the ones changed since.
    let previous = last(&project).filter(|p| p != name);
    head.from = previous.clone();
    let maps = crate::session::map_directories(assets);
    let record = Record::read(&project, tiles::PATCH_RECORD);
    let (dirty, skipped) = tiles::dirty(session, assets, &maps, &record);
    for line in &skipped {
        warn!("publish: {line}");
    }
    // "Regenerate tiles on publish" on the Server panel. With it off, the
    // tiles are the previous patch's, carried over, and none is rebuilt. The
    // status line gives the number of changed tiles left out, so the patch is
    // not mistaken for a complete one.
    let left_out = match server.regenerate_tiles {
        true => 0,
        false => dirty.len(),
    };
    let dirty = match server.regenerate_tiles {
        true => dirty,
        false => Vec::new(),
    };
    head.tiles_to_do = dirty.len();
    project
        .write(LAST_VPATH, format!("{name}\n").as_bytes())
        .map_err(|e| format!("{LAST_VPATH}: {e}"))?;

    let mut said = vec![format!("patch {name}")];
    if let Some(archive) = &head.archive {
        said.push(format!("{archive} ({} files)", head.archive_files));
    }
    if !head.dbcs.is_empty() {
        said.push(format!("{} DBC(s)", head.dbcs.len()));
    }
    said.push(head.migration_line.clone());
    if left_out > 0 {
        said.push(format!(
            "{left_out} changed server tile(s) not regenerated: Regenerate tiles on publish is off"
        ));
    }

    if dirty.is_empty() {
        // No work for the tools: the previous patch's tiles are carried
        // over, and the folder is finished during this call.
        carry_over(&project, previous.as_deref(), &folder)?;
        finish(&project, &head, None)?;
        if head.from.is_some() {
            said.push("server tiles carried over from the last patch".to_string());
        }
        return Ok(said.join(" \u{b7} "));
    }

    // The tool runs, pushed onto the server queue.
    let tools = tiles::tools(server)?;
    tools.check_patched()?;
    let real_data_dir = tiles::data_dir(server)?;
    let stage = project
        .path_for(STAGE)
        .ok_or_else(|| "the staging folder leaves the project".to_string())?;
    let own: Vec<String> = project.published().into_iter().map(|a| a.name).collect();
    let archive_for_tools = head.archive.clone().map(|name| (client.join(&name), name));
    let dll_dirs: Vec<PathBuf> = server
        .conf_path()
        .and_then(|conf| conf.parent().map(Path::to_path_buf))
        .into_iter()
        .collect();
    let progress = step.clone();
    let head_for_finish = head.clone();
    let label = format!("patch {name}: {} server tile(s)", dirty.len());
    said.push(format!("{} server tile(s) to regenerate \u{2014} the bar says which step", dirty.len()));

    let work: super::queue::Work = Box::new(move || {
        // The steps before and after the tools report no count. The tools'
        // own steps report one, and the progress bar fills to it.
        let say = |line: &str| progress.set(line, 0, 0);
        let result = (|| -> Result<Outcome, String> {
            carry_over(&project, previous.as_deref(), &folder)?;
            say("staging a copy of the install for the tools");
            let client_root = match &archive_for_tools {
                Some((archive, name)) => {
                    datadir::stage_client(&data_dir, &own, archive, name, &stage.join("client"))?
                }
                // No archive: the project has no game data, so the tools read
                // the install as it is, without this project's older archives.
                None => {
                    let empty = stage.join("none.MPQ");
                    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
                    std::fs::write(&empty, b"").map_err(|e| e.to_string())?;
                    let root = datadir::stage_client(&data_dir, &own, &empty, "none.MPQ", &stage.join("client"))?;
                    let _ = std::fs::remove_file(root.join("Data").join("none.MPQ"));
                    root
                }
            };
            let staged = datadir::stage_data_dir(&real_data_dir, &stage.join("server"))?;
            let first_map = dirty[0].tile.map;
            let request = Request {
                client_root,
                tiles: dirty.iter().map(|d| (d.tile, d.reach)).collect(),
                heights_as_int: datadir::heights_as_int(&real_data_dir, first_map),
                dll_dirs,
            };
            let scratch = std::env::temp_dir().join(format!("vale-patch-{}", std::process::id()));
            let outcome = datadir::regenerate(&tools, &staged, &request, &scratch, |line, done, total| {
                progress.set(line, done, total)
            })?;
            let _ = std::fs::remove_dir_all(&scratch);
            say("copying the server files into the patch");
            collect(&staged, &folder.join("server"), &outcome)?;
            let mut record = Record::read(&project, tiles::PATCH_RECORD);
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
            record.write(&project, tiles::PATCH_RECORD)?;
            finish(&project, &head_for_finish, Some(&outcome))?;
            Ok(outcome)
        })();
        // On a failure the staging is kept so it can be inspected; the next
        // publish removes it.
        if result.is_ok() {
            let _ = std::fs::remove_dir_all(&stage);
        }
        progress.clear();
        Box::new(move |session: &mut EditSession, _: &mut super::reload::Reloads| {
            session.status = match &result {
                Ok(outcome) => {
                    for line in &outcome.log {
                        info!("patch: {line}");
                    }
                    for (name, why) in &outcome.mmaps_failed {
                        warn!("patch: {name}: {why}");
                    }
                    for (what, seconds) in &outcome.timings {
                        info!("patch: {what}: {seconds:.1} s");
                    }
                    format!(
                        "patch {} finished: {}",
                        head_for_finish.name,
                        outcome.line().trim_end_matches(" \u{2014} restart the server to walk on them")
                    )
                }
                Err(e) => {
                    warn!("patch: {e}");
                    format!("patch {}: server tiles: {e}", head_for_finish.name)
                }
            };
        })
    });
    queue.push(label, work);
    Ok(said.join(" \u{b7} "))
}

/// Copies the previous patch's server tiles into this folder, so this patch
/// starts from them. Does nothing when there is no previous patch or it is
/// this one.
fn carry_over(project: &Project, previous: Option<&str>, folder: &Path) -> Result<(), String> {
    let Some(previous) = previous else {
        return Ok(());
    };
    let Some(from) = project.path_for(&format!("{DIR}\\{previous}\\server")) else {
        return Ok(());
    };
    for sub in ["maps", "vmaps", "mmaps"] {
        let source = from.join(sub);
        let Ok(entries) = std::fs::read_dir(&source) else {
            continue;
        };
        let target = folder.join("server").join(sub);
        std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        for entry in entries.flatten() {
            if entry.path().is_file() {
                std::fs::copy(entry.path(), target.join(entry.file_name()))
                    .map_err(|e| format!("{}: {e}", entry.path().display()))?;
            }
        }
    }
    Ok(())
}

/// Copies the tools' output from the staged `DataDir` into the patch: every
/// file the outcome names, and the `.mmap` header of every map a navmesh was
/// built for. A file the outcome removed is removed from the patch as well.
fn collect(staged: &Path, into: &Path, outcome: &Outcome) -> Result<(), String> {
    let copy = |sub: &str, name: &str| -> Result<(), String> {
        let target = into.join(sub);
        std::fs::create_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
        std::fs::copy(staged.join(sub).join(name), target.join(name))
            .map(|_| ())
            .map_err(|e| format!("{sub}\\{name}: {e}"))
    };
    let remove = |sub: &str, name: &str| {
        let _ = std::fs::remove_file(into.join(sub).join(name));
    };
    for name in &outcome.maps {
        copy("maps", name)?;
    }
    for name in &outcome.maps_removed {
        remove("maps", name);
    }
    for name in &outcome.vmaps {
        match name.strip_suffix(" removed") {
            Some(gone) => remove("vmaps", gone),
            None => copy("vmaps", name)?,
        }
    }
    for name in &outcome.models {
        copy("vmaps", name)?;
    }
    for name in &outcome.mmaps {
        copy("mmaps", name)?;
    }
    let mut maps_built: Vec<u32> = outcome
        .mmaps
        .iter()
        .filter_map(|name| name.get(0..3)?.parse().ok())
        .collect();
    maps_built.sort_unstable();
    maps_built.dedup();
    for map in maps_built {
        let header = datadir::mmap_file(map);
        if staged.join("mmaps").join(&header).is_file() {
            copy("mmaps", &header)?;
        }
    }
    Ok(())
}

/// The README lines for the client tables the project edits that the server
/// reads as rows rather than as files: one line per table of
/// `super::rows::MAPPED` that `files` (the project's virtual paths) carries.
///
/// Such a table is not copied to `server\5875\dbc\`, because vmangos never
/// opens the file: `Spell.dbc` is `spell_template` on the server and
/// `TaxiNodes.dbc` is `taxi_nodes`. Its edit reaches the server as rows of
/// the migration in `server\sql\`. Without this line the dbc folder reads
/// "none" for a project whose only edit is a spell, and the spell looks as
/// if it had not been published.
fn sent_as_rows(files: &[String]) -> String {
    let mut out = String::new();
    for (dbc, table) in super::rows::MAPPED {
        let wanted = format!("DBFilesClient\\{dbc}.dbc");
        let carried = files
            .iter()
            .any(|vpath| vpath.replace('/', "\\").eq_ignore_ascii_case(&wanted));
        if carried {
            out.push_str(&format!(
                "  {dbc}.dbc is not copied here: the server reads {table} from the world\n\
                 \x20   database, so this project's {dbc}.dbc edits are rows in server\\sql\\.\n"
            ));
        }
    }
    out
}

/// Writes the README and the manifest, once the folder holds every other
/// file.
fn finish(project: &Project, head: &Head, outcome: Option<&Outcome>) -> Result<(), String> {
    let folder = &head.folder;
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let count = |sub: &str| -> usize {
        std::fs::read_dir(folder.join("server").join(sub))
            .map(|d| d.flatten().filter(|e| e.path().is_file()).count())
            .unwrap_or(0)
    };
    let (maps, vmaps, mmaps) = (count("maps"), count("vmaps"), count("mmaps"));
    let stamp = vale_mangos::migration::stamp(None);
    let mut text = format!(
        "{} — patch {}\n\
         Written by the world editor on {}-{}-{} {}:{}:{} UTC. Nothing here has been\n\
         applied to any install; every file is to be copied where this says.\n\n",
        project.name,
        head.name,
        &stamp[0..4],
        &stamp[4..6],
        &stamp[6..8],
        &stamp[8..10],
        &stamp[10..12],
        &stamp[12..14],
    );
    text.push_str("client\\\n");
    match &head.archive {
        Some(name) => text.push_str(&format!(
            "  {name}  ({} files)\n\
             \x20   -> the client's Data\\ folder, beside patch.MPQ. The client reads\n\
             \x20      patch archives in order, base < patch < patch-2..9 < Patch-A..Z,\n\
             \x20      and the last wins; if this letter is taken in the Data\\ it lands\n\
             \x20      in, rename it to the next free letter above every archive there.\n\n",
            head.archive_files
        )),
        None => text.push_str("  (none: this project changes no client file)\n\n"),
    }
    text.push_str("server\\5875\\dbc\\\n");
    match head.dbcs.is_empty() {
        true => text.push_str("  (none: this project changes no table the server reads as a file)\n"),
        false => {
            for name in &head.dbcs {
                text.push_str(&format!("  {name}\n"));
            }
            text.push_str(
                "    -> DataDir\\5875\\dbc\\ on the server (DataDir is in mangosd.conf). Read at\n\
                 \x20      startup: restart the server.\n",
            );
        }
    }
    text.push_str(&sent_as_rows(&project.files()));
    text.push('\n');
    text.push_str(&format!(
        "server\\maps\\ ({maps} files), server\\vmaps\\ ({vmaps}), server\\mmaps\\ ({mmaps})\n"
    ));
    match maps + vmaps + mmaps {
        0 => text.push_str("  (none: this project changes no tile)\n\n"),
        _ => text.push_str(
            "    -> DataDir\\maps\\, DataDir\\vmaps\\, DataDir\\mmaps\\ on the server, over the\n\
             \x20      files of the same names. Every tile this project changes, the\n\
             \x20      map's vmap tree where a building or model moved, and the navmesh of\n\
             \x20      each changed tile's four neighbours, which read its edge. Read at\n\
             \x20      startup: restart the server.\n\n",
        ),
    }
    text.push_str("server\\sql\\\n");
    match &head.migration {
        Some(name) => text.push_str(&format!(
            "  {name}  ({})\n\
             \x20   -> the world database, with the mysql client:\n\
             \x20        mysql -u <user> -p <world database> < {name}\n\
             \x20      or into the vmangos checkout's sql\\migrations\\ for its own\n\
             \x20      tooling. It records itself in the migrations table and runs once;\n\
             \x20      the server warns at startup about an id its build does not list\n\
             \x20      and starts. It holds the whole of this project's rows, so it\n\
             \x20      applies with or without this project's earlier patches. Apply\n\
             \x20      this project's patches in order. Restart the server afterwards.\n\n",
            head.migration_line
        )),
        None => text.push_str("  (none: this project changes no row)\n\n"),
    }
    if let Some(from) = &head.from {
        text.push_str(&format!(
            "The server tiles unchanged since patch {from} are that patch's files, copied.\n"
        ));
    }
    if let Some(outcome) = outcome {
        text.push_str(&format!(
            "This publish regenerated {} tile(s): {}.\n",
            head.tiles_to_do,
            outcome.line().trim_start_matches("server tiles: ").trim_end_matches(" \u{2014} restart the server to walk on them")
        ));
        for (name, why) in &outcome.mmaps_failed {
            text.push_str(&format!("  {name} FAILED: {why}\n"));
        }
    }
    text.push_str("\nmanifest.txt lists every file with its size and FNV-1a 64 hash.\n");
    vale_edit::project::write_replacing(&folder.join("README.txt"), text.as_bytes())
        .map_err(|e| format!("README.txt: {e}"))?;

    let mut manifest = format!("# {} — patch {}: every file, its size in bytes, FNV-1a 64 of its bytes.\n", project.name, head.name);
    let mut files: Vec<(String, u64, u64)> = Vec::new();
    walk(folder, folder, &mut files)?;
    files.sort();
    for (path, size, hash) in files {
        manifest.push_str(&format!("{path}\t{size}\t{hash:016x}\n"));
    }
    vale_edit::project::write_replacing(&folder.join("manifest.txt"), manifest.as_bytes())
        .map_err(|e| format!("manifest.txt: {e}"))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64, u64)>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "manifest.txt" || name == "README.txt" {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let rel = path
            .strip_prefix(root)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(name);
        out.push((rel, bytes.len() as u64, fnv1a(&bytes)));
    }
    Ok(())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// `--publish [name]`: runs the bar's Publish button from the command line.
/// Runs once, after every scripted edit has been applied, for the same reason
/// `creatures::on_the_command_line` waits.
pub fn on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    server: Res<super::settings::ServerSettings>,
    mut queue: ResMut<super::queue::ServerQueue>,
    step: Res<Step>,
    waypoints: Res<crate::tools::waypoints::Waypoints>,
    creatures: Res<crate::tools::creatures::Creatures>,
    mut done: Local<bool>,
) {
    if *done || !args.publish {
        return;
    }
    if !waypoints.scripted_done || !creatures.scripted_done {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    let name = args.publish_name.clone().unwrap_or_else(default_name);
    let said = match publish(&mut session, &assets, &server, &mut queue, &step, &name) {
        Ok(said) => said,
        Err(e) => format!("publish: {e}"),
    };
    info!("--publish: {said}");
    session.status = said;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spell edit is named in the README's dbc section as rows in
    /// `server\sql\`, and a table the server reads as a file is not.
    #[test]
    fn a_table_the_server_reads_as_rows_is_pointed_at_the_sql() {
        let files = vec![
            "DBFilesClient\\Spell.dbc".to_string(),
            "DBFilesClient/SpellRange.dbc".to_string(),
            "World\\Maps\\Azeroth\\Azeroth_32_48.adt".to_string(),
        ];
        let said = sent_as_rows(&files);
        assert!(said.contains("Spell.dbc is not copied here: the server reads spell_template"), "{said}");
        assert!(!said.contains("SpellRange") && !said.contains("TaxiNodes"), "{said}");
        assert_eq!(sent_as_rows(&["dbfilesclient/taxinodes.dbc".to_string()]).lines().count(), 2);
        assert!(sent_as_rows(&[]).is_empty());
    }

    #[test]
    fn a_patch_name_is_a_folder_name_that_is_not_hidden() {
        assert!(is_a_name("goldshire-1"));
        assert!(is_a_name(&default_name()));
        assert!(!is_a_name(".stage"));
        assert!(!is_a_name("a/b"));
        assert!(!is_a_name(""));
    }

    /// The patches are the folders under `publish\`, with the staging folder
    /// and the loose files left out, and the last one is remembered while
    /// its folder is there.
    #[test]
    fn the_patches_are_the_folders_and_the_last_is_remembered() {
        let root = std::env::temp_dir().join(format!("vale-patches-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = Project::at(root.join("Edit").join("p")).unwrap();
        assert!(patches(&project).is_empty());
        assert_eq!(last(&project), None);
        for name in ["20260922120000", "goldshire", ".stage"] {
            std::fs::create_dir_all(project.publish_dir().join(name)).unwrap();
        }
        project.write("publish\\released.txt", b"").unwrap();
        assert_eq!(patches(&project), vec!["20260922120000".to_string(), "goldshire".to_string()]);
        project.write(LAST_VPATH, b"goldshire\n").unwrap();
        assert_eq!(last(&project).as_deref(), Some("goldshire"));
        std::fs::remove_dir_all(project.publish_dir().join("goldshire")).unwrap();
        assert_eq!(last(&project), None, "a folder that is gone is not a patch to start from");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The README and the manifest name every file in the folder.
    #[test]
    fn the_readme_and_manifest_name_what_is_there() {
        let root = std::env::temp_dir().join(format!("vale-patch-readme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = Project::at(root.join("Edit").join("p")).unwrap();
        let folder = project.publish_dir().join("one");
        std::fs::create_dir_all(folder.join("client")).unwrap();
        std::fs::create_dir_all(folder.join("server").join("maps")).unwrap();
        std::fs::write(folder.join("client").join("Patch-F.MPQ"), b"mpq").unwrap();
        std::fs::write(folder.join("server").join("maps").join("0004832.map"), b"map").unwrap();
        let head = Head {
            name: "one".into(),
            folder: folder.clone(),
            archive: Some("Patch-F.MPQ".into()),
            archive_files: 1,
            migration: Some("20260922120000_world.sql".into()),
            migration_line: "3 row(s)".into(),
            ..Head::default()
        };
        finish(&project, &head, None).unwrap();
        let readme = std::fs::read_to_string(folder.join("README.txt")).unwrap();
        assert!(readme.contains("Patch-F.MPQ  (1 files)"), "{readme}");
        assert!(readme.contains("mysql -u <user> -p <world database> < 20260922120000_world.sql"));
        assert!(readme.contains("server\\maps\\ (1 files)"));
        let manifest = std::fs::read_to_string(folder.join("manifest.txt")).unwrap();
        assert!(manifest.contains("client\\Patch-F.MPQ\t3\t"), "{manifest}");
        assert!(manifest.contains("server\\maps\\0004832.map\t3\t"));
        assert!(!manifest.contains("README.txt"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
