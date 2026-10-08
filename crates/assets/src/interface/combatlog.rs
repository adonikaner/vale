//! The combat log rules: which sentence, in which window, about which units.
//!
//! In 1.12 the server never sends combat text. It sends binary statements (a
//! swing landed, a spell did 340 shadow damage, a heal crit), and the client
//! composes the English from `GlobalStrings.lua` and pushes it into the chat
//! system under one of the 94 chat types. Every rule in this file is therefore
//! a client rule with no packet or data file behind it, and each follows the
//! 1.12.1 client. A wrong rule here produces a grammatical, plausible sentence
//! about the wrong two units.
//!
//! ## The four decisions
//!
//! Every combat line answers the same four independent questions:
//!
//! 1. What is each unit to the local player? [`Category`], ten values.
//! 2. Which window does the line go in? [`chat_type`], six routing functions
//!    that differ only in a base id.
//! 3. Which `GlobalStrings` key? [`Perspective`] plus the family's stem.
//! 4. What goes in the key's slots? [`Perspective`] again, because a key that
//!    mentions two units takes two names.
//!
//! [`Trailers`] then appends the parenthesised clauses.
//!
//! ## What is not in this module
//!
//! Unit names. This module produces a key and an argument order; resolving a
//! guid to "Kobold Vermin" is the object manager's job, and composition happens
//! in `crates/client`. That split lets `vale combatlog` check every key in
//! the file against the archives with no server and no window.

use super::strings::{substitute_all, Strings};

/// What a unit is to the local player: the ten categories the 1.12.1 client
/// sorts units into.
///
/// They come in pairs: a unit and its pet are adjacent, with the low bit set
/// for the pet, so the routing functions can treat `2` and `3` almost alike.
/// `0` is always the local player, so [`Perspective`] is only a test of `== 0`.
///
/// The names come from the seven range CVars
/// (`CombatLogRangeParty`, `CombatLogRangePartyPet`, …), one per category in
/// this order. The client exposes no other names for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Category {
    /// The local player.
    You = 0,
    YourPet = 1,
    Party = 2,
    PartyPet = 3,
    FriendlyPlayer = 4,
    FriendlyPlayerPet = 5,
    HostilePlayer = 6,
    HostilePlayerPet = 7,
    /// A creature owned by nobody, and anything whose owner resolves to one.
    Creature = 8,
    /// A guid the object manager cannot resolve. This is a valid category, not
    /// an error: the 1.12.1 client assigns 9 in four different cases, and the
    /// routing treats it exactly as [`Category::Creature`].
    Unknown = 9,
}

impl Category {
    /// Every category, in id order.
    pub const ALL: [Category; 10] = [
        Category::You,
        Category::YourPet,
        Category::Party,
        Category::PartyPet,
        Category::FriendlyPlayer,
        Category::FriendlyPlayerPet,
        Category::HostilePlayer,
        Category::HostilePlayerPet,
        Category::Creature,
        Category::Unknown,
    ];

    pub fn id(self) -> u8 {
        self as u8
    }

    pub fn from_id(id: u8) -> Option<Category> {
        Category::ALL.get(id as usize).copied()
    }

    /// Whether this is the local player.
    ///
    /// This is the only test [`Perspective`] makes. Your pet is therefore
    /// "other": the game says "Your Voidwalker hits X", not "You hit X".
    pub fn is_you(self) -> bool {
        self == Category::You
    }

    /// The CVar that bounds how far away this category is still logged, and
    /// the value 5875 registers it with.
    ///
    /// The 1.12.1 client reads the range CVar for the category. You and your
    /// pet have no range CVar, so they get the "no limit" value (100000.0) and
    /// are always logged. So is [`Category::Unknown`], which has no range CVar
    /// either: an unresolvable unit is logged, not dropped.
    pub fn range_cvar(self) -> Option<(&'static str, f32)> {
        match self {
            Category::You | Category::YourPet | Category::Unknown => None,
            Category::Party => Some(("CombatLogRangeParty", 50.0)),
            Category::PartyPet => Some(("CombatLogRangePartyPet", 50.0)),
            Category::FriendlyPlayer => Some(("CombatLogRangeFriendlyPlayers", 50.0)),
            Category::FriendlyPlayerPet => Some(("CombatLogRangeFriendlyPlayersPets", 50.0)),
            Category::HostilePlayer => Some(("CombatLogRangeHostilePlayers", 50.0)),
            Category::HostilePlayerPet => Some(("CombatLogRangeHostilePlayersPets", 50.0)),
            Category::Creature => Some(("CombatLogRangeCreature", 30.0)),
        }
    }

    /// How far away this category is still logged, in yards.
    ///
    /// [`NO_RANGE_LIMIT`] for the three with no CVar. The client tests both
    /// units of a blow, and either one out of range drops the whole line. A
    /// fight where one unit is out of range is not logged at all, as in the
    /// 1.12.1 client.
    pub fn range_yards(self) -> f32 {
        self.range_cvar().map_or(NO_RANGE_LIMIT, |(_, yards)| yards)
    }
}

/// The range the client uses for a category with no CVar: a finite value
/// rather than an infinity.
pub const NO_RANGE_LIMIT: f32 = 100_000.0;

/// The facts about a unit that [`categorise`] needs.
///
/// Six facts in a struct rather than the unit itself, so that the cascade can
/// be tested with no object manager, no session and no window. The renderer
/// resolves the facts; this module decides what they mean.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Standing {
    /// The unit is somebody's pet, guardian or charm rather than its own
    /// master.
    ///
    /// A unit that is its own master leaves this false and puts its own facts
    /// in the `owner_*` fields, which lets the rest of the cascade treat both
    /// cases the same way.
    pub is_pet: bool,
    /// The master, or the unit itself when it has none, resolved to an object
    /// the client holds.
    pub owner_known: bool,
    /// That master is the local player.
    pub owner_is_you: bool,
    /// That master is a player character rather than a creature.
    pub owner_is_player: bool,
    /// That master is in the local player's party or raid.
    pub owner_in_party: bool,
    /// Mutually hostile with the local player: hostility is tested twice,
    /// once each way. One-sided hostility (a flagged player you
    /// are not flagged against) is not a hostile-player line.
    pub owner_hostile: bool,
}

/// What a unit is to the local player, as a cascade.
///
/// The order is the client's and matters: your own first, then hostility, then
/// the party, then everything else. A creature with no owner is
/// [`Category::Creature`], and a guid the client cannot resolve is
/// [`Category::Unknown`], which the routing treats identically. An entity that
/// has not streamed in yet therefore produces a line about a creature rather
/// than no line, as in the 1.12.1 client.
pub fn categorise(standing: Standing) -> Category {
    // A pet is its master's category with the low bit set, which is why the
    // ten values are five pairs.
    let pet = standing.is_pet;
    let pair = |unit: Category, with_pet: Category| if pet { with_pet } else { unit };
    if !standing.owner_known {
        return Category::Unknown;
    }
    if standing.owner_is_you {
        // The 1.12.1 client tells your own unit from your pet by bit 5 of the
        // unit's type mask, not by a separate ownership test.
        return pair(Category::You, Category::YourPet);
    }
    if !standing.owner_is_player {
        return Category::Creature;
    }
    if standing.owner_hostile {
        return pair(Category::HostilePlayer, Category::HostilePlayerPet);
    }
    if standing.owner_in_party {
        return pair(Category::Party, Category::PartyPet);
    }
    pair(Category::FriendlyPlayer, Category::FriendlyPlayerPet)
}

