//! **Friend or foe**, out of `FactionTemplate.dbc`.
//!
//! Nothing in the protocol says whether a unit may be attacked. What the server
//! sends is `UNIT_FIELD_FACTIONTEMPLATE`, a number, and the whole of what that
//! number means is in a 314-row table in the archives — so the question "can I
//! attack this?" is a game-data question, which is why it is answered here and
//! not in the renderer.
//!
//! Three things read the answer, and getting it wrong is visible in all three:
//! Tab-targeting, which must skip the innkeeper; the selection frame, which
//! colours a name red or green; and a cast's target binding, where a spell that
//! wants an enemy must not bind a friend (see [`crate::tables::spellbook`]).
//!
//! ## The rule
//!
//! `FactionTemplateEntry` is fourteen fields and the predicate is six lines
//! (vmangos `DBCStructure.h`, `IsHostileTo`/`IsFriendlyTo`):
//!
//! ```text
//! [0] id  [1] faction  [2] flags
//! [3] ourMask  [4] friendlyMask  [5] hostileMask     the four faction *groups*
//! [6..10] enemyFaction[4]   [10..14] friendFaction[4]  named exceptions
//! ```
//!
//! **The named exceptions win over the masks, and they are checked first.** A
//! faction is hostile if the other side's `faction` is in our `enemyFaction`
//! list, friendly if it is in our `friendFaction` list, and only failing both
//! does the group arithmetic decide — which is what lets one Alliance-group
//! creature hate a specific Alliance faction without hating the group.
//!
//! ## The rule again, as the client actually writes it
//!
//! The six lines above are the **last** thing the client's reaction check
//! tries. Everything before them is state
//! no table carries, and this client answered none of it for a long time —
//! which is the whole of "same-faction guards stay green while the server has
//! them hostile". As the two script functions (`UnitReaction`,
//! `UnitCanAttack`) see it, the order is:
//!
//! ```text
//! reaction  the same unit                              -> Friendly
//!           both player-controlled:
//!             same duel arbiter, both on a team        -> same team ? Friendly : Hostile
//!             one of them is us and the other is in    -> Friendly
//!               our party or raid
//!             both PLAYER_FLAGS_FFA_PVP                -> Hostile
//!           we are the one asking:
//!             a forced reaction on their faction       -> that rank
//!             a faction we have a standing with        -> at war ? Hostile : Friendly
//!           otherwise the template leg, clamped to Honored
//! template  they are asking about *us*:
//!             their template's flag 0x1000 and we are  -> Hostile
//!               PLAYER_FLAGS_CONTESTED_PVP
//!             a forced reaction on their faction       -> that rank
//!             a faction we have a standing with        -> our rank with it
//!           otherwise the masks
//! ```
//!
//! So **three inputs decide most of what a player sees and none of them is in a
//! DBC**: the forced reactions `SMSG_SET_FORCED_REACTIONS` carries, the at-war
//! bit on each of the sixty-four reputation slots, and `PLAYER_FLAGS`. A client
//! that reads only `FactionTemplate.dbc` draws a Stormwind guard green whatever
//! the server has done to the character standing in front of it — and the
//! server, which is answering a different question, damages it anyway. That
//! disagreement is what [`Standing`] and [`Party`] exist to close.
//!
//! ## What is not modelled, and what that costs
//!
//! **The friendliness bump.** After the masks answer, a rank
//! strictly between Hostile and Friendly is raised by one if the asking unit's
//! own faction is one we have a standing with **and** either its
//! `UNIT_FIELD_PERSUADED` names us or its `UNIT_FIELD_FLAGS` carries bit 14.
//! Both legs are dead in 1.12: `PERSUADED` is never written by vmangos and
//! vmangos' own name for bit 14 is `UNIT_FLAG_UNK_14`, "never seen in sniffs".
//! It is left out rather than guessed at, and it can only ever move Unfriendly
//! to Neutral or Neutral to Friendly.

use crate::tables::dbc::Dbc;
use crate::AssetError;
use std::collections::HashMap;

/// **The client's own eight-rank scale**, zero-based, exactly as the client's
/// reaction check returns it — `UnitReaction` is this plus one, which is why the Lua value
/// runs 1..8 and indexes `UnitReactionColor`'s eight rows.
///
/// The same scale a reputation bar is drawn on, and that is not a coincidence:
/// where the character has a standing with the faction, the standing **is** the
/// reaction. See [`Standing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Rank {
    Hated = 0,
    Hostile = 1,
    Unfriendly = 2,
    /// **The default**, and what a template neither side has resolves to — see
    /// [`Factions::rank`].
    #[default]
    Neutral = 3,
    Friendly = 4,
    Honored = 5,
    Revered = 6,
    Exalted = 7,
}

impl Rank {
    /// The reference's own clamp before the friendliness bump — nothing the
    /// second half of the cascade
    /// answers is ever better than Revered, so an Exalted faction's creature
    /// reads one rank below the bar the panel draws.
    pub const CEILING: Rank = Rank::Revered;

    /// `UnitReaction`'s value — the rank plus one.
    pub fn lua(self) -> u8 {
        self as u8 + 1
    }

    /// Build one from a reputation rank or a forced reaction, which both arrive
    /// as a number. Anything outside 0..8 is [`Rank::Neutral`], which is the
    /// answer for "nothing said".
    pub fn from_index(index: u32) -> Rank {
        match index {
            0 => Rank::Hated,
            1 => Rank::Hostile,
            2 => Rank::Unfriendly,
            4 => Rank::Friendly,
            5 => Rank::Honored,
            6 => Rank::Revered,
            7 => Rank::Exalted,
            _ => Rank::Neutral,
        }
    }
}

/// How a unit stands towards another, folded to the three readings this client
/// paints and branches on.
///
/// **A fold of [`Rank`] rather than a scale of its own**, and the fold is
/// `UnitReactionColor`'s: rows 1 and 2 are red, 3 orange, 4 yellow and 5..8
/// green, so Hated and Hostile are [`Reaction::Hostile`], Unfriendly and
/// Neutral are [`Reaction::Neutral`] and everything above is
/// [`Reaction::Friendly`]. What loses information is only the orange, and the
/// one caller that wants it — `UnitReaction` — reads the rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reaction {
    Hostile,
    /// **The default, and it is the same one [`crate::tables::dbc::DisplayTables`]
    /// gives when the table is absent**: standing with nobody. Every other
    /// value is a statement about two factions and has to be read out of the
    /// file to be made.
    #[default]
    Neutral,
    Friendly,
}

