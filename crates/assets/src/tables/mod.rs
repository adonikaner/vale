//! The DBC tables, one file per question they answer.
//!
//! Each file holds rules the game ships as data: which model, which sound,
//! which colour, which page. None of it needs a window, all of it is
//! unit-tested, and `vale <command>` checks the same code the renderer runs.
//!
//! ```text
//! dbc.rs        the table reader, and the loader every other table goes
//!               through: see `DisplayTables::load`
//! light/        the sun, the fill, the sky, the fog, and the water's colour
//! foliage.rs    what grows on the ground: GroundEffectTexture -> the models
//! sound.rs      what anything sounds like: SoundEntries and its nine
//!               companions
//! spell.rs      Spell -> SpellVisual -> SpellVisualKit: the pose a cast makes
//! spellbook.rs  what a spell is to a button, including what it may aim at
//! spelltext.rs  what a spell's description says: its $ variables
//! spellnames.rs the names of spell effect and aura numbers, which the client
//!               ships no table for
//! spellbits.rs  the names of the bits of a spell's twelve masks, and the
//!               values of its enumerations (Attributes, Targets, ProcFlags,
//!               the implicit targets); no table for any of these either
//! schema.rs     what a table's columns are: name, type, and what a number
//!               points at, as data, for a caller that needs every column
//! emotetext.rs  what /dance says, from three tables, and which voice line
//! talent.rs     the three trees a class may spend its points in
//! skills.rs     which page of the spellbook a spell goes on
//! book.rs       the spellbook itself, in the client's two sort orders
//! trainer.rs    the training window, laid out the same way
//! tradeskill.rs the two profession windows: the recipe thresholds, the
//!               difficulty formula, the sort, and which window a cast opens
//! item/         ItemDisplayInfo: what a garment paints and what it adds
//! itemsound.rs  what an item sounds like changing hands: ItemGroupSounds
//! inventory.rs  the four shipped tables the bags and the paper doll need
//! enchant.rs    what an enchantment on an item is called, and what a random
//!               suffix is called and adds
//! itemset.rs    the pieces of an item set, and which bonuses are active
//! itemvisual.rs the glows and flames on a held item, and whether its own or
//!               an enchantment's is drawn
//! pagetext.rs   what a sign or a book is written on: six names the page
//!               window builds four texture paths out of
//! repair.rs     what it costs to repair a worn item
//! resistances.rs what a damage school is called, which is the last word of
//!               every combat log line that names one
//! pet.rs        what a pet is: its happiness bands, its family's name and
//!               icon, its diet and its loyalty level
//! bank.rs       what a bank bag slot costs, and why six slots is the limit
//!               when the table prices twelve
//! faction.rs    friend or foe, which decides what may be attacked
//! reputation.rs the player's own standing with each faction
//! charcreate.rs what a character may be made of
//! area.rs       where the character is, in the game's own words
//! channels.rs   the six chat channels everybody is in, and which of them where
//! wmoarea.rs    where the character is when that is inside a building
//! areatrigger.rs the 432 volumes a character is reported standing in
//! safeloc.rs    WorldSafeLocs: the places a dead character's spirit appears
//! map.rs        Map: the maps by id, the folder each one's files are in, and
//!               what a new one's folder may be called
//! taxi.rs       the flight map, which is all arithmetic
//! worldmap.rs   which parchment the world map shows, plus <continent>.zmp
//! areapoi.rs    the flags drawn on that parchment, gated on exploration
//! minimap.rs    which minimap picture covers which tile
//! loading.rs    the picture shown during a loading screen
//! lock.rs       what it takes to open a door, a chest or an ore vein; the
//!               one table that tells a mining node from a strongbox
//! transport.rs  the elevators, lifts and the tram, which move with no packet
//!               behind them
//! shiptransport.rs the boats and the zeppelins, whose position is never on
//!               the wire: each route is a TaxiPath and a schedule
//! questmark.rs  the ! and the ? over a quest giver's head
//! questsort.rs  the heading a quest goes under in the log
//! stationery.rs the paper a letter is written on, and which of the five the
//!               send panel offers
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
pub mod itemset;
pub mod itemvisual;
pub mod itemsound;
pub mod light;
pub mod loading;
pub mod lock;
pub mod map;
pub mod minimap;
pub mod questmark;
pub mod questsort;
pub mod pet;
pub mod reputation;
pub mod pagetext;
pub mod repair;
pub mod shiptransport;
pub mod resistances;
pub mod safeloc;
pub mod skills;
pub mod sound;
pub mod schema;
pub mod spell;
pub mod spellbook;
pub mod spellbits;
pub mod spellnames;
pub mod spelltext;
pub mod emotetext;
pub mod enchant;
pub mod stationery;
pub mod talent;
pub mod taxi;
pub mod transport;
pub mod tradeskill;
pub mod trainer;
pub mod wmoarea;
pub mod worldmap;