/// Which of the six routing tables a line goes through.
///
/// The six families route the same way and differ only in a base chat type
/// id: 27, 28, 46, 47, 70 and 71. The base is the id of the `…_SELF_…` row,
/// and every other arm is that row plus an even offset, so [`chat_type`] is
/// one table rather than six.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// A weapon swing that landed. `COMBAT_SELF_HITS` and its seven siblings.
    MeleeHit,
    /// A weapon swing that did not land: a miss, a dodge, a parry, a block, an
    /// immunity.
    MeleeMiss,
    /// A spell that did damage, including the direct half of a
    /// damage-over-time.
    SpellDamage,
    /// A spell that did anything else: a heal, an aura going on or coming off,
    /// a dispel, an interrupt.
    SpellBuff,
    /// A tick of a damage-over-time.
    PeriodicDamage,
    /// A tick of any other periodic effect: a heal over time, a drain.
    PeriodicBuff,
}

impl Family {
    /// The `…_SELF_…` row's id, which every other arm is measured from.
    pub fn base(self) -> u8 {
        match self {
            Family::MeleeHit => 27,
            Family::MeleeMiss => 28,
            Family::SpellDamage => 46,
            Family::SpellBuff => 47,
            Family::PeriodicDamage => 70,
            Family::PeriodicBuff => 71,
        }
    }

    /// The periodic families route on the attacker alone, and have five arms
    /// where the others have eight.
    ///
    /// The two periodic families map the attacker's category to `[base, base,
    /// +2, +2, +4, +4, +6, +6, +8, +8]` whatever the victim is. There is no
    /// `SPELL_PERIODIC_CREATURE_VS_PARTY_DAMAGE` row to select, because the
    /// chat type table has no such rows.
    fn is_periodic(self) -> bool {
        matches!(self, Family::PeriodicDamage | Family::PeriodicBuff)
    }
}

/// The chat type a line about these two units goes to: the six families'
/// routing, as one table.
///
/// Returns [`chattype::NONE`](super::chattype::NONE) (94) for a pairing that
/// has no window. It is not an error; the caller drops the line.
///
/// Two arms are conditional, and neither follows from the naming. A party
/// member or a friendly player whose blow lands on you, your pet, or anyone
/// else in your party is logged as a hostile player. In 1.12 that happens only
/// in a duel, under mind control, or in a battleground, all of which are
/// player-versus-player. The pet arms next to them (`3` and `5`) have no such
/// test; that asymmetry is the client's and is kept here.
pub fn chat_type(family: Family, attacker: Category, victim: Category) -> u8 {
    let base = family.base();
    let victim_is_my_side = matches!(
        victim,
        Category::You | Category::YourPet | Category::Party | Category::PartyPet
    );
    if family.is_periodic() {
        // `[0,0,2,2,4,4,6,6,8,8]`: the attacker's pair index, doubled.
        return base + (attacker.id() / 2) * 2;
    }
    match attacker {
        Category::You => base,
        Category::YourPet => base + 2,
        Category::Party if victim_is_my_side => base + 8,
        Category::Party => base + 4,
        Category::PartyPet => base + 4,
        Category::FriendlyPlayer if victim_is_my_side => base + 8,
        Category::FriendlyPlayer => base + 6,
        Category::FriendlyPlayerPet => base + 6,
        Category::HostilePlayer | Category::HostilePlayerPet => base + 8,
        // The creature arms route on the victim's pair, `[0,0,1,1,2,2,2,2,2,2]`
        // by victim category.
        Category::Creature | Category::Unknown => match victim {
            Category::You | Category::YourPet => base + 10,
            Category::Party | Category::PartyPet => base + 12,
            _ => base + 14,
        },
    }
}

/// Which of a family's three or four keys applies, and how many names it takes.
///
/// Two tests produce a suffix and a number 0..3. The number decides how many
/// names are substituted, so the key and the argument list are one decision.
/// A client that gets this wrong prints the victim's name where the attacker's
/// belongs, and the sentence still reads correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Perspective {
    /// Both units are you. There is no line unless the family has a
    /// `SELFSELF` key. A melee swing cannot
    /// reach this, since you cannot swing at yourself, but a spell can, and
    /// `SPELLLOGSELFSELF` exists for those.
    None = 0,
    /// Somebody else did it to you. One name: theirs.
    OtherSelf = 1,
    /// You did it to somebody else. One name: theirs.
    SelfOther = 2,
    /// Two other people. Two names, attacker first.
    OtherOther = 3,
}

impl Perspective {
    /// The two tests: whether the attacker is you, and whether the victim is.
    pub fn of(attacker: Category, victim: Category) -> Perspective {
        match (attacker.is_you(), victim.is_you()) {
            (true, true) => Perspective::None,
            (true, false) => Perspective::SelfOther,
            (false, true) => Perspective::OtherSelf,
            (false, false) => Perspective::OtherOther,
        }
    }

    /// The suffix this perspective puts on a family's stem.
    ///
    /// [`Perspective::None`] gives `SELFSELF`; [`key`] decides whether the
    /// family has a `…SELFSELF` key at all.
    pub fn suffix(self) -> Option<&'static str> {
        match self {
            Perspective::None => Some("SELFSELF"),
            Perspective::OtherSelf => Some("OTHERSELF"),
            Perspective::SelfOther => Some("SELFOTHER"),
            Perspective::OtherOther => Some("OTHEROTHER"),
        }
    }

    /// Whether the attacker is named in this perspective's key.
    ///
    /// A perspective is two bits, this and [`Self::names_victim`]: the key
    /// mentions whichever unit is not you, and the line takes a name for each
    /// unit the key mentions.
    pub fn names_attacker(self) -> bool {
        matches!(self, Perspective::OtherSelf | Perspective::OtherOther)
    }

    pub fn names_victim(self) -> bool {
        matches!(self, Perspective::SelfOther | Perspective::OtherOther)
    }
}

