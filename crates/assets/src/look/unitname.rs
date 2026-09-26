//! **The name that floats over a unit's head** — who gets one, where it hangs,
//! and how big it is.
//!
//! Like [`super::worldtext`], this is a subject `Interface\FrameXML\` says
//! nothing about. There is no frame, no template and no `OnLoad`: the C side
//! builds the string itself and hands it to the same world-space text renderer
//! the damage numbers go through. So every number below is the client's own.
//!
//! **It is not the nameplate.** The plate — a bar with a
//! health fill and a level — is a different object with its own five files and
//! its own two switches, and the two are mutually exclusive: the client refuses
//! a floating name outright for any unit that has a plate up.
//! This client draws the name and not the plate.
//!
//! ## Who gets one, and it is the whole rule
//!
//! ```text
//! the unit may be named at all           — else no
//! …and has no nameplate up               — else no
//! is it me?          -> UnitNameOwn                (bit 0)
//! is it my target?   -> ALWAYS, whatever the CVars say
//! is it a player?    -> UnitNamePlayer             (bit 2)
//! …otherwise         -> UnitNameNPC                (bit 1)
//! ```
//!
//! **The target clause is unconditional and it is what makes the defaults
//! usable**: it answers yes with no mask test at all, so the thing you have
//! clicked on is named even when its whole class is switched off.
//!
//! ## …and the switches are five CVars
//!
//! Each sets or clears one bit of a single mask, and **the defaults are
//! already the interesting ones**: players yes, everybody else no.
//!
//! | CVar | bit | default |
//! |---|---|---|
//! | `UnitNameOwn` | `0x01` | `"0"` |
//! | `UnitNameNPC` | `0x02` | `"0"` |
//! | `UnitNamePlayer` | `0x04` | `"1"` |
//! | `UnitNamePlayerGuild` | `0x10` | `"1"` |
//! | `UnitNamePlayerPVPTitle` | `0x20` | `"1"` |
//!
//! ## Where it hangs, and why a kodo's name is bigger than yours
//!
//! The same `PlayerName` attachment the plate uses — point 18, the
//! one id in the enum that exists for nothing else.
//!
//! The size is the part worth knowing: the client starts at [`BASE_SIZE`] and
//! then, **for a unit taller than four yards**, multiplies by
//! `height / 4 x 1.5`. The height it
//! measures is the attachment's own z above the unit's origin, so a tall
//! creature's name is drawn larger — which is a rule nothing about the picture
//! would suggest and that a reader would otherwise have called a bug.
//!
//! ## …and the size is a **world** height, which was the open reading
//!
//! A name is drawn *in the world*: small across a valley, large at arm's length.
//! That is what a screenshot of the reference shows — a creature thirty yards
//! off is named at about a third of the height the player beside the camera is —
//! and it is not what a fraction of the screen would do.
//!
//! **The evidence in the client is that there are two font objects, built at
//! sizes two orders of magnitude apart.** The subsystem rasterises
//! `DAMAGE_TEXT_FONT` at `0.018333` and `UNIT_NAME_FONT` at
//! **`0.99`** — and a font rasterised at ninety-nine per cent of
//! the screen's height is only worth building if it is going to be scaled *down*
//! by a lot and by a varying amount. A perspective divide is the only thing in
//! the client that does that. The damage numbers' font, at not quite two per
//! cent, is built at about the size it is drawn at — so the two paths differ
//! exactly as the pictures say they do, and [`super::worldtext`]'s heights stay
//! fractions of the screen while this one is yards.
//!
//! Confirming it from the other side: the size computation divides by no distance
//! anywhere. The size it hands the draw is the same at any range, so if the
//! drawn name shrinks — and it does — the shrink belongs to the projection.
//!
//! The arithmetic agrees to within a fifth: 0.2 yards at the default camera
//! reach, through this client's own vertical field of view, is about eighteen
//! units of a 768-unit interface, which is what a 1024x768 screenshot of the
//! reference measures. And nothing has to reconcile it with a field of view at
//! all: `client::render::labels` puts the name on a **quad in the world**, a
//! fixed number of yards tall, and the camera does the rest.

/// The face, out of `Fonts.xml`'s `UNIT_NAME_FONT` — a Lua global holding a
/// path rather than a font object, exactly as `DAMAGE_TEXT_FONT` is.
pub const FONT: &str = r"Fonts\FRIZQT__.TTF";

