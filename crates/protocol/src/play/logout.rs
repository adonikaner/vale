//! **Leaving.** The four packets between pressing Logout and the character
//! screen, and the one thing about them that is not obvious from their names.
//!
//! ```text
//! CMSG_LOGOUT_REQUEST   (no body)   we would like to go
//! SMSG_LOGOUT_RESPONSE  u32 reason, u8 instant
//! CMSG_LOGOUT_CANCEL    (no body)   …changed our mind
//! SMSG_LOGOUT_CANCEL_ACK (no body)
//! SMSG_LOGOUT_COMPLETE  (no body)   the character is out of the world
//! ```
//!
//! ## The client does not own the timer, and that is the surprising half
//!
//! 1.12's interface shows a twenty-second countdown (`StaticPopupDialogs["CAMP"]`,
//! `timeout = 20`) and it is **cosmetic**: the server holds the request and sends
//! `SMSG_LOGOUT_COMPLETE` when its own clock runs out
//! (`vmangos/core/src/game/Maps/Map.cpp:3484`). So nothing here counts, and a
//! client that logged itself out on the popup's timeout would be leaving the
//! world before the server had let go of the character.
//!
//! ## `reason` is not a status code, it is a refusal
//!
//! `HandleLogoutRequestOpcode` (`MiscHandler.cpp:304`) writes
//! **`0` and `instant = 1`** in a tavern, a city, on a taxi or for a GM above
//! `CONFIG_UINT32_INSTANT_LOGOUT` — the character leaves *now* and
//! `SMSG_LOGOUT_COMPLETE` is already on its way. It writes **`0` and
//! `instant = 0`** in the field, sits the character down, roots it, and starts
//! the twenty seconds. Anything **non-zero** is a refusal with the request
//! dropped: `1` in combat, `3` jumping or falling far, `2` frozen by a GM.
//!
//! The refusal has no `GlobalStrings` route in `Interface\FrameXML\` at all —
//! `PLAYER_LOGOUT_FAILED_ERROR` ("You can't log out because you can't sit down
//! right now.") appears in `GlobalStrings.lua` and in none of the other 90 files
//! — so, like `LEVELUPSOUND`, the C client is what shows it. This module only
//! carries the number; who says what about it is the client's.

use crate::bytes::Reader;

/// What the server said about leaving, as one value.
///
/// Four states rather than a struct because every consumer branches on which of
/// the four it is: the interface raises a different event for three of them and
/// the client tears the session down on the fourth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Logout {
    /// `SMSG_LOGOUT_RESPONSE` with `reason == 0`: the request was taken.
    ///
    /// `instant` is the server's own flag and not a timing hint — see the
    /// module comment. When it is set, `SMSG_LOGOUT_COMPLETE` follows in the
    /// same breath and no countdown is ever shown.
    Started { instant: bool },
    /// …and with a non-zero `reason`: refused, and the request is *gone*. There
    /// is nothing to cancel and pressing the button again is the only retry.
    Refused { reason: u32 },
    /// `SMSG_LOGOUT_CANCEL_ACK` — the character stands back up.
    Cancelled,
    /// `SMSG_LOGOUT_COMPLETE` — out of the world. The socket is still open and
    /// the real client goes back to character select on it.
    Complete,
}

/// `SMSG_LOGOUT_RESPONSE` — `{u32 reason, u8 instant}`, five bytes.
///
/// **The order is reason-then-flag** and both are read: a client that took the
/// first byte as the flag would read every refusal as an instant logout, which
/// is a character screen with the character still standing in the world.
pub fn parse_logout_response(body: &[u8]) -> Option<Logout> {
    let mut r = Reader::new(body);
    if !r.has(5) {
        return None;
    }
    let reason = r.u32();
    let instant = r.u8() != 0;
    Some(if reason == 0 {
        Logout::Started { instant }
    } else {
        Logout::Refused { reason }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;

    /// The three shapes `HandleLogoutRequestOpcode` writes, in its own order.
    #[test]
    fn the_response_separates_a_refusal_from_an_instant_logout() {
        let body = |reason: u32, instant: u8| {
            let mut w = Writer::new();
            w.u32(reason).u8(instant);
            w.buf
        };
        assert_eq!(
            parse_logout_response(&body(0, 1)),
            Some(Logout::Started { instant: true }),
            "a tavern, a city or a GM: gone at once"
        );
        assert_eq!(
            parse_logout_response(&body(0, 0)),
            Some(Logout::Started { instant: false }),
            "the field: sat down, rooted, twenty seconds on the server's clock"
        );
        assert_eq!(
            parse_logout_response(&body(1, 0)),
            Some(Logout::Refused { reason: 1 }),
            "in combat"
        );
        // **A truncated body is not an instant logout.** Five bytes or nothing:
        // reading a short packet as `reason = 0` would take the world away.
        assert_eq!(parse_logout_response(&[0, 0, 0, 0]), None);
        assert_eq!(parse_logout_response(&[]), None);
    }
}