impl From<Rank> for Reaction {
    fn from(rank: Rank) -> Reaction {
        match rank {
            Rank::Hated | Rank::Hostile => Reaction::Hostile,
            Rank::Unfriendly | Rank::Neutral => Reaction::Neutral,
            _ => Reaction::Friendly,
        }
    }
}

impl Reaction {
    /// **Attackable means "not friendly"**, which is the surprising half.
    ///
    /// A neutral creature — every critter, every unaligned beast in the world —
    /// can be attacked, and the real client's own test is a single comparison
    /// on the reaction rank rather than a check for hostility (`UnitReaction <
    /// 4`, byte-confirmed). A client that requires hostility cannot attack a
    /// rabbit.
    pub fn is_attackable(self) -> bool {
        !matches!(self, Reaction::Friendly)
    }

    /// The `GlobalStrings.lua` key for a name plate's colour is not a string but
    /// a decision, so this is the nearest thing: which of the three the caller
    /// should paint. Kept as a method so the mapping lives beside the enum.
    pub fn tint(self) -> [f32; 3] {
        match self {
            // The client's own three: `UnitReactionColor` in FrameXML is red
            // 1.0/0.0/0.0 for hostile, yellow 1.0/1.0/0.0 for neutral and green
            // 0.0/1.0/0.0 for friendly.
            Reaction::Hostile => [1.0, 0.0, 0.0],
            Reaction::Neutral => [1.0, 1.0, 0.0],
            Reaction::Friendly => [0.0, 1.0, 0.0],
        }
    }
}

/// **One side of the question**, as the client reads it off a unit.
///
/// Seven fields and not a `WorldEntity`, for the reason every rule in this
/// crate is written this way: the decision must be checkable with no renderer
/// and no session running, and `vale login` builds one of these out of the
/// object manager exactly as the renderer does.
///
/// **`guid` is here only so that "the same unit" can be asked** — the very
/// first line of the cascade — and `0` means "not a unit anybody named", which
/// never matches anything, including another `0`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Party {
    pub guid: u64,
    /// `UNIT_FIELD_FACTIONTEMPLATE`, or `None` for an entity whose fields have
    /// not arrived.
    pub faction: Option<u32>,
    /// `UNIT_FIELD_FLAGS` — read for [`unit_flags::PLAYER_CONTROLLED`] here and
    /// for four more bits in [`can_attack`].
    pub unit_flags: u32,
    /// `PLAYER_FLAGS`, and **zero for anything that is not a player**, which is
    /// what makes the free-for-all and contested legs answer no for a creature
    /// without a test of their own.
    pub player_flags: u32,
    /// `PLAYER_DUEL_ARBITER` — the flag object both duellists point at.
    pub duel_arbiter: u64,
    /// `PLAYER_DUEL_TEAM`, `0` when there is no duel.
    pub duel_team: u32,
    /// **This is the character at the keyboard.** Two legs of the cascade are
    /// behind it, because they read state the server only ever sends about us.
    pub is_local: bool,
    /// …and this one is in that character's party or raid.
    pub in_local_group: bool,
}

impl Party {
    /// `UNIT_FLAG_PLAYER_CONTROLLED` — a player, a pet, a totem or a charm.
    ///
    /// **The flag rather than the object type**, which is the difference that
    /// matters for a hunter's pet: the wire says a pet is player-controlled and
    /// the object type says it is a creature, and every PvP leg of the cascade
    /// is written against the flag.
    pub fn player_controlled(self) -> bool {
        self.unit_flags & unit_flags::PLAYER_CONTROLLED != 0
    }

    fn same_unit_as(&self, other: &Party) -> bool {
        self.guid != 0 && self.guid == other.guid
    }
}

/// One faction's standing, as the character's own reputation list holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactionState {
    pub rank: Rank,
    pub at_war: bool,
}

/// **What the local character's own reputation says**, which is in no file and
/// arrives on the wire.
///
/// Two lists rather than two maps: the reference walks its forced reactions
/// linearly and there are never more than a handful, and the
/// standings are the sixty-four reputation-list slots. Borrowed rather than
/// owned so that the caller's copy is the only one.
///
/// **An empty `Standing` is exactly the old behaviour** — no forced reaction
/// and no faction with a bar — so every caller that has none to offer keeps
/// answering off `FactionTemplate.dbc` alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct Standing<'a> {
    /// `SMSG_SET_FORCED_REACTIONS`: a faction id and the rank the server insists
    /// on for it, whatever the table and the standing say. This is the channel a
    /// `SPELL_AURA_FORCE_REACTION` reaches the client through, and the only one
    /// by which a server can turn a friendly faction hostile without touching
    /// either unit's fields.
    pub forced: &'a [(u32, Rank)],
    /// …and the reputation list, faction id first. A faction absent from it is
    /// one the character has no bar for, which is the same test the client
    /// makes against `Faction.dbc`'s `reputationListID`.
    pub standings: &'a [(u32, FactionState)],
}

impl Standing<'_> {
    fn forced(&self, faction: u32) -> Option<Rank> {
        self.forced
            .iter()
            .find(|(id, _)| *id == faction)
            .map(|(_, rank)| *rank)
    }

    fn of(&self, faction: u32) -> Option<FactionState> {
        self.standings
            .iter()
            .find(|(id, _)| *id == faction)
            .map(|(_, state)| *state)
    }
}

/// Is `faction` one of the four a template names, ignoring the zero padding?
///
/// The client's own walk: it stops at the first zero entry rather than reading
/// all four, so a row with a hole in the middle of its list is read short. The
/// zero test here is on the *needle*, which reaches the same answer for every
/// row in the file and does not depend on the ordering.
fn names(list: &[u32; 4], faction: u32) -> bool {
    faction != 0 && list.contains(&faction)
}

/// One row of `FactionTemplate.dbc`, as the predicate needs it.
#[derive(Debug, Clone, Copy, Default)]
struct Template {
    faction: u32,
    /// Field 2. **One bit of it is read** — [`CONTESTED_GUARD`] — and it is the
    /// whole of why a city guard goes for somebody who has been fighting.
    flags: u32,
    our_mask: u32,
    friendly_mask: u32,
    hostile_mask: u32,
    enemies: [u32; 4],
    friends: [u32; 4],
}

