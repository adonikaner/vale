//! **What the character *is*, as against what it is doing.**
//!
//! ```text
//! vitals.rs    the unit frames' news: health, power, name, level, on a change
//! stats.rs     …and the character sheet's: the nine UNIT_* stat events
//! items.rs     what it is carrying, as the interface asks about it
//! templates.rs …and the round trip behind that: a template landed, redraw
//! received.rs  …and the one packet that says something *arrived* in a bag,
//!              which no field does: SMSG_ITEM_PUSH_RESULT and its four lines
//! timers.rs    the breath meter and the two bars beside it — the server's clock
//! death.rs     …and the end of it: three states over two fields
//! reputation.rs …and what it has *earned*: four packets, three verbs, one event
//! skills.rs    …and what it has *learned*, which is no packet at all
//! talents.rs   …and what it *chose*, which is no packet either — and one out
//! tradeskill.rs …and what it can *make*: the two profession windows, whose
//!              whole content is the book crossed with the bags
//! proficiency.rs …and what it may *hold*, which is one packet per item class
//!              and is in no file at all
//! ```
//!
//! Everything here is a *field* the server keeps about this one character,
//! turned into the event the interface waits on. Nothing in it is a verb.

pub mod death;
pub mod items;
pub mod proficiency;
pub mod received;
pub mod reputation;
pub mod skills;
pub mod stats;
pub mod talents;
pub mod templates;
pub mod tradeskill;
pub mod timers;
pub mod vitals;