/// One slot of a family's format string, in the order the family writes them.
///
/// A fixed "names first, then the numbers" order does not work, because the
/// spell families put the spell's name between the two units.
///
/// ```text
/// COMBATHITOTHEROTHER  = "%s hits %s for %d."          attacker, victim, damage
/// SPELLLOGOTHEROTHER   = "%s's %s hits %s for %d."     attacker, SPELL, victim, damage
/// IMMUNESPELLOTHEROTHER = "%s is immune to %s's %s."   VICTIM, attacker, spell
/// POWERGAINOTHEROTHER  = "%s gains %d %s from %s's %s." victim, amount, power, attacker, spell
/// ```
///
/// Four families have four different orders. A composer that assumed one order
/// and filled left to right would put a creature's name where a spell's belongs
/// in three of the four, and the result would still read as a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The attacker's name, omitted when the attacker is you.
    Attacker,
    /// The victim's name, omitted when the victim is you.
    Victim,
    /// Something the family always writes: a spell's name, a damage figure, a
    /// school, a power type. The index is into `extras`.
    Extra(usize),
}

/// The slot order of the families this client composes.
pub mod parts {
    use super::Part::{self, *};

    /// `"%s hits %s for %d[ %s damage]."`: the melee families.
    pub const MELEE: &[Part] = &[Attacker, Victim, Extra(0), Extra(1)];
    /// `"%s's %s hits %s for %d[ %s damage]."`: every spell family. The
    /// spell's name is between the two units.
    pub const SPELL: &[Part] = &[Attacker, Extra(0), Victim, Extra(1), Extra(2)];
    /// `"%s is immune to %s's %s."`: the victim comes first, which no other
    /// family does.
    pub const IMMUNE: &[Part] = &[Victim, Attacker, Extra(0)];
    /// `"%s gains %d %s from %s's %s."`: `POWERGAIN`, whose two numbers are
    /// between the two names.
    pub const POWER_GAIN: &[Part] = &[Victim, Extra(0), Extra(1), Attacker, Extra(2)];
    /// `"%s's %s drains %d %s from %s."`: `SPELLPOWERDRAIN`, which is
    /// [`SPELL`] with two numbers rather than one before the victim.
    pub const POWER_DRAIN: &[Part] = &[Attacker, Extra(0), Extra(1), Extra(2), Victim];
    /// `"%s interrupts %s's %s."`: `SPELLINTERRUPT`. The spell slot is the
    /// spell that was stopped, and it comes after both names. The interrupting
    /// spell is not in the sentence.
    pub const INTERRUPT: &[Part] = &[Attacker, Victim, Extra(0)];
    /// `"%s reflects %d %s damage to %s."`: `DAMAGESHIELD`, which names no
    /// spell: the shield is an aura, and the line is about the blow it
    /// answered.
    pub const DAMAGE_SHIELD: &[Part] = &[Attacker, Extra(0), Extra(1), Victim];
    /// `"%s casts %s on %s's %s."`: `ITEMENCHANTMENTADD`, the only family whose
    /// second name is an item's owner rather than a victim, and whose last
    /// slot is the item. The spell sits between the two names as it does in
    /// [`SPELL`], and the item follows both.
    pub const ENCHANT: &[Part] = &[Attacker, Extra(0), Victim, Extra(1)];
}

/// The subject of a family about one unit rather than two, whose keys are
/// suffixed `SELF` or `OTHER` instead of by a [`Perspective`].
///
/// `UNITDIESSELF`/`UNITDIESOTHER`, `SPELLEXTRAATTACKSSELF`/`…OTHER`,
/// `AURADISPELSELF`/`…OTHER`, and the twelve
/// `VSENVIRONMENTALDAMAGE_<TYPE>_<SELF|OTHER>` keys. There is no attacker and
/// victim pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    You,
    Other,
}

impl Subject {
    pub fn of(category: Category) -> Subject {
        if category.is_you() {
            Subject::You
        } else {
            Subject::Other
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Subject::You => "SELF",
            Subject::Other => "OTHER",
        }
    }

    /// The key, and whether the subject's name is one of its arguments.
    ///
    /// `"You die."` takes no name and `"%s dies."` takes one.
    pub fn key(self, stem: &str) -> String {
        format!("{stem}{}", self.suffix())
    }

    pub fn names_subject(self) -> bool {
        self == Subject::Other
    }
}

/// The ten `Spell.dbc` effects whose summons are "destroyed" rather than
/// dying.
///
/// `UNITDESTROYEDOTHER` ("%s is destroyed.") is in `GlobalStrings.lua`, and the
/// choice of it does not depend on creature type. It depends on the unit's
/// `UNIT_CREATED_BY_SPELL` (update field 146), looked up in `Spell.dbc` and
/// reduced to its `Effect[0]` (field 61); only effects 50..107 can qualify.
///
/// The ten effects that qualify are listed below: a portal, a totem,
/// the four totem slots and the four object slots. Each is a summoned object.
/// Nothing else in the game "is destroyed".
///
/// A unit that was not summoned by a spell reads 0 here, has no `Spell.dbc`
/// record to look up, and dies like any other unit.
pub const DESTROYED_BY_EFFECT: [u32; 10] = [
    50,  // SPELL_EFFECT_TRANS_DOOR: a summoned portal or door
    74,  // SPELL_EFFECT_SUMMON_TOTEM
    87, 88, 89, 90,      // SPELL_EFFECT_SUMMON_TOTEM_SLOT1..4
    104, 105, 106, 107,  // SPELL_EFFECT_SUMMON_OBJECT_SLOT1..4
];

/// Whether this unit "is destroyed" rather than "dies"; see
/// [`DESTROYED_BY_EFFECT`].
///
/// `None` means a unit nothing summoned, or one whose summoning spell this
/// client cannot resolve; the 1.12.1 client says "dies" for both.
pub fn is_destroyed(created_by_effect: Option<u32>) -> bool {
    created_by_effect.is_some_and(|effect| DESTROYED_BY_EFFECT.contains(&effect))
}

/// The window a death goes in.
///
/// Categories 0..5 (you, your pet, your party and their pets, and any friendly
/// player and theirs) are a friendly death; everything above is a hostile one.
/// The same creature dying is therefore a different colour depending on whose
/// it was, and the split is where the category pairs stop being your side.
pub fn death_chat_type(who: Category) -> u8 {
    // 43 and 44 are the two `COMBAT_*_DEATH` rows.
    if who.id() <= 5 {
        43
    } else {
        44
    }
}

/// The six types of environmental damage, in the order
/// `EnvironmentalDamageType` numbers them, which is the value
/// `SMSG_ENVIRONMENTALDAMAGELOG`'s type byte carries.
///
/// The key is `VSENVIRONMENTALDAMAGE_<this>_<SELF|OTHER>`. This is the only
/// family in the log with an infix; every other family puts everything in the
/// stem.
pub const ENVIRONMENTAL: [&str; 6] = [
    "FATIGUE", "DROWNING", "FALLING", "LAVA", "SLIME", "FIRE",
];

/// The key for one environmental death, or `None` for a type byte outside the
/// six.
pub fn environmental_key(damage_type: u8, subject: Subject) -> Option<String> {
    let name = ENVIRONMENTAL.get(damage_type as usize)?;
    Some(format!(
        "VSENVIRONMENTALDAMAGE_{name}_{}",
        subject.suffix()
    ))
}

