//! A shared editing session: several editors working on one project at once,
//! one of them the host.
//!
//! ```text
//! wire.rs     the messages: what an editor sends the host and what the host
//!             sends an editor, and their framing on a stream
//! groups.rs   what two edits conflict over: a chunk's shape, a tile's
//!             textures, a tile's placements, a DBC table, a server row, a
//!             project file
//! host.rs     the host's half: who is in, the order of every accepted edit,
//!             and the check that refuses an edit made on out-of-date state
//! peer.rs     every editor's half, the host's own included: its edits not
//!             yet accepted, and which of them a remote edit makes the host
//!             refuse, so they are taken back before the remote edit lands
//! ids.rs      blocks of new numbers handed out by the host, so two editors
//!             never give two new spawns the same guid
//! auth.rs     the password check: a challenge answered with a keyed hash,
//!             so the password is not sent
//! net.rs      the TCP connections: the host's listener and an editor's
//!             connection, each a pair of threads and two channels
//! session.rs  the two ways to be in a session, hosting or joined, each one
//!             host or connection driven together with this editor's peer
//! ```
//!
//! ## The model
//!
//! The host decides the order of every edit. An editor applies its own edit at
//! once, so a tool responds as it does alone, and sends it to the host with
//! `base`, the number of the last edit it had received. The host accepts the
//! edit unless another editor changed one of its [`groups::Group`]s after
//! `base`. An accepted edit is numbered and sent to every other editor, which
//! applies it through the same path a redo takes.
//!
//! An edit is any [`groups::Payload`]. The editor sends two kinds: a
//! [`vale_edit::undo::Change`], the state of everything it touched before and
//! after, and the bytes of project files written or removed whole. Applying
//! either needs no recomputation, so every editor ends with the same bytes.
//!
//! ## A refusal
//!
//! A refused edit is taken back on the editor that made it, and so is every
//! edit it made after it that the host has not accepted yet: the host refuses
//! those too, by the epoch they carry. An editor does not wait for the refusal.
//! A remote edit that conflicts with one of its waiting edits is proof that
//! the host will refuse that edit, because the host sends messages in the
//! order it decided them. So the editor takes its waiting edits back from that
//! one on, newest first, and then applies the remote edit. See
//! [`peer::Peer::receive`].
//!
//! An edit the host's own editor marks as forced is never refused or taken
//! back, because it cannot be: a delete drops unsaved edits. See
//! [`peer`]'s *Forced edits*.
//!
//! The connections are plain TCP and carry no encryption: a session is for a
//! local network. The password keeps strangers out; it does not hide the
//! edits.

pub mod auth;
pub mod groups;
pub mod host;
pub mod ids;
pub mod net;
pub mod peer;
pub mod session;
pub mod wire;

pub use groups::{Group, Part, Payload};
pub use host::{Host, Out};
pub use peer::{Action, Cause, Peer};
pub use session::{Hosting, Joined, Session};
pub use wire::{Presence, Seq, ToHost, ToPeer, UserId, PROTOCOL};
