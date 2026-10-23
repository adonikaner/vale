//! The tools that edit the terrain itself.
//!
//! ```text
//! terrain.rs     the height brush: raise, lower, flatten, smooth
//! grade.rs       a ramp between two points, which the height brush cannot make
//! road.rs        a road along a run of points: the ground graded along it and
//!                its textures painted, in one press
//! shading.rs     the colour painted onto the terrain's vertices: MCCV
//! textures.rs    the texture layers the terrain is painted with, and their
//!                blend maps
//! holes.rs       where the terrain is not drawn: the sixteen bits per chunk
//! areas.rs       which place in the world a chunk is: one AreaTable id per
//!                chunk
//! chunks.rs      a selection of map chunks, by click and by a block dragged on
//!                the ground, and the area, flag, hole and texture operations
//!                that write one value to every chunk in it
//! water.rs       the water standing on the terrain: MCLQ, and the flags that
//!                declare it
//! ```

pub(crate) mod areas;
pub(crate) mod chunks;
pub(crate) mod grade;
pub(crate) mod holes;
pub(crate) mod road;
pub(crate) mod shading;
pub(crate) mod terrain;
pub(crate) mod textures;
pub(crate) mod water;
