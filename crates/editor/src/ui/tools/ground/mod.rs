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
//! ```

pub mod areas;
pub mod chunks;
pub mod grade;
pub mod holes;
pub mod road;
pub mod shading;
pub mod terrain;
pub mod textures;
pub mod water;
