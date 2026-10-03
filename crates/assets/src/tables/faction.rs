//! Unit reactions (friend or foe), read from `FactionTemplate.dbc`.
//!
//! The protocol does not say whether a unit may be attacked. The server sends
//! `UNIT_FIELD_FACTIONTEMPLATE`, a number, and the meaning of that number is in
//! a 314-row table in the archives. Whether a unit can be attacked is therefore
//! a game-data question, and it is answered in this crate, not in the renderer.
//!
//! Three callers read the answer, and an error shows in each of them:
//! Tab-targeting, which must skip the innkeeper; the selection frame, which
//! colours a name red or green; and a cast's target binding, where a spell that
//! wants an enemy must not bind a friend (see [`crate::tables::spellbook`]).
//!
//! ## The template rule
//!
//! `FactionTemplateEntry` has fourteen fields and the predicate is six lines
//! (vmangos `DBCStructure.h`, `IsHostileTo`/`IsFriendlyTo`):
//!
//! ```text
//! [0] id  [1] faction  [2] flags
//! [3] ourMask  [4] friendlyMask  [5] hostileMask     the four faction *groups*
//! [6..10] enemyFaction[4]   [10..14] friendFaction[4]  named exceptions
//! ```
//!
//! In vmangos the named exceptions win over the masks and are checked first. A
//! faction is hostile if the other side's `faction` is in our `enemyFaction`
//! list, friendly if it is in our `friendFaction` list, and the group masks
//! decide only when neither list names it. This lets one Alliance-group
//! creature hate a specific Alliance faction without hating the group.
//! [`Factions::template_rank`] describes where the 1.12.1 client's order
//! differs.
//!
//! ## The full reaction order of the 1.12.1 client
//!
//! The 1.12.1 client applies the six lines above last. Every rule before them
//! reads state that no table carries. A client that skips those rules shows
//! same-faction guards green while the server treats them as hostile. As the
//! two script functions (`UnitReaction`, `UnitCanAttack`) report it, the order
//! is:
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
//!           otherwise the template leg, clamped to Revered
//! template  they are asking about *us*:
//!             their template's flag 0x1000 and we are  -> Hostile
//!               PLAYER_FLAGS_CONTESTED_PVP
//!             a forced reaction on their faction       -> that rank
//!             a faction we have a standing with        -> our rank with it
//!           otherwise the masks
//! ```
//!
//! Three inputs decide most of what a player sees, and none of them is in a
//! DBC: the forced reactions that `SMSG_SET_FORCED_REACTIONS` carries, the
//! at-war bit on each of the sixty-four reputation slots, and `PLAYER_FLAGS`. A
//! client that reads only `FactionTemplate.dbc` draws a Stormwind guard green
//! whatever the server has done to the character in front of it, and the
//! server, which applies its own rules, damages the character anyway.
//! [`Standing`] and [`Party`] carry those three inputs so that this module
//! gives the same answer as the server.
//!
//! ## What is not modelled
//!
//! The friendliness bump. After the masks answer, a rank strictly between
//! Hostile and Friendly is raised by one if the asking unit's own faction is
//! one we have a standing with, and either its `UNIT_FIELD_PERSUADED` names us
//! or its `UNIT_FIELD_FLAGS` carries bit 14. Neither condition occurs in 1.12:
//! vmangos never writes `PERSUADED`, and vmangos names bit 14
//! `UNIT_FLAG_UNK_14`, "never seen in sniffs". The bump is left out rather
//! than guessed at. Its only possible effects are Unfriendly to Neutral and
//! Neutral to Friendly.

use crate::tables::dbc::Dbc;
use crate::AssetError;
use std::collections::HashMap;

/// The 1.12.1 client's eight-rank reaction scale, zero-based. `UnitReaction`
/// returns this value plus one, so the Lua value runs 1..8 and indexes the
/// eight rows of `UnitReactionColor`.
///
/// A reputation bar is drawn on the same scale, because where the character
/// has a standing with the faction, the standing is the reaction. See
/// [`Standing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum Rank {
    Hated = 0,
    Hostile = 1,
    Unfriendly = 2,
    /// The default, and the result when neither side has a template. See
    /// [`Factions::rank`].
    #[default]
    Neutral = 3,
    Friendly = 4,
    Honored = 5,
    Revered = 6,
    Exalted = 7,
}

impl Rank {
    /// The highest rank the template leg of the reaction order can return,
    /// applied before the friendliness bump. The 1.12.1 client never answers
    /// better than Revered there, so a creature of an Exalted faction reads one
    /// rank below the bar the reputation panel draws.
    pub const CEILING: Rank = Rank::Revered;

    /// The value `UnitReaction` returns: the rank plus one.
    pub fn lua(self) -> u8 {
        self as u8 + 1
    }

