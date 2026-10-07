//! What a project changes, file by file, against the files it was edited
//! from.
//!
//! [`project::Summary`] counts a project's files by kind, which is enough to
//! tell two projects apart in a list. This module says what each file does:
//! a tile is compared against the archives' tile chunk by chunk
//! ([`adt::diff`]), a table against the archives' table row by row
//! ([`dbc::diff`]), and every other file is named with its kind and size.
//!
//! The originals come from a reader the caller supplies, because this crate
//! does not open the archives. The editor passes a read that steps over the
//! project's own overlay; a test passes a closure over a map.
//!
//! The server bookkeeping under `sql\` and `server\` is listed by file only.
//! What those files mean is `vale-ide`'s, which reads them with the
//! writers' own readers and adds the row counts.

use crate::adt::{self, AdtFile};
use crate::dbc::{self, DbcFile};
use crate::project::Project;
use crate::TileKey;

/// The kind of file a virtual path names, by its extension and its first
/// path component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `.adt`: a tile, edited or made.
    Tile,
    /// `.dbc`: a client table.
    Table,
    /// `.m2`: a baked model copy.
    Model,
    /// `.blp` under `textures\Minimap\`: a tile's minimap picture.
    Minimap,
    /// Any other `.blp`.
    Texture,
    /// `.wdt`: a map's tile index.
    Wdt,
    /// Anything under `sql\`: statements for the server and what puts them
    /// back.
    Sql,
    /// Anything under `server\`: the editor's record of what it changes on
    /// the server, and the server files it saved before replacing them.
    Server,
    Other,
}

impl Kind {
    pub fn of(vpath: &str) -> Kind {
        let lower = vpath.replace('/', "\\").to_ascii_lowercase();
        let first = lower.split('\\').next().unwrap_or("");
        if first == "sql" {
            return Kind::Sql;
        }
        if first == "server" {
            return Kind::Server;
        }
        match lower.rsplit('.').next() {
            Some("adt") => Kind::Tile,
            Some("dbc") => Kind::Table,
            Some("m2") => Kind::Model,
            Some("blp") if lower.starts_with("textures\\minimap\\") => Kind::Minimap,
            Some("blp") => Kind::Texture,
            Some("wdt") => Kind::Wdt,
            _ => Kind::Other,
        }
    }

    /// `(singular, plural)`, for a count.
    pub fn words(self) -> (&'static str, &'static str) {
        match self {
            Kind::Tile => ("tile", "tiles"),
            Kind::Table => ("table", "tables"),
            Kind::Model => ("model", "models"),
            Kind::Minimap => ("minimap", "minimaps"),
            Kind::Texture => ("texture", "textures"),
            Kind::Wdt => ("WDT", "WDTs"),
            Kind::Sql => ("SQL file", "SQL files"),
            Kind::Server => ("server record", "server records"),
            Kind::Other => ("other file", "other files"),
        }
    }
}

/// What a compared file came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The archives have no file at this path: the project made it.
    New,
    /// The project's file is the archives' file.
    Same,
    /// The differences, as one line.
    Differs(String),
    /// One of the two files did not parse, with the reason.
    Unreadable(String),
}

impl Change {
    pub fn line(&self) -> String {
        match self {
            Change::New => "new".to_string(),
            Change::Same => "same as in the archives".to_string(),
            Change::Differs(line) => line.clone(),
            Change::Unreadable(why) => format!("not compared: {why}"),
        }
    }
}

/// One tile the project carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileEntry {
    pub key: TileKey,
    pub vpath: String,
    pub bytes: u64,
    pub change: Change,
}

/// One table the project carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableEntry {
    /// The bare name: `Spell` for `DBFilesClient\Spell.dbc`.
    pub name: String,
    pub vpath: String,
    pub bytes: u64,
    /// Records in the project's copy, when it parsed.
    pub rows: Option<usize>,
    pub change: Change,
}

/// Any other file, by path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub vpath: String,
    pub kind: Kind,
    pub bytes: u64,
}

/// Every file of a project, described.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    pub tiles: Vec<TileEntry>,
    pub tables: Vec<TableEntry>,
    /// Models, textures, minimaps, WDTs and anything unclassified, in path
    /// order.
    pub files: Vec<FileEntry>,
    /// The `sql\` files, in path order.
    pub sql: Vec<FileEntry>,
    /// The `server\` files, in path order.
    pub server: Vec<FileEntry>,
}

