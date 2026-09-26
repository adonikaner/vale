//! **What colour the ring under a unit is** — the client's own selector, and
//! the one rule the selection circle is made of that is not geometry.
//!
//! The circle itself is `Textures\UnitSelectTexture.blp`, a 256x256 image whose
//! RGB is **white everywhere** and whose alpha is the ring: nothing from r = 0
//! to about 0.28 of the half-width, a soft ~44/255 fill out to 0.85, a bright
//! 136/255 rim at 0.875, and gone by 0.97. So the file states the *shape* and
//! the client states the *colour*, which is what this module is.
//!
//! ## The selector
//!
//! The client's selection-circle colour is the whole visible output of the
//! feature, and it is this table.
//! It lazily builds a palette of `ARGB` dwords and then branches:
//!
//! ```text
//! in melee combat                 -> the melee combat flash's own colour
//! player-controlled?
//!   unit may attack me?
//!     ...and I may attack it?     -> HOSTILE   else PLAYER (pale blue)
//!   I may attack it?              -> NEUTRAL (yellow)
//!   it is PvP-flagged?            -> FRIENDLY (green), party -> pale
//!   otherwise                     -> PLAYER (pale blue), party -> pale
//! otherwise: dead -> grey, else the rank table by reaction
//! ```
//!
//! The palette's own dwords, `(a, r, g, b)`:
//!
//! | value | used for |
//! |---|---|
//! | `ffff0000` | reaction rank 0 and 1 — hostile red |
//! | `ffff8000` | rank 2 — unfriendly orange |
//! | `ffffff00` | rank 3 — neutral yellow |
//! | `ff00ff00` | ranks 4..7 — friendly green |
//! | `ff6060ff` | a player, pale blue |
//! | `ffaaaaff` | …in our party |
//! | `ffaaffaa` | a PvP-flagged party member, pale green |
//! | `ffffaaaa` | pale red (unreached by the branches above) |
//!
//! The player branch is the same shape as `GameTooltip_UnitColor` in the shipped
//! `GameTooltip.lua`, which is a useful cross-check: the same three questions in
//! the same order, one layer up in Lua and about the name rather than the ring.
//!
//! ## What is deliberately not modelled, and each is one-sided
//!
//! * **the combat flash**, which is the selector's *first* branch and outranks
//!   everything below it: a red↔orange pulse while the unit is in melee. The bit
//!   is a live field this client does not parse.
//! * **the party legs**, because there is no party: the two pale variants are
//!   here as constants and nothing chooses them.
//! * **the orange rank**, which needs a four-value reaction where
//!   [`crate::tables::faction::Reaction`] is three — an "unfriendly" creature reads
//!   hostile here. That is the same approximation `Reaction` already documents,
//!   not a second one.

use crate::tables::faction::Reaction;

/// The ring's own art, named by the client beside the object that draws it.
/// See the module comment for what is in it.
pub const TEXTURE: &str = r"Textures\UnitSelectTexture.blp";

/// An `ARGB` dword as the client stores it, so the table above can be compared
/// by eye.
type Argb = u32;

/// The eight the selector's palette holds. Named rather than inlined because
/// the *names* are what the branches are argued in.
pub mod palette {
    use super::Argb;
    pub const HOSTILE: Argb = 0xffff_0000;
    pub const UNFRIENDLY: Argb = 0xffff_8000;
    pub const NEUTRAL: Argb = 0xffff_ff00;
    pub const FRIENDLY: Argb = 0xff00_ff00;
    /// A player nobody may attack — the pale blue, and **not** the nameplate
    /// palette's pure blue.
    pub const PLAYER: Argb = 0xff60_60ff;
    /// A dead creature. Players skip the health check entirely, which is why
    /// this is only reachable from the creature branch.
    pub const DEAD: Argb = 0xff7f_7f7f;
    /// The two party variants, present and unchosen — see the module comment.
    pub const PARTY: Argb = 0xffaa_aaff;
    pub const PARTY_PVP: Argb = 0xffaa_ffaa;
}