    /// Builds a rank from a reputation rank or a forced reaction, which both
    /// arrive as a number. A value outside 0..8 gives [`Rank::Neutral`], the
    /// rank used when nothing is known.
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

/// How a unit stands towards another, reduced to the three values this client
/// draws and branches on.
///
/// `Reaction` is [`Rank`] grouped the way `UnitReactionColor` groups it: rows
/// 1 and 2 are red, 3 orange, 4 yellow and 5..8 green. Hated and Hostile are
/// [`Reaction::Hostile`], Unfriendly and Neutral are [`Reaction::Neutral`],
/// and everything above is [`Reaction::Friendly`]. The grouping loses only the
/// orange row, and the one caller that needs it, `UnitReaction`, reads the
/// rank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reaction {
    Hostile,
    /// The default, and the value [`crate::tables::dbc::DisplayTables`] gives
    /// when the table is absent: no standing with anybody. Every other value
    /// is a statement about two factions and comes from the file.
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
    /// Attackable means "not friendly", not "hostile".
    ///
    /// A neutral creature, such as a critter or an unaligned beast, can be
    /// attacked. The 1.12.1 client treats a unit as attackable when its
    /// zero-based reaction rank is below 4 ([`Rank::Friendly`]); it does not
    /// require hostility. A client that requires hostility cannot attack a
    /// rabbit.
    pub fn is_attackable(self) -> bool {
        !matches!(self, Reaction::Friendly)
    }

    /// The RGB colour for a name plate in this reaction. `GlobalStrings.lua`
    /// has no key for this colour, so the mapping is a method kept beside the
    /// enum.
    pub fn tint(self) -> [f32; 3] {
        match self {
            // `UnitReactionColor` in FrameXML: red 1.0/0.0/0.0 for hostile,
            // yellow 1.0/1.0/0.0 for neutral and green 0.0/1.0/0.0 for
            // friendly.
            Reaction::Hostile => [1.0, 0.0, 0.0],
            Reaction::Neutral => [1.0, 1.0, 0.0],
            Reaction::Friendly => [0.0, 1.0, 0.0],
        }
    }
}

/// One unit in a reaction query: the unit fields the reaction rules read.
///
/// A plain struct, not a `WorldEntity`, for the reason every rule in this
/// crate is written this way: the decision must be testable with no renderer
/// and no session running. `vale login` builds a `Party` from the object
/// manager the same way the renderer does.
///
/// `guid` exists only for the "same unit" test, the first rule of the reaction
/// order. `0` means "no named unit" and matches nothing, including another
/// `0`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Party {
    pub guid: u64,
    /// `UNIT_FIELD_FACTIONTEMPLATE`, or `None` for an entity whose fields have
    /// not arrived.
    pub faction: Option<u32>,
    /// `UNIT_FIELD_FLAGS`: read for [`unit_flags::PLAYER_CONTROLLED`] here and
    /// for four more bits in [`can_attack`].
    pub unit_flags: u32,
    /// `PLAYER_FLAGS`; zero for anything that is not a player. Because it is
    /// zero, the free-for-all and contested rules answer no for a creature
    /// without a separate test.
    pub player_flags: u32,
    /// `PLAYER_DUEL_ARBITER`: the flag object both duellists point at.
    pub duel_arbiter: u64,
    /// `PLAYER_DUEL_TEAM`, `0` when there is no duel.
    pub duel_team: u32,
    /// This unit is the character at the keyboard. Two rules of the reaction
    /// order require it, because they read state the server sends only about
    /// the local character.
    pub is_local: bool,
    /// This unit is in the local character's party or raid.
    pub in_local_group: bool,
}

