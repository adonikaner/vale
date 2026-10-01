//! The name drawn over a unit's head: which units get one, where it is
//! anchored, and how large it is.
//!
//! Like [`super::worldtext`], this subject is absent from
//! `Interface\FrameXML\`. There is no frame, no template and no `OnLoad`: the
//! 1.12.1 client builds the string itself and draws it with the same world text
//! renderer the damage numbers use. Every number below is therefore the
//! client's own and not an interface file's.
//!
//! The floating name is not the nameplate. The plate, a bar with a health fill
//! and a level, is a different object with its own five files and its own two
//! switches. The two are mutually exclusive: the client draws no floating name
//! for a unit that has a plate up. This client draws the name and not the
//! plate.
//!
//! ## Which units get a name
//!
//! This is the complete rule:
//!
//! ```text
//! the unit may be named at all           — else no
//! it has no nameplate up                 — else no
//! is it me?          -> UnitNameOwn                (bit 0)
//! is it my target?   -> ALWAYS, whatever the CVars say
//! is it a player?    -> UnitNamePlayer             (bit 2)
//! otherwise          -> UnitNameNPC                (bit 1)
//! ```
//!
//! The target clause is unconditional: it answers yes without a mask test, so
//! the selected unit is named even when its whole class is switched off. The
//! defaults depend on it.
//!
//! ## The five CVars that switch names on and off
//!
//! Each sets or clears one bit of a single mask. By default players are named
//! and no other unit is.
//!
//! | CVar | bit | default |
//! |---|---|---|
//! | `UnitNameOwn` | `0x01` | `"0"` |
//! | `UnitNameNPC` | `0x02` | `"0"` |
//! | `UnitNamePlayer` | `0x04` | `"1"` |
//! | `UnitNamePlayerGuild` | `0x10` | `"1"` |
//! | `UnitNamePlayerPVPTitle` | `0x20` | `"1"` |
//!
//! ## The text of a name
//!
//! The text is one string of up to four lines, and every line has the name's
//! size and colour. [`floating_text`] builds it:
//!
//! ```text
//! <AFK><DND><GM>Sergeant Dolgrin     PLAYER_FLAGS 2, 4 and 8; the rank title
//!                                    under UnitNamePlayerPVPTitle
//! Protector of Stormwind             a player's city title, with the rank
//! <The Watch>                        a player's guild, under
//!                                    UnitNamePlayerGuild
//! <Innkeeper>                        a creature's subname
//! <Dolgrin's Pet>                    the unit's owner; see [`OwnerTitle`]
//! ```
//!
//! The first three lines are a player's and the last two are a creature's.
//! No CVar switches the creature's two lines off. The angle brackets round
//! the guild, the subname and the owner line are the client's own and are in
//! no interface file. The unit tooltip shows the same subname and owner line
//! without brackets and has no guild line; see [`titled_name`].
//!
//! Three parts of the client's text are not built here: the `Civilian`
//! prefix of a non-combatant NPC, the suffix for a player from another realm,
//! and the rule for which realm a name belongs to.
//!
//! ## The anchor point, and the larger name of a tall unit
//!
//! The name hangs from the `PlayerName` attachment, point 18, which the plate
//! also uses. Nothing else uses that attachment id.
//!
//! The client starts at [`BASE_SIZE`] and, for a unit taller than four yards,
//! multiplies by `height / 4 x 1.5`. The height it measures is the attachment's
//! own z above the unit's origin, so a tall creature's name is drawn larger.
//! This is the 1.12.1 client's rule and not a fault in this one.
//!
//! ## The size is a world height
//!
//! A name is drawn in the world: small across a valley, large at arm's length.
//! A screenshot of the 1.12.1 client shows this: a creature thirty yards off is
//! named at about a third of the height the player beside the camera is. A size
//! that was a fraction of the screen would not do that.
//!
//! The client has two fonts for world text, built at sizes two orders of
//! magnitude apart: `DAMAGE_TEXT_FONT` at `0.018333` and `UNIT_NAME_FONT` at
//! `0.99`. A font rasterised at ninety-nine per cent of the screen's height is
//! only useful if it is scaled down by a large and varying amount, and a
//! perspective divide is the only thing in the client that does that. The
//! damage numbers' font, at just under two per cent, is built at about the size
//! it is drawn at. The two paths therefore differ as the screenshots show:
//! [`super::worldtext`]'s heights stay fractions of the screen and this one is
//! in yards.
//!
//! The size the client gives a name also does not depend on distance: it is the
//! same at any range. The drawn name does shrink with range, so the shrink
//! comes from the projection.
//!
//! The arithmetic agrees to within a fifth: 0.2 yards at the default camera
//! reach, through this client's own vertical field of view, is about eighteen
//! units of a 768-unit interface, which is what a 1024x768 screenshot of the
//! 1.12.1 client measures. No code has to reconcile the size with a field of
//! view: `client::render::labels` puts the name on a quad in the world, a fixed
//! number of yards tall, and the camera projects it.

