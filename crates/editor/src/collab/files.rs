//! Project files in a shared session: what an operation wrote into or removed
//! from the project folder, sent as one step, and the same step applied on
//! the other editors.
//!
//! ## What is sent
//!
//! The project's journal ([`vale_edit::project::Project::journal`]) records
//! every path written or removed while a session runs. Each frame [`take`]
//! makes [`Files`] steps of what it holds: each path with its bytes now, or
//! with none when the file is gone. That covers every operation that writes
//! files: an import, a delete, a created or pasted tile, a minimap picture, a
//! baked skin, a flight map, a lab model, world map art and locks.
//!
//! Left out:
//!
//! ```text
//! a save             written unjournaled: every editor holds the edits saved
//! a remote edit      written unjournaled: each editor writes it for itself
//! sql\ and server\   the host's record of its own database and server files
//! publish\           what a publish wrote
//! the import record  sent as lines; see below
//! ```
//!
//! The import record (`crate::import::sources`) says which files were
//! imported. Every import rewrites it, so sent whole it would make two imports
//! made at once clash. Each step carries the record's lines for its own files
//! instead, and each editor merges them into its own record. A step also
//! carries the lines of the files it removed, as the record held them before
//! ([`Files::forgotten`]), so the step taken back records them again: a
//! deleted import put back by a revert is an import again, with the source
//! and choices Re-import needs.
//!
//! A step carries at most [`STEP_BYTES`]; a larger batch is split into
//! several. A single file larger than that is not sent, and the status line
//! says so.
//!
//! ## The host's steps are forced
//!
//! A file step from the host is forced ([`vale_collab::Payload::forced`]):
//! accepted where it arrives in the host's order and never reverted. A
//! delete drops unsaved edits and undo entries that writing the old bytes back
//! would not restore.
//!
//! ## What a joined editor may write
//!
//! Textures, models, sounds and lock files ([`joinable`]). Only the host
//! writes any other file, such as a tile, a table, a tile index, the minimap
//! index or a zone grid. A joined editor's step that writes one is reverted
//! before it is sent. The host checks the same rule ([`Files::check`]) and
//! refuses a step that breaks it or names a path outside the project. A joined
//! editor deletes no file itself; it asks the host ([`super::Step::Delete`]).
//!
//! ## Applying a step
//!
//! [`apply`] writes or removes every file unjournaled, merges the import
//! record's lines, and has the session read the files again
//! ([`EditSession::reread_files`]). A joined editor's step that the host
//! refuses is reverted by applying [`Files::backward`]: each file as it was
//! before. Those bytes ([`File::before`]) stay with the editor that made the
//! step and are not sent.
//!
//! ## Another author's step on the host
//!
//! The host's folder holds submitted files only. A joined editor's step is
//! therefore not written there when it arrives: [`stage`] holds it in
//! `EditSession::staged`, which the archives' overlay reads before the
//! folder, so the files are seen at once. [`unstage`] writes the author's
//! staged steps to the folder when they submit, or drops them when another
//! editor takes their edits back. A joined editor writes every step to its
//! own copy at once, since its copy is not the project's record.

use crate::import::sources::{Sources, RECORD};
use crate::session::EditSession;
use serde::{Deserialize, Serialize};
use vale_client::assets::GameAssets;
use vale_collab::Group;
use vale_edit::manifest::Kind;

/// The most file bytes one step carries: 48 MiB, under the 64 MiB frame
/// limit (`vale_collab::wire::MAX_FRAME`) with room for the rest.
pub(crate) const STEP_BYTES: usize = 48 << 20;

/// Project files written or removed by one operation. See the module comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Files {
    pub(crate) files: Vec<File>,
    /// The import record's lines for the files written, as
    /// [`Sources::text`] writes them.
    pub(crate) imported: String,
    /// …and its lines for the files removed, as the record held them before
    /// the step. See the module comment.
    #[serde(default)]
    pub(crate) forgotten: String,
}

