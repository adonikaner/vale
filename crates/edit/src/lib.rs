//! Editing the game's own files.
//!
//! ```text
//! adt/       one terrain tile, taken apart and put back together byte for byte
//! dbc/       …and one client table, on the same terms
//! wdt.rs     …and the map file that says which tiles exist at all
//! shadow.rs  MCSH recomputed: what the world on a chunk throws across it
//! minimap.rs …and the little round map's picture, drawn from the tile
//! ops.rs     what an edit is: applied, inverted, and named
//! undo.rs    …and the stack the two directions are read off
//! m2.rs      a rigid transform baked into a copy of a model, which is how 1.12
//!            positions a spell effect: the tables carry no offset, so the
//!            offset lives in the file
//! project.rs where an edit is written, and how it reaches a client
//! manifest.rs …and what a project changes, file by file, against the files
//!            it was edited from
//! ```
//!
//! Nothing here opens a window, and nothing here needs one. The split against
//! `vale-ide` is the same one `vale-assets` keeps against
//! `vale-client`: a decision that could be made with no renderer running is
//! made here, where it can be unit-tested and where a command-line check can
//! call the same copy the editor calls.
//!
//! ## What this crate may not decide
//!
//! Every rule about what a file means belongs to `vale-assets` and is
//! called from here rather than restated: the 145-vertex layout of a map chunk,
//! the tile grid, the world axes, the chunk origin sitting at the maximum x and
//! y corner. An editor with its own copy of any of those places a doodad a unit
//! off the ground the client walks on, and neither side is visibly wrong.
//!
//! ## Losslessness is the first requirement
//!
//! An editor that rewrites a file it only partly understands destroys the parts
//! it does not. [`adt::AdtFile`] is therefore a container rather than a model:
//! every region of the file is carried as it was written, and only the regions
//! an edit names are replaced. The check that this holds is
//! [`adt::tests`]' round trip — parse a real tile, write it back, and compare
//! the bytes.

pub mod adt;
pub mod dbc;
pub mod m2;
pub mod manifest;
pub mod ops;
pub mod minimap;
pub mod project;
pub mod shadow;
pub mod undo;
pub mod wdt;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EditError {
    #[error("{format} is malformed: {detail}")]
    Malformed {
        format: &'static str,
        detail: String,
    },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("writing the archive failed: {0}")]
    Archive(String),

    /// What was asked for is not a thing this crate will do — a project
    /// name that is not one, deleting the project every install has. It is
    /// not a failure of the filesystem and it is not malformed data, so it is
    /// neither of the two above.
    #[error("{0}")]
    Refused(String),
}

impl EditError {
    pub(crate) fn malformed(format: &'static str, detail: impl Into<String>) -> Self {
        EditError::Malformed {
            format,
            detail: detail.into(),
        }
    }
}

/// Which tile of which map an edit belongs to.
///
/// The map is named rather than numbered because that is what the file path
/// carries; `MapTerrain` and the WDT both key on the same string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey {
    pub map: String,
    pub x: u32,
    pub y: u32,
}

impl TileKey {
    pub fn new(map: impl Into<String>, x: u32, y: u32) -> TileKey {
        TileKey {
            map: map.into(),
            x,
            y,
        }
    }

    /// The virtual path this tile is read from and written to.
    pub fn vpath(&self) -> String {
        vale_assets::adt_path(&self.map, self.x, self.y)
    }

    /// The tile a virtual path names: `World\Maps\<map>\<map>_<x>_<y>.adt`,
    /// with either separator and in any case. `None` for any other path.
    pub fn from_vpath(vpath: &str) -> Option<TileKey> {
        let parts: Vec<&str> = vpath.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
        let [world, maps, map, file] = parts.as_slice() else {
            return None;
        };
        if !world.eq_ignore_ascii_case("World") || !maps.eq_ignore_ascii_case("Maps") {
            return None;
        }
        let stem = file
            .strip_suffix(".adt")
            .or_else(|| file.strip_suffix(".ADT"))?;
        let prefix = stem.get(..map.len())?;
        if !prefix.eq_ignore_ascii_case(map) {
            return None;
        }
        let (x, y) = stem[map.len()..].strip_prefix('_')?.split_once('_')?;
        Some(TileKey::new(*map, x.parse().ok()?, y.parse().ok()?))
    }
}

impl std::fmt::Display for TileKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}", self.map, self.x, self.y)
    }
}