/// The face, from `Fonts.xml`'s `UNIT_NAME_FONT`. That is a Lua global holding
/// a path and not a font object, as `DAMAGE_TEXT_FONT` is.
pub const FONT: &str = r"Fonts\FRIZQT__.TTF";

/// The attachment the name hangs from: `PlayerName`, point 18. This is the only
/// place in this client that asks for it.
///
/// In one case, decided by state this client does not read, the 1.12.1 client
/// uses point `18 + 11 = 29` instead; 29 is unidentified and 18 is the ordinary
/// case. A model that does not carry the point falls back to its helm point.
/// That fallback is this client's own: the 1.12.1 client has none, because
/// every model it names has point 18. The helm point replaced the top of the
/// declared box as the fallback; see also [`BASE_SIZE`].
pub const ANCHOR_ATTACHMENT: u32 = 18;

/// The name's height for an ordinary unit, in yards.
///
/// This is the 1.12.1 client's number. The module comment explains why it is a
/// world height and not a fraction of the screen. A name therefore has a fixed
/// size in the world: small across a valley, large at arm's length. Drawing it
/// at a constant size on the screen, as the first version of this did, is
/// wrong.
pub const BASE_SIZE: f32 = 0.2;

/// Above this height, in yards, a unit's name is drawn larger.
pub const TALL: f32 = 4.0;

/// The multiplier applied to `height / TALL` for a unit above [`TALL`].
pub const TALL_GAIN: f32 = 1.5;

/// The name's height in yards for a unit whose `PlayerName` point is `height`
/// yards above its origin.
///
/// The client compares the height against four yards and leaves the base
/// unchanged at or below it, so the function is a step followed by a ramp and
/// not a smooth curve: a unit of exactly four yards takes `0.2` and one just
/// over four takes `0.3`. Every human-sized unit takes the base unchanged.
///
/// The result is a world height, so a kodo's name is physically larger and
/// still shrinks with distance like everything else.
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

/// Which units this client names, with one field per bit of the 1.12.1 client's
/// mask.
///
/// It is a struct and not the bare mask because the five are changed one at a
/// time from the interface options, and a bare `u32` lets a transposed bit
/// compile.
///
/// It is built from the CVars and not defaulted. The settings a session runs on
/// are the ones `Config.wtf` carries, under the names build 5875 registers, so
/// a build placed in a real WoW folder uses that folder's settings and needs no
/// configuration of its own. A session builds its policy with
/// [`Policy::from_cvars`] only. `Default` exists only for a test and returns
/// what the client's own registrations give.
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
    /// The defaults 1.12 registers: players named, no other unit named, and the
    /// target named regardless. The table in the module comment lists each.
    ///
    /// This is the fallback and not the source: a session runs on
    /// [`Policy::from_cvars`] over the store `Config.wtf` filled.
    fn default() -> Self {
        Policy { own: false, npc: false, player: true, guild: true, pvp_title: true }
    }
}

/// The five CVar names, in [`Policy`]'s own field order.
///
/// They are spelled as build 5875 registers them, which is the spelling a real
/// `Config.wtf` carries and the spelling the interface passes to `GetCVar`. The
/// lookup itself is case-insensitive. The file is not written from this array.
pub const CVARS: [&str; 5] = [
    "UnitNameOwn",
    "UnitNameNPC",
    "UnitNamePlayer",
    "UnitNamePlayerGuild",
    "UnitNamePlayerPVPTitle",
];

