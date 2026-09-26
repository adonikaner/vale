//! **When the weapons are out** — which is a decision the client makes and the
//! server only records.
//!
//! `UNIT_FIELD_BYTES_2` byte 0 carries three values (0 stowed, 1 melee drawn,
//! 2 ranged drawn) and it looks like a field to read. It is not: vmangos writes
//! it *from* `CMSG_SETSHEATHED`, whose only sender is the client
//! (`HandleSetSheathedOpcode`, and no other call site touches a `Player`'s), so
//! for the local character the wire is an **echo of our own decision** and never
//! its source. A client that only reads the byte therefore fights with its bare
//! hands for ever: the byte is 0 at login and nothing in the game will ever
//! change it.
//!
//! That was this client's bug. Creatures hid it, because `Creature::Create`
//! calls `SetSheath(SHEATH_STATE_MELEE)` server-side — so every guard in the
//! world had his mace out and only the player punched.
//!
//! ## The three parts, and each is a different mechanism
//!
//! ```text
//! the committed state   a client-side cache, not the descriptor byte
//! the one setter        every path that changes it funnels through one
//! the reconcile         the playing animation forces it, data-driven
//! ```
//!
//! The third is the one worth reading twice, and it is why this module is in
//! `vale-assets` rather than in the renderer: **which weapons an animation
//! implies is a column of `AnimationData.dbc`**, not a rule anybody has to
//! invent. Swimming, mounting, sitting in a chair and casting carry `4`; every
//! emote, both unarmed attacks, sitting on the ground and the three ranged
//! shots carry `0x10`; the armed attacks, the Ready stances and the two fishing
//! rows carry `0x20`. The whole column in 5875 is `{0, 4, 16, 20, 32}` — exactly
//! the three bits the client tests and their one combination — which is what
//! says the column has been identified rather than guessed.
//!
//! So a vendor gossip stows the weapon because `EmoteTalk` carries `0x10`, and
//! nobody has to write a rule about vendors. See [`reconcile`].
//!
//! ## Provenance
//!
//! The structure follows the 1.12.1 client; the **table** is measured here,
//! against this repo's own copy —
//! `vale dbc AnimationData 17` reads `Attack1H`, column 2 = 32 — and the
//! spot rows in [`tests`] are that measurement written down. Which half a claim
//! belongs to matters: the bit *meanings* are transcribed, the
//! column's contents are measured.

/// `UNIT_BYTES_2_OFFSET_SHEATH_STATE`'s three values — the whole vocabulary.
///
/// They decide which point each of the three weapon slots hangs from
/// ([`crate::look::dress`]) and which family of attack animation the unit swings
/// ([`crate::tables::item::WeaponAnim`]), which is why they live here rather than in
/// either of those.
pub const UNARMED: u8 = 0;
pub const MELEE: u8 = 1;
pub const RANGED: u8 = 2;

/// `MAX_SHEATH_STATE` — vmangos' own bound. `HandleSetSheathedOpcode` returns
/// without doing anything for a value at or above it, so a request past the end
/// is silently ignored rather than refused, and the server and the client would
/// then disagree for the rest of the session.
pub const MAX_STATE: u8 = 3;

/// The `AnimationData.dbc` **WeaponFlags** bits (column 2), which are the whole
/// input to [`reconcile`] besides the unit's own state.
pub mod weapon_flags {
    /// Put them away: the casts, `Swim` 42..45, `Mount` 91, the chair sits
    /// 102..104, `Loot` 50.
    pub const STOW_HANDS_BUSY: u32 = 4;
    /// Put them away because this clip **needs empty hands**: every emote,
    /// `AttackUnarmed` 16 and `AttackUnarmedOff` 117, the ground sits 96..98,
    /// the kneels 114..116, and the bow/rifle/thrown shots.
    pub const STOW_EMPTY_HANDS: u32 = 0x10;
    /// Draw the melee weapon: the armed attacks 17..19 / 85..88, the Ready
    /// stances 26..28, the specials, `FishingCast`/`FishingLoop` 133..134.
    pub const DRAW_MELEE: u32 = 0x20;
}

/// The ranged-handling animations exempt from [`weapon_flags::STOW_EMPTY_HANDS`]
/// **while already ranged-drawn** — the client's predicate, whose sole
/// caller is the `cur == 2` path.
///
/// Without it an archer stows the bow on the frame it fires, which is the one
/// moment it must not. `ReadyThrown` (108) is deliberately **not** in the set
/// and genuinely stows; what keeps a real thrown wind-up on screen is a
/// call-order bracket around the kit play rather than an exemption, and this
/// client does not model ranged wind-ups at all yet.
const RANGED_EXEMPT: [u16; 9] = [46, 49, 105, 106, 107, 109, 110, 111, 112];