mod template_fields {
    pub const FACTION: usize = 1;
    pub const FLAGS: usize = 2;
    pub const OUR_MASK: usize = 3;
    pub const FRIENDLY_MASK: usize = 4;
    pub const HOSTILE_MASK: usize = 5;
    pub const ENEMY_FACTION: usize = 6;
    pub const FRIEND_FACTION: usize = 10;
    pub const LIST_LEN: usize = 4;
}

/// `FactionTemplate.dbc` field 2, bit 12 — **this faction's units go for a
/// player who has been in a fight**, tested against
/// [`player_flags::CONTESTED_PVP`].
///
/// It is what "a guard" *is* to the client: the 44 rows that carry it are the
/// city and road guards of both sides, and the branch is the only place a
/// creature's reaction to the local player depends on a `PLAYER_FLAGS` bit.
pub const CONTESTED_GUARD: u32 = 1 << 12;

/// The `UNIT_FIELD_FLAGS` bits this module reads, by their vmangos names.
pub mod unit_flags {
    /// `UNIT_FLAG_PLAYER_CONTROLLED` — a player, a pet, a totem, a charm.
    /// Every branch of the reaction check above the masks is behind it.
    pub const PLAYER_CONTROLLED: u32 = 0x0000_0008;
    /// `UNIT_FLAG_IMMUNE_TO_PLAYER`.
    pub const IMMUNE_TO_PLAYER: u32 = 0x0000_0100;
    /// `UNIT_FLAG_IMMUNE_TO_NPC`.
    pub const IMMUNE_TO_NPC: u32 = 0x0000_0200;
    /// `UNIT_FLAG_PVP` — open to the other side.
    pub const PVP: u32 = 0x0000_1000;
}

/// …and the `PLAYER_FLAGS` bits, likewise.
pub mod player_flags {
    /// `PLAYER_FLAGS_GHOST`.
    pub const GHOST: u32 = 0x0000_0010;
    /// `PLAYER_FLAGS_FFA_PVP` — everybody is fair game and is fair game to
    /// you. Two flagged players are **Hostile to each
    /// other whatever their factions say**, which is the leg a same-faction
    /// duel-free free-for-all rides on.
    pub const FFA_PVP: u32 = 0x0000_0080;
    /// `PLAYER_FLAGS_CONTESTED_PVP` — has swung at the other side recently, and
    /// is therefore fair game to a [`super::CONTESTED_GUARD`] faction.
    pub const CONTESTED_PVP: u32 = 0x0000_0100;
}

/// `FactionGroup.dbc`'s four rows — **which side a group mask names**, in words.
///
/// Four records and twelve fields: `id`, `MaskID`, `internalName`, then the
/// eight localised names and their flag word. What is kept is the mask bit and
/// the two strings, because that is the whole of what `UnitFactionGroup`
/// returns.
///
/// ```text
/// id  MaskID  internalName  name[enUS]
///  1     0      Player        (empty)
///  2     1      Alliance      Alliance
///  3     2      Horde         Horde
///  4     3      Monster       (empty)
/// ```
mod group_fields {
    pub const MASK_ID: usize = 1;
    pub const INTERNAL_NAME: usize = 2;
    pub const NAME: usize = 3;
}

/// One `FactionGroup.dbc` row.
#[derive(Debug, Clone, Default)]
struct Group {
    /// `1 << MaskID` — already shifted, because every comparison wants the bit.
    bit: u32,
    /// `internalName` — `"Alliance"`, and the string the interface *builds a
    /// texture path out of*: `Interface\GroupFrame\UI-Group-PVP-<group>`.
    internal: String,
    /// …and the localised one, which is `UnitFactionGroup`'s second return.
    ///
    /// **Empty for Player and Monster, and that is load-bearing rather than
    /// missing data** — see [`Factions::group_name`].
    localised: String,
}

/// `FactionTemplate.dbc`, indexed by template id, **and `FactionGroup.dbc`
/// beside it** — the second is four rows and answers a different question about
/// the same column.
#[derive(Debug, Clone, Default)]
pub struct Factions(HashMap<u32, Template>, Vec<Group>);

