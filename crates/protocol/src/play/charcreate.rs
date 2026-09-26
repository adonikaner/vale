//! **Making a character, and unmaking one** — the character screen's two
//! writes, each one packet out and one byte back.
//!
//! ```text
//! CMSG_CHAR_CREATE   cstring name, u8 race, class, gender,
//!                    skin, face, hairStyle, hairColour, facialHair, outfitId
//! SMSG_CHAR_CREATE   u8 code — a ResponseCodes value; 46 is success
//!
//! CMSG_CHAR_DELETE   u64 guid — plain, not packed
//! SMSG_CHAR_DELETE   u8 code — the same array eleven entries on; 57 is success
//! ```
//!
//! The two live in one file because they are one shape: the same socket, the
//! same 84-entry key array, and the same reason for not being a handler (below).
//! Where they differ is what silence means, and [`delete_character`] is the one
//! that has to have an opinion about it.
//!
//! Ten bytes and a name, and **every one of them is a decision the client made
//! with no help from the server**: which races there are, which classes each may
//! be, how many faces a dwarf has. That half is
//! [`vale_assets::tables::charcreate`](../../vale_assets/charcreate/index.html);
//! this is only the sentence that carries it.
//!
//! ## The exchange belongs here rather than in a handler
//!
//! Every other packet in this crate goes socket → frame → parse → apply, through
//! the one dispatch in [`crate::socket::handler`]. This one does not, for the same
//! reason [`crate::socket::world::WorldSession::char_enum`] does not: it happens
//! **before there is a world**, on the authenticated socket the character screen
//! is holding, and the client has nothing to do between the send and the reply.
//! A request and its answer, in one call, with the intervening packets recorded
//! rather than dropped silently.
//!
//! ## `outfitId` is 0 and that is the server's own reading
//!
//! vmangos' `HandleCharCreateOpcode` reads the byte and never uses it; the
//! starting gear comes from `playercreateinfo_item` keyed by race and class. The
//! reference sends 0 for it, which is what this sends.
//!
//! ## What the reply can say
//!
//! Two windows of `ResponseCodes` and not one — `CHAR_CREATE_*` at 45..=55 and
//! `CHAR_NAME_*` at 69..=82, because a name the server will not take is refused
//! by the name checker rather than by the creator. Both are keys in
//! `Interface\GlueXML\GlueStrings.lua` and both are looked up the same way; see
//! [`crate::codes::response_key`], which is the client's own flat array.

use std::io;
use std::time::Duration;

use crate::bytes::Writer;
use crate::codes::{response_key, CHAR_CREATE_SUCCESS, CHAR_DELETE_SUCCESS};
use crate::opcodes::Opcode;
use crate::socket::world::WorldSession;

/// `CMSG_CHAR_CREATE`'s body.
///
/// **The appearance travels as one `[u8; 5]` rather than as five arguments**,
/// in `PLAYER_BYTES` order — skin, face, hair style, hair colour, facial hair —
/// which is the same order [`crate::socket::world::CharListEntry::appearance`] comes
/// back in. Five small integers in a row is the shape that transposes silently,
/// and a transposed skin and face is a character who looks like somebody else
/// rather than a packet the server refuses; keeping the array whole means the
/// two ends cannot disagree about the order without the *type* disagreeing.
///
/// The name travels as the player typed it: **1.12 capitalises server-side**
/// (`ObjectMgr::NormalizePlayerName`), so a client that title-cased it here
/// would be doing the same work twice and would differ from the reference the
/// first time a locale disagreed about what an initial is.
pub fn char_create_body(
    name: &str,
    race: u8,
    class: u8,
    gender: u8,
    appearance: [u8; 5],
) -> Vec<u8> {
    let mut w = Writer::new();
    w.cstring(name);
    w.u8(race).u8(class).u8(gender);
    for byte in appearance {
        w.u8(byte);
    }
    // `outfitId`, read and ignored by the server — see the module comment.
    w.u8(0);
    w.buf
}

