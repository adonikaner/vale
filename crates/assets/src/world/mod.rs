//! **The world's own files** — the ones that describe a place rather than a
//! rule.
//!
//! ```text
//! chunk.rs     the IFF-style chunk format every file below is built from
//! wdt.rs       which of a map's 64x64 tiles exist at all
//! adt/         …and one of them: heightmap, textures, doodads, buildings
//! m2/          the models — trees, chairs, creatures, spell effects
//! wmo/         …and the buildings: a root file plus one file per group
//! blp.rs       the textures all three of them name
//! collision.rs the *solid* half of a WMO and of an M2, which is a different
//!              set of triangles from the drawn half
//! terrain.rs   …and the join a walking character asks twenty times a second:
//!              the height at a position, and how deep the water over it is
//! foliage.rs   …and what grows on it: which of a chunk's 64 detail cells
//!              plants something, and where each tuft of it stands
//! glow.rs      …and which of a model's batches is a *light*: in 1.12 a
//!              lamppost is an additive unlit quad and nothing else
//! tga.rs       …and the texture format an addon's art is in, which the
//!              archives never use
//! ```

pub mod adt;
pub mod blp;
pub mod tga;
pub mod chunk;
pub mod collision;
pub mod foliage;
pub mod glow;
pub mod m2;
pub mod terrain;
pub mod wdt;
pub mod wmo;