/// Fill a family's slot order for one perspective, dropping the names the key
/// does not mention.
pub fn arrange<'a>(
    order: &[Part],
    perspective: Perspective,
    attacker: &'a str,
    victim: &'a str,
    extras: &[&'a str],
) -> Vec<&'a str> {
    order
        .iter()
        .filter_map(|part| match part {
            Part::Attacker => perspective.names_attacker().then_some(attacker),
            Part::Victim => perspective.names_victim().then_some(victim),
            // A family whose longest key has a school slot still has keys
            // without one. A missing extra contributes nothing; later extras
            // keep their own indices.
            Part::Extra(i) => extras.get(*i).copied(),
        })
        .collect()
}

/// The `GlobalStrings` key for a family stem and a perspective.
///
/// `has_self_self` cannot be derived from the stem: a melee swing has three
/// keys (`COMBATHITSELFOTHER`, `…OTHERSELF`, `…OTHEROTHER`) and a spell has
/// four, and only the file shows which. A family with no `SELFSELF` key
/// returns `None` for [`Perspective::None`], and the 1.12.1 client prints no
/// line in that case.
pub fn key(stem: &str, perspective: Perspective, has_self_self: bool) -> Option<String> {
    if perspective == Perspective::None && !has_self_self {
        return None;
    }
    perspective.suffix().map(|suffix| format!("{stem}{suffix}"))
}

/// The parenthesised clauses that follow a damage line, in the order the
/// client appends them.
///
/// The order is neither alphabetical nor the order the fields arrive in: it is
/// glancing, crushing, resist-or-vulnerability, block, absorb. It is kept
/// because players recognise these clauses in a scrolling window by position.
///
/// A negative resist is a different clause. The client tests the sign and uses
/// `VULNERABLE_TRAILER` for a negative value (" (+%d vulnerability bonus)"),
/// which is extra damage the victim took, not damage resisted. Printing `abs()`
/// into the resist clause would report a vulnerability as a resistance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Trailers {
    pub glancing: bool,
    pub crushing: bool,
    /// Positive is resisted, negative is a vulnerability bonus.
    pub resisted: i32,
    pub blocked: u32,
    pub absorbed: u32,
}

impl Trailers {
    /// Append every clause that applies, in the client's order.
    ///
    /// A key the shipped file does not carry contributes nothing, as in the
    /// 1.12.1 client: a clause whose string is missing or empty is skipped.
    pub fn append(&self, strings: &Strings, line: &mut String) {
        let mut plain = |key: &str| {
            if let Some(text) = strings.get(key).filter(|t| !t.is_empty()) {
                line.push_str(text);
            }
        };
        if self.glancing {
            plain("GLANCING_TRAILER");
        }
        if self.crushing {
            plain("CRUSHING_TRAILER");
        }
        let mut counted = |key: &str, amount: i32| {
            if let Some(text) = strings.get(key).filter(|t| !t.is_empty()) {
                line.push_str(&substitute_all(text, &[&amount.to_string()]));
            }
        };
        match self.resisted.cmp(&0) {
            std::cmp::Ordering::Greater => counted("RESIST_TRAILER", self.resisted),
            // The value is negated: the clause reads "(+8 vulnerability
            // bonus)", not "(+-8 …)". The negation is easy to omit in a
            // reimplementation.
            std::cmp::Ordering::Less => {
                counted("VULNERABLE_TRAILER", self.resisted.saturating_neg())
            }
            std::cmp::Ordering::Equal => {}
        }
        if self.blocked != 0 {
            counted("BLOCK_TRAILER", self.blocked as i32);
        }
        if self.absorbed != 0 {
            counted("ABSORB_TRAILER", self.absorbed as i32);
        }
    }
}

/// The two `HitInfo` flags that produce a trailer.
///
/// `hit_info::CRUSHING` is also named in `vale_protocol`, because it selects
/// one of the victim's three wound voices. The glancing bit is named nowhere
/// else in this project, because a glancing blow has no pose and no sound; its
/// only effect is this clause.
pub mod hit_info {
    /// `HITINFO_GLANCING`.
    pub const GLANCING: u32 = 0x0000_4000;
    /// `HITINFO_CRUSHING`.
    pub const CRUSHING: u32 = 0x0000_8000;
}

/// One kind of combat message: everything [`compose`] needs except the two
/// units and the numbers.
///
/// The catalogue below is what this client composes. It is a list rather than
/// a match because two callers walk it: the renderer, which picks a row and
/// fills it, and `vale combatlog`, which checks every key every row can
/// produce against the shipped `GlobalStrings.lua`. A stem whose keys are not
/// in the file produces no line, and nothing else in this client reports that.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kind {
    /// What to call this in a report. Not a key and not on screen.
    pub label: &'static str,
    pub stem: &'static str,
    pub family: Family,
    pub order: &'static [Part],
    /// Whether the file carries a `…SELFSELF` key for this stem.
    pub self_self: bool,
}