/// Send it and wait up to `wait` for the one byte back.
///
/// `Ok(())` for `CHAR_CREATE_SUCCESS` and an `Err` naming the
/// `GlueStrings.lua` key for anything else — including a code the client's own
/// array does not reach, which reports the number so a strange server is visible
/// rather than silent.
///
/// **Bounded where [`WorldSession::char_enum`] is not**, and the difference is
/// which thread is waiting: the character list is fetched on the task pool and
/// this runs on the frame, because the socket has to stay where the character
/// screen can see it. See `crate::socket::world::session::Session::create_character` in
/// the client, where that argument is.
///
/// **The appearance is the five bytes in `PLAYER_BYTES` order**: skin, face,
/// hair style, hair colour, facial hair. The same order
/// [`crate::socket::world::CharListEntry::appearance`] comes back in, which is what lets
/// a character be created and then recognised on the plinth without a second
/// convention.
pub fn create_character(
    session: &mut WorldSession,
    name: &str,
    race: u8,
    class: u8,
    gender: u8,
    appearance: [u8; 5],
    wait: Duration,
) -> io::Result<Result<(), CreateRefusal>> {
    let body = char_create_body(name, race, class, gender, appearance);
    session.send(Opcode::CMSG_CHAR_CREATE, &body)?;
    let reply = session.recv_until_within(Opcode::SMSG_CHAR_CREATE, wait)?;
    Ok(read_create_result(&reply.body))
}

/// Why the server would not make the character — or delete it, since both
/// refusals are a byte out of the same array and neither carries anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateRefusal {
    /// The wire byte, kept whether or not it names anything — unknown values are
    /// data, not errors.
    pub code: u8,
    /// Its `GlueStrings.lua` key, or `None` for a byte past the client's own
    /// array. A `None` here shows **no dialog at all**, which is the client's
    /// behaviour for a key the shipped file does not carry.
    pub key: Option<&'static str>,
}

/// The reply body, which is one byte.
///
/// **An empty body is a refusal rather than a success.** Nothing in vmangos
/// sends `SMSG_CHAR_CREATE` without a code, but reading "no bytes" as success
/// would put a character on the list that does not exist, and the failure would
/// then be the character screen's rather than this function's.
pub fn read_create_result(body: &[u8]) -> Result<(), CreateRefusal> {
    let Some(&code) = body.first() else {
        return Err(CreateRefusal {
            code: 0,
            key: Some("CHAR_CREATE_ERROR"),
        });
    };
    if code == CHAR_CREATE_SUCCESS {
        return Ok(());
    }
    Err(CreateRefusal {
        code,
        key: response_key(code),
    })
}

/// `CMSG_CHAR_DELETE`'s body: the guid, and nothing else.
///
/// **Plain, not packed.** `HandleCharDeleteOpcode` is `recv_data >> guid` into
/// an `ObjectGuid`, whose stream operator reads eight bytes outright — this is
/// the character screen rather than the world, and the packed form only ever
/// appears in world traffic. Sending a packed guid here is a short body the
/// server reads garbage out of.
pub fn char_delete_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// Send it and wait up to `wait` for the one byte back.
///
/// `Ok(Ok(()))` for `CHAR_DELETE_SUCCESS`, `Ok(Err(..))` for a refusal, and an
/// `Err` only for the socket.
///
/// ## Silence is a refusal here, and that is the whole difference from
/// [`create_character`]
///
/// `HandleCharCreateOpcode` answers on every path. `HandleCharDeleteOpcode`
/// answers on **two of five**: `CHAR_DELETE_SUCCESS`, and `CHAR_DELETE_FAILED`
/// for a guild leader. The other three — a character still loaded (the ALT-F4
/// case), a guid the player cache does not know, and a guid belonging to another
/// account — are bare `return`s with no packet at all.
///
/// So this waits through [`WorldSession::try_recv_until_within`], where the
/// deadline is `Ok(None)` rather than an `io::Error`, and reports the deadline
/// as an ordinary refusal. Reporting it as a broken socket would take a working
/// character screen down every time the server declined quietly — which is a
/// far worse answer than "that did not work", and is not what has happened.
pub fn delete_character(
    session: &mut WorldSession,
    guid: u64,
    wait: Duration,
) -> io::Result<Result<(), CreateRefusal>> {
    session.send(Opcode::CMSG_CHAR_DELETE, &char_delete_body(guid))?;
    match session.try_recv_until_within(Opcode::SMSG_CHAR_DELETE, wait)? {
        Some(reply) => Ok(read_delete_result(&reply.body)),
        None => Ok(Err(SILENTLY_REFUSED)),
    }
}

/// What a delete the server never answered reports — see [`delete_character`].
///
/// `code: 0` is not a `ResponseCodes` value and is not pretending to be one:
/// nothing came back, so there is no byte. The key is the one the *answered*
/// failure uses, because from the character screen the two are the same event.
pub const SILENTLY_REFUSED: CreateRefusal = CreateRefusal {
    code: 0,
    key: Some("CHAR_DELETE_FAILED"),
};

