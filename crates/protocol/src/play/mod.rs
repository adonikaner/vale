//! Packets for play in the world, one file per subject.
//!
//! Every module here has the same shape: one or two packets in, a parse that
//! holds no state and no socket, and a value the game layer reads. The one
//! dispatch that calls them is [`super::socket::handler`].
//!
//! ```text
//! action.rs      what a unit is visibly doing: the melee swing, the emote, the
//!                spell cast
//! sound.rs       the five packets the server sends with no other content: a
//!                scripted sound, a music track, a sound at an object, and the
//!                two that play a SpellVisualKit on a unit
//! combatlog.rs   the nine packets that only feed the combat log: experience,
//!                the killing blow, environmental damage such as drowning, a
//!                damage shield's reply, every kind of spell miss, and a
//!                periodic tick
//! spells.rs      the player's own spells: the spellbook, the action bar, the
//!                cooldowns
//! stats.rs       the character sheet's twenty fields
//! skills.rs      the 128 skill slots, empty slots included
//! talents.rs     spending a talent point; no packet states which talents
//!                were chosen
//! items.rs       what the character carries, read through three layers of
//!                update fields
//! wdb.rs         the client's WDB\ caches, which keep query answers across a
//!                logout, one file per kind of query
//! loot.rs        the loot window of a corpse
//! lootroll.rs    the group-loot roll that decides who gets an item
//! object.rs      using a game object (a door, a chest, an ore vein), and the
//!                two packets that play a one-shot animation on one
//! pagetext.rs    the page chain read from a sign, a plaque, a tombstone or a
//!                book on a stand
//! quest.rs       the quest log, and a quest giver's pages
//! gossip.rs      talking to an NPC
//! trainer.rs     the trainer's list of services, and buying one
//! mail.rs        the mailbox: the inbox, one letter, and sending a letter
//! taxi.rs        flight paths: four packets and a map
//! reputation.rs  reputation: four packets in and three commands out
//! pet.rs         the pet's action bar, happiness, name and ten commands; the
//!                hunter and warlock classes need this packet family
//! stable.rs      the stable master: five packets and two slots; the slot byte
//!                is one-based on the wire and zero-based in the panel
//! bank.rs        the bank: the window is a banker guid, a slot is a byte, and
//!                two packets share one body
//! trade.rs       trading with another player: twelve opcodes, seven slots a
//!                side, and the server states both offers
//! group.rs       the party
//! social.rs      the friends list, the ignore list, and the /who search
//! guild.rs       the guild: its name and ranks, the roster, the events, the
//!                nineteen requests of the guild tab, and the emblem
//! petition.rs    the guild charter: the registrar's offer, the signatures,
//!                and the nine requests about a charter item
//! channels.rs    chat channels: General, Trade, and /join
//! chat.rs        SMSG_MESSAGECHAT and CMSG_MESSAGECHAT, and the GM commands
//!                sent as chat
//! emotetext.rs   text emotes such as /dance, both ways; the client composes
//!                the emote's words
//! death.rs       dying and resurrecting; no packet announces a death, only
//!                health reaching zero
//! duel.rs        a duel: the flag, the countdown, the boundary and the result
//! summon.rs      being summoned by another player: an offer and an accept
//! inspect.rs     inspecting another player: the request, and the honor tab's
//!                separate request and answer
//! played.rs      the /played time
//! logout.rs      logging out, timed by the server
//! timers.rs      the three mirror timer bars the server counts down
//! time.rs        the world clock, sent once at login
//! weather.rs     the weather: one packet with a type, a grade and a ramp
//! explored.rs    which sub-regions the character has explored
//! areatrigger.rs CMSG_AREATRIGGER, the one packet the client sends unprompted,
//!                and the server's refusal, SMSG_TRANSFER_ABORTED
//! bindpoint.rs   the hearthstone's bind point, and the innkeeper who changes
//!                it; the client must answer a confirmation before the bind
//!                happens
//! charcreate.rs  creating and deleting a character
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
pub mod inspect;
pub mod gossip;
pub mod group;
pub mod guild;
pub mod items;
pub mod logout;
pub mod mail;
pub mod loot;
pub mod lootroll;
pub mod object;
pub mod pagetext;
pub mod petition;
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
