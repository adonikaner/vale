//! **What is *played* on that world**, one file per subject.
//!
//! Everything here is the same shape: a packet or two in, a parse with no state
//! and no socket, and something the game layer can read. The dispatch that
//! reaches them is [`super::socket::handler`] and there is only one.
//!
//! ```text
//! action.rs      what a unit is visibly *doing*: the swing, the emote, the cast
//! sound.rs       …and the five the server plays *at* you with nothing else
//!                behind them: a scripted noise, a music track, a noise at an
//!                object, and the two that put a SpellVisualKit on a unit
//! combatlog.rs   …and the nine packets a fight is only *narrated* by: the
//!                experience, the killing blow, the drowning, the shield that
//!                answered, every way a spell missed, and a tick
//! spells.rs      …and what *we* can do: the book, the bar, the cooldowns
//! stats.rs       …and what a character *is*: the sheet's twenty fields
//! skills.rs      …and what it has *learned*: 128 slots, holes and all
//! talents.rs     …and what it *chose*, which no packet states at all
//! items.rs       …and what it is carrying: three objects deep through fields
//! wdb.rs         …and the answers that outlive a logout: this client's own
//!                WDB\ caches, one file per kind of query
//! loot.rs        what is on a body
//! lootroll.rs    …and, in a group, the roll that decides who gets it
//! object.rs      …and the one packet that opens the door, the chest and the vein
//! pagetext.rs    …and the one that opens the *sign*: the page chain behind a
//!                plaque, a tombstone and a book on a stand
//! quest.rs       the log, and the page in front of a giver
//! gossip.rs      talking to one
//! trainer.rs     …and what one will teach you
//! mail.rs        …and the box on the corner: the inbox, a letter, and sending one
//! taxi.rs        …and how you leave: four packets and a map
//! reputation.rs  …and what it has earned: four packets and three verbs
//! pet.rs         …and what is with you: the bar, the mood, the name, and the
//!                ten orders — the packet family two of the nine classes are
//!                unplayable without
//! stable.rs      …and where a hunter leaves one: five packets, two slots, and
//!                a slot byte that is one-based on the wire and zero-based in
//!                the panel
//! bank.rs        …and where anyone leaves anything: a window that is a guid,
//!                a slot that is a byte, and two packets that share a body
//! trade.rs       …and handing something to another player: twelve opcodes,
//!                seven slots a side, and both offers stated by the server
//! group.rs       who is with you
//! social.rs      …and who you know: the two lists, and the /who search
//! channels.rs    …and the rooms everybody talks in: General, Trade, /join
//! chat.rs        SMSG_MESSAGECHAT both ways, and the GM commands it carries
//! emotetext.rs   …and /dance both ways: the text emote, whose words the client composes
//! death.rs       dying, and getting up again — none of it announced
//! duel.rs        …and fighting another player by agreement: the flag, the
//!                countdown, the boundary and the result
//! summon.rs      …and being fetched across the world by one: an offer and a yes
//! played.rs      …and how long the character has been played, which is /played
//! logout.rs      …and leaving on purpose, on the server's own clock
//! timers.rs      the three bars it counts down for you
//! time.rs        …and the world's own clock, said once
//! weather.rs     …and what its sky is doing: one packet, a grade, a ramp
//! explored.rs    which sub-regions the character has walked into
//! areatrigger.rs the one packet this client volunteers, and its refusal
//! bindpoint.rs   where the hearthstone returns you, and the innkeeper who
//!                changes it — a confirmation the client has to answer before
//!                anything is bound at all
//! charcreate.rs  making a character, and unmaking one
//! ```

pub mod action;
pub mod areatrigger;
pub mod bank;
pub mod bindpoint;
pub mod channels;
pub mod charcreate;
pub mod chat;
pub mod emotetext;
pub mod combatlog;
pub mod death;
pub mod duel;
pub mod explored;
pub mod gossip;
pub mod group;
pub mod items;
pub mod logout;
pub mod mail;
pub mod loot;
pub mod lootroll;
pub mod object;
pub mod pagetext;
pub mod pet;
pub mod played;
pub mod quest;
pub mod reputation;
pub mod skills;
pub mod social;
pub mod stable;
pub mod trade;
pub mod wdb;
pub mod sound;
pub mod spells;
pub mod stats;
pub mod summon;
pub mod talents;
pub mod taxi;
pub mod time;
pub mod timers;
pub mod weather;
pub mod trainer;