/// The mode CVar. It summarises the five switches, and build 5875 does not read
/// it.
///
/// `UnitNameRenderMode` (default `"2"`) has this help string:
///
/// ```text
/// sets unitname mode (0=none,1=lockedunits/lockedplayers,
///                     2=playersalways+lockedunits, 3=all units always
/// ```
///
/// Mode 2, players always plus the unit you have locked, is what the five bits'
/// defaults come to and what this client draws. The CVar is named here and not
/// implemented because the 1.12.1 client does not use its value: the naming
/// rule consults the five bits. Code that wires the options panel should change
/// the five and leave this one alone.
pub const MODE_CVAR: &str = "UnitNameRenderMode";

impl Policy {
    /// The mask the 1.12.1 client keeps, for a check against it.
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

    /// Reads the five switches from the settings store. `flag` answers for the
    /// string a `Config.wtf` carried or for the client's own registration. The
    /// interface's own test is `== "1"`, so anything but `"0"` is on. That is
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

    /// The inverse of [`Policy::mask`], for a CVar file that has been read.
    pub fn from_mask(mask: u32) -> Policy {
        Policy {
            own: mask & switches::OWN != 0,
            npc: mask & switches::NPC != 0,
            player: mask & switches::PLAYER != 0,
            guild: mask & switches::GUILD != 0,
            pvp_title: mask & switches::PVP_TITLE != 0,
        }
    }

    /// Whether this unit gets a name. The tests are in the 1.12.1 client's
    /// order.
    pub fn names(&self, unit: &Candidate) -> bool {
        // This test comes before the 1.12.1 client's tests, because that client
        // never needs it. See [`Candidate::unit`].
        if !unit.unit || !unit.nameable {
            return false;
        }
        if unit.is_self {
            return self.own;
        }
        // The target is named unconditionally: the 1.12.1 client does not test
        // the mask for it. That makes a default of "players only" usable.
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
    /// Whether this is a unit at all.
    ///
    /// The 1.12.1 client applies its naming rule only to units, so it never
    /// asks this question: a chest, a door, a mailbox or a signpost is not a
    /// candidate. This client's world holds game objects in the same table as
    /// units, so the test has to be made somewhere. It is made here and not in
    /// the renderer because it is a rule about which objects get a name and not
    /// a detail of one renderer.
    ///
    /// A game object's name is its tooltip, which is a separate subject:
    /// `GAMEOBJECT_TYPE_GENERIC`'s `floatingTooltip` and
    /// [`super::object::hover_of`], drawn by the interface. Naming one here
    /// would show the same words in two places, and would do it under a CVar
    /// called `UnitNameNPC`.
    pub unit: bool,
    /// Whether the unit may be named at all. This folds the 1.12.1 client's
    /// first check and the two conditions under it into one. This client
    /// answers it as "alive, in the world and carrying a name"; the 1.12.1
    /// client's version depends on state this client does not read.
    pub nameable: bool,
    pub is_self: bool,
    pub player: bool,
    /// The unit the local player has selected.
    pub targeted: bool,
}

/// `PLAYER_FLAGS` bits that put a tag in front of a player's floating name.
/// Values follow vmangos' `PlayerFlags`.
pub mod player_flags {
    pub const AFK: u32 = 0x02;
    pub const DND: u32 = 0x04;
    pub const GM: u32 = 0x08;
}

/// `CreatureType.dbc`'s id for a beast, which is what makes an owned unit a
/// pet and not a minion.
pub const CREATURE_TYPE_BEAST: u32 = 1;

/// The line that names a unit's owner: `Dolgrin's Pet`.
///
/// The owner is the unit's charmer when it has one, and otherwise its
/// creator. `UNIT_FIELD_SUMMONEDBY` is not read. See [`OwnerTitle::of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerTitle {
    Pet,
    Minion,
    Guardian,
    Creation,
}

/// `SPELL_EFFECT_SUMMON_GUARDIAN`: the `Effect[0]` of a creating spell whose
/// unit is titled a guardian.
const EFFECT_SUMMON_GUARDIAN: u32 = 42;

