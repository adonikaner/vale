//! **The DBC chain, one file per question it answers.**
//!
//! Everything here is a *rule* the game ships as data: which model, which
//! sound, which colour, which page. None of it needs a window, all of it is
//! unit-tested, and `vale <command>` checks the same copy the renderer runs.
//!
//! ```text
//! dbc.rs        the tables themselves, and the loader every other one goes
//!               through — see `DisplayTables::load`
//! light/        the sun, the fill, the sky, the fog, and the water's colour
//! foliage.rs    …and what grows on it: GroundEffectTexture -> the models
//! sound.rs      …and what any of it sounds like: SoundEntries and its nine
//! spell.rs      Spell -> SpellVisual -> SpellVisualKit: the pose a cast makes
//! spellbook.rs  …and what a spell is to a *button*, including what it may aim at
//! spelltext.rs  …and what it *says*: Description's own $ variables
//! spellnames.rs …and what its effect and aura numbers are called, which the
//!               client ships no table for at all
//! spellbits.rs  …and what the bits of its twelve masks are called, and the
//!               values of its half-dozen enumerations — Attributes,
//!               Targets, ProcFlags, the implicit targets. No table for any
//!               of these either
//! schema.rs     what a table's columns *are*: name, type, and what a number
//!               points at — the layout as data, for a caller that needs all
//!               173 of them rather than the six it reads
//! emotetext.rs  what /dance says, from three tables, and which voice line
//! talent.rs     …and the three trees a class may spend its points in
//! skills.rs     …which page of the book it goes on
//! book.rs       …and the book itself, in the client's own two sort orders
//! trainer.rs    …and the training window, laid out the same way
//! tradeskill.rs …and the two profession windows: the recipe thresholds, the
//!               difficulty formula, the sort, and which window a cast opens
//! item/         ItemDisplayInfo: what a garment paints and what it adds
//! itemsound.rs  …and what it sounds like changing hands: ItemGroupSounds
//! inventory.rs  …and the four shipped tables the bags and paper doll need
//! pagetext.rs    …and what a sign or a book is written *on*: six names the
//!                page window builds four texture paths out of
//! repair.rs     …and what it costs to make a worn one whole
//! resistances.rs what a damage school is *called*, which is the last word of
//!               every combat log line that names one
//! pet.rs        what a pet *is*: its happiness bands, its family's name and
//!               icon, its diet and its loyalty level — four tables the wire
//!               never mentions and the pet paper doll is made of
//! bank.rs       what a bank bag slot costs, and why six is full when the
//!               table prices twelve
//! faction.rs    friend or foe, which is the whole of what may be attacked
//! reputation.rs …and the *player's* own standing, which is the other half
//! charcreate.rs what a character may be *made* of
//! area.rs       where you are, in the game's own words
//! channels.rs   the six chat channels everybody is in, and which of them where
//! wmoarea.rs    …and where you are when that is inside a building
//! areatrigger.rs …and the 432 volumes you are *reported* standing in
//! taxi.rs       …and how you leave: the flight map, which is all arithmetic
//! worldmap.rs   …and which parchment shows it, plus <continent>.zmp
//! areapoi.rs    …and the flags drawn on that parchment, gated on exploration
//! minimap.rs    …and the little round one: which picture covers which tile
//! loading.rs    …and the picture shown while you are getting there
//! lock.rs       what it takes to open a door, a chest or an ore vein — the
//!               one table that tells a mining node from a strongbox
//! transport.rs  …and what a *moving* one does: the elevator, the lift and the
//!               tram, which are the one population in the world that moves
//!               with no packet behind it
//! shiptransport.rs …and the *other* half of that population, which does not
//!               even have a position on the wire: the boats and the zeppelins,
//!               whose whole route is a TaxiPath and a schedule
//! questmark.rs  the ! and the ? over a giver's head
//! questsort.rs  …and what heading the quest goes under in the log
//! stationery.rs …and the paper a letter is written on, plus which of the
//!               five the send panel offers, which is a rule and not a list
//! ```

pub mod area;
pub mod areapoi;
pub mod areatrigger;
pub mod bank;
pub mod book;
pub mod channels;
pub mod charcreate;
pub mod dbc;
pub mod faction;
pub mod foliage;
pub mod inventory;
pub mod item;
pub mod itemsound;
pub mod light;
pub mod loading;
pub mod lock;
pub mod minimap;
pub mod questmark;
pub mod questsort;
pub mod pet;
pub mod reputation;
pub mod pagetext;
pub mod repair;
pub mod shiptransport;
pub mod resistances;
pub mod skills;
pub mod sound;
pub mod schema;
pub mod spell;
pub mod spellbook;
pub mod spellbits;
pub mod spellnames;
pub mod spelltext;
pub mod emotetext;
pub mod stationery;
pub mod talent;
pub mod taxi;
pub mod transport;
pub mod tradeskill;
pub mod trainer;
pub mod wmoarea;
pub mod worldmap;
