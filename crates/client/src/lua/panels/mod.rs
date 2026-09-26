//! **The C functions each panel of the game's interface calls**, one file per
//! panel.
//!
//! This is the half of [`super::api`] that is about a *subject* rather than
//! about the interpreter: `ContainerAnswers` is what `ContainerFrame.lua` asks,
//! `TaxiAnswers` is what `TaxiFrame.lua` asks, and the `Answers` trait one
//! directory up is the sum of them. A read is declared, answered, registered
//! and listed in the file its subject is named after.
//!
//! ```text
//! container.rs   the bags: fourteen reads, an item link, and the whole drag
//! spellbook.rs   the book, in the client's own page order
//! paperdoll.rs   …and what is worn, which is the same question one panel over
//! auras.rs       the buff bar, and everybody else's rows
//! quest.rs       the log, and the page in front of a giver
//! gossip.rs      talking to one — and the "npc" token the rest are written on
//! merchant.rs    …the shop
//! mail.rs        …and the box on the corner, which no NPC owns at all: the
//!                inbox, one letter, and the one being written
//! trainer.rs     …the training window, which is a load-on-demand addon
//! tradeskill.rs  …and the profession window, which is another — and is
//!                almost entirely a rule over SkillLineAbility.dbc
//! craft.rs       …and its sibling for Enchanting, one door along
//! talent.rs      …and the talent trees, which are another — and are almost
//!                entirely a rule over two DBCs
//! keybindings.rs …and the key bindings panel, which is a fourth — nine C
//!                functions over the one key table the keyboard also reads
//! taxi.rs        …and the flight map, whose parchment is painted from a table
//! pagetext.rs    …and what is written on a thing: the sign, the plaque and
//!                the book on a stand, which is the one window no person owns
//! duel.rs        …and the four script functions a duel is made of, all writes
//! summon.rs      …and the summon popup's three reads
//! loot.rs        what is on the body
//! lootroll.rs    …and, in a group, the four frames that decide who gets it
//! reputation.rs  …and what you have earned: the panel with two sorts in it
//! skills.rs      …and what you have learned, which is the same shape again
//! party.rs       who is with you
//! social.rs      …and who you know without being with them: the friends
//!                list, the ignore list and the /who search
//! channels.rs    …and the rooms everybody talks in: ten numbered slots and
//!                the chat frame's twenty-five channel globals
//! raid.rs        …and the same roster with its first byte set, which is a
//!                different panel and eight reads the party has no use for
//! pet.rs         …and what is beside them: the three gates on a pet frame
//! stable.rs      …and where one is left: the stable master's seven reads over
//!                one packet, four verbs and a DBC of prices
//! bank.rs        …and where anything is left: three reads and two verbs, since
//!                the thirty squares read as the paper doll does
//! trade.rs       the window two players open at each other: six reads over
//!                two packets, ten verbs
//! shapeshift.rs  …and the bar beside the pet's, which shares its unbound
//!                commands and none of its packets: the stance buttons, which
//!                are a client read of Spell.dbc and nothing on the wire
//! worldmap.rs    where you are
//! charcreate.rs  …and the screen before any of it
//! glue.rs        …and the two screens before *that*: login and character select
//! addons.rs      …and the list behind that screen's AddOns button: every
//!                addon the folder carries, which each character has on, and
//!                the loader `LoadAddOn` runs
//! ```
pub mod addons;
pub mod auras;
pub mod bank;
pub mod channels;
pub mod charcreate;
pub mod container;
pub mod craft;
pub mod glue;
pub mod gossip;
pub mod keybindings;
pub mod loot;
pub mod mail;
pub mod pagetext;
pub mod duel;
pub mod summon;
pub mod lootroll;
pub mod merchant;
pub mod paperdoll;
pub mod party;
pub mod pet;
pub mod quest;
pub mod raid;
pub mod reputation;
pub mod shapeshift;
pub mod skills;
pub mod stable;
pub mod trade;
pub mod social;
pub mod spellbook;
pub mod talent;
pub mod taxi;
pub mod tradeskill;
pub mod trainer;
pub mod worldmap;