impl Factions {
    pub fn parse(raw: &[u8]) -> Result<Factions, AssetError> {
        let dbc = Dbc::parse(raw)?;
        let mut rows = HashMap::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let field = |f: usize| dbc.u32_at(record, f).unwrap_or(0);
            let list = |first: usize| {
                let mut out = [0u32; template_fields::LIST_LEN];
                for (i, slot) in out.iter_mut().enumerate() {
                    *slot = field(first + i);
                }
                out
            };
            let Some(id) = dbc.u32_at(record, 0) else {
                continue;
            };
            rows.insert(
                id,
                Template {
                    faction: field(template_fields::FACTION),
                    flags: field(template_fields::FLAGS),
                    our_mask: field(template_fields::OUR_MASK),
                    friendly_mask: field(template_fields::FRIENDLY_MASK),
                    hostile_mask: field(template_fields::HOSTILE_MASK),
                    enemies: list(template_fields::ENEMY_FACTION),
                    friends: list(template_fields::FRIEND_FACTION),
                },
            );
        }
        Ok(Factions(rows, Vec::new()))
    }

    /// **Attach `FactionGroup.dbc`**, which is the second half of
    /// [`Factions::group_name`] and useless on its own.
    ///
    /// Separate from [`Factions::parse`] because the two files are separate and
    /// this one is optional: without it the group mask still answers (the taxi
    /// filter and the character-creation join are unaffected) and only the
    /// *word* is missing, which is a party frame with no PvP icon rather than a
    /// broken table.
    pub fn with_groups(mut self, raw: &[u8]) -> Factions {
        let Ok(dbc) = Dbc::parse(raw) else {
            return self;
        };
        let mut rows: Vec<(u32, Group)> = Vec::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(record, 0) else {
                continue;
            };
            let mask = dbc.u32_at(record, group_fields::MASK_ID).unwrap_or(0);
            rows.push((
                id,
                Group {
                    // **`1 << MaskID`, not the id.** `FactionTemplate`'s four
                    // mask columns are bitfields over `MaskID`, so Alliance is
                    // bit 1 and an Alliance *player*'s mask is 3 — Player and
                    // Alliance together, which is why the walk below cannot
                    // simply take the first bit set.
                    bit: 1u32 << mask,
                    internal: dbc
                        .string_at(record, group_fields::INTERNAL_NAME)
                        .unwrap_or_default(),
                    localised: dbc.string_at(record, group_fields::NAME).unwrap_or_default(),
                },
            ));
        }
        // **In id order**, which is the order the client walks the table in and
        // the order the empty-name rule below depends on.
        rows.sort_by_key(|(id, _)| *id);
        self.1 = rows.into_iter().map(|(_, group)| group).collect();
        self
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// **The `factionGroup` mask** — which of Player, Alliance, Horde and
    /// Monster this template belongs to, as `1 << FactionGroup.MaskID`.
    ///
    /// The same column [`Template::our_mask`] is read from for friend-or-foe,
    /// exposed by itself for the one caller that wants the *side* rather than
    /// the relationship: the taxi map filters its nodes on it
    /// ([`crate::tables::taxi::Team::of_group`]), and so does the character-creation
    /// screen through [`crate::tables::charcreate`]'s own join.
    pub fn group(&self, template: u32) -> Option<u32> {
        Some(self.0.get(&template)?.our_mask)
    }

    /// **`UnitFactionGroup`** — `(internalName, localisedName)` for the side a
    /// faction template is on, or `None` for one that has no side in words.
    ///
    /// The client ends in a walk of `FactionGroup.dbc` in id order, taking the
    /// first row whose `1 << MaskID` is in the template's `factionGroup` mask
    /// **and whose localised name is not the empty string**. Both halves of
    /// that condition are needed and the second is the one that is easy to
    /// miss: an Alliance player's mask is `Player | Alliance` = 3, so a walk
    /// that stopped at the first matching bit would answer `"Player"` for every
    /// character in the game — and `PartyMemberFrame_UpdatePvPStatus` builds
    /// `Interface\GroupFrame\UI-Group-PVP-<group>` out of it, so the icon would
    /// be a missing texture rather than a wrong one.
    ///
    /// `None` for a creature (Monster's name is empty too), which is exactly
    /// what that function's `if ( factionGroup and UnitIsPVP(unit) )` guard is
    /// written for: there is no such icon for a wolf.
    pub fn group_name(&self, template: u32) -> Option<(&str, &str)> {
        let mask = self.group(template)?;
        self.1
            .iter()
            .find(|group| group.bit & mask != 0 && !group.localised.is_empty())
            .map(|group| (group.internal.as_str(), group.localised.as_str()))
    }

    /// How the unit on `template` stands towards the unit on `towards`.
    ///
    /// **Directional, and deliberately asked one way round.** The real client's
    /// player-versus-creature branch tests a single direction — what *we* think
    /// of them — so the caller passes its own template second. A template
    /// neither side has (an entity whose field has not arrived yet) reads
    /// [`Reaction::Neutral`], which errs towards attackable: the server refuses
    /// a swing at a friend and says why, where a client that refused it locally
    /// would leave the player pressing a button that does nothing.
    /// **The base reaction the two templates alone decide** — the
    /// six lines the module comment opens with, and the last thing the full
    /// cascade tries.
    ///
    /// Directional. `template` is the unit whose opinion is being asked and
    /// `towards` the unit it is about. A template neither side has reads
    /// [`Rank::Neutral`], which errs towards attackable: the
    /// server refuses a swing at a friend and says why, where a client that
    /// refused it locally would leave the player pressing a button that does
    /// nothing.
    ///
    /// **The masks are tried before the named lists**, which is the order
    /// the client walks and the opposite of what this function used to do. It
    /// only shows where a row states both — `hostileMask` covering a group and
    /// `friendFaction` naming one member of it — and there the reference says
    /// hostile.
    pub fn template_rank(&self, template: Option<u32>, towards: Option<u32>) -> Rank {
        let (Some(a), Some(b)) = (template, towards) else {
            return Rank::Neutral;
        };
        let (Some(us), Some(them)) = (self.0.get(&a), self.0.get(&b)) else {
            return Rank::Neutral;
        };
        // The group first, then the four named
        // enemies — a creature can hate one faction of a group it otherwise
        // likes, and this is where that is written.
        if us.hostile_mask & them.our_mask != 0 || names(&us.enemies, them.faction) {
            return Rank::Hostile;
        }
        // Our friendly mask, our friend list, then the
        // same pair the other way round. **Both directions**, which is not a
        // symmetry the hostile half has.
        if us.friendly_mask & them.our_mask != 0
            || names(&us.friends, them.faction)
            || them.friendly_mask & us.our_mask != 0
            || names(&them.friends, us.faction)
        {
            return Rank::Friendly;
        }
        Rank::Neutral
    }

    /// The same answer folded to the three readings this client paints — the
    /// entry point for every caller that has no character standing to offer
    /// (`vale login`, the spell-book's aiming rule, the CLI).
    pub fn reaction(&self, template: Option<u32>, towards: Option<u32>) -> Reaction {
        self.template_rank(template, towards).into()
    }

    /// **The whole cascade the client's reaction check walks** — how
    /// `a` stands towards `b`, with the character's own state folded in.
    ///
    /// This is the function `UnitReaction`, `UnitIsFriend`, `UnitIsEnemy` and
    /// the name-plate tint are all decided by, and the module comment has the
    /// cascade written out. [`Factions::template_rank`] is only its last line.
    ///
    /// `standing` is the *local character's* reputation — forced reactions and
    /// the at-war bit — and it is consulted on exactly two legs, both of which
    /// require one of the two units to be that character. Nothing here asks it
    /// about anybody else, because nothing on the wire says what anybody else's
    /// standings are.
    pub fn rank(&self, a: &Party, b: &Party, standing: &Standing) -> Rank {
        // The same unit is its own friend, before anything is read.
        if a.same_unit_as(b) {
            return Rank::Friendly;
        }
        if a.player_controlled() && b.player_controlled() {
            // A duel settles it for as long as one is running, and
            // it is the only leg that can make two members of one faction
            // hostile without either of them being flagged.
            if a.duel_team != 0 && b.duel_team != 0 && a.duel_arbiter == b.duel_arbiter {
                return if a.duel_team == b.duel_team {
                    Rank::Friendly
                } else {
                    Rank::Hostile
                };
            }
            // Whichever of the two is us, the other
            // being in our party or raid is friendly — tested both ways round
            // in the client and both ways round here.
            if (a.is_local && b.in_local_group) || (b.is_local && a.in_local_group) {
                return Rank::Friendly;
            }
            // Both free-for-all.
            if a.player_flags & player_flags::FFA_PVP != 0
                && b.player_flags & player_flags::FFA_PVP != 0
            {
                return Rank::Hostile;
            }
        }
        // **We** are the one asking, so our own standings decide.
        // Note that the branch needs `a` to be player-controlled as well as
        // ours, which is the same test the client makes and matters for a pet.
        if a.player_controlled() && a.is_local {
            if let Some(row) = b.faction.and_then(|id| self.0.get(&id)) {
                if let Some(forced) = standing.forced(row.faction) {
                    return forced;
                }
                if let Some(state) = standing.of(row.faction) {
                    // The *flag*, not the rank. A faction we have a
                    // bar for is friendly unless we have declared war on it,
                    // which is why the Bloodsail Buccaneers are red on a fresh
                    // character and Booty Bay is green.
                    return if state.at_war { Rank::Hostile } else { Rank::Friendly };
                }
            }
        }
        // Everything else, clamped at [`Rank::CEILING`]. The friendliness
        // bump above the clamp is not modelled — see the module comment.
        self.asked_of(a, b, standing).min(Rank::CEILING)
    }

    /// What `a`'s faction template makes of `b`, with the two legs
    /// that only apply when `b` is the local character.
    fn asked_of(&self, a: &Party, b: &Party, standing: &Standing) -> Rank {
        let (Some(us), Some(_them)) = (
            a.faction.and_then(|id| self.0.get(&id)),
            b.faction.and_then(|id| self.0.get(&id)),
        ) else {
            // Either row missing and the answer is Neutral, which
            // is also what an entity whose field has not arrived yet reads.
            return Rank::Neutral;
        };
        if b.player_controlled() && b.is_local {
            // A guard faction against a player who has been
            // fighting. This is the one that makes a city guard go red.
            if us.flags & CONTESTED_GUARD != 0 && b.player_flags & player_flags::CONTESTED_PVP != 0
            {
                return Rank::Hostile;
            }
            if let Some(forced) = standing.forced(us.faction) {
                return forced;
            }
            if let Some(state) = standing.of(us.faction) {
                // Here it *is* the rank — a creature of a faction
                // we are Honored with stands Honored towards us.
                return state.rank;
            }
        }
        self.template_rank(a.faction, b.faction)
    }
}