/// Everything the selector asks about the unit under the ring.
///
/// A struct rather than five arguments because four of the five are booleans
/// and a transposed pair would colour every player in the world wrong while
/// still compiling.
#[derive(Debug, Clone, Copy)]
pub struct Selected {
    /// Is a person driving it? The selector's first real branch.
    pub player_controlled: bool,
    /// May **it** attack the local player, and may the local player attack it?
    /// Two separate questions, in the selector's own order.
    pub attacks_me: bool,
    pub i_attack_it: bool,
    /// `UNIT_FIELD_FLAGS` bit 12.
    pub pvp: bool,
    /// Health has reached zero. Only consulted on the creature branch.
    pub dead: bool,
    /// **How the unit stands towards the local player**, for the rank table —
    /// the reaction of the unit being coloured towards the local player. So it
    /// is `GetReaction(unit, player)` and **not**
    /// the other way round, which is the direction `TargetFrame_CheckFaction`
    /// asks it in too (`UnitReaction("target", "player")`).
    ///
    /// The distinction was invisible while a reaction was `FactionTemplate.dbc`
    /// alone — the friendly half of that rule is symmetric — and it stopped
    /// being invisible the moment the character's own state joined it: a forced
    /// reaction and the at-war bit are read on the leg where *we* are asking,
    /// and the contested-guard flag on the leg where *they* are. Ask it
    /// backwards and a guard the server has turned hostile keeps a green name
    /// and a green ring while the same client lets you attack it.
    pub reaction: Reaction,
}

/// The ring's colour, as `ARGB`.
pub fn ring_colour(unit: &Selected) -> Argb {
    if unit.player_controlled {
        // The unit's own attackability first, then ours. The pair
        // is not symmetric — a flagged player may attack an unflagged one and
        // not the reverse — which is the whole reason the selector asks twice.
        if unit.attacks_me {
            return if unit.i_attack_it { palette::HOSTILE } else { palette::PLAYER };
        }
        if unit.i_attack_it {
            return palette::NEUTRAL;
        }
        return if unit.pvp { palette::FRIENDLY } else { palette::PLAYER };
    }
    // A corpse is grey whatever its faction says, and only then the
    // rank table.
    if unit.dead {
        return palette::DEAD;
    }
    match unit.reaction {
        Reaction::Hostile => palette::HOSTILE,
        Reaction::Neutral => palette::NEUTRAL,
        Reaction::Friendly => palette::FRIENDLY,
    }
}