/// One file of a [`Files`] step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct File {
    /// The virtual path.
    pub(crate) path: String,
    /// The bytes it holds now, or `None` when it was removed.
    pub(crate) bytes: Option<Vec<u8>>,
    /// The bytes it held before the step, for the editor that made the step
    /// to revert it. Not sent: an editor that receives a step never reverts
    /// it.
    #[serde(skip)]
    pub(crate) before: Option<Vec<u8>>,
}

impl Files {
    /// Every group the step touches: each file's own, every group of a tile
    /// whose file it writes or removes, and the table of a DBC file.
    pub(crate) fn groups(&self) -> Vec<Group> {
        let mut out = Vec::new();
        for file in &self.files {
            out.push(Group::file(&file.path));
            match Kind::of(&file.path) {
                Kind::Tile => {
                    if let Some(tile) = vale_edit::TileKey::from_vpath(&file.path) {
                        out.extend(vale_collab::groups::whole_tile(&tile));
                    }
                }
                Kind::Table => {
                    if let Some(name) = table_of(&file.path) {
                        out.push(Group::Table { name });
                    }
                }
                _ => {}
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Whether a joined editor may send this step, or the first file it may
    /// not and why. See [`joinable`].
    pub(crate) fn check(&self) -> Result<(), String> {
        for file in &self.files {
            joinable(&file.path).map_err(|why| format!("{}: {why}", file.path))?;
        }
        Ok(())
    }

    /// The same step backward: each file as it was before it.
    pub(crate) fn backward(&self) -> Files {
        let files = self
            .files
            .iter()
            .map(|file| File { path: file.path.clone(), bytes: file.before.clone(), before: file.bytes.clone() })
            .collect();
        Files { files, imported: self.forgotten.clone(), forgotten: self.imported.clone() }
    }

    /// What the step changed, for a status line: the path of a single file,
    /// or a count.
    pub(crate) fn what(&self) -> String {
        match self.files.as_slice() {
            [one] => one.path.clone(),
            files => format!("{} files", files.len()),
        }
    }

    /// What the step did, for a status line after the editor's name:
    /// `deleted World\Maps\…\x.adt`, `wrote 3 files`.
    pub(crate) fn words(&self) -> String {
        let removed = self.files.iter().filter(|file| file.bytes.is_none()).count();
        let written = self.files.len() - removed;
        let files = |n: usize| match n {
            1 => "1 file".to_string(),
            n => format!("{n} files"),
        };
        match (self.files.as_slice(), written, removed) {
            ([one], 1, 0) => format!("wrote {}", one.path),
            ([one], 0, 1) => format!("deleted {}", one.path),
            (_, written, 0) => format!("wrote {}", files(written)),
            (_, 0, removed) => format!("deleted {}", files(removed)),
            (_, written, removed) => format!("wrote {} and deleted {removed}", files(written)),
        }
    }
}

/// The table a DBC file holds, by the name `vale_assets::tables::dbc::dbc_path`
/// was given: `Spell` for `DBFilesClient\Spell.dbc`.
fn table_of(vpath: &str) -> Option<String> {
    let name = vpath.rsplit(['\\', '/']).next()?;
    let stem = name.get(..name.len().checked_sub(4)?)?;
    name[stem.len()..].eq_ignore_ascii_case(".dbc").then(|| stem.to_string())
}

/// Whether a journaled path is sent to the other editors. See *What is sent*.
fn shared(vpath: &str) -> bool {
    let first = vpath.split(['\\', '/']).next().unwrap_or("");
    !vpath.replace('/', "\\").eq_ignore_ascii_case(RECORD)
        && !first.eq_ignore_ascii_case("publish")
        && !matches!(Kind::of(vpath), Kind::Sql | Kind::Server)
}

/// Whether a path names a file inside the project: relative, with no empty,
/// `.` or `..` part, no drive and no control character, and not under
/// `publish\`. A host takes no other path from a joined editor.
fn inside(vpath: &str) -> bool {
    let first = vpath.split(['\\', '/']).next().unwrap_or("");
    !vpath.is_empty()
        && vpath.len() <= 260
        && !vpath.chars().any(|c| c.is_control() || c == ':')
        && vpath.split(['\\', '/']).all(|part| !part.is_empty() && part != "." && part != "..")
        && !first.eq_ignore_ascii_case("publish")
}

/// Whether a joined editor may write or remove the file at `vpath`, or why
/// not. The import dialogs, the lab and the bakes that a joined editor can run
/// write textures, models and sounds; every lock writes a lock file. Only the
/// host writes tiles, tables, tile indexes, the minimap index and the world
/// map's zone grids.
pub(crate) fn joinable(vpath: &str) -> Result<(), &'static str> {
    if !inside(vpath) {
        return Err("not a path inside the project");
    }
    let lower = vpath.replace('/', "\\").to_ascii_lowercase();
    match Kind::of(vpath) {
        Kind::Texture | Kind::Model | Kind::Sound => Ok(()),
        _ if lower.starts_with("editor\\locks\\") && lower.ends_with(".txt") => Ok(()),
        _ => Err("only the host changes tiles, tables and map files"),
    }
}

/// The steps the project's journal holds since the last call, and the paths
/// too large to send. A path written and then put back as it was is left
/// out.
pub(super) fn take(session: &EditSession) -> (Vec<Files>, Vec<String>) {
    let mut files = Vec::new();
    // The record as it was before this frame's writes, for the lines of the
    // files removed. See the module comment.
    let mut record_before = None;
    for written in session.project.take_journal() {
        if written.vpath.replace('/', "\\").eq_ignore_ascii_case(RECORD) {
            record_before = Some(Sources::parse(&String::from_utf8_lossy(written.before.as_deref().unwrap_or_default())));
        }
        if !shared(&written.vpath) {
            continue;
        }
        let bytes = session.project.read(&written.vpath);
        if bytes == written.before {
            continue;
        }
        files.push(File { path: written.vpath, bytes, before: written.before });
    }
    let mut steps = Vec::new();
    let mut too_large = Vec::new();
    let mut batch: Vec<File> = Vec::new();
    let mut size = 0;
    for file in files {
        let length = file.bytes.as_ref().map_or(0, Vec::len);
        if length > STEP_BYTES {
            too_large.push(file.path);
            continue;
        }
        if size + length > STEP_BYTES && !batch.is_empty() {
            steps.push(step_of(session, std::mem::take(&mut batch), record_before.as_ref()));
            size = 0;
        }
        size += length;
        batch.push(file);
    }
    if !batch.is_empty() {
        steps.push(step_of(session, batch, record_before.as_ref()));
    }
    (steps, too_large)
}

/// A step of `files`, with the import record's lines for those written, and
/// for those removed the lines `record_before` held.
fn step_of(session: &EditSession, files: Vec<File>, record_before: Option<&Sources>) -> Files {
    let written: Vec<&str> = files.iter().filter(|file| file.bytes.is_some()).map(|file| file.path.as_str()).collect();
    let imported = Sources::lines_for(&session.project, &written);
    let removed = |vpath: &str| files.iter().any(|file| file.bytes.is_none() && file.path.eq_ignore_ascii_case(vpath));
    let forgotten = record_before
        .map(|before| Sources { entries: before.entries.iter().filter(|entry| removed(&entry.vpath)).cloned().collect() }.text())
        .unwrap_or_default();
    Files { files, imported, forgotten }
}

/// Write or remove every file of a step, merge its import record lines, and
/// have the session read the files again. Nothing is journaled, so applying
/// a step sends nothing. Answers the first file that could not be written.
pub(super) fn apply(session: &mut EditSession, game: &GameAssets, step: &Files) -> Result<(), String> {
    let _unjournaled = session.project.unjournaled();
    let mut failed = None;
    for file in &step.files {
        let done = match &file.bytes {
            Some(bytes) => session.project.write(&file.path, bytes).map(|_| ()),
            None => session.project.revert(&file.path).map(|_| ()),
        };
        if let Err(e) = done {
            failed.get_or_insert(format!("{}: {e}", file.path));
        }
    }
    let written: Vec<&str> = step.files.iter().filter(|file| file.bytes.is_some()).map(|file| file.path.as_str()).collect();
    let removed: Vec<&str> = step.files.iter().filter(|file| file.bytes.is_none()).map(|file| file.path.as_str()).collect();
    if let Err(e) = Sources::merge(&session.project, &step.imported, &written, &removed) {
        failed.get_or_insert(format!("{RECORD}: {e}"));
    }
    let paths: Vec<String> = step.files.iter().map(|file| file.path.clone()).collect();
    session.reread_files(game, &paths);
    failed.map_or(Ok(()), Err)
}

/// Hold another author's step in the session's overlay, out of the folder,
/// and have the session read its files again. See *Another author's step on
/// the host*.
pub(super) fn stage(session: &mut EditSession, game: &GameAssets, author: [u8; 16], step: &Files) {
    if let Ok(mut staged) = session.staged_files.write() {
        for file in &step.files {
            staged.insert(file.path.to_ascii_lowercase(), file.bytes.clone());
        }
    }
    session.staged.push((author, step.clone()));
    let paths: Vec<String> = step.files.iter().map(|file| file.path.clone()).collect();
    session.reread_files(game, &paths);
}

/// Take an author's staged steps out of the overlay, oldest first: written
/// to the folder with `write`, for their Submit, or dropped, for a Take
/// Back. Answers the first file that could not be written.
pub(super) fn unstage(session: &mut EditSession, game: &GameAssets, author: &[u8; 16], write: bool) -> Result<(), String> {
    let (theirs, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut session.staged).into_iter().partition(|(by, _)| by == author);
    if theirs.is_empty() {
        session.staged = rest;
        return Ok(());
    }
    if let Ok(mut staged) = session.staged_files.write() {
        staged.clear();
        for (_, step) in &rest {
            for file in &step.files {
                staged.insert(file.path.to_ascii_lowercase(), file.bytes.clone());
            }
        }
    }
    session.staged = rest;
    let mut failed = Ok(());
    for (_, step) in &theirs {
        match write {
            true => {
                if let Err(e) = apply(session, game, step) {
                    failed = failed.and(Err(e));
                }
            }
            false => {
                let paths: Vec<String> = step.files.iter().map(|file| file.path.clone()).collect();
                session.reread_files(game, &paths);
            }
        }
    }
    failed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A joined editor may write textures, models, sounds and locks, and may
    /// not write tiles, tables, tile indexes, minimaps or paths outside the
    /// project.
    #[test]
    fn a_joined_editor_writes_assets_and_locks_only() {
        for good in [r"Textures\Ground\Grass.blp", r"Creature\Wolf\Wolf.m2", r"Sound\Wolf.wav", r"editor\locks\Azeroth_32_48.txt"] {
            assert_eq!(joinable(good), Ok(()), "{good}");
        }
        for host in [r"World\Maps\Azeroth\Azeroth_32_48.adt", r"DBFilesClient\Spell.dbc", r"World\Maps\Azeroth\Azeroth.wdt", r"textures\Minimap\ab.blp", r"Interface\WorldMap\Elwynn.zmp"] {
            assert!(joinable(host).is_err(), "{host}");
        }
        for outside in [r"..\x.blp", r"C:\x.blp", r"\x.blp", r"a\\b.blp", r"publish\x.blp", "a\u{0}.blp", ""] {
            assert_eq!(joinable(outside), Err("not a path inside the project"), "{outside:?}");
        }
    }

    /// A tile's file touches every group of the tile, a DBC file its table,
    /// and any file its own path; the bytes before are not sent.
    #[test]
    fn a_file_step_touches_what_its_files_replace_and_sends_no_before() {
        let step = Files {
            forgotten: String::new(),
            files: vec![
                File { path: r"World\Maps\Azeroth\Azeroth_32_48.adt".into(), bytes: None, before: Some(b"tile".to_vec()) },
                File { path: r"DBFilesClient\Spell.dbc".into(), bytes: Some(b"rows".to_vec()), before: None },
            ],
            imported: String::new(),
        };
        let groups = step.groups();
        let tile = vale_edit::TileKey::new("Azeroth", 32, 48);
        assert!(groups.contains(&Group::Placements { tile }));
        assert!(groups.contains(&Group::Table { name: "Spell".into() }));
        assert!(groups.contains(&Group::file(r"dbfilesclient\spell.dbc")));
        assert_eq!(groups.len(), 256 * 5 + 2 + 3);
        assert_eq!(step.words(), "wrote 1 file and deleted 1");

        let mut bytes = Vec::new();
        vale_collab::wire::write_frame(&mut bytes, &step).unwrap();
        let read: Files = vale_collab::wire::read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(read.files[0].before, None);
        assert_eq!(read.files[1].bytes.as_deref(), Some(&b"rows"[..]));
        assert_eq!(step.backward().files[0].bytes.as_deref(), Some(&b"tile"[..]));
    }

    /// A step taken from the journal carries what each file holds now and
    /// what it held before, leaves out saves, the server's files and a file
    /// put back as it was, and carries the import record's line for its
    /// file rather than the record. Applied to a second project, it writes
    /// and removes the same files and merges the record line.
    #[test]
    fn a_journaled_import_and_delete_reach_a_second_project() {
        let install = std::env::temp_dir().join(format!("vale-collab-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let open = |name: &str| EditSession::for_tests(vale_edit::project::Project::open(&install, name).unwrap());
        let (host, mut other) = (open("Host"), open("Other"));
        let game = GameAssets::new(install.join("Data").to_string_lossy().into_owned());
        for session in [&host, &other] {
            session.project.write(r"Textures\Old.blp", b"old").unwrap();
        }
        host.project.journal(true);

        host.project.write(r"Textures\New.blp", b"new").unwrap();
        Sources::record(
            &host.project,
            crate::import::sources::Imported { vpath: r"Textures\New.blp".into(), source: "C:\\art\\new.png".into(), settings: Vec::new() },
        )
        .unwrap();
        host.project.revert(r"Textures\Old.blp").unwrap();
        host.project.write(r"sql\creatures.sql", b"--").unwrap();
        host.project.write(r"Textures\Same.blp", b"x").unwrap();
        host.project.revert(r"Textures\Same.blp").unwrap();
        {
            let _saving = host.project.unjournaled();
            host.project.write(r"World\Maps\Azeroth\Azeroth_32_48.adt", b"saved").unwrap();
        }

        let (steps, too_large) = take(&host);
        assert!(too_large.is_empty());
        let [step] = steps.as_slice() else { panic!("{steps:?}") };
        let paths: Vec<&str> = step.files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(paths, [r"Textures\New.blp", r"Textures\Old.blp"]);
        assert_eq!(step.files[1].before.as_deref(), Some(&b"old"[..]));
        assert!(step.imported.starts_with("Textures\\New.blp\t"), "{:?}", step.imported);

        apply(&mut other, &game, step).unwrap();
        assert_eq!(other.project.read(r"Textures\New.blp").as_deref(), Some(&b"new"[..]));
        assert!(!other.project.has(r"Textures\Old.blp"));
        assert!(Sources::read(&other.project).of(r"textures\new.blp").is_some());
        assert!(other.project.take_journal().is_empty(), "applying sends nothing");

        apply(&mut other, &game, &step.backward()).unwrap();
        assert!(!other.project.has(r"Textures\New.blp"));
        assert_eq!(other.project.read(r"Textures\Old.blp").as_deref(), Some(&b"old"[..]));
        assert!(Sources::read(&other.project).of(r"textures\new.blp").is_none(), "a reverted import loses its line");
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A delete of an imported file carries the file's import line, and the
    /// delete taken back records the line again.
    #[test]
    fn a_deleted_import_taken_back_is_an_import_again() {
        let install = std::env::temp_dir().join(format!("vale-collab-forgotten-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let mut session = EditSession::for_tests(vale_edit::project::Project::open(&install, "Imports").unwrap());
        let game = GameAssets::new(install.join("Data").to_string_lossy().into_owned());
        session.project.write(r"Textures\Rock.blp", b"rock").unwrap();
        let line = crate::import::sources::Imported { vpath: r"Textures\Rock.blp".into(), source: "C:\\art\\rock.png".into(), settings: Vec::new() };
        Sources::record(&session.project, line).unwrap();
        session.project.journal(true);

        session.project.revert(r"Textures\Rock.blp").unwrap();
        Sources::forget(&session.project, r"Textures\Rock.blp");
        let (steps, _) = take(&session);
        let [step] = steps.as_slice() else { panic!("{steps:?}") };
        assert!(step.forgotten.starts_with("Textures\\Rock.blp\t"), "{:?}", step.forgotten);

        apply(&mut session, &game, &step.backward()).unwrap();
        assert_eq!(session.project.read(r"Textures\Rock.blp").as_deref(), Some(&b"rock"[..]));
        let source = Sources::read(&session.project).of(r"textures\rock.blp").map(|entry| entry.source.clone());
        assert_eq!(source, Some(std::path::PathBuf::from("C:\\art\\rock.png")));
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A tile the host creates reaches the other editor claimed, and the
    /// host's delete of it takes it away there with the other editor's
    /// unsaved edit and undo entry, as the delete did on the host.
    #[test]
    fn a_created_tile_and_its_delete_reach_the_other_editor() {
        let install = std::env::temp_dir().join(format!("vale-collab-tile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let game = GameAssets::new(String::new());
        let wdt = vale_edit::wdt::wdt_path("Azeroth");
        let open = |name: &str| {
            let project = vale_edit::project::Project::open(&install, name).unwrap();
            project.write(&wdt, &vale_edit::wdt::WdtFile::blank().write()).unwrap();
            EditSession::for_tests(project)
        };
        let (mut host, mut other) = (open("Host"), open("Other"));
        host.project.journal(true);
        let at = (30, 40);
        crate::map::tiles::create(&mut host, &game, &crate::map::tiles::Tiles::default(), at).unwrap();
        host.set_claimed(at, true);

        let (created, _) = take(&host);
        let [created] = created.as_slice() else { panic!("{created:?}") };
        assert_eq!(created.files.len(), 2, "the tile and its map's WDT");
        apply(&mut other, &game, created).unwrap();
        assert!(other.wdt_claims(at));
        let key = other.key(at);
        let tile = vale_edit::adt::AdtFile::parse(&other.project.read(&key.vpath()).unwrap()).unwrap();
        other.insert_opened(at, tile);
        other.touched(at, 0);
        other.history.begin("Raise");
        other.history.record(&key, [vale_edit::ops::Edit::Heights { chunk: 0, before: vec![0.0; 145], after: vec![1.0; 145] }]);
        other.history.end();

        crate::import::assets::delete_files(&mut host, &game, &[key.vpath()]);
        let (deleted, _) = take(&host);
        let [deleted] = deleted.as_slice() else { panic!("{deleted:?}") };
        assert!(deleted.groups().contains(&Group::Placements { tile: key.clone() }));
        apply(&mut other, &game, deleted).unwrap();
        assert!(!other.project.has(&key.vpath()));
        assert!(!other.wdt_claims(at));
        assert!(!other.tiles.contains_key(&at));
        assert!(!other.unsaved.contains(&at));
        assert_eq!(other.history.depth_done(), 0);
        let _ = std::fs::remove_dir_all(&install);
    }
}