/// Every two-unit kind this client composes.
///
/// The four melee stems come from one packet. A swing arrives as
/// `SMSG_ATTACKERSTATEUPDATE`, and the stem is chosen from two facts in it:
/// `HITINFO_CRITICALHIT`, and whether the damage had a school other than
/// physical. Each of the four combinations has its own stem.
pub const KINDS: &[Kind] = &[
    // --- the weapon swing: `SMSG_ATTACKERSTATEUPDATE` ---
    Kind { label: "a swing", stem: "COMBATHIT", family: Family::MeleeHit, order: parts::MELEE, self_self: false },
    Kind { label: "a critical swing", stem: "COMBATHITCRIT", family: Family::MeleeHit, order: parts::MELEE, self_self: false },
    Kind { label: "a swing with a school", stem: "COMBATHITSCHOOL", family: Family::MeleeHit, order: parts::MELEE, self_self: false },
    Kind { label: "a critical swing with a school", stem: "COMBATHITCRITSCHOOL", family: Family::MeleeHit, order: parts::MELEE, self_self: false },
    // The nine ways a swing can fail, from `VictimState` and `HITINFO_MISS`.
    Kind { label: "a miss", stem: "MISSED", family: Family::MeleeMiss, order: parts::MELEE, self_self: false },
    Kind { label: "a dodge", stem: "VSDODGE", family: Family::MeleeMiss, order: parts::MELEE, self_self: false },
    Kind { label: "a parry", stem: "VSPARRY", family: Family::MeleeMiss, order: parts::MELEE, self_self: false },
    Kind { label: "a full block", stem: "VSBLOCK", family: Family::MeleeMiss, order: parts::MELEE, self_self: false },
    Kind { label: "a full absorb", stem: "VSABSORB", family: Family::MeleeMiss, order: parts::MELEE, self_self: false },
    Kind { label: "a full resist", stem: "VSRESIST", family: Family::MeleeMiss, order: parts::MELEE, self_self: false },
    Kind { label: "an immunity", stem: "VSIMMUNE", family: Family::MeleeMiss, order: parts::MELEE, self_self: false },
    Kind { label: "an evade", stem: "VSEVADE", family: Family::MeleeMiss, order: parts::MELEE, self_self: false },
    Kind { label: "a deflect", stem: "VSDEFLECT", family: Family::MeleeMiss, order: parts::MELEE, self_self: false },
    // --- a spell landing: `SMSG_SPELLNONMELEEDAMAGELOG` ---
    Kind { label: "a spell", stem: "SPELLLOG", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a critical spell", stem: "SPELLLOGCRIT", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a spell with a school", stem: "SPELLLOGSCHOOL", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a critical spell with a school", stem: "SPELLLOGCRITSCHOOL", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a fully absorbed spell", stem: "SPELLLOGABSORB", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    // --- a heal: `SMSG_SPELLHEALLOG` ---
    Kind { label: "a heal", stem: "HEALED", family: Family::SpellBuff, order: parts::SPELL, self_self: true },
    Kind { label: "a critical heal", stem: "HEALEDCRIT", family: Family::SpellBuff, order: parts::SPELL, self_self: true },
    // --- a spell that did not land: `SMSG_SPELLLOGMISS` ---
    Kind { label: "a spell miss", stem: "SPELLMISS", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a spell resist", stem: "SPELLRESIST", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a spell dodged", stem: "SPELLDODGED", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a spell parried", stem: "SPELLPARRIED", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a spell blocked", stem: "SPELLBLOCKED", family: Family::SpellDamage, order: parts::SPELL, self_self: false },
    Kind { label: "a spell evaded", stem: "SPELLEVADED", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a spell deflected", stem: "SPELLDEFLECTED", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "a spell reflected", stem: "SPELLREFLECT", family: Family::SpellDamage, order: parts::SPELL, self_self: true },
    Kind { label: "an immunity to a spell", stem: "IMMUNESPELL", family: Family::SpellDamage, order: parts::IMMUNE, self_self: true },
    // --- power: `SMSG_SPELLENERGIZELOG` and the drain half of the aura log ---
    Kind { label: "a power gain", stem: "POWERGAIN", family: Family::SpellBuff, order: parts::POWER_GAIN, self_self: true },
    Kind { label: "a power drain", stem: "SPELLPOWERDRAIN", family: Family::SpellDamage, order: parts::POWER_DRAIN, self_self: true },
    // --- a damage shield: `SMSG_SPELLDAMAGESHIELD` ---
    Kind { label: "a damage shield", stem: "DAMAGESHIELD", family: Family::SpellDamage, order: parts::DAMAGE_SHIELD, self_self: false },
    // --- two effects of `SMSG_SPELLLOGEXECUTE` that name two units ---
    //
    // Neither has a `…SELFSELF` key, and neither could use one: you cannot
    // interrupt your own cast, and the durability line is about another unit's
    // gear. `SPELLDURABILITYDAMAGE` takes [`parts::SPELL`]'s order with the
    // damaged item where the damage figure would be ("%s casts %s on %s: %s
    // damaged."), and its `…ALL…` variant has the same order with one fewer
    // extra.
    Kind { label: "an interrupted cast", stem: "SPELLINTERRUPT", family: Family::SpellDamage, order: parts::INTERRUPT, self_self: false },
    Kind { label: "durability damage", stem: "SPELLDURABILITYDAMAGE", family: Family::SpellDamage, order: parts::SPELL, self_self: false },
    Kind { label: "durability damage to everything", stem: "SPELLDURABILITYDAMAGEALL", family: Family::SpellDamage, order: parts::SPELL, self_self: false },
    // --- a proc resisted, `SMSG_PROCRESIST`, and a dispel that failed,
    // `SMSG_DISPEL_FAILED` ---
    //
    // The family listed is one of two: each line goes to the spell-damage or
    // the spell-buff windows by [`spell_window`]. `SMSG_SPELLORDAMAGE_IMMUNE`
    // composes `IMMUNESPELL`, listed above.
    Kind { label: "a proc resisted", stem: "PROCRESIST", family: Family::SpellDamage, order: parts::IMMUNE, self_self: true },
    Kind { label: "a dispel that failed", stem: "DISPELFAILED", family: Family::SpellDamage, order: parts::INTERRUPT, self_self: true },
];

/// The implicit targets that make [`spell_window`] answer the damage windows.
const HARMFUL_TARGETS: [u32; 8] = [2, 6, 15, 16, 24, 28, 53, 54];

/// Which family a `PROCRESIST` or `DISPELFAILED` line goes to, chosen from the
/// spell the line names. The 1.12.1 client classifies the spell, in order:
///
/// 1. `Targets` bit `0x100` makes it helpful, then bit `0x80` harmful;
/// 2. any effect with an implicit target (A or B) in [`HARMFUL_TARGETS`]
///    makes it harmful;
/// 3. a further test sorts the rest into helpful and neither, by implicit
///    targets and auras.
///
/// A harmful spell goes to [`Family::SpellDamage`]. A helpful one and one that
/// is neither both go to [`Family::SpellBuff`], so the third test does not
/// change the window and is not made here.
pub fn spell_window(spell: &crate::tables::spellbook::SpellInfo) -> Family {
    let harmful = if spell.targets & 0x100 != 0 {
        false
    } else if spell.targets & 0x80 != 0 {
        true
    } else {
        spell.effects.iter().any(|effect| {
            HARMFUL_TARGETS.contains(&effect.target_a) || HARMFUL_TARGETS.contains(&effect.target_b)
        })
    };
    if harmful {
        Family::SpellDamage
    } else {
        Family::SpellBuff
    }
}

/// One one-unit kind: a stem, and which of its two subjects the file carries.
///
/// Keyed by [`Subject`] rather than [`Perspective`], so each has at most two
/// keys rather than three or four. `has_self` cannot be derived from the stem.
/// `UNITDESTROYED` has only `…OTHER`, which agrees with [`is_destroyed`]: only
/// a summoned totem or portal "is destroyed", and that is never you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoloKind {
    pub label: &'static str,
    pub stem: &'static str,
    pub has_self: bool,
}

impl SoloKind {
    /// The subjects this stem has keys for, for a caller checking its keys.
    pub fn subjects(&self) -> &'static [Subject] {
        match self.has_self {
            true => &[Subject::You, Subject::Other],
            false => &[Subject::Other],
        }
    }
}

/// Every one-unit kind this client composes.
pub const SOLO_KINDS: &[SoloKind] = &[
    SoloKind { label: "a death", stem: "UNITDIES", has_self: true },
    SoloKind { label: "a construct destroyed", stem: "UNITDESTROYED", has_self: false },
    SoloKind { label: "extra attacks", stem: "SPELLEXTRAATTACKS", has_self: true },
    SoloKind { label: "an aura dispelled", stem: "AURADISPEL", has_self: true },
    SoloKind { label: "a spell that killed outright", stem: "INSTAKILL", has_self: true },
];

