//! **The two sockets and what runs on them.**
//!
//! ```text
//! auth.rs     realmd :3724 — the SRP6 logon, and the realm list it answers with
//! srp.rs      …and the maths itself
//! crypt.rs    …and the header cipher the *other* socket turns on afterwards
//! world.rs    mangosd :8085 — framing, the cipher, sending, and one Packet
//! handler/    …and EVERY packet this client understands, in one match: the
//!             arms delegate and the bodies live beside it
//! session.rs  …and the live session: a thread that keeps one running
//! ```
//!
//! **There is exactly one dispatch.** It used to be two — `apply_packet` had no
//! way to answer the server, so every packet needing a reply was intercepted by
//! the session loop ahead of it — and the seam between them silently dropped
//! every speed change about another unit. See [`handler`].

pub mod auth;
pub mod crypt;
pub mod handler;
pub mod session;
pub mod srp;
pub mod world;