impl Manifest {
    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
            && self.tables.is_empty()
            && self.files.is_empty()
            && self.sql.is_empty()
            && self.server.is_empty()
    }

    pub fn total(&self) -> usize {
        self.tiles.len() + self.tables.len() + self.files.len() + self.sql.len() + self.server.len()
    }
}

/// Describe every file in `project`, comparing tiles and tables against what
/// `original` answers for the same path. `None` from the reader is a path
/// the archives do not have.
pub fn manifest(project: &Project, mut original: impl FnMut(&str) -> Option<Vec<u8>>) -> Manifest {
    let mut out = Manifest::default();
    for vpath in project.files() {
        let bytes = project
            .path_for(&vpath)
            .and_then(|at| std::fs::metadata(at).ok())
            .map_or(0, |meta| meta.len());
        let kind = Kind::of(&vpath);
        match kind {
            Kind::Tile => {
                let Some(key) = TileKey::from_vpath(&vpath) else {
                    out.files.push(FileEntry { vpath, kind: Kind::Other, bytes });
                    continue;
                };
                let change = compare(project.read(&vpath), original(&vpath), |edited, was| {
                    let edited = AdtFile::parse(edited).map_err(|e| e.to_string())?;
                    let was = AdtFile::parse(was).map_err(|e| e.to_string())?;
                    let d = adt::diff::diff(&edited, &was);
                    Ok((!d.is_empty()).then(|| d.line()))
                });
                out.tiles.push(TileEntry { key, vpath, bytes, change });
            }
            Kind::Table => {
                let name = table_name(&vpath);
                let mut rows = None;
                let change = compare(project.read(&vpath), original(&vpath), |edited, was| {
                    let edited = DbcFile::parse(edited).map_err(|e| e.to_string())?;
                    rows = Some(edited.record_count());
                    let was = DbcFile::parse(was).map_err(|e| e.to_string())?;
                    let d = dbc::diff::diff(&edited, &was);
                    Ok((!d.is_empty()).then(|| d.line()))
                });
                if rows.is_none() {
                    rows = project
                        .read(&vpath)
                        .and_then(|bytes| DbcFile::parse(&bytes).ok())
                        .map(|table| table.record_count());
                }
                out.tables.push(TableEntry { name, vpath, bytes, rows, change });
            }
            Kind::Sql => out.sql.push(FileEntry { vpath, kind, bytes }),
            Kind::Server => out.server.push(FileEntry { vpath, kind, bytes }),
            _ => out.files.push(FileEntry { vpath, kind, bytes }),
        }
    }
    out
}

/// Run `differ` over the two files when both are there. `Ok(None)` from it
/// is [`Change::Same`], `Ok(Some(line))` is [`Change::Differs`].
fn compare(
    edited: Option<Vec<u8>>,
    original: Option<Vec<u8>>,
    differ: impl FnOnce(&[u8], &[u8]) -> Result<Option<String>, String>,
) -> Change {
    let Some(edited) = edited else {
        return Change::Unreadable("the project's file could not be read".to_string());
    };
    let Some(original) = original else {
        return Change::New;
    };
    match differ(&edited, &original) {
        Ok(None) => Change::Same,
        Ok(Some(line)) => Change::Differs(line),
        Err(why) => Change::Unreadable(why),
    }
}