/// The reply body, which is one byte.
///
/// **An empty body is a refusal**, on the same argument
/// [`read_create_result`] makes: reading "no bytes" as success would take a
/// character off the list that is still on the server, and the next
/// `CMSG_CHAR_ENUM` would put it back — a list that flickers rather than a
/// failure that says so.
pub fn read_delete_result(body: &[u8]) -> Result<(), CreateRefusal> {
    let Some(&code) = body.first() else {
        return Err(SILENTLY_REFUSED);
    };
    if code == CHAR_DELETE_SUCCESS {
        return Ok(());
    }
    Err(CreateRefusal {
        code,
        key: response_key(code),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The body is a name and ten bytes, in vmangos' own read order** —
    /// `name >> race >> class >> gender >> skin >> face >> hairStyle >>
    /// hairColor >> facialHair >> outfitId`. A transposition here compiles and
    /// creates a character who looks like somebody else, so the order is what
    /// the test is about.
    #[test]
    fn the_body_is_the_name_then_the_nine_choices_and_a_zero() {
        let body = char_create_body("Hollowell", 4, 8, 1, [3, 5, 9, 2, 4]);
        assert_eq!(&body[..9], b"Hollowell");
        assert_eq!(body[9], 0, "the name is a cstring");
        assert_eq!(&body[10..], &[4, 8, 1, 3, 5, 9, 2, 4, 0]);
        assert_eq!(body.len(), 19);
    }

    /// **46 is the whole of success**, and everything else names a key —
    /// including the two windows that are not adjacent.
    #[test]
    fn the_reply_is_one_byte_and_the_refusals_name_their_own_key() {
        assert_eq!(read_create_result(&[CHAR_CREATE_SUCCESS]), Ok(()));
        assert_eq!(
            read_create_result(&[49]),
            Err(CreateRefusal {
                code: 49,
                key: Some("CHAR_CREATE_NAME_IN_USE")
            })
        );
        // …and a name refused by the *name* checker, thirteen codes further on.
        assert_eq!(
            read_create_result(&[70]),
            Err(CreateRefusal {
                code: 70,
                key: Some("CHAR_NAME_TOO_SHORT")
            })
        );
        // A byte past the client's own array is data: kept, and shown as
        // nothing.
        assert_eq!(
            read_create_result(&[200]),
            Err(CreateRefusal {
                code: 200,
                key: None
            })
        );
        // …and no body at all is not a success.
        assert!(read_create_result(&[]).is_err());
    }

    /// **Eight bytes, little-endian, and not packed** — see
    /// [`char_delete_body`], where the reason is that `ObjectGuid`'s stream
    /// operator reads a whole `uint64` and the packed form is world traffic.
    #[test]
    fn the_delete_body_is_the_guid_and_nothing_else() {
        assert_eq!(
            char_delete_body(0x0000_0000_0000_1234),
            vec![0x34, 0x12, 0, 0, 0, 0, 0, 0]
        );
    }

    /// **57 is the whole of success**, and the one refusal vmangos actually
    /// sends names its own key. The three it does *not* send are the next test.
    #[test]
    fn the_delete_reply_is_one_byte_out_of_the_same_array() {
        assert_eq!(read_delete_result(&[CHAR_DELETE_SUCCESS]), Ok(()));
        // The guild-leader refusal, which is the only code that ever arrives.
        assert_eq!(
            read_delete_result(&[58]),
            Err(CreateRefusal {
                code: 58,
                key: Some("CHAR_DELETE_FAILED")
            })
        );
        // …and the success byte is not the *create* success byte, which is the
        // transposition this pair of constants exists to make impossible.
        assert_ne!(CHAR_DELETE_SUCCESS, CHAR_CREATE_SUCCESS);
        assert!(read_delete_result(&[CHAR_CREATE_SUCCESS]).is_err());
    }

    /// **A delete the server never answers is a refusal, not a dead socket.**
    /// Three of `HandleCharDeleteOpcode`'s five paths `return` without sending
    /// anything; the character screen has to survive all three.
    #[test]
    fn a_delete_that_is_never_answered_reports_the_answered_failures_key() {
        assert_eq!(read_delete_result(&[]), Err(SILENTLY_REFUSED));
        assert_eq!(SILENTLY_REFUSED.key, Some("CHAR_DELETE_FAILED"));
        // Not a `ResponseCodes` value, and deliberately: nothing came back.
        assert_eq!(SILENTLY_REFUSED.code, 0);
        assert_ne!(response_key(0), SILENTLY_REFUSED.key);
    }
}