/// The per-animation sheath reconcile — the client's own, run inside
/// `PlayAnimation` and therefore **once per play, never per frame**.
///
/// Returns the state the policy forces, or `None` to leave the committed state
/// alone. Every force is a snap: only the manual `ToggleSheath` plays the
/// draw/stow ceremony, and that is a property of the *request* rather than of
/// this function.
///
/// Priority-ordered, first match wins:
///
/// 1. [`weapon_flags::STOW_HANDS_BUSY`] — a cast, a swim, a mount, a chair.
/// 2. **mounted** — a persistent draw-block: the reconcile forces
///    0 on every recompute while a mount is attached, so the manual toggle can
///    never stick, and dismounting does *not* restore what was drawn before.
/// 3. [`weapon_flags::STOW_EMPTY_HANDS`] — an emote, a fist, sitting on the
///    floor — except [`RANGED_EXEMPT`] while ranged-drawn.
/// 4. **engaged, or** [`weapon_flags::DRAW_MELEE`] — draw the melee weapon,
///    and only while not already ranged-drawn. *This is the branch that puts
///    the sword in the player's hand*, and note that there is no `!engaged`
///    branch anywhere: leaving combat never stows. That is the real behaviour —
///    a weapon stays out after a fight until something with a stow flag plays.
/// 5. A **remote** unit with no force is pulled back to the server's descriptor
///    byte. The local player's committed state is never
///    server-reconciled, because it is the thing the server is echoing.
///
/// `anim` is the id that was **asked for**, not the one the model resolved to.
/// The client's arm descriptor carries the request, so a weaponless chicken
/// asked for `Attack2H` reconciles on `0x20` even though it resolved to
/// `AttackUnarmed`'s clip.
pub fn reconcile(
    cur: u8,
    anim: u16,
    flags: u32,
    engaged: bool,
    local: bool,
    server_byte: u8,
    mounted: bool,
) -> Option<u8> {
    if flags & weapon_flags::STOW_HANDS_BUSY != 0 {
        return Some(UNARMED);
    }
    if mounted {
        return Some(UNARMED);
    }
    if cur == RANGED {
        if flags & weapon_flags::STOW_EMPTY_HANDS != 0 && !RANGED_EXEMPT.contains(&anim) {
            return Some(UNARMED);
        }
    } else {
        if flags & weapon_flags::STOW_EMPTY_HANDS != 0 {
            return Some(UNARMED);
        }
        if engaged || flags & weapon_flags::DRAW_MELEE != 0 {
            return Some(MELEE);
        }
    }
    if !local && cur != server_byte {
        return Some(server_byte);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use weapon_flags::*;

    /// **The branch the reported bug was about.** A player with a sword,
    /// engaged, playing anything without a stow flag: the weapon comes out. Both
    /// halves matter — `engaged` alone does it, and so does the swing's own
    /// `0x20` for a unit the client has not yet noticed is in combat.
    #[test]
    fn engaging_draws_the_melee_weapon() {
        // Standing (`Stand` carries no policy at all) but in combat.
        assert_eq!(reconcile(UNARMED, 0, 0, true, true, 0, false), Some(MELEE));
        // …and the armed swing draws on its own flag, engaged or not.
        assert_eq!(
            reconcile(UNARMED, 17, DRAW_MELEE, false, true, 0, false),
            Some(MELEE)
        );
    }

    /// **Leaving combat does not stow**, and there is deliberately no branch
    /// that would. A weapon drawn for a fight stays out afterwards until
    /// something with a stow flag plays, which is what the real client does.
    #[test]
    fn disengaging_leaves_the_weapon_out() {
        assert_eq!(reconcile(MELEE, 0, 0, false, true, 0, false), None);
    }

    /// The three stow families, each from a different flag and each a case this
    /// client can actually reach: a cast, a swim, an emote, a fist.
    #[test]
    fn the_stow_flags_put_them_away() {
        for (anim, flags) in [
            (42u16, STOW_HANDS_BUSY),  // Swim
            (91, STOW_HANDS_BUSY),     // Mount
            (60, STOW_EMPTY_HANDS),    // EmoteTalk — the vendor gossip
            (16, STOW_EMPTY_HANDS),    // AttackUnarmed: a fist needs an empty hand
        ] {
            assert_eq!(
                reconcile(MELEE, anim, flags, true, true, 0, false),
                Some(UNARMED),
                "anim {anim} should stow even while engaged"
            );
        }
    }

    /// **A stow outranks `engaged`**, which is the ordering that makes a
    /// swimming warrior put the sword away instead of flickering: were the
    /// engaged draw tested first, every stow would be undone by the next play.
    #[test]
    fn a_stow_outranks_the_engaged_draw() {
        assert_eq!(
            reconcile(MELEE, 42, STOW_HANDS_BUSY, true, true, 0, false),
            Some(UNARMED)
        );
    }

    /// **Entering combat does not draw on its own, and that is the ordering
    /// rather than a hole.**
    ///
    /// A stowed character in combat is playing `ReadyUnarmed` (25) — which
    /// carries the empty-hands flag, because an unarmed guard stance genuinely
    /// needs empty hands — so priority 3 answers before priority 4 ever runs.
    /// The real client's `engaged` is the auto-attack *target*, set by
    /// `Player::Attack`, which is the same press that sends the explicit draw.
    ///
    /// Pinned because the consequence is load-bearing: the attack-start request
    /// in `game::action` is **the** mechanism and this branch is a backstop, so
    /// deleting the request as redundant leaves the weapon on the wearer's back
    /// for ever — which is the bug this whole module was written for.
    #[test]
    fn a_stowed_fighter_in_combat_stays_stowed_until_something_asks() {
        const READY_UNARMED: u16 = 25;
        assert_eq!(
            reconcile(
                UNARMED,
                READY_UNARMED,
                STOW_EMPTY_HANDS,
                true,
                true,
                0,
                false
            ),
            Some(UNARMED),
            "the empty-hands stow answers before the engaged draw"
        );
        // …and once something *has* asked, the drawn stance keeps it out: 26
        // `Ready1H` carries the draw flag, so the answer agrees with the state
        // already held rather than flapping back on the next play. It is
        // `Some(MELEE)` and not `None` because this function answers "what the
        // policy forces", and idempotence is the caller's filter — see
        // `world::entities::sheath::reconcile`, which is what stops a volunteer
        // packet going out on every play.
        assert_eq!(
            reconcile(MELEE, 26, DRAW_MELEE, true, true, 0, false),
            Some(MELEE),
            "already drawn, and the drawn stance agrees"
        );
    }

    /// Mounted is a persistent block rather than a one-off stow: it forces 0 on
    /// **every** recompute, so a toggle pressed in the saddle cannot stick.
    #[test]
    fn a_mounted_rider_cannot_draw() {
        assert_eq!(reconcile(MELEE, 17, DRAW_MELEE, true, true, 0, true), Some(UNARMED));
        assert_eq!(reconcile(UNARMED, 0, 0, true, true, MELEE, true), Some(UNARMED));
    }

    /// **An archer does not stow on the frame it shoots.** `AttackBow` carries
    /// the empty-hands flag like every other unarmed-ish clip, and the exemption
    /// is what stops the bow vanishing mid-volley — but only while already
    /// ranged-drawn, which is the exemption's own gate.
    #[test]
    fn the_ranged_family_is_exempt_only_while_ranged_drawn() {
        assert_eq!(
            reconcile(RANGED, 46, STOW_EMPTY_HANDS, true, true, 0, false),
            None,
            "AttackBow while the bow is out"
        );
        assert_eq!(
            reconcile(MELEE, 46, STOW_EMPTY_HANDS, true, true, 0, false),
            Some(UNARMED),
            "…and the same clip with a sword out stows, as any 0x10 does"
        );
        // ReadyThrown is *not* exempt, which is the one that reads as an
        // oversight and is not — see `RANGED_EXEMPT`.
        assert_eq!(
            reconcile(RANGED, 108, STOW_EMPTY_HANDS, true, true, 0, false),
            Some(UNARMED)
        );
    }

    /// **Only a remote unit is pulled back to the wire.** Ours is the source of
    /// that byte, so reconciling the local player against it would undo every
    /// draw the moment the echo arrived — and the echo is always one round trip
    /// behind the decision that caused it.
    #[test]
    fn the_server_byte_reconciles_remotes_and_not_us() {
        assert_eq!(reconcile(UNARMED, 0, 0, false, false, MELEE, false), Some(MELEE));
        assert_eq!(
            reconcile(UNARMED, 0, 0, false, true, MELEE, false),
            None,
            "our own committed state is not overwritten by our own echo"
        );
    }

    /// A play with no policy and nothing else to say changes nothing. Most
    /// plays are this — `Stand`, `Run`, `Walk` all read 0 in the column — and it
    /// is why the reconcile is cheap.
    #[test]
    fn an_ordinary_gait_says_nothing() {
        assert_eq!(reconcile(MELEE, 5, 0, false, true, MELEE, false), None);
    }
}