impl Party {
    /// `UNIT_FLAG_PLAYER_CONTROLLED`: a player, a pet, a totem or a charm.
    ///
    /// The test uses the flag, not the object type. The two differ for a
    /// hunter's pet: the flag says it is player-controlled and the object type
    /// says it is a creature. Every PvP rule of the reaction order tests the
    /// flag.
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

/// The local character's reputation state, which is in no file and arrives
/// from the server.
///
/// Two lists rather than two maps: there are never more than a handful of
/// forced reactions and they are searched in order, and the standings are the
/// sixty-four reputation-list slots. Borrowed rather than owned, so that the
/// caller's copy is the only one.
///
/// An empty `Standing` has no forced reaction and no faction with a bar, so
/// [`Factions::rank`] answers from `FactionTemplate.dbc` alone. Callers with
/// no reputation state pass an empty one.
#[derive(Debug, Clone, Copy, Default)]
pub struct Standing<'a> {
    /// `SMSG_SET_FORCED_REACTIONS`: a faction id and the rank the server insists
    /// on for it, whatever the table and the standing say. A
    /// `SPELL_AURA_FORCE_REACTION` reaches the client through this packet. It
    /// is the only way a server can turn a friendly faction hostile without
    /// changing either unit's fields.
    pub forced: &'a [(u32, Rank)],
    /// The reputation list, faction id first. A faction absent from it is one
    /// the character has no bar for. The 1.12.1 client decides the same thing
    /// from `Faction.dbc`'s `reputationListID`.
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
/// The 1.12.1 client stops at the first zero entry rather than reading all
/// four, so a row with a gap in the middle of its list is read short. This
/// function tests the searched faction for zero instead. It gives the same
/// answer for every row in the file and does not depend on the order of the
/// entries.
fn names(list: &[u32; 4], faction: u32) -> bool {
    faction != 0 && list.contains(&faction)
}

/// One row of `FactionTemplate.dbc`, as the predicate needs it.
#[derive(Debug, Clone, Copy, Default)]
struct Template {
    faction: u32,
    /// Field 2. One bit of it is read, [`CONTESTED_GUARD`], and that bit is
    /// what makes a city guard attack somebody who has been fighting.
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

/// `FactionTemplate.dbc` field 2, bit 12: this faction's units are hostile to
/// a player who has been in a fight, tested against
/// [`player_flags::CONTESTED_PVP`].
///
/// The bit marks a guard faction. The 44 rows that carry it are the city and
/// road guards of both sides. It is the only case where a creature's reaction
/// to the local player depends on a `PLAYER_FLAGS` bit.
pub const CONTESTED_GUARD: u32 = 1 << 12;

/// The `UNIT_FIELD_FLAGS` bits this module reads, by their vmangos names.
pub mod unit_flags {
    /// `UNIT_FLAG_PLAYER_CONTROLLED`: a player, a pet, a totem, a charm.
    /// Every rule of the reaction order before the masks requires it.
    pub const PLAYER_CONTROLLED: u32 = 0x0000_0008;
    /// `UNIT_FLAG_IMMUNE_TO_PLAYER`.
    pub const IMMUNE_TO_PLAYER: u32 = 0x0000_0100;
    /// `UNIT_FLAG_IMMUNE_TO_NPC`.
    pub const IMMUNE_TO_NPC: u32 = 0x0000_0200;
    /// `UNIT_FLAG_PVP`: attackable by the other side.
    pub const PVP: u32 = 0x0000_1000;
    /// `UNIT_FLAG_NOT_SELECTABLE`: the unit cannot be targeted or assisted.
    pub const NOT_SELECTABLE: u32 = 0x0200_0000;
}

/// The `PLAYER_FLAGS` bits this module reads, by their vmangos names.
pub mod player_flags {
    /// `PLAYER_FLAGS_GHOST`.
    pub const GHOST: u32 = 0x0000_0010;
    /// `PLAYER_FLAGS_FFA_PVP`: the player can attack, and be attacked by,
    /// everybody. Two flagged players are Hostile to each other whatever their
    /// factions say. This rule is what lets two players of the same faction
    /// fight outside a duel.
    pub const FFA_PVP: u32 = 0x0000_0080;
    /// `PLAYER_FLAGS_CONTESTED_PVP`: the player has attacked the other side
    /// recently, and a [`super::CONTESTED_GUARD`] faction is hostile to them.
    pub const CONTESTED_PVP: u32 = 0x0000_0100;
}

/// Field indices of `FactionGroup.dbc`, whose four rows give the name of the
/// side a group mask bit stands for.
///
/// Four records and twelve fields: `id`, `MaskID`, `internalName`, then the
/// eight localised names and their flag word. The mask bit and the two strings
/// are kept, because they are all that `UnitFactionGroup` returns.
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
    /// `1 << MaskID`, stored shifted because every comparison uses the bit.
    bit: u32,
    /// `internalName`, such as `"Alliance"`. The interface builds a texture
    /// path from it: `Interface\GroupFrame\UI-Group-PVP-<group>`.
    internal: String,
    /// The localised name, which is the second value `UnitFactionGroup`
    /// returns.
    ///
    /// It is empty for Player and Monster in the shipped file, and
    /// [`Factions::group_name`] depends on that.
    localised: String,
}

/// `FactionTemplate.dbc`, indexed by template id, and `FactionGroup.dbc`
/// beside it. The second table has four rows and gives the side's name for the
/// same mask column.
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