/// A composed line, and the window it goes in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// The chat type id: an index into
    /// [`chattype::TYPES`](super::chattype::TYPES).
    pub chat_type: u8,
    /// The sentence, including trailers.
    pub text: String,
}

/// Compose one line.
///
/// Applies the four decisions in the module comment, in the 1.12.1 client's
/// order. It returns `None` instead of guessing: an unroutable pairing, a
/// perspective with no key, or a key the shipped `GlobalStrings.lua` does not
/// carry all produce `None`, and the 1.12.1 client prints no line in those
/// cases either.
///
/// `order` is the family's slot order (one of [`parts`]), and `extras` is what
/// its [`Part::Extra`] slots take, in index order: a spell's name, the damage
/// figure, the school.
#[allow(clippy::too_many_arguments)]
pub fn compose(
    strings: &Strings,
    family: Family,
    stem: &str,
    has_self_self: bool,
    order: &[Part],
    attacker: (Category, &str),
    victim: (Category, &str),
    extras: &[&str],
    trailers: Trailers,
) -> Option<Line> {
    let chat_type = chat_type(family, attacker.0, victim.0);
    if chat_type >= super::chattype::NONE {
        return None;
    }
    let perspective = Perspective::of(attacker.0, victim.0);
    let key = key(stem, perspective, has_self_self)?;
    let format = strings.get(&key).filter(|t| !t.is_empty())?;

    let arguments = arrange(order, perspective, attacker.1, victim.1, extras);
    let mut text = substitute_all(format, &arguments);
    trailers.append(strings, &mut text);
    Some(Line { chat_type, text })
}

#[cfg(test)]
mod tests {
    use super::super::chattype::TYPES;
    use super::*;

    fn strings() -> Strings {
        Strings::parse(
            br#"
COMBATHITSELFOTHER = "You hit %s for %d.";
COMBATHITOTHERSELF = "%s hits you for %d.";
COMBATHITOTHEROTHER = "%s hits %s for %d.";
COMBATHITSCHOOLOTHEROTHER = "%s hits %s for %d %s damage.";
SPELLLOGSELFSELF = "Your %s hits you for %d.";
SPELLLOGSELFOTHER = "Your %s hits %s for %d.";
SPELLLOGOTHERSELF = "%s's %s hits you for %d.";
SPELLLOGOTHEROTHER = "%s's %s hits %s for %d.";
SPELLLOGSCHOOLOTHEROTHER = "%s's %s hits %s for %d %s damage.";
IMMUNESPELLOTHEROTHER = "%s is immune to %s's %s.";
IMMUNESPELLSELFOTHER = "%s is immune to your %s.";
IMMUNESPELLOTHERSELF = "You are immune to %s's %s.";
IMMUNESPELLSELFSELF = "You are immune to your %s.";
POWERGAINOTHEROTHER = "%s gains %d %s from %s's %s.";
POWERGAINOTHERSELF = "You gain %d %s from %s's %s.";
POWERGAINSELFOTHER = "%s gains %d %s from %s.";
POWERGAINSELFSELF = "You gain %d %s from %s.";
ABSORB_TRAILER = " (%d absorbed)";
BLOCK_TRAILER = " (%d blocked)";
RESIST_TRAILER = " (%d resisted)";
VULNERABLE_TRAILER = " (+%d vulnerability bonus)";
GLANCING_TRAILER = " (glancing)";
CRUSHING_TRAILER = " (crushing)";
"#,
        )
    }

    /// The eight arms of the melee-hit routing, against the names the
    /// chat-type table gives those ids.
    #[test]
    fn melee_hit_routing_matches_the_client() {
        use Category::*;
        let cases = [
            (You, Creature, "COMBAT_SELF_HITS"),
            (YourPet, Creature, "COMBAT_PET_HITS"),
            (Party, Creature, "COMBAT_PARTY_HITS"),
            (PartyPet, Creature, "COMBAT_PARTY_HITS"),
            (FriendlyPlayer, Creature, "COMBAT_FRIENDLYPLAYER_HITS"),
            (FriendlyPlayerPet, Creature, "COMBAT_FRIENDLYPLAYER_HITS"),
            (HostilePlayer, Creature, "COMBAT_HOSTILEPLAYER_HITS"),
            (HostilePlayerPet, Creature, "COMBAT_HOSTILEPLAYER_HITS"),
            (Creature, You, "COMBAT_CREATURE_VS_SELF_HITS"),
            (Creature, YourPet, "COMBAT_CREATURE_VS_SELF_HITS"),
            (Creature, Party, "COMBAT_CREATURE_VS_PARTY_HITS"),
            (Creature, PartyPet, "COMBAT_CREATURE_VS_PARTY_HITS"),
            (Creature, Creature, "COMBAT_CREATURE_VS_CREATURE_HITS"),
            (Unknown, Creature, "COMBAT_CREATURE_VS_CREATURE_HITS"),
        ];
        for (attacker, victim, expected) in cases {
            let id = chat_type(Family::MeleeHit, attacker, victim);
            assert_eq!(TYPES[id as usize].name, expected, "{attacker:?} -> {victim:?}");
        }
    }

    /// The two conditional arms: a party member or friendly player hitting your
    /// side is logged as hostile.
    #[test]
    fn a_friendly_blow_on_your_side_is_a_hostile_one() {
        use Category::*;
        for attacker in [Party, FriendlyPlayer] {
            for victim in [You, YourPet, Party, PartyPet] {
                let id = chat_type(Family::MeleeHit, attacker, victim);
                assert_eq!(
                    TYPES[id as usize].name, "COMBAT_HOSTILEPLAYER_HITS",
                    "{attacker:?} -> {victim:?}"
                );
            }
        }
        // The pet arms beside them have no such test.
        let id = chat_type(Family::MeleeHit, Category::PartyPet, Category::You);
        assert_eq!(TYPES[id as usize].name, "COMBAT_PARTY_HITS");
    }

    /// Every miss id is the matching hit id plus one.
    #[test]
    fn misses_are_hits_plus_one() {
        for attacker in Category::ALL {
            for victim in Category::ALL {
                assert_eq!(
                    chat_type(Family::MeleeMiss, attacker, victim),
                    chat_type(Family::MeleeHit, attacker, victim) + 1
                );
                assert_eq!(
                    chat_type(Family::SpellBuff, attacker, victim),
                    chat_type(Family::SpellDamage, attacker, victim) + 1
                );
            }
        }
    }

    /// The periodic families have five arms and route on the attacker alone.
    #[test]
    fn periodic_routing_ignores_the_victim() {
        use Category::*;
        for victim in Category::ALL {
            assert_eq!(
                TYPES[chat_type(Family::PeriodicDamage, You, victim) as usize].name,
                "SPELL_PERIODIC_SELF_DAMAGE"
            );
            assert_eq!(
                TYPES[chat_type(Family::PeriodicDamage, Creature, victim) as usize].name,
                "SPELL_PERIODIC_CREATURE_DAMAGE"
            );
            assert_eq!(
                TYPES[chat_type(Family::PeriodicBuff, HostilePlayer, victim) as usize].name,
                "SPELL_PERIODIC_HOSTILEPLAYER_BUFFS"
            );
        }
    }

