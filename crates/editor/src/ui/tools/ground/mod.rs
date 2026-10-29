//! The panels of the ground tools, one file per tool at the same stem as its
//! state in `crate::tools::ground`.
//!
//! ```text
//! terrain.rs    the height brush, the vertex selection, and the switch that
//!               moves placed objects with the ground
//! grade.rs      the grade tool: a ramp between two picked points
//! road.rs       the road tool: the points along a road, and its textures
//! shading.rs    the vertex colour brush
//! textures.rs   the texture brush, the tileset picker, and the layers of the
//!               chunk under the pointer
//! holes.rs      the holes tool
//! areas.rs      the area under the pointer, the area tree, and the
//!               impassable flag
//! chunks.rs     a selection of chunks: copy and paste, stitching, areas,
//!               holes and textures
//! water.rs      the water tool
//! presets.rs    the brush preset grid and drop-down the terrain and texture
//!               panels share
//! ```

pub(crate) mod areas;
pub(crate) mod chunks;
pub(crate) mod grade;
pub(crate) mod holes;
pub(crate) mod presets;
pub(crate) mod road;
pub(crate) mod shading;
pub(crate) mod terrain;
pub(crate) mod textures;
pub(crate) mod water;