/// The `Effect[0]` values of a creating spell whose unit is titled a
/// creation: a summoned portal, and the nine totem and object summons. The
/// same ten values make the combat log say a unit is destroyed; see
/// [`crate::interface::combatlog::DESTROYED_BY_EFFECT`].
const EFFECT_CREATIONS: [u32; 10] = crate::interface::combatlog::DESTROYED_BY_EFFECT;

impl OwnerTitle {
    /// The `GlobalStrings.lua` key, whose one `%s` takes the owner's name.
    pub fn key(self) -> &'static str {
        match self {
            OwnerTitle::Pet => "UNITNAME_TITLE_PET",
            OwnerTitle::Minion => "UNITNAME_TITLE_MINION",
            OwnerTitle::Guardian => "UNITNAME_TITLE_GUARDIAN",
            OwnerTitle::Creation => "UNITNAME_TITLE_CREATION",
        }
    }

    /// Which title an owned unit takes.
    ///
    /// `charmed` is whether `UNIT_FIELD_CHARMEDBY` names the owner. When it
    /// does not, the owner is `UNIT_FIELD_CREATEDBY` and `created_effect` is
    /// the `Effect[0]` of the unit's `UNIT_CREATED_BY_SPELL`, or `None` for a
    /// unit with no such spell or one `Spell.dbc` does not have. That effect
    /// decides a guardian and a creation. Every other owned unit is a pet
    /// when its creature type is a beast and a minion otherwise, a charmed
    /// unit included.
    pub fn of(charmed: bool, created_effect: Option<u32>, creature_type: u32) -> OwnerTitle {
        if !charmed {
            match created_effect {
                Some(EFFECT_SUMMON_GUARDIAN) => return OwnerTitle::Guardian,
                Some(effect) if EFFECT_CREATIONS.contains(&effect) => return OwnerTitle::Creation,
                _ => {}
            }
        }
        if creature_type == CREATURE_TYPE_BEAST {
            OwnerTitle::Pet
        } else {
            OwnerTitle::Minion
        }
    }
}

/// What a unit's name text is built from.
#[derive(Debug, Clone, Copy, Default)]
pub struct NameParts<'a> {
    pub name: &'a str,
    pub player: bool,
    /// `PLAYER_FLAGS`, for the three tags in [`player_flags`].
    pub player_flags: u32,
    /// The honor rank byte of `PLAYER_BYTES_3`, 0 for none. It is the number
    /// in the `PVP_RANK_<rank>_<team>` key, unchanged.
    pub pvp_rank: u8,
    /// The city title byte of `PLAYER_BYTES_3`, 0 for none: the number in a
    /// `PVP_MEDAL<n>` key.
    pub pvp_medal: u8,
    /// The team number in the rank key: 0 for the Horde and 1 for the
    /// Alliance. `None` for a unit on neither side, which gets no rank title.
    pub team: Option<u8>,
    pub female: bool,
    /// A player's guild name, empty for none or for a guild whose name has
    /// not arrived.
    pub guild: &'a str,
    /// A creature's template subname, empty for none.
    pub sub_name: &'a str,
    /// `UNIT_FIELD_PETNUMBER`. A unit with one shows no subname.
    pub pet_number: u32,
    /// The title and the owner's name, for a unit whose owner is in the
    /// world.
    pub owner: Option<(OwnerTitle, &'a str)>,
}

/// The team number a rank key takes, from a `FactionTemplate.dbc` group
/// mask: 0 for a mask with the Horde bit (4), 1 for one with the Alliance
/// bit (2).
pub fn team_of_group(mask: u32) -> Option<u8> {
    if mask & 4 != 0 {
        Some(0)
    } else if mask & 2 != 0 {
        Some(1)
    } else {
        None
    }
}