/// `UNIT_FIELD_FLAGS` bits that disqualify a unit from being attacked at all,
/// whatever its faction says.
///
/// The five the client's own `CanAttack` tests (names as in vmangos'
/// `UnitFlags`):
/// `NON_ATTACKABLE` (1), `NOT_ATTACKABLE_1` (7), `NON_ATTACKABLE_2` (16),
/// `TAXI_FLIGHT` (20) and `NOT_SELECTABLE` (25).
///
/// This is what keeps a quest giver, a flight master mid-flight and a spawning
/// creature out of the Tab-target pool — none of which their faction would
/// exclude, since most of them are ordinary friendly NPCs and some are not.
pub const UNATTACKABLE_FLAGS: u32 = (1 << 1) | (1 << 7) | (1 << 16) | (1 << 20) | (1 << 25);

/// Can this unit be attacked? The flags first, then the reaction.
///
/// Both halves are needed and neither implies the other: a critter is
/// attackable and neutral, a quest giver is friendly *and* flagged, and a
/// creature still spawning is hostile and flagged.
///
/// **The creature half of [`Factions::can_attack`]**, kept as a free function
/// because five callers have a reaction and a flags word and nothing else —
/// the spell-book's aiming rule and the CLI among them. Where the attacker is
/// the local character and the victim may be another player, use the method:
/// this one cannot see a duel, a free-for-all or a PvP flag, and answers "no"
/// for every one of them.
pub fn can_attack(unit_flags: u32, reaction: Reaction) -> bool {
    unit_flags & UNATTACKABLE_FLAGS == 0 && reaction.is_attackable()
}

