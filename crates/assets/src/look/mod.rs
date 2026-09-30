//! How a unit looks. This is the one subject in this crate that is neither a
//! file format nor a table lookup.
//!
//! These modules read the tables in `tables/`, but their subject is the join:
//! what to draw an entity as, what colour the ring under it is, which pointer
//! goes over it, and where a camera stands to see it. Placing this directory in
//! this crate is arguable; it has been left here.
//!
//! ```text
//! character.rs    a player's skin, which the game ships no file for: composed
//!                 at runtime from CharSections into one 256x256 body texture
//! dress.rs        what to draw an entity as: geosets, attachments, and whether
//!                 its skin is a file or has to be built
//! sheath.rs       whether the weapons are out
//! weapon_trail.rs the strip a weapon leaves behind it during a melee ability
//! conform.rs      which models lean with the ground
//! scenery.rs      what a thing that is not a unit is doing: which clip a
//!                 doodad plays, and which of its variations this one took
//! selection.rs    what colour the ring under a unit is
//! unitname.rs     the name over a unit's head, the one part of 1.12's
//!                 interface that ships no XML
//! worldtext.rs    the numbers that float off a unit when it is hit, the other
//!                 part of the interface that ships no XML
//! cursor.rs       which pointer the world puts up over a unit
//! object.rs       which pointer goes over a game object, and what a door, a
//!                 chest or an ore vein is for
//! pick.rs         what the pointer is on: the volume a click must hit
//! blips.rs        the dot a unit gets on the minimap, if any
//! anchor.rs       where a third-person camera looks at a unit
//! portrait.rs     where to stand to take a unit's picture
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
pub mod weapon_trail;
pub mod worldtext;