    #[test]
    fn perspective_is_a_test_of_who_is_you() {
        use Category::*;
        assert_eq!(Perspective::of(You, Creature), Perspective::SelfOther);
        assert_eq!(Perspective::of(Creature, You), Perspective::OtherSelf);
        assert_eq!(Perspective::of(Creature, Party), Perspective::OtherOther);
        assert_eq!(Perspective::of(You, You), Perspective::None);
        // Your own pet is "other": the game says "Your Voidwalker hits X".
        assert_eq!(Perspective::of(YourPet, Creature), Perspective::OtherOther);
    }

    #[test]
    fn a_family_with_no_selfself_key_has_no_line() {
        assert_eq!(key("COMBATHIT", Perspective::None, false), None);
        assert_eq!(
            key("SPELLLOG", Perspective::None, true).as_deref(),
            Some("SPELLLOGSELFSELF")
        );
        assert_eq!(
            key("COMBATHIT", Perspective::OtherOther, false).as_deref(),
            Some("COMBATHITOTHEROTHER")
        );
    }

    const YOU: (Category, &str) = (Category::You, "Alden");
    const MOB: (Category, &str) = (Category::Creature, "Kobold Vermin");
    const MATE: (Category, &str) = (Category::Party, "Bram");

    #[test]
    fn composes_the_three_melee_perspectives() {
        let s = strings();

        let line = compose(
            &s,
            Family::MeleeHit,
            "COMBATHIT",
            false,
            parts::MELEE,
            YOU,
            MOB,
            &["12"],
            Trailers::default(),
        )
        .unwrap();
        assert_eq!(line.text, "You hit Kobold Vermin for 12.");
        assert_eq!(TYPES[line.chat_type as usize].name, "COMBAT_SELF_HITS");

        let line = compose(
            &s,
            Family::MeleeHit,
            "COMBATHIT",
            false,
            parts::MELEE,
            MOB,
            YOU,
            &["7"],
            Trailers::default(),
        )
        .unwrap();
        assert_eq!(line.text, "Kobold Vermin hits you for 7.");
        assert_eq!(
            TYPES[line.chat_type as usize].name,
            "COMBAT_CREATURE_VS_SELF_HITS"
        );

        // Attacker first, victim second: the order the key's slots take them.
        let line = compose(
            &s,
            Family::MeleeHit,
            "COMBATHIT",
            false,
            parts::MELEE,
            MATE,
            MOB,
            &["9"],
            Trailers::default(),
        )
        .unwrap();
        assert_eq!(line.text, "Bram hits Kobold Vermin for 9.");
    }

    /// The school slot comes after the two names, so that
    /// `"%s hits %s for %d %s damage."` fills correctly.
    #[test]
    fn the_school_slot_follows_the_names() {
        let s = strings();
        let line = compose(
            &s,
            Family::MeleeHit,
            "COMBATHITSCHOOL",
            false,
            parts::MELEE,
            MATE,
            MOB,
            &["9", "Fire"],
            Trailers::default(),
        )
        .unwrap();
        assert_eq!(line.text, "Bram hits Kobold Vermin for 9 Fire damage.");
    }

    /// The spell families put the spell's name between the two units. A
    /// composer sharing the melee order would get every perspective wrong.
    #[test]
    fn a_spells_name_sits_between_the_two_units() {
        let s = strings();
        let spell = |attacker, victim| {
            compose(
                &s,
                Family::SpellDamage,
                "SPELLLOG",
                true,
                parts::SPELL,
                attacker,
                victim,
                &["Fireball", "31"],
                Trailers::default(),
            )
            .unwrap()
            .text
        };
        assert_eq!(
            spell(MATE, MOB),
            "Bram's Fireball hits Kobold Vermin for 31."
        );
        assert_eq!(spell(YOU, MOB), "Your Fireball hits Kobold Vermin for 31.");
        assert_eq!(spell(MOB, YOU), "Kobold Vermin's Fireball hits you for 31.");
        assert_eq!(spell(YOU, YOU), "Your Fireball hits you for 31.");
    }

    /// In the spell families the school slot is the third extra, not the
    /// second.
    #[test]
    fn a_spells_school_follows_its_damage() {
        let s = strings();
        let line = compose(
            &s,
            Family::SpellDamage,
            "SPELLLOGSCHOOL",
            true,
            parts::SPELL,
            MATE,
            MOB,
            &["Fireball", "31", "Fire"],
            Trailers::default(),
        )
        .unwrap();
        assert_eq!(
            line.text,
            "Bram's Fireball hits Kobold Vermin for 31 Fire damage."
        );
    }

    /// `IMMUNESPELL*` leads with the victim, and `POWERGAIN*` puts two numbers
    /// between the names. These are two further orders, different from the
    /// melee and spell orders.
    #[test]
    fn the_other_two_slot_orders() {
        let s = strings();
        let immune = |attacker, victim| {
            compose(
                &s,
                Family::SpellBuff,
                "IMMUNESPELL",
                true,
                parts::IMMUNE,
                attacker,
                victim,
                &["Fireball"],
                Trailers::default(),
            )
            .unwrap()
            .text
        };
        assert_eq!(
            immune(MATE, MOB),
            "Kobold Vermin is immune to Bram's Fireball."
        );
        assert_eq!(immune(YOU, MOB), "Kobold Vermin is immune to your Fireball.");
        assert_eq!(
            immune(MOB, YOU),
            "You are immune to Kobold Vermin's Fireball."
        );
        assert_eq!(immune(YOU, YOU), "You are immune to your Fireball.");

        let gain = |attacker, victim| {
            compose(
                &s,
                Family::SpellBuff,
                "POWERGAIN",
                true,
                parts::POWER_GAIN,
                attacker,
                victim,
                &["40", "Mana", "Evocation"],
                Trailers::default(),
            )
            .unwrap()
            .text
        };
        assert_eq!(
            gain(MATE, MOB),
            "Kobold Vermin gains 40 Mana from Bram's Evocation."
        );
        assert_eq!(gain(YOU, MOB), "Kobold Vermin gains 40 Mana from Evocation.");
        assert_eq!(
            gain(MOB, YOU),
            "You gain 40 Mana from Kobold Vermin's Evocation."
        );
        assert_eq!(gain(YOU, YOU), "You gain 40 Mana from Evocation.");
    }

    /// Every clause, in the client's order.
    #[test]
    fn trailers_append_in_the_clients_order() {
        let s = strings();
        let line = compose(
            &s,
            Family::MeleeHit,
            "COMBATHIT",
            false,
            parts::MELEE,
            YOU,
            MOB,
            &["12"],
            Trailers {
                glancing: true,
                crushing: true,
                resisted: 5,
                blocked: 3,
                absorbed: 2,
            },
        )
        .unwrap();
        assert_eq!(
            line.text,
            "You hit Kobold Vermin for 12. (glancing) (crushing) (5 resisted) (3 blocked) (2 absorbed)"
        );
    }

