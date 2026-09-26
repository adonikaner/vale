//! **Everything a right-click on somebody — or something — else opens.**
//!
//! ```text
//! gossip.rs    talking to one — and the "npc" token every panel here is
//!              written against, which is why this file is the door
//! merchant.rs  …one click deeper: the shop, buying, and the right-click sale
//! mail.rs      …and the one window in this directory no person owns: the box
//!              on the corner, which the *client* opens with no packet at all
//! trainer.rs   …and the third window: what one will teach, and buying a line
//! taxi.rs      …and the fourth: the flight map, which is all client-side
//! stable.rs    …and the fifth: where a hunter leaves a pet, whose whole panel
//!              is one packet and four verbs that share one answer
//! quest.rs     the log the character carries, and the page in front of a giver
//! loot.rs      …and what is on the body once the fight is over
//! lootroll.rs  …and, in a group, the frame that decides who gets it
//! pagetext.rs  …and what a sign, a plaque and a book on a stand say: the page
//!              chain behind them, and the one window an *object* opens
//! object.rs    …and the half that is not a person at all: the door, the
//!              chest, the ore vein, the mailbox
//! binder.rs    …and the one conversation that is not over when the window
//!              shuts: making an inn your home, and where the stone points
//! untrainer.rs …and the same shape one trainer over: resetting a pet's
//!              skills, whose confirm also arrives after the window has closed
//! bank.rs      …and the sixth window, whose contents were never a packet:
//!              the banker's guid, the slot it sells, the click across
//! ```

pub mod bank;
pub mod binder;
pub mod gossip;
pub mod loot;
pub mod mail;
pub mod lootroll;
pub mod merchant;
pub mod object;
pub mod pagetext;
pub mod quest;
pub mod stable;
pub mod taxi;
pub mod trainer;
pub mod untrainer;
