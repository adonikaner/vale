//! **A duel: six packets in, two out, and a flag between them.**
//!
//! A challenge is a spell — Duel, 7266, cast at another player — and
//! `Spell::EffectDuel` answers it by spawning the flag (the *arbiter*, a game
//! object halfway between the two) and sending both players the same
//! `SMSG_DUEL_REQUESTED`. Everything after that is keyed on the arbiter's guid.
//!
//! ```text
//! SMSG_DUEL_REQUESTED    u64 arbiter, u64 initiator      both players
//! SMSG_DUEL_COUNTDOWN    u32 milliseconds (3000)          both, on the accept
//! SMSG_DUEL_OUTOFBOUNDS  (empty)                          past 75 yards of the flag
//! SMSG_DUEL_INBOUNDS     (empty)                          back inside 70
//! SMSG_DUEL_COMPLETE     u8 started                       both
//! SMSG_DUEL_WINNER       u8 fled, cstring winner, cstring loser   everyone nearby
//! CMSG_DUEL_ACCEPTED     u64 arbiter
//! CMSG_DUEL_CANCELLED    u64 arbiter
//! ```
//!
//! The layouts are vmangos' `WorldPackets::Duel` and `Spell::EffectDuel`; the
//! meaning of each is the reference client's six handlers:
//!
//! * **The initiator's own client accepts on its own behalf.** When the
//!   initiator named in the request is the local player, the client says
//!   `ERR_DUEL_REQUESTED` and sends `CMSG_DUEL_ACCEPTED` at once. vmangos ignores it (`HandleDuelAcceptedOpcode` returns for
//!   the initiator), so it is harmless, and it is what the reference does.
//! * **A challenger on the ignore list is declined without asking**:
//!   the client tests the ignore list and sends `CMSG_DUEL_CANCELLED`.
//! * **A challenger who is not in view raises nothing.** The event carries the
//!   name, and the client looks the guid up among the loaded players only.
//! * **`started` decides one sentence.** `SMSG_DUEL_COMPLETE` with 0 is a
//!   duel that was declined or abandoned before the countdown ended, and the
//!   reference says `ERR_DUEL_CANCELLED` for it; with 1 it says nothing. Both
//!   raise `DUEL_FINISHED`.
//! * **The countdown is a chat line a second**, not a frame: the client
//!   formats `DUEL_COUNTDOWN` into the system chat type and re-arms itself
//!   every 1,000 ms until it reaches zero.
//! * **The winner is a chat line too**, `DUEL_WINNER_KNOCKOUT` or
//!   `DUEL_WINNER_RETREAT` on `fled`, with the winner first and the loser
//!   second. Both keys are positional (`%1$s`, `%2$s`), and the retreat one
//!   names the loser first in its sentence.

use crate::bytes::{Reader, Writer};

/// `SMSG_DUEL_REQUESTED` — `(arbiter, initiator)`.
pub fn parse_duel_requested(body: &[u8]) -> Option<(u64, u64)> {
    if body.len() < 16 {
        return None;
    }
    let mut r = Reader::new(body);
    Some((r.u64(), r.u64()))
}

/// `SMSG_DUEL_COUNTDOWN` — milliseconds until the duel starts. vmangos always
/// sends 3000; the reference divides by 1,000 and counts down whole seconds.
pub fn parse_duel_countdown(body: &[u8]) -> Option<u32> {
    if body.len() < 4 {
        return None;
    }
    Some(Reader::new(body).u32())
}

/// `SMSG_DUEL_COMPLETE` — whether the duel had started. `false` is a duel
/// declined or abandoned during the countdown.
pub fn parse_duel_complete(body: &[u8]) -> Option<bool> {
    body.first().map(|&started| started != 0)
}

/// `SMSG_DUEL_WINNER`, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuelWinner {
    /// The loser left the area rather than being beaten.
    pub fled: bool,
    pub winner: String,
    pub loser: String,
}

/// `SMSG_DUEL_WINNER` — a flag and two names.
///
/// **Both names must be terminated.** A body cut inside the second name would
/// otherwise read as a shorter name, which prints a plausible wrong sentence.
pub fn parse_duel_winner(body: &[u8]) -> Option<DuelWinner> {
    let (&fled, rest) = body.split_first()?;
    if rest.iter().filter(|&&b| b == 0).count() < 2 {
        return None;
    }
    let mut r = Reader::new(rest);
    Some(DuelWinner {
        fled: fled != 0,
        winner: r.cstring(),
        loser: r.cstring(),
    })
}

/// `CMSG_DUEL_ACCEPTED` and `CMSG_DUEL_CANCELLED` — the arbiter's guid, the
/// same body for both. vmangos reads the guid and uses neither: it answers off
/// the player's own `m_duel`. The reference sends the stored arbiter anyway.
pub fn duel_answer_body(arbiter: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(arbiter);
    w.buf
}

/// The Duel spell, which is what a challenge is: `CMSG_CAST_SPELL` 7266 at the
/// other player. `StartDuel` and `StartDuelUnit` both end in the reference's
/// cast-by-id call with this id.
pub const DUEL_SPELL: u32 = 7266;

#[cfg(test)]
mod tests {
    use super::*;

    /// **The five inbound bodies as vmangos writes them.** The request's two
    /// guids are the pair worth pinning: swapped, the initiator's own client
    /// would ask *itself* whether to accept.
    #[test]
    fn the_duel_bodies_read_back_what_was_written() {
        let mut w = Writer::new();
        w.u64(0xF110_0000_0000_0042).u64(0x0000_0000_0000_0007);
        assert_eq!(
            parse_duel_requested(&w.buf),
            Some((0xF110_0000_0000_0042, 7))
        );
        assert_eq!(parse_duel_countdown(&3000u32.to_le_bytes()), Some(3000));
        assert_eq!(parse_duel_complete(&[1]), Some(true));
        assert_eq!(parse_duel_complete(&[0]), Some(false));

        let mut w = Writer::new();
        w.u8(1).cstring("Bram").cstring("Alden");
        assert_eq!(
            parse_duel_winner(&w.buf),
            Some(DuelWinner {
                fled: true,
                winner: "Bram".into(),
                loser: "Alden".into()
            })
        );
        assert_eq!(duel_answer_body(0x42), 0x42u64.to_le_bytes().to_vec());
    }

    /// A short body is `None`: the request, the countdown, the completion, and
    /// a winner cut inside its second name.
    #[test]
    fn a_truncated_duel_body_is_declined() {
        assert_eq!(parse_duel_requested(&[0; 15]), None);
        assert_eq!(parse_duel_countdown(&[0; 3]), None);
        assert_eq!(parse_duel_complete(&[]), None);
        let mut w = Writer::new();
        w.u8(0).cstring("Bram");
        w.buf.extend_from_slice(b"Thorn");
        assert_eq!(parse_duel_winner(&w.buf), None);
    }
}