/// A unit's name with its honor rank in front and its city title under it:
/// `Sergeant Dolgrin`.
///
/// `with_title` is the `UnitNamePlayerPVPTitle` switch for the floating name.
/// The unit tooltip passes true whatever the switch says. `word` answers a
/// `GlobalStrings.lua` key.
///
/// The rank and the name are joined by `UNIT_PVP_NAME`. The rank's key has a
/// `_FEMALE` form, which is tried first for a female character. The city
/// title is added only to a name that has a rank.
pub fn titled_name(
    parts: &NameParts,
    with_title: bool,
    word: &dyn Fn(&str) -> Option<String>,
) -> String {
    let rank = (parts.player && with_title && parts.pvp_rank != 0)
        .then_some(parts.team)
        .flatten()
        .and_then(|team| {
            let key = format!("PVP_RANK_{}_{team}", parts.pvp_rank);
            parts
                .female
                .then(|| word(&format!("{key}_FEMALE")))
                .flatten()
                .filter(|title| !title.is_empty())
                .or_else(|| word(&key))
        })
        .filter(|title| !title.is_empty());
    let Some(rank) = rank else {
        return parts.name.to_string();
    };
    let mut text = match word("UNIT_PVP_NAME") {
        Some(format) => crate::interface::strings::substitute_all(&format, &[&rank, parts.name]),
        None => parts.name.to_string(),
    };
    if parts.pvp_medal != 0 {
        if let Some(medal) = word(&format!("PVP_MEDAL{}", parts.pvp_medal)) {
            text.push('\n');
            text.push_str(&medal);
        }
    }
    text
}

/// The owner line without brackets, as the unit tooltip shows it:
/// `Dolgrin's Pet`. `None` for a player, for a unit with no owner in the
/// world, and when the key is not in `GlobalStrings.lua`.
pub fn owner_line(parts: &NameParts, word: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if parts.player {
        return None;
    }
    let (title, owner) = parts.owner?;
    word(title.key()).map(|format| crate::interface::strings::substitute(&format, owner))
}

/// The creature subname a unit shows, or nothing: a player has none and a
/// unit with a pet number shows none.
pub fn sub_name<'a>(parts: &NameParts<'a>) -> Option<&'a str> {
    (!parts.player && parts.pet_number == 0 && !parts.sub_name.is_empty()).then_some(parts.sub_name)
}

