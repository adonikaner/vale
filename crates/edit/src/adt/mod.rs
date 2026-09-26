//! One terrain tile, taken apart and put back together byte for byte.
//!
//! ```text
//! file.rs     the container: top-level chunks, the 256 map chunks, the writer
//! blank.rs    …and a tile made from nothing, for ground that was not there
//! header.rs   the 128 bytes at the front of a map chunk, field by field
//! heights.rs  MCVT and MCNR: the shape of the ground and the light on it
//! colours.rs  …and MCCV, the shading painted onto its vertices — the one
//!             region this crate adds rather than edits
//! holes.rs    …and the sixteen bits that say where it is not drawn at all
//! area.rs     …and the one word that says which place in the world it is
//! impass.rs   …and the one bit that says a server may not walk on it
//! liquid.rs   MCLQ: the water standing on it, which the header declares
//! alpha.rs    MCLY and MCAL: what it is painted with, and the first region an
//!             edit can change the length of
//! mesh.rs     …and where an edited chunk's vertices sit in a tile mesh, so a
//!             renderer can be handed the new ones without reading the file
//! place.rs    MDDF and MODF: what stands on it
//! relocate.rs …and moving the whole of it to other coordinates, which is two
//!             frames and one axis swap
//! diff.rs     what one tile changes against another, counted by kind
//! ```
//!
//! ## Why a container and not a model
//!
//! `vale_assets::Adt` parses a tile into what a renderer needs and drops the
//! rest: the MCIN index, the sub-chunk order, the padding after MCNR, the sound
//! emitters, the reference lists. Reading is all it is for, and dropping what it
//! does not use costs nothing.
//!
//! An editor cannot do that. Writing a file back from a model that lost half of
//! it destroys the half it lost, silently, in a file the client will still load.
//! So [`file::AdtFile`] carries every region as the file wrote it and replaces
//! only the regions an edit names, and [`file::AdtFile::write`] is checked by
//! parsing a real tile and comparing the bytes it produces against the bytes it
//! was given.
//!
//! ## The layout, as measured
//!
//! Over 1,280 map chunks in five tiles from two maps, without exception:
//!
//! * The top-level chunks are `MVER`, `MHDR`, `MCIN`, `MTEX`, `MMDX`, `MMID`,
//!   `MWMO`, `MWID`, `MDDF`, `MODF`, then 256 `MCNK`.
//! * `MHDR`'s eight offsets are relative to the start of its own payload.
//!   `MCIN`'s 256 entries hold an absolute file offset and a size that includes
//!   the eight-byte chunk header.
//! * A map chunk's regions appear in the order `MCVT`, `MCNR`, `MCLY`, `MCRF`,
//!   `MCSH`, `MCAL`, `MCLQ`, `MCSE` — which is not the order the header lists
//!   their offsets in. The offsets are relative to the start of the `MCNK`
//!   chunk header, so the first region begins at 136.
//! * There is no `MCCV`, and the header dword its offset lives in (`0x74`)
//!   is zero on all 256 chunks of `Azeroth_32_48`, as are the two after it. So
//!   [`colours`] writes a region 1.12 never reads, appended after `MCSE` so that
//!   every other region of a tile that has one keeps the offset it had.
//! * `MCNR` is followed by thirteen bytes that no size field covers. A walk that
//!   trusts sub-chunk sizes desynchronises there and finds nothing after it.
//! * `sizeAlpha` and `sizeLiquid` include the eight-byte sub-chunk header;
//!   `sizeShadow` does not.
//! * `MCLQ`'s own size field disagrees with its extent in 431 of the 1,280,
//!   so the header's `sizeLiquid` is the authority for how long it is.

pub mod alpha;
pub mod area;
pub mod blank;
pub mod colours;
pub mod diff;
pub mod file;
pub mod header;
pub mod heights;
pub mod holes;
pub mod impass;
pub mod liquid;
pub mod mesh;
pub mod place;
pub mod relocate;

pub use blank::blank_tile;
pub use file::{AdtFile, MapChunk, Region, SubChunk};
pub use header::McnkHeader;
pub use alpha::Paint;
pub use place::{Doodad, Building};

#[cfg(test)]
mod tests;