/// The same as linear-space floats plus opacity, for a renderer that wants to
/// multiply a texel by it.
///
/// **Not gamma-decoded**: the dword is the colour the fixed-function client
/// wrote into a vertex, and this project's own rule is that a value out of the
/// game's data stays in the game's space until the frame is composed — see
/// `render::present`.
pub fn ring_rgba(unit: &Selected) -> [f32; 4] {
    let argb = ring_colour(unit);
    let byte = |shift: u32| ((argb >> shift) & 0xff) as f32 / 255.0;
    [byte(16), byte(8), byte(0), byte(24)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The creature branch is the rank table**, and a corpse overrides it —
    /// which is the one arm that is about health rather than about faction.
    #[test]
    fn a_creature_takes_its_reaction_and_a_corpse_takes_grey() {
        let creature = |reaction, dead| Selected {
            player_controlled: false,
            attacks_me: false,
            i_attack_it: true,
            pvp: false,
            dead,
            reaction,
        };
        assert_eq!(ring_colour(&creature(Reaction::Hostile, false)), palette::HOSTILE);
        assert_eq!(ring_colour(&creature(Reaction::Neutral, false)), palette::NEUTRAL);
        assert_eq!(ring_colour(&creature(Reaction::Friendly, false)), palette::FRIENDLY);
        // Dead outranks all three, and it is checked *before* the table.
        assert_eq!(ring_colour(&creature(Reaction::Hostile, true)), palette::DEAD);
    }

    /// **The ring and the name plate ask the reaction of the *unit*, not of
    /// us**, and the two answers differ exactly where it matters.
    ///
    /// The property, stated where the direction can be checked rather than
    /// asserted: build a guard whose faction template is friendly to the
    /// character's both ways, put the character in the state a contested-guard
    /// faction is hostile to, and the two directions come apart — one Friendly,
    /// one Hostile. Whichever this crate's consumers pass is the one the ring is
    /// painted from, and the reference passes the second.
    #[test]
    fn the_two_directions_of_one_pair_are_not_the_same_answer() {
        use crate::tables::faction::{
            player_flags, unit_flags, Factions, Party, Rank, Standing, CONTESTED_GUARD,
        };
        use crate::tables::dbc::testing::dbc;

        // One faction group, so the masks call the pair friendly both ways, and
        // one row with the guard flag on it.
        let rows = vec![
            vec![1, 1, 0, 0b0010, 0b0010, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            vec![2, 1, CONTESTED_GUARD, 0b0010, 0b0010, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ];
        let table = Factions::parse(&dbc(&rows, 14, &[0])).expect("a faction table");

        let me = Party {
            guid: 7,
            faction: Some(1),
            unit_flags: unit_flags::PLAYER_CONTROLLED,
            player_flags: player_flags::CONTESTED_PVP,
            is_local: true,
            ..Party::default()
        };
        let guard = Party { guid: 8, faction: Some(2), ..Party::default() };
        let nothing = Standing::default();

        assert_eq!(
            table.rank(&me, &guard, &nothing),
            Rank::Friendly,
            "how we see the guard"
        );
        assert_eq!(
            table.rank(&guard, &me, &nothing),
            Rank::Hostile,
            "how the guard sees us — and this is the one the ring is painted from"
        );
        // …and the ring the second one produces is the red one.
        let ring = |reaction| {
            ring_colour(&Selected {
                player_controlled: false,
                attacks_me: true,
                i_attack_it: true,
                pvp: false,
                dead: false,
                reaction,
            })
        };
        assert_eq!(ring(table.rank(&guard, &me, &nothing).into()), palette::HOSTILE);
        assert_eq!(ring(table.rank(&me, &guard, &nothing).into()), palette::FRIENDLY);
    }

    /// **A player is four answers off two attackability questions**, and the
    /// order matters: an enemy player is red only when the pair agrees.
    #[test]
    fn a_player_takes_the_two_attackability_questions_in_order() {
        let player = |attacks_me, i_attack_it, pvp| Selected {
            player_controlled: true,
            attacks_me,
            i_attack_it,
            pvp,
            // A dead player is *not* grey — the health check is on the other
            // branch entirely, which is the asymmetry worth pinning.
            dead: true,
            reaction: Reaction::Hostile,
        };
        assert_eq!(ring_colour(&player(true, true, false)), palette::HOSTILE);
        assert_eq!(ring_colour(&player(true, false, false)), palette::PLAYER);
        assert_eq!(ring_colour(&player(false, true, false)), palette::NEUTRAL);
        assert_eq!(ring_colour(&player(false, false, true)), palette::FRIENDLY);
        assert_eq!(ring_colour(&player(false, false, false)), palette::PLAYER);
    }

    /// The dword unpacks the way the table in the module comment reads it —
    /// a swapped channel here is a ring of a plausible wrong colour.
    #[test]
    fn the_dword_unpacks_argb() {
        let orange = Selected {
            player_controlled: false,
            attacks_me: false,
            i_attack_it: true,
            pvp: false,
            dead: false,
            reaction: Reaction::Neutral,
        };
        // Neutral yellow, `ffffff00`: full red, full green, no blue, opaque.
        assert_eq!(ring_rgba(&orange), [1.0, 1.0, 0.0, 1.0]);
    }
}
