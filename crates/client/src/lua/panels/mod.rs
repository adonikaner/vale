//! The C functions each panel of the game's interface calls, one file per
//! panel.
//!
//! These are the parts of [`super::api`] that serve one subject rather than
//! the interpreter: `ContainerAnswers` answers what `ContainerFrame.lua` asks,
//! `TaxiAnswers` answers what `TaxiFrame.lua` asks, and the `Answers` trait one
//! directory up combines all of them. Each read is declared, answered,
//! registered and listed in the file named after its subject.
//!
//! ```text
//! container.rs   the bags: fourteen reads, an item link, and item dragging
//! spellbook.rs   the spellbook, in the client's page order
//! paperdoll.rs   the character sheet's numbers: stats, resistances, armour,
//!                attack and defence
//! auras.rs       the buff bar, and the aura rows of other units
//! quest.rs       the quest log, and a quest giver's pages
//! gossip.rs      talking to an NPC, and the "npc" unit token the other NPC
//!                panels use
//! merchant.rs    the merchant window
//! mail.rs        the mailbox, which belongs to no NPC: the inbox, one letter,
//!                and the letter being written
//! trainer.rs     the training window, a load-on-demand addon
//! tradeskill.rs  the profession window, another load-on-demand addon, mostly
//!                a rule over SkillLineAbility.dbc
//! craft.rs       the Enchanting window, the craft counterpart of tradeskill.rs
//! talent.rs      the talent trees, another load-on-demand addon, mostly a rule
//!                over two DBCs
//! keybindings.rs the key bindings panel, a fourth load-on-demand addon: nine
//!                C functions over the key table the keyboard also reads
//! taxi.rs        the flight map, whose background is painted from a table
//! pagetext.rs    text read from an object: a sign, a plaque, or a book on a
//!                stand; the one window no NPC owns
//! duel.rs        the four script functions a duel uses, all of them writes
//! summon.rs      the summon popup's three reads
//! inspect.rs     the inspect window's reads: who may be inspected, and the
//!                honor tab
//! loot.rs        the loot window of a corpse
//! lootroll.rs    the four group-loot roll frames
//! reputation.rs  the reputation panel, which has two sort orders
//! skills.rs      the skills panel, which has the same shape as reputation.rs
//! party.rs       the party
//! social.rs      the friends list, the ignore list and the /who search
//! guild.rs       the guild tab: the roster, the ranks and their rights, and
//!                the guild names `GetGuildInfo` answers from
//! petition.rs    the guild charter: the registrar window's four functions
//!                and the petition window's eight
//! tabard.rs      the tabard designer: two functions and the ten methods of
//!                `TabardModel`
//! channels.rs    chat channels: ten numbered slots and the chat frame's
//!                twenty-five channel globals
//! raid.rs        the raid roster: the party roster with its first byte set,
//!                a separate panel, and eight reads the party does not use
//! pet.rs         the three predicates the pet frame and its menu depend on
//! stable.rs      the stable master: seven reads over one packet, four commands
//!                and a DBC of prices
//! bank.rs        the bank: three reads and two commands; the thirty bank
//!                slots are read the same way as the paper doll's
//! trade.rs       the trade window between two players: six reads over two
//!                packets, ten commands
//! shapeshift.rs  the stance bar beside the pet bar: it shares the pet bar's
//!                unbound commands but none of its packets; the stance
//!                buttons come from the client's reading of Spell.dbc, with
//!                nothing on the wire
//! worldmap.rs    the world map
//! charcreate.rs  the character creation screen
//! glue.rs        the login and character select screens
//! addons.rs      the list behind character select's AddOns button: every
//!                addon in the folder, which ones each character has enabled,
//!                and the loader `LoadAddOn` runs
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
pub mod guild;
pub mod inspect;
pub mod lootroll;
pub mod merchant;
pub mod paperdoll;
pub mod party;
pub mod pet;
pub mod petition;
pub mod quest;
pub mod raid;
pub mod reputation;
pub mod shapeshift;
pub mod skills;
pub mod stable;
pub mod tabard;
pub mod trade;
pub mod social;
pub mod spellbook;
pub mod talent;
pub mod taxi;
pub mod tradeskill;
pub mod trainer;
pub mod worldmap;