impl Factions {
    /// **May `a` swing at `b`?** — the client's attack check, which is
    /// what `UnitCanAttack` ends in.
    ///
    /// Four gates and then one of three verdicts, and the third is the one this
    /// client had no answer for at all:
    ///
    /// ```text
    /// b is a ghost and a is …                   -> no
    /// b carries any of UNATTACKABLE_FLAGS       -> no
    /// either side immune to the other's kind    -> no
    /// a player against a creature: rank(a,b) < Friendly
    /// creature against creature: not both sides Unfriendly or better
    /// player against player: rank(a,b) < Friendly, and then one of
    ///   a duel, b's UNIT_FLAG_PVP, or both free-for-all
    /// ```
    ///
    /// **The ghost gate is not modelled.** The client asks a further predicate
    /// of the attacker that is not known, so refusing on the ghost bit alone
    /// would stop a spirit healer's own attacks and every case the predicate
    /// exists to let through. Left out rather than guessed at; the
    /// server refuses a swing at a corpse and says so.
    pub fn can_attack(&self, a: &Party, b: &Party, standing: &Standing) -> bool {
        // The five bits that disqualify a unit whatever anybody
        // thinks of it.
        if b.unit_flags & UNATTACKABLE_FLAGS != 0 {
            return false;
        }
        // Immunity, both ways round and both kinds.
        let (a_player, b_player) = (a.player_controlled(), b.player_controlled());
        if a_player && b.unit_flags & unit_flags::IMMUNE_TO_PLAYER != 0 {
            return false;
        }
        if !a_player && b.unit_flags & unit_flags::IMMUNE_TO_NPC != 0 {
            return false;
        }
        if a.unit_flags & unit_flags::IMMUNE_TO_PLAYER != 0 && b_player {
            return false;
        }
        if a.unit_flags & unit_flags::IMMUNE_TO_NPC != 0 && !b_player {
            return false;
        }
        match (a_player, b_player) {
            // Two creatures. **Both** directions have to be better
            // than Hostile for the swing to be refused, so one-sided hatred is
            // enough — which is what lets a guard go for something that is
            // merely neutral towards it.
            (false, false) => {
                self.rank(a, b, standing) <= Rank::Hostile
                    || self.rank(b, a, standing) <= Rank::Hostile
            }
            // A player, or a pet, against a creature. Neutral is
            // attackable — a client that required hostility could not attack a
            // rabbit — and Friendly is not.
            (_, false) => self.rank(a, b, standing) < Rank::Friendly,
            // Against another player, and Friendly still refuses.
            (_, true) => {
                if self.rank(a, b, standing) >= Rank::Friendly {
                    return false;
                }
                // A duel in progress, whichever side either is on.
                if a.duel_team != 0 && b.duel_team != 0 && a.duel_arbiter == b.duel_arbiter {
                    return true;
                }
                // They are flagged, which is the ordinary way one
                // player becomes attackable to another.
                if b.unit_flags & unit_flags::PVP != 0 {
                    return true;
                }
                // …or both of us are free-for-all, which needs no
                // flag and no faction.
                a.player_flags & player_flags::FFA_PVP != 0
                    && b.player_flags & player_flags::FFA_PVP != 0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// Build a two-row `FactionTemplate.dbc`: 1 is an Alliance-group unit and
    /// 2 a Horde-group one, with masks that make them hate each other.
    fn table() -> Factions {
        // Fourteen fields: id, faction, flags, ourMask, friendlyMask,
        // hostileMask, enemy[4], friend[4].
        let rows = vec![
            vec![1, 1, 0, 0b0010, 0b0010, 0b0100, 0, 0, 0, 0, 0, 0, 0, 0],
            vec![2, 2, 0, 0b0100, 0b0100, 0b0010, 0, 0, 0, 0, 0, 0, 0, 0],
            // 3 is in the Alliance group but names faction 1 as an enemy — the
            // exception that has to beat the mask.
            vec![3, 3, 0, 0b0010, 0b0010, 0, 1, 0, 0, 0, 0, 0, 0, 0],
            // 4 is in the Horde group but names faction 1 as a friend — the
            // row where the two orders disagree, and the reference says the
            // mask wins.
            vec![4, 4, 0, 0b0100, 0b0100, 0b0010, 0, 0, 0, 0, 1, 0, 0, 0],
            // 5 names faction 1 as a friend with no mask in the way, which is
            // the leg the friend list is actually for.
            vec![5, 5, 0, 0b1000, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0],
            // 6 is faction 1's own group with the contested-guard flag on it.
            vec![6, 1, CONTESTED_GUARD, 0b0010, 0b0010, 0b0100, 0, 0, 0, 0, 0, 0, 0, 0],
        ];
        Factions::parse(&dbc(&rows, 14, &[0])).expect("a faction table")
    }

    /// `FactionGroup.dbc` as 1.12 ships it: four rows, and **two of them have
    /// an empty localised name**. Twelve fields — id, MaskID, internalName,
    /// then eight locales and their flag word.
    fn groups() -> Vec<u8> {
        // The string block, offset 0 being the empty string as every DBC's is.
        let mut strings = vec![0u8];
        let at = |text: &str, block: &mut Vec<u8>| {
            let offset = block.len() as u32;
            block.extend_from_slice(text.as_bytes());
            block.push(0);
            offset
        };
        let player = at("Player", &mut strings);
        let alliance = at("Alliance", &mut strings);
        let horde = at("Horde", &mut strings);
        let monster = at("Monster", &mut strings);
        let rows = vec![
            // id, MaskID, internalName, name[enUS], …
            vec![1, 0, player, 0],
            vec![2, 1, alliance, alliance],
            vec![3, 2, horde, horde],
            vec![4, 3, monster, 0],
        ];
        dbc(&rows, 12, &strings)
    }

    /// **An Alliance player's group mask is `Player | Alliance`**, and the walk
    /// that answers `"Player"` for it is the one this rule exists to stop.
    ///
    /// The client takes the first `FactionGroup.dbc` row whose bit is in the
    /// mask **and whose localised name is non-empty** — and Player's is empty
    /// in the shipped file, as is Monster's. Drop the second half of that
    /// condition and every character in the game answers `"Player"`, which
    /// `PartyMemberFrame_UpdatePvPStatus` turns into
    /// `Interface\GroupFrame\UI-Group-PVP-Player`: a texture that does not
    /// exist. The archives carry only the Alliance and Horde pair.
    #[test]
    fn a_players_faction_group_skips_the_row_with_no_name() {
        // Templates 1 and 2 above are bare group bits; give this its own table
        // with the masks a real player carries.
        let rows = vec![
            // An Alliance player: Player | Alliance = 0b011.
            vec![1, 1, 0, 0b011, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            // …a Horde one: Player | Horde = 0b101.
            vec![2, 2, 0, 0b101, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            // …and a wolf: Monster alone.
            vec![3, 3, 0, 0b1000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ];
        let t = Factions::parse(&dbc(&rows, 14, &[0]))
            .expect("a faction table")
            .with_groups(&groups());

        assert_eq!(t.group_name(1), Some(("Alliance", "Alliance")));
        assert_eq!(t.group_name(2), Some(("Horde", "Horde")));
        // **A creature has no side in words**, which is what
        // `if ( factionGroup and UnitIsPVP(unit) )` is guarding: Monster's
        // localised name is empty too, so the walk finds nothing.
        assert_eq!(t.group_name(3), None);
        // …and a template the table has never heard of.
        assert_eq!(t.group_name(99), None);
    }

    /// **Without `FactionGroup.dbc` the mask still answers and the word does
    /// not**, which is the degradation `with_groups` is optional for: the taxi
    /// filter and the character-creation join go on working and a party frame
    /// simply shows no PvP icon.
    #[test]
    fn a_missing_group_table_costs_the_word_and_nothing_else() {
        let rows = vec![vec![1, 1, 0, 0b011, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]];
        let t = Factions::parse(&dbc(&rows, 14, &[0])).expect("a faction table");
        assert_eq!(t.group(1), Some(0b011), "the mask is off the template alone");
        assert_eq!(t.group_name(1), None);
        // …and a damaged one is the same as an absent one rather than a panic.
        let t = t.with_groups(b"not a dbc");
        assert_eq!(t.group_name(1), None);
    }

    #[test]
    fn the_group_masks_decide_the_ordinary_case() {
        let t = table();
        assert_eq!(t.reaction(Some(1), Some(2)), Reaction::Hostile);
        assert_eq!(t.reaction(Some(1), Some(1)), Reaction::Friendly);
        assert_eq!(t.len(), 6);
    }

    /// **The masks are tried before the named lists**, which is the client's
    /// own order and the opposite of what this module used to do.
    ///
    /// The two rows that make the difference visible are the ones that state
    /// both: 3 shares 1's group *and* names it as an enemy, which both orders
    /// call hostile; 4 is in the group 1's mask hates *and* names 1 as a
    /// friend, which the old order called friendly and the reference calls
    /// hostile. 5 names 1 as a friend with no mask in the way, which is the leg
    /// the list is actually for.
    #[test]
    fn the_hostile_mask_is_tried_before_the_named_lists() {
        let t = table();
        assert_eq!(t.reaction(Some(3), Some(1)), Reaction::Hostile);
        assert_eq!(t.reaction(Some(4), Some(1)), Reaction::Hostile);
        assert_eq!(t.reaction(Some(5), Some(1)), Reaction::Friendly);
    }

    /// A missing template reads neutral, which is attackable — the safe
    /// direction, because the server refuses a bad swing and says why.
    #[test]
    fn an_unknown_template_is_neutral_and_attackable() {
        let t = table();
        assert_eq!(t.reaction(None, Some(1)), Reaction::Neutral);
        assert_eq!(t.reaction(Some(99), Some(1)), Reaction::Neutral);
        assert!(t.reaction(None, None).is_attackable());
    }

    /// **Attackable is "not friendly", not "hostile".** A client that requires
    /// hostility cannot attack a rabbit.
    #[test]
    fn neutral_units_can_be_attacked_and_flags_can_still_forbid_it() {
        assert!(can_attack(0, Reaction::Neutral));
        assert!(can_attack(0, Reaction::Hostile));
        assert!(!can_attack(0, Reaction::Friendly));
        // A quest giver is friendly *and* flagged; a spawning creature is
        // hostile and flagged. Either half is enough to refuse.
        assert!(!can_attack(1 << 25, Reaction::Hostile));
        assert!(!can_attack(1 << 1, Reaction::Neutral));
    }

    /// **The second half of the cascade is clamped and the first is not.**
    /// A creature of a faction the character is Exalted with reads Revered,
    /// and the same standing reached through the *other* leg —
    /// where we are the one asking — is a flat Friendly, because that leg reads
    /// the at-war bit rather than the rank.
    #[test]
    fn a_creatures_answer_is_clamped_at_revered() {
        let t = table();
        let mut me = player(7, 1);
        me.is_local = true;
        let exalted = [(2u32, FactionState { rank: Rank::Exalted, at_war: false })];
        let standing = Standing { forced: &[], standings: &exalted };
        assert_eq!(t.rank(&creature(2), &me, &standing), Rank::Revered);
        assert_eq!(t.rank(&me, &creature(2), &standing), Rank::Friendly);
    }

    /// A creature on template `id`.
    fn creature(id: u32) -> Party {
        Party { guid: id as u64 + 1000, faction: Some(id), ..Party::default() }
    }

    /// …and a player, which is the flag rather than the object type.
    fn player(guid: u64, id: u32) -> Party {
        Party {
            guid,
            faction: Some(id),
            unit_flags: unit_flags::PLAYER_CONTROLLED,
            ..Party::default()
        }
    }

    /// Nothing forced and no faction with a bar — the state of every session
    /// before a reputation packet arrives, and the one under which the cascade
    /// has to agree with the bare table.
    fn nothing() -> Standing<'static> {
        Standing::default()
    }

    /// **With no character state at all the cascade is the table**, which is
    /// what makes every caller that has none to offer — the CLI, the aiming
    /// rule — keep the answer it had.
    #[test]
    fn an_empty_standing_leaves_the_table_deciding() {
        let t = table();
        let (a, b) = (creature(1), creature(2));
        assert_eq!(t.rank(&a, &b, &nothing()), Rank::Hostile);
        assert_eq!(t.rank(&a, &creature(1), &nothing()), Rank::Friendly);
        assert_eq!(t.rank(&a, &creature(99), &nothing()), Rank::Neutral);
    }

    /// `UnitReaction` is the rank plus one, and the eight rows of
    /// `UnitReactionColor` are what it indexes.
    #[test]
    fn the_lua_value_is_the_rank_plus_one() {
        assert_eq!(Rank::Hostile.lua(), 2);
        assert_eq!(Rank::Unfriendly.lua(), 3);
        assert_eq!(Rank::Neutral.lua(), 4);
        assert_eq!(Rank::Friendly.lua(), 5);
        assert_eq!(Reaction::from(Rank::Unfriendly), Reaction::Neutral);
        assert_eq!(Reaction::from(Rank::Honored), Reaction::Friendly);
    }

    /// **A forced reaction beats the table outright** — the channel a
    /// `SPELL_AURA_FORCE_REACTION` reaches the client through, and the one that
    /// can turn a friendly faction hostile with no field on any unit changed.
    ///
    /// Asked both ways round, because the two legs are separate code in the
    /// client: one when we are the one asking and one when the
    /// creature is.
    #[test]
    fn a_forced_reaction_beats_the_faction_table_both_ways() {
        let t = table();
        let mut me = player(7, 1);
        me.is_local = true;
        let guard = creature(1);
        // Same template, so the table says friendly on both sides.
        assert_eq!(t.rank(&me, &guard, &nothing()), Rank::Friendly);
        assert_eq!(t.rank(&guard, &me, &nothing()), Rank::Friendly);

        let forced = [(1u32, Rank::Hostile)];
        let standing = Standing { forced: &forced, standings: &[] };
        assert_eq!(t.rank(&me, &guard, &standing), Rank::Hostile);
        assert_eq!(t.rank(&guard, &me, &standing), Rank::Hostile);
        // …and it is a real attackability change, not only a colour.
        assert!(t.can_attack(&me, &guard, &standing));
        assert!(!t.can_attack(&me, &guard, &nothing()));
    }

    /// **A faction we have a bar for is decided by the at-war bit, not by the
    /// masks** — which is why the Bloodsail Buccaneers are red on
    /// a character who has never met them and Booty Bay is green.
    #[test]
    fn a_faction_with_a_standing_is_decided_by_the_at_war_bit() {
        let t = table();
        let mut me = player(7, 1);
        me.is_local = true;
        // Template 2's faction is 2, which the table calls hostile to us.
        let peaceful = [(2u32, FactionState { rank: Rank::Neutral, at_war: false })];
        assert_eq!(
            t.rank(&me, &creature(2), &Standing { forced: &[], standings: &peaceful }),
            Rank::Friendly
        );
        let warring = [(2u32, FactionState { rank: Rank::Neutral, at_war: true })];
        assert_eq!(
            t.rank(&me, &creature(2), &Standing { forced: &[], standings: &warring }),
            Rank::Hostile
        );
    }

    /// …and the other direction is the **rank**, not the flag: a
    /// creature of a faction we are Honored with stands Honored towards us.
    #[test]
    fn a_creature_of_a_faction_we_have_a_standing_with_answers_that_rank() {
        let t = table();
        let mut me = player(7, 1);
        me.is_local = true;
        let honored = [(2u32, FactionState { rank: Rank::Honored, at_war: false })];
        assert_eq!(
            t.rank(&creature(2), &me, &Standing { forced: &[], standings: &honored }),
            Rank::Honored
        );
    }

    /// **A guard faction goes for a player who has been fighting** — the one
    /// leg where a `PLAYER_FLAGS` bit decides a creature's reaction, and the
    /// mechanism behind "the guards turned red".
    #[test]
    fn a_contested_guard_is_hostile_to_a_contested_player() {
        // Row 6 is template 1's faction with the guard flag on it.
        let t = table();
        let mut me = player(7, 1);
        me.is_local = true;
        assert_eq!(t.rank(&creature(6), &me, &nothing()), Rank::Friendly);
        me.player_flags |= player_flags::CONTESTED_PVP;
        assert_eq!(t.rank(&creature(6), &me, &nothing()), Rank::Hostile);
        // …and the same player is left alone by a faction with no guard flag.
        assert_eq!(t.rank(&creature(1), &me, &nothing()), Rank::Friendly);
    }

    /// **Two free-for-all players are hostile whatever their factions say**,
    /// and it takes both of them.
    #[test]
    fn two_free_for_all_players_are_hostile_to_each_other() {
        let t = table();
        let (mut a, mut b) = (player(7, 1), player(8, 1));
        assert_eq!(t.rank(&a, &b, &nothing()), Rank::Friendly);
        a.player_flags |= player_flags::FFA_PVP;
        assert_eq!(t.rank(&a, &b, &nothing()), Rank::Friendly, "one of the two is not enough");
        b.player_flags |= player_flags::FFA_PVP;
        assert_eq!(t.rank(&a, &b, &nothing()), Rank::Hostile);
        assert!(t.can_attack(&a, &b, &nothing()));
    }

    /// A duel settles it before either faction is read, and the same-team half
    /// is what keeps a duelling partner's own pet friendly.
    #[test]
    fn a_duel_decides_before_the_factions_do() {
        let t = table();
        let (mut a, mut b) = (player(7, 1), player(8, 1));
        a.duel_arbiter = 900;
        b.duel_arbiter = 900;
        a.duel_team = 1;
        b.duel_team = 2;
        assert_eq!(t.rank(&a, &b, &nothing()), Rank::Hostile);
        assert!(t.can_attack(&a, &b, &nothing()));
        b.duel_team = 1;
        assert_eq!(t.rank(&a, &b, &nothing()), Rank::Friendly);
        // A different duel is no duel between these two.
        b.duel_arbiter = 901;
        b.duel_team = 2;
        assert_eq!(t.rank(&a, &b, &nothing()), Rank::Friendly);
    }

    /// **A group mate is friendly**, tested whichever of the two is us — which
    /// is how the client writes it and matters because the caller decides the
    /// direction.
    #[test]
    fn a_group_mate_is_friendly_from_either_end() {
        let t = table();
        // Opposite factions, so nothing but the group can make them friends.
        let (mut me, mut mate) = (player(7, 1), player(8, 2));
        assert_eq!(t.rank(&me, &mate, &nothing()), Rank::Hostile);
        me.is_local = true;
        mate.in_local_group = true;
        assert_eq!(t.rank(&me, &mate, &nothing()), Rank::Friendly);
        assert_eq!(t.rank(&mate, &me, &nothing()), Rank::Friendly);
    }

    /// **A flagged player of the other side may be attacked and an unflagged
    /// one may not** — which is the ordinary PvP rule and was
    /// answered by nothing here before.
    #[test]
    fn the_pvp_flag_is_what_opens_a_player_to_attack() {
        let t = table();
        let (me, mut them) = (player(7, 1), player(8, 2));
        assert_eq!(t.rank(&me, &them, &nothing()), Rank::Hostile);
        assert!(!t.can_attack(&me, &them, &nothing()), "unflagged and not in a duel");
        them.unit_flags |= unit_flags::PVP;
        assert!(t.can_attack(&me, &them, &nothing()));
    }

    /// The five bits and the two immunities, which are the gates in front of
    /// every verdict.
    #[test]
    fn the_flag_gates_refuse_before_any_faction_is_read() {
        let t = table();
        let (me, mut mob) = (player(7, 1), creature(2));
        assert!(t.can_attack(&me, &mob, &nothing()));
        mob.unit_flags |= 1 << 25;
        assert!(!t.can_attack(&me, &mob, &nothing()), "not selectable");
        mob.unit_flags = unit_flags::IMMUNE_TO_PLAYER;
        assert!(!t.can_attack(&me, &mob, &nothing()));
        // …and a creature is stopped by the other immunity rather than that one.
        assert!(t.can_attack(&creature(1), &mob, &nothing()));
        mob.unit_flags = unit_flags::IMMUNE_TO_NPC;
        assert!(!t.can_attack(&creature(1), &mob, &nothing()));
        assert!(t.can_attack(&me, &mob, &nothing()));
    }

    /// **A creature swings when either side hates the other**,
    /// which is not the same test as the player's.
    #[test]
    fn one_sided_hatred_is_enough_between_two_creatures() {
        let t = table();
        // 3 names 1 as an enemy; 1 has nothing to say about 3's faction and
        // shares its group, so 1 reads 3 as friendly.
        assert_eq!(t.rank(&creature(1), &creature(3), &nothing()), Rank::Friendly);
        assert_eq!(t.rank(&creature(3), &creature(1), &nothing()), Rank::Hostile);
        assert!(t.can_attack(&creature(1), &creature(3), &nothing()));
    }
}