/// The whole text of a floating name, its lines separated by `\n`. See the
/// module note for the lines.
pub fn floating_text(
    parts: &NameParts,
    policy: &Policy,
    word: &dyn Fn(&str) -> Option<String>,
) -> String {
    let mut text = String::new();
    if parts.player {
        for (bit, key) in [
            (player_flags::AFK, "CHAT_FLAG_AFK"),
            (player_flags::DND, "CHAT_FLAG_DND"),
            (player_flags::GM, "CHAT_FLAG_GM"),
        ] {
            if parts.player_flags & bit != 0 {
                text.push_str(&word(key).unwrap_or_default());
            }
        }
    }
    text.push_str(&titled_name(parts, policy.pvp_title, word));
    let mut bracketed = |line: &str| {
        text.push_str("\n<");
        text.push_str(line);
        text.push('>');
    };
    if parts.player {
        if policy.guild && !parts.guild.is_empty() {
            bracketed(parts.guild);
        }
    } else {
        if let Some(line) = sub_name(parts) {
            bracketed(line);
        }
        if let Some(line) = owner_line(parts, word) {
            bracketed(&line);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(is_self: bool, player: bool, targeted: bool) -> Candidate {
        Candidate { unit: true, nameable: true, is_self, player, targeted }
    }

    /// The default names players and the target and nothing else. It is 1.12's
    /// default and not this client's choice: the CVars ship with
    /// `UnitNamePlayer = "1"` and `UnitNameOwn` and `UnitNameNPC` off.
    #[test]
    fn the_default_names_players_and_the_target_and_nobody_else() {
        let policy = Policy::default();
        assert!(policy.names(&unit(false, true, false)), "another player");
        assert!(!policy.names(&unit(false, false, false)), "a wolf");
        assert!(policy.names(&unit(false, false, true)), "the wolf you clicked");
        assert!(!policy.names(&unit(true, true, false)), "yourself");
    }

    /// The target clause outranks the mask and does not outrank `own`. The
    /// 1.12.1 client tests self first and target second, so targeting yourself
    /// still obeys `UnitNameOwn`.
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

    /// A chest is not an NPC, and `UnitNameNPC` must not name one. It did
    /// before [`Candidate::unit`] existed, because this client keeps game
    /// objects in the same table as units and the 1.12.1 client's rule is
    /// applied only to units. A door's name is its tooltip; see
    /// [`Candidate::unit`].
    #[test]
    fn a_game_object_is_never_named_however_the_switches_stand() {
        let all = Policy { own: true, npc: true, player: true, guild: true, pvp_title: true };
        let mut chest = unit(false, false, false);
        chest.unit = false;
        assert!(!all.names(&chest));
        // Not even a targeted one. For a real unit the target clause outranks
        // every switch.
        chest.targeted = true;
        assert!(!all.names(&chest));
        assert!(!Policy::default().names(&chest));
    }

    /// The mask round-trips, which an options panel and a `Config.wtf` both
    /// need. Without this test a transposed bit would go unnoticed.
    #[test]
    fn the_mask_round_trips_and_matches_the_references_bits() {
        let policy = Policy::default();
        assert_eq!(policy.mask(), switches::PLAYER | switches::GUILD | switches::PVP_TITLE);
        assert_eq!(Policy::from_mask(policy.mask()), policy);
        let all = Policy { own: true, npc: true, player: true, guild: true, pvp_title: true };
        assert_eq!(all.mask(), 0x37);
        assert_eq!(Policy::from_mask(0x37), all);
    }

    /// A tall creature's name is physically larger, with a step at four yards.
    /// This rule is why [`size_for`] is a function and not a constant.
    #[test]
    fn a_creature_over_four_yards_tall_gets_a_larger_name() {
        assert_eq!(size_for(2.2), BASE_SIZE, "a human");
        assert_eq!(size_for(4.0), BASE_SIZE, "exactly four is not tall");
        let kodo = size_for(8.0);
        assert!((kodo - BASE_SIZE * 2.0 * 1.5).abs() < 1e-6, "{kodo}");
        assert!(kodo > BASE_SIZE);
    }

    /// The policy is read from the settings store under build 5875's own names,
    /// so a build placed in a real WoW folder reads that folder's `Config.wtf`.
    /// The spellings are what the file carries and what the interface asks by.
    #[test]
    fn the_policy_is_the_five_cvars_under_the_names_the_client_registered() {
        // What the client's own registrations come to, which is `Default`.
        let registered = |name: &str| matches!(name, "UnitNamePlayer" | "UnitNamePlayerGuild" | "UnitNamePlayerPVPTitle");
        assert_eq!(Policy::from_cvars(registered), Policy::default());
        // A file that turned NPC names on changes exactly one field.
        let with_npcs = |name: &str| registered(name) || name == "UnitNameNPC";
        assert_eq!(Policy::from_cvars(with_npcs), Policy { npc: true, ..Policy::default() });
        assert_eq!(Policy::from_cvars(|_| false), Policy::from_mask(0));
    }

    /// The keys the name text reads, with the shipped file's values.
    fn words(key: &str) -> Option<String> {
        Some(
            match key {
                "CHAT_FLAG_AFK" => "<AFK>",
                "CHAT_FLAG_DND" => "<DND>",
                "UNIT_PVP_NAME" => "%s %s",
                "PVP_RANK_6_1" => "Corporal",
                "PVP_RANK_6_0" => "Grunt",
                "PVP_RANK_6_0_FEMALE" => "Grunt (f)",
                "PVP_MEDAL1" => "Protector of Stormwind",
                "UNITNAME_TITLE_PET" => "%s's Pet",
                "UNITNAME_TITLE_MINION" => "%s's Minion",
                "UNITNAME_TITLE_GUARDIAN" => "%s's Guardian",
                "UNITNAME_TITLE_CREATION" => "%s's Creation",
                _ => return None,
            }
            .to_string(),
        )
    }

    /// A player's text is the tags, the rank and name, the city title and
    /// the guild, and the two switches remove the rank and the guild.
    #[test]
    fn a_players_text_is_tags_rank_name_title_and_guild() {
        let player = NameParts {
            name: "Dolgrin",
            player: true,
            player_flags: player_flags::AFK,
            pvp_rank: 6,
            pvp_medal: 1,
            team: Some(1),
            guild: "The Watch",
            // A player has neither of these lines, whatever the fields say.
            sub_name: "Innkeeper",
            owner: Some((OwnerTitle::Pet, "Cade")),
            ..NameParts::default()
        };
        assert_eq!(
            floating_text(&player, &Policy::default(), &words),
            "<AFK>Corporal Dolgrin\nProtector of Stormwind\n<The Watch>"
        );
        let plain = Policy { guild: false, pvp_title: false, ..Policy::default() };
        assert_eq!(floating_text(&player, &plain, &words), "<AFK>Dolgrin");
        // The tooltip's name has the rank whatever the switch says.
        assert_eq!(titled_name(&player, true, &words), "Corporal Dolgrin\nProtector of Stormwind");
    }

    /// The rank key takes the team, and a female character takes the
    /// `_FEMALE` form when the file has one.
    #[test]
    fn the_rank_title_is_chosen_by_team_and_gender() {
        let mut orc = NameParts {
            name: "Grasha",
            player: true,
            pvp_rank: 6,
            team: Some(0),
            female: true,
            ..NameParts::default()
        };
        assert_eq!(titled_name(&orc, true, &words), "Grunt (f) Grasha");
        orc.female = false;
        assert_eq!(titled_name(&orc, true, &words), "Grunt Grasha");
        orc.team = Some(1);
        orc.female = true;
        assert_eq!(titled_name(&orc, true, &words), "Corporal Grasha", "no female form");
        orc.pvp_rank = 0;
        assert_eq!(titled_name(&orc, true, &words), "Grasha");
        orc.pvp_rank = 6;
        orc.team = None;
        assert_eq!(titled_name(&orc, true, &words), "Grasha", "on neither side");
        assert_eq!(team_of_group(5), Some(0));
        assert_eq!(team_of_group(3), Some(1));
        assert_eq!(team_of_group(1), None);
    }

    /// A creature's text is its name, its subname and its owner line, and no
    /// switch removes either line. A unit with a pet number shows no subname.
    #[test]
    fn a_creatures_text_is_name_subname_and_owner() {
        let nothing = Policy::from_mask(0);
        let innkeeper = NameParts { name: "Farley", sub_name: "Innkeeper", ..NameParts::default() };
        assert_eq!(floating_text(&innkeeper, &nothing, &words), "Farley\n<Innkeeper>");
        let wolf = NameParts {
            name: "Growlfang",
            sub_name: "Wolf",
            pet_number: 7,
            owner: Some((OwnerTitle::Pet, "Dolgrin")),
            // A creature has no guild line and no tags.
            guild: "The Watch",
            player_flags: player_flags::AFK,
            ..NameParts::default()
        };
        assert_eq!(floating_text(&wolf, &nothing, &words), "Growlfang\n<Dolgrin's Pet>");
        assert_eq!(owner_line(&wolf, &words).as_deref(), Some("Dolgrin's Pet"));
        assert_eq!(sub_name(&wolf), None);
        assert_eq!(sub_name(&innkeeper), Some("Innkeeper"));
    }

    /// The creating spell's effect decides a guardian and a creation, a
    /// charmed unit ignores it, and every other owned unit is a pet when it
    /// is a beast and a minion when it is not.
    #[test]
    fn the_owner_title_is_chosen_by_effect_then_creature_type() {
        assert_eq!(OwnerTitle::of(false, Some(42), 3), OwnerTitle::Guardian);
        assert_eq!(OwnerTitle::of(false, Some(74), 11), OwnerTitle::Creation);
        assert_eq!(OwnerTitle::of(false, Some(105), 11), OwnerTitle::Creation);
        assert_eq!(OwnerTitle::of(false, Some(56), 1), OwnerTitle::Pet);
        assert_eq!(OwnerTitle::of(false, Some(56), 3), OwnerTitle::Minion);
        assert_eq!(OwnerTitle::of(false, None, 3), OwnerTitle::Minion);
        assert_eq!(OwnerTitle::of(true, Some(42), 1), OwnerTitle::Pet, "charmed");
        assert_eq!(OwnerTitle::of(true, Some(74), 7), OwnerTitle::Minion, "charmed");
        assert_eq!(OwnerTitle::Guardian.key(), "UNITNAME_TITLE_GUARDIAN");
    }
}
