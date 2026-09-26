//! The WoW 1.12.1 (build 5875) network protocol, as spoken by vmangos.
//!
//! Layering, outermost first:
//!
//! ```text
//! opcodes.rs   827 opcodes, both directions, as vmangos numbers them
//! codes.rs     LogonResult / WorldResult: two overlapping single-byte tables,
//!              and what each refusal *says*, a GlueStrings key apiece
//! version.rs   the client build identity and its integrity hashes
//! bytes.rs     …and the little-endian reader and writer everything uses
//!
//! socket/      the two sockets and what runs on them: the logon, the cipher,
//!              the framing, the one dispatch, and the live session
//! state/       what the world *is*: the object manager, the update blocks, the
//!              field indices, the queries, and where anything is
//! play/        …and what is *played* on it, one file per subject — nineteen of
//!              them, from a swing to a flight path
//! ```
//!
//! A packet goes **socket -> frame -> parse -> apply**, and the four stages are
//! deliberately separate: `socket::world` turns bytes into a
//! [`socket::world::Packet`], [`socket::handler::apply_packet`] turns an opcode
//! into an action, the `parse_*` functions in `state::movement`, `state::query`
//! and `state::update` are pure `&[u8] -> Option<T>` with no state and no
//! socket, and [`state::objects::ObjectManager`] holds what they mean.
//! That is why the parsers can be unit-tested by the hundred without a server.
//!
//! **There is exactly one dispatch, and one shape for an outbound body.** The
//! dispatch used to be two — `apply_packet` had no way to answer the server, so
//! every packet needing a reply was intercepted by the session loop ahead of it
//! — and the seam between them silently dropped every speed change about another
//! unit. Bodies used to be built two ways, inline in a `WorldSession` method or
//! by a `*_body()` function, with both used inside one loop. See [`socket::handler`].
//!
//! Every constant here is transcribed from vmangos source rather than guessed.

pub mod bytes;
pub mod codes;
pub mod opcodes;
pub mod version;

pub mod play;
pub mod socket;
pub mod state;

pub use codes::{LogonResult, WorldResult};
pub use opcodes::Opcode;
pub use version::ClientVersion;

/// The client build we identify as. The server matches this against its
/// `allowed_clients` table, so it is not a cosmetic value.
pub const CLIENT_BUILD: u16 = version::WOW_1_12_1_WIN_X86.build;
