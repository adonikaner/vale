//! Reading WoW 1.12 game data off disk.
//!
//! ```text
//! archive.rs   the MPQ chain (patch load order) — the container handling
//!              itself is delegated to the `wow-mpq` crate
//!
//! world/       the files that describe a *place*: the chunk format, the tile
//!              grid, one tile, the models, the buildings, the textures, what
//!              is solid, and the height at a position
//! tables/      the DBC chain, one file per question it answers: which model,
//!              which sound, which colour, which page, which parchment
//! interface/   `Interface\` itself — the load order, the markup, the widget
//!              vocabulary, the fonts, the strings and the key bindings
//! look/        …and the join the three of them meet in: what to draw a unit
//!              as, what colour its ring is, and where to stand to see it
//! ```
//!
//! The container format is a solved problem and delegated. The formats layered
//! on top are written by hand here, because that is the part this project needs
//! to understand, extend, and eventually feed to a renderer.
//!
//! ## Coordinates
//!
//! WoW's world axes are unusual and a frequent source of mirrored terrain:
//! the map is a 64x64 grid of tiles, tile (32, 32) straddles the origin, and
//! **+X runs north while +Y runs west**. Height is +Z (up). See
//! [`world::adt::TILE_SIZE`] and [`world::adt::MAP_ORIGIN`].

pub mod archive;

pub mod interface;
pub mod look;
pub mod tables;
pub mod world;

pub use world::adt::Adt;
pub use archive::Assets;
pub use world::blp::Blp;
pub use world::collision::{Collider, CollisionMesh, CollisionWorld};
pub use tables::dbc::Dbc;
pub use world::m2::M2;
pub use world::terrain::{tile_for_position, MapTerrain, Terrain};
pub use world::wdt::Wdt;
pub use world::wmo::{WmoModel, WmoRoot};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("no .MPQ archives found in {0} (expected a WoW 1.12 Data directory)")]
    NoArchives(String),

    #[error("file not found in any archive: {0}")]
    NotFound(String),

    #[error("{format} is malformed: {detail}")]
    Malformed {
        format: &'static str,
        detail: String,
    },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl AssetError {
    pub(crate) fn malformed(format: &'static str, detail: impl Into<String>) -> Self {
        AssetError::Malformed {
            format,
            detail: detail.into(),
        }
    }
}

/// Virtual path of a map's WDT, e.g. `World\Maps\Azeroth\Azeroth.wdt`.
pub fn wdt_path(map: &str) -> String {
    format!("World\\Maps\\{map}\\{map}.wdt")
}

/// Virtual path of one terrain tile, e.g. `World\Maps\Azeroth\Azeroth_32_48.adt`.
///
/// Note the ordering: the **x** (column) index comes first in the filename,
/// even though the WDT tile table is indexed row-major.
pub fn adt_path(map: &str, x: u32, y: u32) -> String {
    format!("World\\Maps\\{map}\\{map}_{x}_{y}.adt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_use_backslashes_and_x_first() {
        assert_eq!(wdt_path("Azeroth"), "World\\Maps\\Azeroth\\Azeroth.wdt");
        assert_eq!(
            adt_path("Azeroth", 32, 48),
            "World\\Maps\\Azeroth\\Azeroth_32_48.adt"
        );
    }
}
