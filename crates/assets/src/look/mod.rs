//! **How a unit *looks*, which is the one subject here that is not a file
//! format and not a table lookup.**
//!
//! These read the tables one directory over, but their subject is the join:
//! what to draw an entity as, what colour the ring under it is, which pointer
//! goes over it, and where a camera stands to see it. The plan that made this
//! directory flagged the placement as arguable and it is left arguable.
//!
//! ```text
//! character.rs a player's skin, which the game ships no file for: composed at
//!              runtime from CharSections into one 256x256 body texture
//! dress.rs     …and what to draw an entity *as*: geosets, attachments, and
//!              whether its skin is a file or has to be built
//! sheath.rs    …and whether the weapons are out
//! conform.rs   …and which models lean with the ground
//! scenery.rs   …and what a thing that is not a unit is *doing*: which clip a
//!              doodad plays, and which of its variations this one took
//! selection.rs what colour the ring under it is
//! unitname.rs  …and the name over its head, which is the one part of 1.12's
//!              interface that ships no XML at all
//! worldtext.rs …and the numbers that float off it when you hit it, which is
//!              the other part
//! cursor.rs    …and which pointer the world puts up over it
//! object.rs    …and the same question for a thing rather than a person: what
//!              a door, a chest or an ore vein is *for*
//! pick.rs      …and what that pointer is *on*: the volume a click must hit
//! blips.rs     …and the dot it gets on the minimap, if any
//! anchor.rs    where a third-person camera looks at it
//! portrait.rs  …and where to stand to take its picture
//! ```

pub mod anchor;
pub mod blips;
pub mod character;
pub mod conform;
pub mod cursor;
pub mod dress;
pub mod object;
pub mod pick;
pub mod portrait;
pub mod scenery;
pub mod selection;
pub mod sheath;
pub mod unitname;
pub mod worldtext;