/// The attachment the name hangs from — **`PlayerName`**, point 18, and the
/// only place in this client that asks for it.
///
/// The client picks `18 + 11 = 29` instead on a field this client does not
/// read; 29 is unidentified and 18 is the ordinary case.
/// A model that does not carry the point falls back to its **helm** point,
/// which is this client's answer rather than the reference's — the reference has
/// no fallback because every model it names has 18. It used to fall back to the
/// top of the declared box, and see [`BASE_SIZE`] for what that cost.
pub const ANCHOR_ATTACHMENT: u32 = 18;

/// The name's height for an ordinary unit, **in yards**.
///
/// The reference's own number, and see the module comment for why it is a world
/// height rather than a fraction of the screen. A name is therefore a fixed size
/// *in the world*: small across a valley, large at arm's length, and never a
/// label pasted onto the picture at a constant size — which is exactly the fault
/// the first cut of this had.
pub const BASE_SIZE: f32 = 0.2;

/// Above this height, in yards, a unit's name is drawn larger.
pub const TALL: f32 = 4.0;

/// …and by how much per multiple of [`TALL`].
pub const TALL_GAIN: f32 = 1.5;

/// The name's height **in yards** for a unit whose `PlayerName` point is
/// `height` yards above its origin.
///
/// The client compares the height against four yards and leaves the base alone
/// below it, which is why this is a step into a ramp rather than a smooth curve
/// — a unit of exactly four yards takes `0.2` and one of four and a bit takes
/// `0.3`. Everything human-sized takes the base unchanged.
///
/// Because the answer is a world height, the ramp means what it says: a kodo's
/// name is **physically bigger**, and still shrinks with distance like
/// everything else.
pub fn size_for(height: f32) -> f32 {
    if height <= TALL {
        return BASE_SIZE;
    }
    BASE_SIZE * (height / TALL) * TALL_GAIN
}

/// The five switches, as the bits they occupy in the client's mask.
pub mod switches {
    pub const OWN: u32 = 0x01;
    pub const NPC: u32 = 0x02;
    pub const PLAYER: u32 = 0x04;
    pub const GUILD: u32 = 0x10;
    pub const PVP_TITLE: u32 = 0x20;
}

/// **Which units this client names**, as the reference's own bit field.
///
/// A struct over the mask rather than the mask itself, because the whole point
/// of the five is that a person moves them one at a time from the interface
/// options — and a bare `u32` here is the shape that makes a transposed bit
/// compile.
///
/// **Built from the CVars rather than defaulted**, which is the rule this repo
/// is now working to: the settings a session runs on are the ones
/// `Config.wtf` carries, under the names 5875 registered them by, so a build
/// dropped into a real WoW folder picks up that folder's settings and needs no
/// configuration of its own. [`Policy::from_cvars`] is the one door; `Default`
/// exists only for a test and answers what the client's own registrations do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    /// `UnitNameOwn` — your own character's name over your own head.
    pub own: bool,
    /// `UnitNameNPC` — everything that is not a player.
    pub npc: bool,
    /// `UnitNamePlayer` — every other player.
    pub player: bool,
    /// `UnitNamePlayerGuild` — `<Guild Name>` on its own line under a player's.
    pub guild: bool,
    /// `UnitNamePlayerPVPTitle` — the rank prefix on a player's name.
    pub pvp_title: bool,
}

impl Default for Policy {
    /// **1.12's own registrations**, which are the ones worth having: players
    /// named, nobody else, and your target named regardless. See the table in
    /// the module comment for where each comes from.
    ///
    /// This is the *fallback*, not the source: what a session runs on is
    /// [`Policy::from_cvars`] over the store `Config.wtf` filled.
    fn default() -> Self {
        Policy { own: false, npc: false, player: true, guild: true, pvp_title: true }
    }
}

/// The five CVar names, in [`Policy`]'s own field order.
///
/// Spelled as 5875 registered them, because that is the spelling a
/// real `Config.wtf` carries and the spelling the interface asks `GetCVar` by.
/// The lookup itself is case-insensitive; the *file* is not written from this.
pub const CVARS: [&str; 5] = [
    "UnitNameOwn",
    "UnitNameNPC",
    "UnitNamePlayer",
    "UnitNamePlayerGuild",
    "UnitNamePlayerPVPTitle",
];

