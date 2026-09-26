//! **The session itself: getting in, being in it with other people, getting
//! out.**
//!
//! ```text
//! glue.rs       the two screens before there is a world: login, character select
//! autologin.rs  …and the way past both of them, for a caller that already
//!               knows who it wants to be
//! charcreate.rs …and the third, which is the only one with a decision in it
//! chat.rs       what was said, under the CHAT_MSG_* name its kind arrives as
//! emotetext.rs  …and what /dance says: the sentence composed here, and the voice line
//! party.rs      …and who is with you: the roster, and the popup's six verbs
//! raid.rs       …and what the same roster is once its first byte is 1: the
//!               order, the rank and the column, none of which is on the wire
//! social.rs     …and who you know without being with them: the friends list,
//!               the ignore list and the /who search
//! channels.rs   …and the rooms everybody talks in: the zone channels joined
//!               on a zone change, and every notice as a chat line
//! keybindings.rs …and the two files a session's keys are kept in, which want
//!               an account, a realm and a character name to be found at all
//! trade.rs      …and handing something to one of them: the window, both
//!               offers as the server states them, and the two accept flags
//! cameracache.rs …and the one file the session keeps about the camera, per
//!               character: how far back and how steep
//! duel.rs       …and fighting one of them by agreement: the popup, the
//!               countdown lines, the boundary warning and the result line
//! summon.rs     …and being fetched by one: the offer held, the popup's three
//!               reads and its one answer
//! played.rs     …and /played, which is one question and an event
//! logout.rs     …and the way out: the escape menu's four verbs
//! addons.rs     …and the two ends of the addon board: Interface\AddOns\ read
//!               into it once per interpreter, the character's AddOns.txt
//!               read into it at login and written back at logout
//! ```

pub mod addons;
pub mod autologin;
pub mod cameracache;
pub mod channels;
pub mod charcreate;
pub mod chat;
pub mod duel;
pub mod emotetext;
pub mod glue;
pub mod keybindings;
pub mod logout;
pub mod party;
pub mod played;
pub mod raid;
pub mod social;
pub mod summon;
pub mod trade;