    /// Attaches `FactionGroup.dbc`, which [`Factions::group_name`] needs and
    /// which has no other use.
    ///
    /// Separate from [`Factions::parse`] because the two files are separate and
    /// this one is optional. Without it the group mask still answers (the taxi
    /// filter and the character-creation join are unaffected) and only the
    /// side's name is missing, so a party frame shows no PvP icon.
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
                    // `1 << MaskID`, not the id. `FactionTemplate`'s four mask
                    // columns are bitfields over `MaskID`, so Alliance is bit 1
                    // and an Alliance player's mask is 3, Player and Alliance
                    // together. For that reason `group_name` cannot take the
                    // first bit set.
                    bit: 1u32 << mask,
                    internal: dbc
                        .string_at(record, group_fields::INTERNAL_NAME)
                        .unwrap_or_default(),
                    localised: dbc.string_at(record, group_fields::NAME).unwrap_or_default(),
                },
            ));
        }
        // Sorted by id. The 1.12.1 client searches the table in id order, and
        // the empty-name rule in `group_name` depends on that order.
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

    /// The `factionGroup` mask: which of Player, Alliance, Horde and Monster
    /// this template belongs to, as `1 << FactionGroup.MaskID`.
    ///
    /// This is the column [`Template::our_mask`] holds for the reaction rules,
    /// exposed for callers that want the side rather than the relationship.
    /// The taxi map filters its nodes on it
    /// ([`crate::tables::taxi::Team::of_group`]), and so does the character-creation
    /// screen through [`crate::tables::charcreate`]'s own join.
    pub fn group(&self, template: u32) -> Option<u32> {
        Some(self.0.get(&template)?.our_mask)
    }

    /// `UnitFactionGroup`: `(internalName, localisedName)` for the side a
    /// faction template is on, or `None` for a template whose side has no
    /// name.
    ///
    /// The 1.12.1 client takes the first `FactionGroup.dbc` row, in id order,
    /// whose `1 << MaskID` is in the template's `factionGroup` mask and whose
    /// localised name is not the empty string. Both conditions are needed. An
    /// Alliance player's mask is `Player | Alliance` = 3, so a search that
    /// stopped at the first matching bit would answer `"Player"` for every
    /// character. `PartyMemberFrame_UpdatePvPStatus` builds
    /// `Interface\GroupFrame\UI-Group-PVP-<group>` from the result, and
    /// `UI-Group-PVP-Player` is a missing texture.
    ///
    /// `None` for a creature, because Monster's name is empty too. That
    /// function's `if ( factionGroup and UnitIsPVP(unit) )` guard handles this
    /// case: there is no such icon for a wolf.
    pub fn group_name(&self, template: u32) -> Option<(&str, &str)> {
        let mask = self.group(template)?;
        self.1
            .iter()
            .find(|group| group.bit & mask != 0 && !group.localised.is_empty())
            .map(|group| (group.internal.as_str(), group.localised.as_str()))
    }

    /// How the unit on `template` stands towards the unit on `towards`, decided
    /// by the two templates alone. This is the six-line rule at the top of the
    /// module and the last step of the full reaction order ([`Factions::rank`]).
    ///
    /// Directional. `template` is the unit whose opinion is being asked and
    /// `towards` the unit it is about. For a player against a creature the
    /// 1.12.1 client asks one direction only: what the player thinks of the
    /// creature. A missing template on either side (an entity whose field has
    /// not arrived yet) reads [`Rank::Neutral`], which errs towards
    /// attackable. The server refuses a swing at a friend and says why; a
    /// client that refused it locally would leave the player pressing a button
    /// that does nothing.
    ///
    /// The masks are tried before the named lists. The 1.12.1 client uses this
    /// order; an earlier version of this function used the reverse. The order
    /// matters only for a row that states both, such as `hostileMask` covering
    /// a group and `friendFaction` naming one member of it. The 1.12.1 client
    /// answers hostile for that row.
    pub fn template_rank(&self, template: Option<u32>, towards: Option<u32>) -> Rank {
        let (Some(a), Some(b)) = (template, towards) else {
            return Rank::Neutral;
        };
        let (Some(us), Some(them)) = (self.0.get(&a), self.0.get(&b)) else {
            return Rank::Neutral;
        };
        // The hostile group mask first, then the four named enemies. The enemy
        // list lets a creature hate one faction of a group it otherwise likes.
        if us.hostile_mask & them.our_mask != 0 || names(&us.enemies, them.faction) {
            return Rank::Hostile;
        }
        // Our friendly mask, our friend list, then the same pair the other way
        // round. The friendly test checks both directions; the hostile test
        // above checks only ours.
        if us.friendly_mask & them.our_mask != 0
            || names(&us.friends, them.faction)
            || them.friendly_mask & us.our_mask != 0
            || names(&them.friends, us.faction)
        {
            return Rank::Friendly;
        }
        Rank::Neutral
    }

    /// [`Factions::template_rank`] reduced to the three values this client
    /// draws. Callers with no character standing use it: `vale login`, the
    /// spell-book's aiming rule, the CLI.
    pub fn reaction(&self, template: Option<u32>, towards: Option<u32>) -> Reaction {
        self.template_rank(template, towards).into()
    }

    /// How `a` stands towards `b` under the full reaction order of the 1.12.1
    /// client, including the local character's own state.
    ///
    /// `UnitReaction`, `UnitIsFriend`, `UnitIsEnemy` and the name-plate tint
    /// are all decided by this function. The module comment lists the order.
    /// [`Factions::template_rank`] is its last step.
    ///
    /// `standing` is the local character's reputation (forced reactions and
    /// the at-war bit). It is read by exactly two rules, and both require one
    /// of the two units to be that character. It is never read for another
    /// unit, because the server does not send other characters' standings.
    pub fn rank(&self, a: &Party, b: &Party, standing: &Standing) -> Rank {
        // The same unit is its own friend, before anything is read.
        if a.same_unit_as(b) {
            return Rank::Friendly;
        }
        if a.player_controlled() && b.player_controlled() {
            // A running duel decides the reaction. It is the only rule that
            // makes two members of one faction hostile without either of them
            // being flagged.
            if a.duel_team != 0 && b.duel_team != 0 && a.duel_arbiter == b.duel_arbiter {
                return if a.duel_team == b.duel_team {
                    Rank::Friendly
                } else {
                    Rank::Hostile
                };
            }
            // Whichever of the two is the local character, the other is
            // friendly if it is in our party or raid. The 1.12.1 client checks
            // both orders, and so does this test.
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
        // The local character is asking, so its own standings decide. `a` must
        // be player-controlled as well as local. The 1.12.1 client requires
        // the same, and the difference matters for a pet.
        if a.player_controlled() && a.is_local {
            if let Some(row) = b.faction.and_then(|id| self.0.get(&id)) {
                if let Some(forced) = standing.forced(row.faction) {
                    return forced;
                }
                if let Some(state) = standing.of(row.faction) {
                    // The at-war flag decides, not the rank. A faction we have
                    // a bar for is friendly unless we are at war with it. For
                    // that reason the Bloodsail Buccaneers are red on a new
                    // character and Booty Bay is green.
                    return if state.at_war { Rank::Hostile } else { Rank::Friendly };
                }
            }
        }
        // Every other case, clamped at [`Rank::CEILING`]. The friendliness
        // bump after the clamp is not modelled; see the module comment.
        self.asked_of(a, b, standing).min(Rank::CEILING)
    }

    /// How `a`'s faction template stands towards `b`, including the two rules
    /// that apply only when `b` is the local character.
    fn asked_of(&self, a: &Party, b: &Party, standing: &Standing) -> Rank {
        let (Some(us), Some(_them)) = (
            a.faction.and_then(|id| self.0.get(&id)),
            b.faction.and_then(|id| self.0.get(&id)),
        ) else {
            // If either row is missing the answer is Neutral, which is also
            // what an entity whose field has not arrived yet reads.
            return Rank::Neutral;
        };
        if b.player_controlled() && b.is_local {
            // A guard faction against a player who has been fighting. This
            // rule turns a city guard red.
            if us.flags & CONTESTED_GUARD != 0 && b.player_flags & player_flags::CONTESTED_PVP != 0
            {
                return Rank::Hostile;
            }
            if let Some(forced) = standing.forced(us.faction) {
                return forced;
            }
            if let Some(state) = standing.of(us.faction) {
                // In this direction the rank decides: a creature of a faction
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
/// The 1.12.1 client refuses an attack on a unit that carries any of these
/// five bits (names as in vmangos' `UnitFlags`):
/// `NON_ATTACKABLE` (1), `NOT_ATTACKABLE_1` (7), `NON_ATTACKABLE_2` (16),
/// `TAXI_FLIGHT` (20) and `NOT_SELECTABLE` (25).
///
/// These bits keep a quest giver, a flight master mid-flight and a spawning
/// creature out of the Tab-target pool. Faction does not exclude
/// them: most are ordinary friendly NPCs, and some are not friendly.
pub const UNATTACKABLE_FLAGS: u32 = (1 << 1) | (1 << 7) | (1 << 16) | (1 << 20) | (1 << 25);

/// Can this unit be attacked? The flags first, then the reaction.
///
/// Both tests are needed and neither implies the other: a critter is
/// attackable and neutral, a quest giver is friendly and flagged, and a
/// creature still spawning is hostile and flagged.
///
/// This is the creature case of [`Factions::can_attack`], kept as a free
/// function because five callers have only a reaction and a flags word, among
/// them the spell-book's aiming rule and the CLI. Where the attacker is the
/// local character and the target may be another player, use the method.
/// This function cannot see a duel, a free-for-all or a PvP flag, and answers
/// "no" in each of those cases.
pub fn can_attack(unit_flags: u32, reaction: Reaction) -> bool {
    unit_flags & UNATTACKABLE_FLAGS == 0 && reaction.is_attackable()
}

impl Factions {
    /// May `a` attack `b`: the 1.12.1 client's answer to `UnitCanAttack`.
    ///
    /// Four gates, then one of three verdicts. This client had no answer for
    /// the third (player against player) before this method:
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
    /// The ghost gate is not modelled. The 1.12.1 client also tests a property
    /// of the attacker that is not known, so refusing on the ghost bit alone
    /// would stop a spirit healer's own attacks and every other case that
    /// property allows. The gate is left out rather than guessed at; the
    /// server refuses a swing at a corpse and says so.
    pub fn can_attack(&self, a: &Party, b: &Party, standing: &Standing) -> bool {
        // The five bits that disqualify a unit whatever its reaction.
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
            // Two creatures. The swing is refused only when both directions
            // are better than Hostile, so hostility on one side is enough. This
            // lets a guard attack a creature that is only neutral towards it.
            (false, false) => {
                self.rank(a, b, standing) <= Rank::Hostile
                    || self.rank(b, a, standing) <= Rank::Hostile
            }
            // A player, or a pet, against a creature. Neutral is attackable
            // (a client that required hostility could not attack a rabbit);
            // Friendly is not.
            (_, false) => self.rank(a, b, standing) < Rank::Friendly,
            // Against another player. Friendly still refuses.
            (_, true) => {
                if self.rank(a, b, standing) >= Rank::Friendly {
                    return false;
                }
                // A duel in progress, whichever side either is on.
                if a.duel_team != 0 && b.duel_team != 0 && a.duel_arbiter == b.duel_arbiter {
                    return true;
                }
                // `b` carries the PvP flag, the ordinary way one player
                // becomes attackable to another.
                if b.unit_flags & unit_flags::PVP != 0 {
                    return true;
                }
                // Both are free-for-all, which needs neither the PvP flag nor
                // a faction test.
                a.player_flags & player_flags::FFA_PVP != 0
                    && b.player_flags & player_flags::FFA_PVP != 0
            }
        }
    }

    /// May `a` assist `b`: the 1.12.1 client's `UnitCanAssist`, and the test
    /// `TargetNearestFriend` applies to its candidates.
    ///
    /// ```text
    /// b carries UNIT_FLAG_NOT_SELECTABLE                  -> no
    /// rank(a, b) below Friendly                           -> no
    /// b is player-controlled:
    ///   b is in a duel and a is not on b's side of it     -> no
    ///   b is free-for-all and a is not                    -> no
    ///   otherwise                                         -> yes
    /// b is not, a is player-controlled:  b's UNIT_FLAG_PVP
    /// neither is player-controlled                        -> yes
    /// ```
    ///
    /// For a player asking about a creature the answer is the creature's PvP
    /// flag: a friendly city or road guard carries it and can be assisted; a
    /// friendly vendor or quest giver does not and cannot.
    ///
    /// Two differences from the client, because [`Party`] does not carry the
    /// unit that controls `b`. The client reads the duel and free-for-all
    /// state of the player controlling `b` (a pet's owner), and for a creature
    /// with a controller it reads the controller's PvP flag and refuses one
    /// that carries `UNIT_FLAG_IMMUNE_TO_PLAYER`. This reads `b`'s own fields,
    /// so a pet, which carries no `PLAYER_FLAGS` and no duel, passes those two
    /// rows, and a guardian is judged by its own PvP flag.
    pub fn can_assist(&self, a: &Party, b: &Party, standing: &Standing) -> bool {
        if b.unit_flags & unit_flags::NOT_SELECTABLE != 0 {
            return false;
        }
        if self.rank(a, b, standing) < Rank::Friendly {
            return false;
        }
        if b.player_controlled() {
            if b.duel_team != 0 && (a.duel_arbiter != b.duel_arbiter || a.duel_team != b.duel_team) {
                return false;
            }
            return b.player_flags & player_flags::FFA_PVP == 0
                || a.player_flags & player_flags::FFA_PVP != 0;
        }
        if a.player_controlled() {
            return b.unit_flags & unit_flags::PVP != 0;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// Builds a six-row `FactionTemplate.dbc`. 1 is an Alliance-group unit and
    /// 2 a Horde-group one, with masks that make them hate each other; rows 3
    /// to 6 are described below.
    fn table() -> Factions {
        // Fourteen fields: id, faction, flags, ourMask, friendlyMask,
        // hostileMask, enemy[4], friend[4].
        let rows = vec![
            vec![1, 1, 0, 0b0010, 0b0010, 0b0100, 0, 0, 0, 0, 0, 0, 0, 0],
            vec![2, 2, 0, 0b0100, 0b0100, 0b0010, 0, 0, 0, 0, 0, 0, 0, 0],
            // 3 is in the Alliance group but names faction 1 as an enemy. The
            // enemy list must win over the shared friendly mask.
            vec![3, 3, 0, 0b0010, 0b0010, 0, 1, 0, 0, 0, 0, 0, 0, 0],
            // 4 is in the Horde group but names faction 1 as a friend. The two
            // orders disagree on this row, and the 1.12.1 client lets the mask
            // win.
            vec![4, 4, 0, 0b0100, 0b0100, 0b0010, 0, 0, 0, 0, 1, 0, 0, 0],
            // 5 names faction 1 as a friend with no mask in the way, which is
            // the case the friend list exists for.
            vec![5, 5, 0, 0b1000, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0],
            // 6 is faction 1's own group with the contested-guard flag on it.
            vec![6, 1, CONTESTED_GUARD, 0b0010, 0b0010, 0b0100, 0, 0, 0, 0, 0, 0, 0, 0],
        ];
        Factions::parse(&dbc(&rows, 14, &[0])).expect("a faction table")
    }

    /// `FactionGroup.dbc` as 1.12 ships it: four rows, two of which have an
    /// empty localised name. Twelve fields: id, MaskID, internalName, then
    /// eight locales and their flag word.
    fn groups() -> Vec<u8> {
        // The string block. Offset 0 is the empty string, as in every DBC.
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

    /// An Alliance player's group mask is `Player | Alliance`, and
    /// `group_name` must not answer `"Player"` for it.
    ///
    /// The 1.12.1 client takes the first `FactionGroup.dbc` row whose bit is
    /// in the mask and whose localised name is non-empty. Player's name is
    /// empty in the shipped file, as is Monster's. Without the second condition
    /// every character answers `"Player"`, which
    /// `PartyMemberFrame_UpdatePvPStatus` turns into
    /// `Interface\GroupFrame\UI-Group-PVP-Player`: a texture that does not
    /// exist. The archives carry only the Alliance and Horde pair.
    #[test]
    fn a_players_faction_group_skips_the_row_with_no_name() {
        // Templates 1 and 2 in `table()` are bare group bits; this test builds
        // its own table with the masks a real player carries.
        let rows = vec![
            // An Alliance player: Player | Alliance = 0b011.
            vec![1, 1, 0, 0b011, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            // A Horde player: Player | Horde = 0b101.
            vec![2, 2, 0, 0b101, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            // A wolf: Monster alone.
            vec![3, 3, 0, 0b1000, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ];
        let t = Factions::parse(&dbc(&rows, 14, &[0]))
            .expect("a faction table")
            .with_groups(&groups());

        assert_eq!(t.group_name(1), Some(("Alliance", "Alliance")));
        assert_eq!(t.group_name(2), Some(("Horde", "Horde")));
        // A creature's side has no name, which is the case
        // `if ( factionGroup and UnitIsPVP(unit) )` guards: Monster's
        // localised name is empty too, so the search finds nothing.
        assert_eq!(t.group_name(3), None);
        // A template that is not in the table.
        assert_eq!(t.group_name(99), None);
    }

    /// Without `FactionGroup.dbc` the mask still answers and the name does
    /// not. This is why `with_groups` can be optional: the taxi filter and the
    /// character-creation join keep working, and a party frame shows no PvP
    /// icon.
    #[test]
    fn a_missing_group_table_costs_the_word_and_nothing_else() {
        let rows = vec![vec![1, 1, 0, 0b011, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]];
        let t = Factions::parse(&dbc(&rows, 14, &[0])).expect("a faction table");
        assert_eq!(t.group(1), Some(0b011), "the mask is off the template alone");
        assert_eq!(t.group_name(1), None);
        // A damaged group table behaves like an absent one and does not panic.
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

    /// The masks are tried before the named lists. The 1.12.1 client uses this
    /// order; an earlier version of this module used the reverse.
    ///
    /// The rows that show the difference are the ones that state both. 3
    /// shares 1's group and names it as an enemy, which both orders call
    /// hostile. 4 is in the group 1's mask hates and names 1 as a friend,
    /// which the old order called friendly and the 1.12.1 client calls
    /// hostile. 5 names 1 as a friend with no mask in the way, which is the
    /// case the list exists for.
    #[test]
    fn the_hostile_mask_is_tried_before_the_named_lists() {
        let t = table();
        assert_eq!(t.reaction(Some(3), Some(1)), Reaction::Hostile);
        assert_eq!(t.reaction(Some(4), Some(1)), Reaction::Hostile);
        assert_eq!(t.reaction(Some(5), Some(1)), Reaction::Friendly);
    }

    /// A missing template reads neutral, which is attackable. Erring this way
    /// is safe because the server refuses a bad swing and says why.
    #[test]
    fn an_unknown_template_is_neutral_and_attackable() {
        let t = table();
        assert_eq!(t.reaction(None, Some(1)), Reaction::Neutral);
        assert_eq!(t.reaction(Some(99), Some(1)), Reaction::Neutral);
        assert!(t.reaction(None, None).is_attackable());
    }

    /// Attackable is "not friendly", not "hostile". A client that requires
    /// hostility cannot attack a rabbit.
    #[test]
    fn neutral_units_can_be_attacked_and_flags_can_still_forbid_it() {
        assert!(can_attack(0, Reaction::Neutral));
        assert!(can_attack(0, Reaction::Hostile));
        assert!(!can_attack(0, Reaction::Friendly));
        // A quest giver is friendly and flagged; a spawning creature is
        // hostile and flagged. Either test is enough to refuse.
        assert!(!can_attack(1 << 25, Reaction::Hostile));
        assert!(!can_attack(1 << 1, Reaction::Neutral));
    }

    /// The template leg of the reaction order is clamped and the earlier rules
    /// are not. A creature of a faction the character is Exalted with reads
    /// Revered. The same standing read the other way, with the local character
    /// asking, is Friendly, because that rule reads the at-war bit rather than
    /// the rank.
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

    /// A player: a unit with `UNIT_FLAG_PLAYER_CONTROLLED`, whatever its object
    /// type.
    fn player(guid: u64, id: u32) -> Party {
        Party {
            guid,
            faction: Some(id),
            unit_flags: unit_flags::PLAYER_CONTROLLED,
            ..Party::default()
        }
    }

    /// Nothing forced and no faction with a bar. This is the state of every
    /// session before a reputation packet arrives, and in it [`Factions::rank`]
    /// must agree with the bare table.
    fn nothing() -> Standing<'static> {
        Standing::default()
    }

    /// With no character state, [`Factions::rank`] gives the table's answer.
    /// Callers with no state to offer, such as the CLI and the aiming rule,
    /// therefore get the same answer as from the table alone.
    #[test]
    fn an_empty_standing_leaves_the_table_deciding() {
        let t = table();
        let (a, b) = (creature(1), creature(2));
        assert_eq!(t.rank(&a, &b, &nothing()), Rank::Hostile);
        assert_eq!(t.rank(&a, &creature(1), &nothing()), Rank::Friendly);
        assert_eq!(t.rank(&a, &creature(99), &nothing()), Rank::Neutral);
    }

    /// `UnitReaction` is the rank plus one, and it indexes the eight rows of
    /// `UnitReactionColor`.
    #[test]
    fn the_lua_value_is_the_rank_plus_one() {
        assert_eq!(Rank::Hostile.lua(), 2);
        assert_eq!(Rank::Unfriendly.lua(), 3);
        assert_eq!(Rank::Neutral.lua(), 4);
        assert_eq!(Rank::Friendly.lua(), 5);
        assert_eq!(Reaction::from(Rank::Unfriendly), Reaction::Neutral);
        assert_eq!(Reaction::from(Rank::Honored), Reaction::Friendly);
    }

    /// A forced reaction overrides the table. A `SPELL_AURA_FORCE_REACTION`
    /// reaches the client as a forced reaction, and it can turn a friendly
    /// faction hostile with no field on any unit changed.
    ///
    /// Tested in both directions, because the reaction order has two separate
    /// forced-reaction rules: one for the local character asking and one for
    /// the creature asking.
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
        // The forced reaction changes attackability, not only the colour.
        assert!(t.can_attack(&me, &guard, &standing));
        assert!(!t.can_attack(&me, &guard, &nothing()));
    }

    /// A player may assist a friendly player, and a friendly creature only
    /// when the creature carries `UNIT_FLAG_PVP`; never a hostile unit or an
    /// unselectable one.
    #[test]
    fn a_player_assists_friendly_players_and_flagged_creatures() {
        let t = table();
        let mut me = player(7, 1);
        me.is_local = true;
        assert!(t.can_assist(&me, &player(8, 1), &nothing()));
        assert!(!t.can_assist(&me, &player(9, 2), &nothing()), "the other side");

        let vendor = creature(1);
        assert!(!t.can_assist(&me, &vendor, &nothing()), "friendly but not flagged");
        let guard = Party { unit_flags: unit_flags::PVP, ..creature(1) };
        assert!(t.can_assist(&me, &guard, &nothing()));
        let hidden = Party { unit_flags: unit_flags::PVP | unit_flags::NOT_SELECTABLE, ..creature(1) };
        assert!(!t.can_assist(&me, &hidden, &nothing()));

        // Two creatures of one side assist each other with no flag.
        assert!(t.can_assist(&creature(1), &vendor, &nothing()));
    }

    /// A friendly player in a duel can be assisted only by the other member of
    /// the same team. A free-for-all player can be assisted by nobody: a
    /// player who is not flagged is refused by the free-for-all row, and one
    /// who is flagged is Hostile to it.
    #[test]
    fn a_duel_and_free_for_all_limit_who_may_assist() {
        let t = table();
        let mut me = player(7, 1);
        me.is_local = true;
        let mut duellist = player(8, 1);
        duellist.duel_arbiter = 500;
        duellist.duel_team = 1;
        assert!(!t.can_assist(&me, &duellist, &nothing()));
        let mut second = me;
        second.duel_arbiter = 500;
        second.duel_team = 1;
        assert!(t.can_assist(&second, &duellist, &nothing()));
        second.duel_team = 2;
        assert!(!t.can_assist(&second, &duellist, &nothing()));

        let mut brawler = player(9, 1);
        brawler.player_flags = player_flags::FFA_PVP;
        assert!(!t.can_assist(&me, &brawler, &nothing()));
        // Two free-for-all players are Hostile to each other, so the rank
        // refuses before the free-for-all row is reached.
        let mut also = me;
        also.player_flags = player_flags::FFA_PVP;
        assert!(!t.can_assist(&also, &brawler, &nothing()));
    }

    /// The reaction to a faction we have a bar for is decided by the at-war
    /// bit, not by the masks. For that reason the Bloodsail Buccaneers are red
    /// on a character who has never met them and Booty Bay is green.
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

    /// In the other direction the rank decides, not the at-war flag: a
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

    /// A guard faction is hostile to a player who has been fighting. This is
    /// the only rule where a `PLAYER_FLAGS` bit decides a creature's reaction,
    /// and it is why guards turn red.
    #[test]
    fn a_contested_guard_is_hostile_to_a_contested_player() {
        // Row 6 is template 1's faction with the guard flag on it.
        let t = table();
        let mut me = player(7, 1);
        me.is_local = true;
        assert_eq!(t.rank(&creature(6), &me, &nothing()), Rank::Friendly);
        me.player_flags |= player_flags::CONTESTED_PVP;
        assert_eq!(t.rank(&creature(6), &me, &nothing()), Rank::Hostile);
        // A faction without the guard flag stays friendly to the same player.
        assert_eq!(t.rank(&creature(1), &me, &nothing()), Rank::Friendly);
    }

    /// Two free-for-all players are hostile whatever their factions say. Both
    /// must carry the flag.
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

    /// A duel decides the reaction before either faction is read. The
    /// same-team case keeps a duelling partner's own pet friendly.
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
        // Different arbiters: the two are not in the same duel.
        b.duel_arbiter = 901;
        b.duel_team = 2;
        assert_eq!(t.rank(&a, &b, &nothing()), Rank::Friendly);
    }

    /// A group member is friendly whichever of the two units is the local
    /// character. The 1.12.1 client behaves the same way, and it matters
    /// because the caller chooses the direction.
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

    /// A player of the other side may be attacked when flagged for PvP and not
    /// otherwise. This is the ordinary PvP rule; no code here answered it
    /// before [`Factions::can_attack`].
    #[test]
    fn the_pvp_flag_is_what_opens_a_player_to_attack() {
        let t = table();
        let (me, mut them) = (player(7, 1), player(8, 2));
        assert_eq!(t.rank(&me, &them, &nothing()), Rank::Hostile);
        assert!(!t.can_attack(&me, &them, &nothing()), "unflagged and not in a duel");
        them.unit_flags |= unit_flags::PVP;
        assert!(t.can_attack(&me, &them, &nothing()));
    }

    /// The five bits and the two immunities, which are checked before every
    /// verdict.
    #[test]
    fn the_flag_gates_refuse_before_any_faction_is_read() {
        let t = table();
        let (me, mut mob) = (player(7, 1), creature(2));
        assert!(t.can_attack(&me, &mob, &nothing()));
        mob.unit_flags |= 1 << 25;
        assert!(!t.can_attack(&me, &mob, &nothing()), "not selectable");
        mob.unit_flags = unit_flags::IMMUNE_TO_PLAYER;
        assert!(!t.can_attack(&me, &mob, &nothing()));
        // A creature attacker is stopped by the other immunity, not this one.
        assert!(t.can_attack(&creature(1), &mob, &nothing()));
        mob.unit_flags = unit_flags::IMMUNE_TO_NPC;
        assert!(!t.can_attack(&creature(1), &mob, &nothing()));
        assert!(t.can_attack(&me, &mob, &nothing()));
    }

    /// A creature may attack another creature when either side is hostile to
    /// the other. A player's test is different.
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