/// **The mode CVar, which is a summary of the five and is not read by 5875.**
///
/// `UnitNameRenderMode` (default `"2"`) carries the clearest
/// statement of intent in the whole client in its own help string:
///
/// ```text
/// sets unitname mode (0=none,1=lockedunits/lockedplayers,
///                     2=playersalways+lockedunits, 3=all units always
/// ```
///
/// Mode 2 — *players always, plus the unit you have locked* — is exactly what
/// the five bits' defaults come to, and exactly what this client draws. It is
/// named here rather than implemented because **nothing in 5875 reads it**:
/// its value is written once and never looked at again, and the naming rule
/// consults the five bits instead. Wiring the options panel should move the
/// five and leave this alone.
pub const MODE_CVAR: &str = "UnitNameRenderMode";

impl Policy {
    /// The mask the reference keeps, for a check against it.
    pub fn mask(&self) -> u32 {
        let mut mask = 0;
        for (on, bit) in [
            (self.own, switches::OWN),
            (self.npc, switches::NPC),
            (self.player, switches::PLAYER),
            (self.guild, switches::GUILD),
            (self.pvp_title, switches::PVP_TITLE),
        ] {
            if on {
                mask |= bit;
            }
        }
        mask
    }

    /// **The five, read off the settings store.** `get` answers the string a
    /// `Config.wtf` carried or the client's own registration, and the
    /// interface's own test is `== "1"` — so anything but `"0"` is on, which is
    /// `CVars::flag`'s rule and the one used here.
    pub fn from_cvars(flag: impl Fn(&str) -> bool) -> Policy {
        Policy {
            own: flag(CVARS[0]),
            npc: flag(CVARS[1]),
            player: flag(CVARS[2]),
            guild: flag(CVARS[3]),
            pvp_title: flag(CVARS[4]),
        }
    }

    /// …and back, for a CVar file that has been read.
    pub fn from_mask(mask: u32) -> Policy {
        Policy {
            own: mask & switches::OWN != 0,
            npc: mask & switches::NPC != 0,
            player: mask & switches::PLAYER != 0,
            guild: mask & switches::GUILD != 0,
            pvp_title: mask & switches::PVP_TITLE != 0,
        }
    }

    /// Does this unit get a name? In the reference's own order.
    pub fn names(&self, unit: &Candidate) -> bool {
        // **Before anything the reference does**, because the reference never
        // has to ask — see [`Candidate::unit`].
        if !unit.unit || !unit.nameable {
            return false;
        }
        if unit.is_self {
            return self.own;
        }
        // **The target, unconditionally** — the reference returns without touching
        // the mask, which is what makes a default of "players only" workable.
        if unit.targeted {
            return true;
        }
        if unit.player {
            self.player
        } else {
            self.npc
        }
    }
}

