//! Editing the game's own files.
//!
//! ```text
//! adt/       one ADT terrain tile, parsed and written back byte for byte
//! dbc/       one DBC table, parsed and written back byte for byte
//! wdt.rs     the WDT map file, which lists the tiles a map has
//! shadow.rs  MCSH recomputed: the shadow cast across a chunk by the ground
//!            and the objects standing on it
//! minimap.rs a tile's minimap picture, drawn from the tile
//! flightmap.rs the row a map needs before the client opens its flight map,
//!            and the picture drawn behind the nodes
//! ops/       the edit operations: how each is applied, inverted and named
//! undo.rs    the undo stack, from which edits are undone and redone
//! m2/        writing models: a rigid transform baked into a copy of a model,
//!            which is how 1.12 positions a spell effect, and a whole model
//!            written from a glTF file with its skeleton and animations
//! sound.rs   a WAV or MP3 file read to samples, and samples written as the
//!            16-bit PCM WAV the shipped sounds are
//! project.rs where an edit is written, and how it reaches a client
//! manifest.rs what a project changes, file by file, against the files it
//!            was edited from
//! ```
//!
//! This crate opens no window. It is split from `vale-ide` the way
//! `vale-assets` is split from `vale-client`: a decision that can be made with
//! no renderer running is made here, where it can be unit-tested and where a
//! command-line check can call the same code the editor calls.
//!
//! ## What this crate may not decide
//!
//! Every rule about what a file means belongs to `vale-assets` and is
//! called from here rather than restated: the 145-vertex layout of a map chunk,
//! the tile grid, the world axes, the chunk origin sitting at the maximum x and
//! y corner. An editor with its own copy of any of those can place a doodad a
//! unit off the ground the client walks on, with neither copy visibly wrong.
//!
//! ## Why an edit leaves every unedited byte as it was
//!
//! An editor that rewrites a file it only partly understands destroys the parts
//! it does not. [`adt::AdtFile`] is therefore a container rather than a model:
//! every region of the file is carried as it was written, and only the regions
//! an edit names are replaced. [`adt::tests`] checks this with a round trip:
//! it parses a real tile, writes it back, and compares the bytes.

pub mod adt;
pub mod dbc;
pub mod flightmap;
pub mod m2;
pub mod manifest;
pub mod ops;
pub mod minimap;
pub mod project;
pub mod shadow;
pub mod sound;
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

    /// A request this crate refuses, such as an invalid project name or
    /// deleting the default project that every install has. It is neither a
    /// filesystem failure (`Io`) nor malformed data (`Malformed`).
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A tile path reads back to its map and numbers with either separator,
    /// and the file name's map is matched in any case. Any other path is no
    /// tile.
    #[test]
    fn a_tile_path_is_read_back_to_its_map_and_numbers() {
        assert_eq!(
            TileKey::from_vpath(r"World\Maps\Azeroth\Azeroth_32_48.adt"),
            Some(TileKey::new("Azeroth", 32, 48))
        );
        assert_eq!(
            TileKey::from_vpath("World/Maps/Kalimdor/Kalimdor_3_57.adt"),
            Some(TileKey::new("Kalimdor", 3, 57))
        );
        assert_eq!(
            TileKey::from_vpath(r"world\maps\Azeroth\azeroth_32_48.ADT"),
            Some(TileKey::new("Azeroth", 32, 48))
        );
        assert_eq!(TileKey::from_vpath(r"World\Maps\Azeroth\Azeroth.wdt"), None);
        assert_eq!(TileKey::from_vpath(r"DBFilesClient\Spell.dbc"), None);
        assert_eq!(TileKey::from_vpath(r"World\Maps\Azeroth\Other_32_48.adt"), None);
        let key = TileKey::new("Azeroth", 32, 48);
        assert_eq!(TileKey::from_vpath(&key.vpath()), Some(key));
    }
}