/// `Spell` from `DBFilesClient\Spell.dbc`.
fn table_name(vpath: &str) -> String {
    let file = vpath.rsplit(['\\', '/']).next().unwrap_or(vpath);
    file.strip_suffix(".dbc")
        .or_else(|| file.strip_suffix(".DBC"))
        .unwrap_or(file)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adt::{blank_tile, heights};
    use std::collections::HashMap;

    fn install(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("vale-manifest-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn table(rows: &[(u32, u32)]) -> Vec<u8> {
        let strings: &[u8] = b"\0first\0second\0";
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

    #[test]
    fn a_path_is_classified_by_its_extension_and_its_first_component() {
        assert_eq!(Kind::of(r"World\Maps\Azeroth\Azeroth_32_48.adt"), Kind::Tile);
        assert_eq!(Kind::of(r"DBFilesClient\Spell.dbc"), Kind::Table);
        assert_eq!(Kind::of(r"Custom\Thing_pos.m2"), Kind::Model);
        assert_eq!(Kind::of(r"textures\Minimap\ea28.blp"), Kind::Minimap);
        assert_eq!(Kind::of(r"Tileset\Elwynn\Grass.blp"), Kind::Texture);
        assert_eq!(Kind::of(r"World\Maps\Azeroth\Azeroth.wdt"), Kind::Wdt);
        assert_eq!(Kind::of(r"sql\creatures.sql"), Kind::Sql);
        assert_eq!(Kind::of(r"server\dbc-before\TaxiPath.dbc"), Kind::Server);
        assert_eq!(Kind::of(r"notes.txt"), Kind::Other);
        assert_eq!(Kind::of("sql/world.sql"), Kind::Sql);
    }

    /// Each file lands in its list, and a tile or a table is compared
    /// against what the reader answers for its path.
    #[test]
    fn every_file_is_described_against_its_original() {
        let install = install("described");
        let project = Project::open(&install, "goldshire").unwrap();

        let original_tile = blank_tile(32, 48, r"Tileset\Elwynn\ElwynnGrass01.blp", 100.0, 12);
        let mut edited_tile = original_tile.clone();
        let chunk = edited_tile.chunk_mut(3).unwrap();
        let mut raised = heights::heights(chunk);
        raised[7] += 2.0;
        heights::set_heights(chunk, &raised);
        let made_tile = blank_tile(32, 49, r"Tileset\Elwynn\ElwynnGrass01.blp", 100.0, 12);

        project
            .write(r"World\Maps\Azeroth\Azeroth_32_48.adt", &edited_tile.write())
            .unwrap();
        project
            .write(r"World\Maps\Azeroth\Azeroth_32_49.adt", &made_tile.write())
            .unwrap();
        project
            .write(r"DBFilesClient\Spell.dbc", &table(&[(10, 1), (20, 1), (40, 7)]))
            .unwrap();
        project
            .write(r"DBFilesClient\TaxiNodes.dbc", &table(&[(1, 0)]))
            .unwrap();
        project.write(r"Custom\Thing_pos.m2", b"m").unwrap();
        project.write(r"World\Maps\Azeroth\Azeroth.wdt", b"w").unwrap();
        project.write(r"sql\creatures.sql", b"-- row creature guid=1\n").unwrap();
        project.write(r"server\rows.txt", b"").unwrap();

        let mut archives: HashMap<String, Vec<u8>> = HashMap::new();
        archives.insert(
            r"World\Maps\Azeroth\Azeroth_32_48.adt".to_string(),
            original_tile.write(),
        );
        archives.insert(
            r"DBFilesClient\Spell.dbc".to_string(),
            table(&[(10, 1), (20, 7), (30, 0)]),
        );
        archives.insert(r"DBFilesClient\TaxiNodes.dbc".to_string(), table(&[(1, 0)]));

        let m = manifest(&project, |vpath| archives.get(vpath).cloned());
        assert_eq!(m.total(), 8);
        assert_eq!(m.tiles.len(), 2);
        assert_eq!(m.tiles[0].key, TileKey::new("Azeroth", 32, 48));
        assert_eq!(
            m.tiles[0].change,
            Change::Differs("heights in 1 chunk".to_string())
        );
        assert_eq!(m.tiles[1].key, TileKey::new("Azeroth", 32, 49));
        assert_eq!(m.tiles[1].change, Change::New);
        assert!(m.tiles[0].bytes > 0);

        assert_eq!(m.tables.len(), 2);
        assert_eq!(m.tables[0].name, "Spell");
        assert_eq!(m.tables[0].rows, Some(3));
        assert_eq!(
            m.tables[0].change,
            Change::Differs("1 row changed (20), 1 added (40), 1 removed (30)".to_string())
        );
        assert_eq!(m.tables[1].name, "TaxiNodes");
        assert_eq!(m.tables[1].change, Change::Same);

        let kinds: Vec<Kind> = m.files.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, vec![Kind::Model, Kind::Wdt]);
        assert_eq!(m.sql.len(), 1);
        assert_eq!(m.server.len(), 1);
        let _ = std::fs::remove_dir_all(install);
    }

    #[test]
    fn a_file_that_will_not_parse_is_reported_rather_than_skipped() {
        let install = install("unreadable");
        let project = Project::open(&install, "broken").unwrap();
        project
            .write(r"World\Maps\Azeroth\Azeroth_32_48.adt", b"not a tile")
            .unwrap();
        let m = manifest(&project, |_| Some(b"nor this".to_vec()));
        assert_eq!(m.tiles.len(), 1);
        assert!(matches!(m.tiles[0].change, Change::Unreadable(_)), "{:?}", m.tiles[0].change);
        assert!(m.tiles[0].change.line().starts_with("not compared: "));
        let _ = std::fs::remove_dir_all(install);
    }
}