/// Everything [`Policy::names`] asks about a unit.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    /// **Is this a unit at all?**
    ///
    /// The reference's rule is reached only from the unit
    /// list, so the reference never asks the question — a chest, a door, a
    /// mailbox or a signpost is not in the population. This client's world holds
    /// game objects in the same table as units, so the test has to be made
    /// somewhere, and it is made here rather than at the call site because it is
    /// a *rule* about who gets a name and not a detail of one renderer.
    ///
    /// **A game object's name is its tooltip**, which is a different subject
    /// with its own dig: `GAMEOBJECT_TYPE_GENERIC`'s `floatingTooltip` and
    /// [`super::object::hover_of`], drawn by the interface. Naming one here
    /// would put the same words in two places and would do it under a CVar
    /// called `UnitNameNPC`.
    pub unit: bool,
    /// The reference's first check and the two clauses under it, folded into
    /// one: a unit that may be named at all. This client answers it as "alive,
    /// in the world and carrying a name" — the reference's own version reaches
    /// a virtual and two fields this client does not read.
    pub nameable: bool,
    pub is_self: bool,
    pub player: bool,
    /// The unit the local player has selected.
    pub targeted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(is_self: bool, player: bool, targeted: bool) -> Candidate {
        Candidate { unit: true, nameable: true, is_self, player, targeted }
    }

    /// **The default is players and your target and nothing else**, and it is
    /// 1.12's own rather than this client's choice — the five CVars ship
    /// `UnitNamePlayer = "1"` with the other two off.
    #[test]
    fn the_default_names_players_and_the_target_and_nobody_else() {
        let policy = Policy::default();
        assert!(policy.names(&unit(false, true, false)), "another player");
        assert!(!policy.names(&unit(false, false, false)), "a wolf");
        assert!(policy.names(&unit(false, false, true)), "the wolf you clicked");
        assert!(!policy.names(&unit(true, true, false)), "yourself");
    }

    /// **The target clause outranks the mask and does not outrank `own`.** Both
    /// halves are the reference's order: the self test comes first and the
    /// target test second, so targeting
    /// yourself still obeys `UnitNameOwn`.
    #[test]
    fn the_target_is_named_whatever_is_switched_off_but_you_are_not() {
        let nothing = Policy { own: false, npc: false, player: false, guild: false, pvp_title: false };
        assert!(nothing.names(&unit(false, false, true)));
        assert!(nothing.names(&unit(false, true, true)));
        assert!(!nothing.names(&unit(true, true, true)), "targeting yourself is still `own`");

        let own = Policy { own: true, ..nothing };
        assert!(own.names(&unit(true, true, false)));
    }

    /// A unit that cannot be named at all is refused before anything else, and
    /// that includes the target.
    #[test]
    fn a_unit_that_cannot_be_named_is_refused_even_as_the_target() {
        let policy = Policy::default();
        let mut hidden = unit(false, true, true);
        hidden.nameable = false;
        assert!(!policy.names(&hidden));
    }

    /// **A chest is not an NPC**, and `UnitNameNPC` must not name one — which is
    /// exactly what it did, because this client keeps game objects in the same
    /// table as units and the reference never has to tell them apart. A door's
    /// name is its tooltip; see [`Candidate::unit`].
    #[test]
    fn a_game_object_is_never_named_however_the_switches_stand() {
        let all = Policy { own: true, npc: true, player: true, guild: true, pvp_title: true };
        let mut chest = unit(false, false, false);
        chest.unit = false;
        assert!(!all.names(&chest));
        // …not even the one you have clicked on, which is the clause that
        // outranks every switch for a real unit.
        chest.targeted = true;
        assert!(!all.names(&chest));
        assert!(!Policy::default().names(&chest));
    }

    /// The mask round-trips, which is what an options panel and a `Config.wtf`
    /// both want — and it is the one place a transposed bit would be silent.
    #[test]
    fn the_mask_round_trips_and_matches_the_references_bits() {
        let policy = Policy::default();
        assert_eq!(policy.mask(), switches::PLAYER | switches::GUILD | switches::PVP_TITLE);
        assert_eq!(Policy::from_mask(policy.mask()), policy);
        let all = Policy { own: true, npc: true, player: true, guild: true, pvp_title: true };
        assert_eq!(all.mask(), 0x37);
        assert_eq!(Policy::from_mask(0x37), all);
    }

    /// **A tall creature's name is physically bigger**, on a step at four yards
    /// — which is a rule nothing about the picture would suggest, and the reason
    /// this function exists rather than a constant.
    #[test]
    fn a_creature_over_four_yards_tall_gets_a_larger_name() {
        assert_eq!(size_for(2.2), BASE_SIZE, "a human");
        assert_eq!(size_for(4.0), BASE_SIZE, "exactly four is not tall");
        let kodo = size_for(8.0);
        assert!((kodo - BASE_SIZE * 2.0 * 1.5).abs() < 1e-6, "{kodo}");
        assert!(kodo > BASE_SIZE);
    }

    /// **The policy comes off the settings store under 5875's own names**, so a
    /// build dropped into a real WoW folder reads that folder's `Config.wtf`.
    /// The spellings are what the file carries and what the interface asks by.
    #[test]
    fn the_policy_is_the_five_cvars_under_the_names_the_client_registered() {
        // What the client's own registrations come to, which is `Default`.
        let registered = |name: &str| matches!(name, "UnitNamePlayer" | "UnitNamePlayerGuild" | "UnitNamePlayerPVPTitle");
        assert_eq!(Policy::from_cvars(registered), Policy::default());
        // …and a file that turned NPC names on moves exactly one field.
        let with_npcs = |name: &str| registered(name) || name == "UnitNameNPC";
        assert_eq!(Policy::from_cvars(with_npcs), Policy { npc: true, ..Policy::default() });
        assert_eq!(Policy::from_cvars(|_| false), Policy::from_mask(0));
    }
}
