//! Writing M2 models: a transform baked into a copy of a shipped model, and a
//! whole model written from a description of it.
//!
//! ```text
//! bake.rs   a rigid transform baked into a copy of a model, which is how 1.12
//!           positions a spell effect: the tables carry no offset, so the
//!           offset lives in the file
//! write.rs  a whole model written from a description: vertices, sections,
//!           textures, bones with their tracks, and sequences
//! gltf.rs   a glTF 2.0 file read into that description: its meshes, its
//!           skeleton and its animations in the model's axes
//! ```
//!
//! Every offset and record size both writers use is
//! `vale_assets::world::m2::offsets` and `::layout`, which is the parser's
//! reading of the format, and [`write`]'s output is checked by parsing it
//! with that parser.

mod bake;
pub mod gltf;
pub mod write;

pub use bake::{bake, Bake};
