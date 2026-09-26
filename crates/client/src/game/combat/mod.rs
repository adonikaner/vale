//! **Pressing a button, and what happens when it lands.**
//!
//! ```text
//! target.rs    who the player is pointing at — the pick, Tab, Esc, the acquire
//! action.rs    …and the press: the twelve-slot bar, the cast, the cooldowns
//! log.rs       …and what the *log* says about all of it: ten packets, one
//!              sentence each, in the game's own words and windows
//! spellbook.rs …and where a button's spell came from: the book, in its pages
//! pet.rs       …and the *other* bar: the ten slots a pet is commanded by, which
//!              the server states outright and a class is unplayable without
//! shapeshift.rs …and the *third*: the stance buttons, which no packet mentions
//!              at all — the list, its order and which one is pressed in are
//!              read out of Spell.dbc
//! cursor.rs    …and what is on the pointer, which is what fills a slot
//! supersede.rs …and the dead ranks left on it before the packet was read
//! auras.rs     what is *on* a unit afterwards: the buff bar, and everyone's rows
//! desync.rs    …and an instrument: what this client measured when the server
//!              said the target was too far away
//! spellmods.rs …and what a *talent* does to a spell's numbers, which is two
//!              packets and is in no file
//! ```
//!
//! The one ordering that matters between them is stated in [`action`]'s own
//! plugin: the targeting chain runs first, because a cast binds against the
//! selection and a swing goes at it.

pub mod action;
pub mod auras;
pub mod cursor;
pub mod desync;
pub mod log;
pub mod pet;
pub mod shapeshift;
pub mod spellbook;
pub mod spellmods;
pub mod supersede;
pub mod target;