    /// A negative resist is a vulnerability bonus and takes its own key.
    #[test]
    fn a_negative_resist_is_a_vulnerability() {
        let s = strings();
        let mut text = String::new();
        Trailers {
            resisted: -8,
            ..Default::default()
        }
        .append(&s, &mut text);
        assert_eq!(text, " (+8 vulnerability bonus)");
    }

    /// Every key the catalogue can produce, enumerated as `vale combatlog`
    /// does, without the archives.
    #[test]
    fn the_catalogue_produces_the_keys_it_claims() {
        let perspectives = [
            Perspective::None,
            Perspective::OtherSelf,
            Perspective::SelfOther,
            Perspective::OtherOther,
        ];
        let mut keys = Vec::new();
        for kind in KINDS {
            for perspective in perspectives {
                if let Some(k) = key(kind.stem, perspective, kind.self_self) {
                    keys.push(k);
                }
            }
        }
        // 37 stems: three keys for the eighteen with no `SELFSELF` and four for
        // the nineteen that have one.
        assert_eq!(KINDS.len(), 37);
        let with = KINDS.iter().filter(|k| k.self_self).count();
        assert_eq!(with, 19);
        assert_eq!(keys.len(), (KINDS.len() - with) * 3 + with * 4);
        assert_eq!(keys.len(), 130);
        assert!(keys.contains(&"PROCRESISTSELFSELF".to_string()));
        assert!(keys.contains(&"DISPELFAILEDOTHEROTHER".to_string()));
        assert!(keys.contains(&"COMBATHITCRITSCHOOLOTHEROTHER".to_string()));
        assert!(keys.contains(&"SPELLLOGSELFSELF".to_string()));
        assert!(!keys.contains(&"SPELLBLOCKEDSELFSELF".to_string()));
        // The three `SMSG_SPELLLOGEXECUTE` stems have no `…SELFSELF`: you do
        // not interrupt your own cast or cast durability damage on yourself.
        assert!(keys.contains(&"SPELLINTERRUPTOTHEROTHER".to_string()));
        assert!(!keys.contains(&"SPELLINTERRUPTSELFSELF".to_string()));
        assert!(keys.contains(&"SPELLDURABILITYDAMAGEALLSELFOTHER".to_string()));

        // Not every one-unit kind has two keys; `SoloKind::has_self` records
        // which do.
        let solo: Vec<String> = SOLO_KINDS
            .iter()
            .flat_map(|kind| kind.subjects().iter().map(|s| s.key(kind.stem)))
            .collect();
        assert!(solo.contains(&"UNITDESTROYEDOTHER".to_string()));
        assert!(!solo.contains(&"UNITDESTROYEDSELF".to_string()));
        assert!(solo.contains(&"UNITDIESSELF".to_string()));

        // The twelve environmental keys, the only family with an infix.
        assert_eq!(
            environmental_key(2, Subject::You).as_deref(),
            Some("VSENVIRONMENTALDAMAGE_FALLING_SELF")
        );
        assert_eq!(
            environmental_key(0, Subject::Other).as_deref(),
            Some("VSENVIRONMENTALDAMAGE_FATIGUE_OTHER")
        );
        // `DAMAGE_FALL_TO_VOID` is vmangos' seventh value, and the server
        // rewrites it to `DAMAGE_FALL` before sending. If it arrives anyway,
        // there is no key.
        assert_eq!(environmental_key(6, Subject::You), None);
    }

    /// A death is friendly up to category 5 and hostile from 6.
    #[test]
    fn a_death_is_friendly_up_to_category_five() {
        for who in Category::ALL {
            let expected = if who.id() <= 5 {
                "COMBAT_FRIENDLY_DEATH"
            } else {
                "COMBAT_HOSTILE_DEATH"
            };
            assert_eq!(TYPES[death_chat_type(who) as usize].name, expected, "{who:?}");
        }
    }

    /// Every case of [`categorise`], in the order the 1.12.1 client tests
    /// them, and the pairing that makes ten values out of five.
    #[test]
    fn the_categoriser_walks_the_clients_cascade() {
        let base = Standing {
            owner_known: true,
            ..Default::default()
        };
        // Unresolvable is 9, a valid category, not a failure.
        assert_eq!(categorise(Standing::default()), Category::Unknown);

        let you = Standing { owner_is_you: true, ..base };
        assert_eq!(categorise(you), Category::You);
        assert_eq!(categorise(Standing { is_pet: true, ..you }), Category::YourPet);

        // A creature with no player behind it is 8 whether or not it is a
        // pet; the client has no "creature's pet" category.
        let mob = base;
        assert_eq!(categorise(mob), Category::Creature);
        assert_eq!(categorise(Standing { is_pet: true, ..mob }), Category::Creature);

        let player = Standing { owner_is_player: true, ..base };
        assert_eq!(categorise(player), Category::FriendlyPlayer);
        assert_eq!(
            categorise(Standing { is_pet: true, ..player }),
            Category::FriendlyPlayerPet
        );

        let mate = Standing { owner_in_party: true, ..player };
        assert_eq!(categorise(mate), Category::Party);
        assert_eq!(categorise(Standing { is_pet: true, ..mate }), Category::PartyPet);

        // Hostility is tested before the party, so a duel with a party member
        // reads as hostile, not party. `chat_type`'s two conditional arms give
        // the same result from the other side.
        let foe = Standing { owner_hostile: true, ..mate };
        assert_eq!(categorise(foe), Category::HostilePlayer);
        assert_eq!(categorise(Standing { is_pet: true, ..foe }), Category::HostilePlayerPet);
    }

    /// A one-unit family is two keys, and only the `OTHER` one takes a name.
    #[test]
    fn a_solo_family_is_two_keys() {
        assert_eq!(Subject::of(Category::You), Subject::You);
        assert_eq!(Subject::of(Category::Creature), Subject::Other);
        assert_eq!(Subject::You.key("UNITDIES"), "UNITDIESSELF");
        assert_eq!(Subject::Other.key("UNITDIES"), "UNITDIESOTHER");
        assert!(!Subject::You.names_subject());
        assert!(Subject::Other.names_subject());
    }

    /// The three range CVars that are absent, and the four that are not.
    #[test]
    fn you_and_your_pet_are_always_in_range() {
        assert_eq!(Category::You.range_yards(), NO_RANGE_LIMIT);
        assert_eq!(Category::YourPet.range_yards(), NO_RANGE_LIMIT);
        assert_eq!(Category::Unknown.range_yards(), NO_RANGE_LIMIT);
        assert_eq!(Category::Party.range_yards(), 50.0);
        assert_eq!(Category::Creature.range_yards(), 30.0);
        assert_eq!(
            Category::Creature.range_cvar().map(|(n, _)| n),
            Some("CombatLogRangeCreature")
        );
    }
}
