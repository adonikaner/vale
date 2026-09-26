//! **Spending a talent point** — one outbound packet, and the two counters it
//! moves.
//!
//! The thinnest subject in this directory, and deliberately so: almost the whole
//! of the talent panel is *rules* over `Talent.dbc`, which live in
//! `vale_assets::tables::talent` where they can be checked with no server.
//! What genuinely crosses the wire is three things:
//!
//! ```text
//! how many points   PLAYER_CHARACTER_POINTS1 / 2, in an ordinary values block
//! what is spent     nothing — a rank is read back out of the known spells
//! spending one      CMSG_LEARN_TALENT, and no reply of its own
//! ```
//!
//! ## Nothing on the wire says "3 points in Impale"
//!
//! That is the fact this module exists to state. A talent's rank is not sent and
//! is not stored anywhere the client can see; each rank is a separate
//! `Spell.dbc` row, the server teaches it with `SMSG_LEARNED_SPELL`, and both
//! sides work the rank back out by asking which of the nine rank spells the
//! character knows. `Player::LearnTalent` (`Objects/Player.cpp:20786`) does
//! exactly that, and so does `GetTalentInfo`. See
//! `vale_assets::tables::talent`, where the walk is.
//!
//! The consequence for this crate is that **no packet needs to be added to keep
//! a talent panel current**: `SMSG_INITIAL_SPELLS` and its two deltas already
//! move `spellbook_version`, and the panel is rebuilt off that.
//!
//! ## The reply is not a reply
//!
//! `CMSG_LEARN_TALENT` is acknowledged by nothing. A refusal is silence —
//! `Player::LearnTalent` returns `false` down eleven paths and says nothing on
//! any of them — and a success arrives as an ordinary `SMSG_LEARNED_SPELL` plus
//! `PLAYER_CHARACTER_POINTS1` moving in the next values block. So the client
//! sends and then waits for the world to change, which is the same shape
//! [`super::reputation`]'s three verbs have.

/// `CMSG_LEARN_TALENT` — `{ u32 talentId, u32 requestedRank }`.
///
/// **The rank is zero-based**: a talent with no points asks for rank 0, and
/// `Player::LearnTalent`'s own test is `if (talentRank >= MAX_TALENT_RANK)`.
/// Both halves agree — vmangos reads exactly these two dwords
/// (`Server/Packets/Skill.cpp:3`), and the client writes exactly these two after
/// opcode `0x251`.
///
/// Which rank to ask for is `vale_assets::tables::talent::TalentTree::learn`'s
/// decision, and the *only* gate either side of this packet applies locally.
pub fn learn_talent_body(talent_id: u32, rank: u32) -> Vec<u8> {
    let mut body = Vec::with_capacity(8);
    body.extend_from_slice(&talent_id.to_le_bytes());
    body.extend_from_slice(&rank.to_le_bytes());
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_two_little_endian_dwords() {
        assert_eq!(
            learn_talent_body(124, 0),
            vec![124, 0, 0, 0, 0, 0, 0, 0],
            "an unspent talent asks for rank 0"
        );
        assert_eq!(learn_talent_body(0x0102_0304, 4)[..4], [4, 3, 2, 1]);
    }
}
