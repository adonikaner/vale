//! The archives' own copies: telling a project file that changes nothing
//! from one that does, and removing the first kind when the project opens.

use vale_client::assets::GameAssets;
use vale_edit::dbc::DbcFile;
use vale_edit::project::Project;
use vale_edit::TileKey;
use bevy::prelude::*;
use bevy::tasks::{block_on, AsyncComputeTaskPool, Task};
use bevy::tasks::futures_lite::future;
use std::sync::Arc;
use super::EditSession;

/// The archives' own files, read past this project's archives in `Data\`,
/// for telling a tile that changes something from one that changes nothing.
///
/// Reads through a chain borrowed from the client's pool (`GameAssets::chains`)
/// rather than through `GameAssets`, so the check [`Self::look_for_unchanged`]
/// starts can hold one on a worker. A pool chain has no overlay, so it answers
/// the archives' bytes whatever the project holds.
#[derive(Clone)]
pub struct Shipped {
    chains: Arc<vale_assets::archive::ChainPool>,
    /// The folder the pool is keyed on: `GameAssets::gamedata_dir` as it is
    /// written. A borrow under any other spelling empties the pool, and the
    /// tile loaders would open their chains again.
    gamedata_dir: String,
}

impl Shipped {
    pub fn new(assets: &GameAssets) -> Shipped {
        Shipped {
            chains: assets.chains(),
            gamedata_dir: assets.gamedata_dir.clone(),
        }
    }

    /// The archives' own file at `vpath`, read past this project's own
    /// archives. `None` when the archives do not have it or cannot be read,
    /// so a file is taken out of a project only on a positive answer.
    pub fn original(&self, project: &Project, vpath: &str) -> Option<Vec<u8>> {
        let mut chain = self.chains.take(&self.gamedata_dir).ok()?;
        chain
            .read_past_overlay_skipping(vpath, &own_archives(project))
            .ok()
    }

    /// Start finding, on a worker, every file in `project` that changes
    /// nothing the archives' copy has ([`changes_nothing`]).
    ///
    /// Save stops a file like this from being written, but a project saved
    /// before it did may hold some. The check reads each file the folder
    /// holds and the archives' copy, so its cost grows with the number of
    /// files, and it runs off the main thread. The server's files (`sql\`,
    /// `server\`) are the project's own and are not read.
    /// [`drop_unchanged_tiles`] collects the answer and removes the files.
    pub(super) fn look_for_unchanged(&self, project: &Project) -> Task<Vec<Unchanged>> {
        let shipped = self.clone();
        let project = project.clone();
        AsyncComputeTaskPool::get().spawn(async move {
            let own = own_archives(&project);
            let Ok(mut chain) = shipped.chains.take(&shipped.gamedata_dir) else {
                return Vec::new();
            };
            use vale_edit::manifest::Kind;
            project
                .files()
                .into_iter()
                .filter(|vpath| !matches!(Kind::of(vpath), Kind::Sql | Kind::Server))
                .filter_map(|vpath| {
                    let held = match Kind::of(&vpath) {
                        Kind::Tile => Held::Tile(TileKey::from_vpath(&vpath)?),
                        Kind::Table => Held::Table(table_name(&vpath)),
                        _ => Held::File,
                    };
                    let path = project.path_for(&vpath)?;
                    // Read before the bytes, so a save that lands between
                    // the two leaves a record that no longer matches the file
                    // and the file is kept.
                    let meta = std::fs::metadata(&path).ok()?;
                    let bytes = std::fs::read(&path).ok()?;
                    let shipped = chain.read_past_overlay_skipping(&vpath, &own).ok()?;
                    changes_nothing(&vpath, &bytes, &shipped).then(|| Unchanged {
                        vpath,
                        held,
                        len: meta.len(),
                        modified: meta.modified().ok(),
                    })
                })
                .collect()
        })
    }
}

/// This project's own archives in `Data\`, which a comparison with the
/// archives reads past. A chain that included them would compare an edit
/// with its own published copy.
fn own_archives(project: &Project) -> Vec<String> {
    project
        .published()
        .into_iter()
        .map(|archive| archive.name)
        .collect()
}

/// Whether a project's copy of a file changes nothing the archives' copy
/// has.
///
/// A table is compared row by row ([`vale_edit::dbc::diff`], which the
/// project dialog's "same as in the archives" is): [`DbcFile::set_string`]
/// only appends to the string block, so a table whose string edit was undone
/// has the archives' rows and not the archives' bytes. A table that does not
/// parse on either side falls back to its bytes. Every other file, tiles
/// among them, is compared byte for byte: `vale_edit::adt::diff` does not
/// compare every chunk, so a tile it calls the same can still differ.
pub(super) fn changes_nothing(vpath: &str, edited: &[u8], original: &[u8]) -> bool {
    if edited == original {
        return true;
    }
    if vale_edit::manifest::Kind::of(vpath) != vale_edit::manifest::Kind::Table {
        return false;
    }
    match (DbcFile::parse(edited), DbcFile::parse(original)) {
        (Ok(edited), Ok(original)) => {
            edited.field_count() == original.field_count()
                && edited.record_size() == original.record_size()
                && vale_edit::dbc::diff::diff(&edited, &original).is_empty()
        }
        _ => false,
    }
}

/// `Spell` from `DBFilesClient\Spell.dbc`.
fn table_name(vpath: &str) -> String {
    let file = vpath.rsplit(['\\', '/']).next().unwrap_or(vpath);
    let cut = file.len().saturating_sub(4);
    match file.get(cut..).is_some_and(|tail| tail.eq_ignore_ascii_case(".dbc")) {
        true => file[..cut].to_string(),
        false => file.to_string(),
    }
}

/// What a file the open-time check found is, for the unsaved state that
/// keeps it.
enum Held {
    Tile(TileKey),
    /// A table, by its bare name.
    Table(String),
    /// Any other file. The editor writes these whole when it saves them, so
    /// none has unsaved state of its own.
    File,
}

/// A file the open-time check found changes nothing the archives' copy has,
/// and the size and time the file had when it was read.
pub(super) struct Unchanged {
    vpath: String,
    held: Held,
    len: u64,
    modified: Option<std::time::SystemTime>,
}

/// Remove the files [`Shipped::look_for_unchanged`] found, once it has
/// finished.
///
/// A file is kept when its tile or table has unsaved edits, or when its size
/// or time has changed since the check read it: a save landed in between,
/// and the check's answer is about bytes that are no longer there.
pub(super) fn drop_unchanged_tiles(session: Option<ResMut<EditSession>>) {
    let Some(mut session) = session else { return };
    if session.unchanged.is_none() {
        return;
    }
    let Some(found) = session
        .unchanged
        .as_mut()
        .and_then(|task| block_on(future::poll_once(task)))
    else {
        return;
    };
    session.unchanged = None;
    let mut dropped = 0;
    for one in found {
        let unsaved = match &one.held {
            Held::Tile(key) => {
                key.map.eq_ignore_ascii_case(&session.map)
                    && session.unsaved.contains(&(key.x, key.y))
            }
            Held::Table(name) => session.unsaved_tables.contains(name),
            Held::File => false,
        };
        if unsaved {
            continue;
        }
        let Some(path) = session.project.path_for(&one.vpath) else {
            continue;
        };
        let as_read = std::fs::metadata(&path)
            .is_ok_and(|meta| meta.len() == one.len && meta.modified().ok() == one.modified);
        if as_read && session.project.revert(&one.vpath).is_ok_and(|gone| gone) {
            dropped += 1;
        }
    }
    if dropped > 0 {
        let line = format!(
            "{dropped} files in {} match the archives, so they are no longer kept",
            session.project.name
        );
        info!("{line}");
        session.status = line;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-field table: an id and a string offset per row, then the string
    /// block as given.
    fn dbc(rows: &[(u32, u32)], strings: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for (id, at) in rows {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&at.to_le_bytes());
        }
        out.extend_from_slice(strings);
        out
    }

    /// A table whose string edit was undone carries the text it no longer
    /// uses at the end of its string block. Its rows are the archives', so it
    /// changes nothing and is not kept; a changed row is.
    #[test]
    fn a_table_with_the_archives_rows_changes_nothing() {
        let path = r"DBFilesClient\Spell.dbc";
        let original = dbc(&[(1, 1), (2, 7)], b"\0first\0second\0");
        let undone = dbc(&[(1, 1), (2, 7)], b"\0first\0second\0edited\0");
        let changed = dbc(&[(1, 1), (2, 14)], b"\0first\0second\0edited\0");
        assert!(changes_nothing(path, &original, &original));
        assert!(changes_nothing(path, &undone, &original));
        assert!(!changes_nothing(path, &changed, &original));
        assert_eq!(table_name(path), "Spell");
    }

    /// Any other file is compared by its bytes, tiles among them.
    #[test]
    fn any_other_file_changes_nothing_only_when_its_bytes_are_the_same() {
        let wdt = r"World\Maps\Azeroth\Azeroth.wdt";
        assert!(changes_nothing(wdt, b"abc", b"abc"));
        assert!(!changes_nothing(wdt, b"abcd", b"abc"));
        let tile = r"World\Maps\Azeroth\Azeroth_32_48.adt";
        assert!(!changes_nothing(tile, b"MVER", b"MVER\0"));
    }
}
