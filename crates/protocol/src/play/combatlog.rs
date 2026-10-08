//! The thirteen packets whose only consumer is the combat log.
//!
//! `SMSG_ATTACKERSTATEUPDATE`, `SMSG_SPELLNONMELEEDAMAGELOG` and
//! `SMSG_SPELLHEALLOG` are in [`super::action`], because each of them also
//! moves something on screen: a swing, a flinch, a floating number. This
//! module reads the rest of the log, the packets whose only consumer is a line
//! of text.
//!
//! The rule that turns any of these into a sentence is
//! `vale_assets::interface::combatlog`; this module only reads bytes.
//!
//! ## Which packets write a guid packed and which write it raw
//!
//! Most of this protocol writes a guid packed. Nine of these thirteen do not:
//! `SMSG_LOG_XPGAIN`, `SMSG_PARTYKILLLOG`, `SMSG_ENVIRONMENTALDAMAGELOG`,
//! `SMSG_SPELLDAMAGESHIELD`, `SMSG_SPELLLOGMISS`, `SMSG_SPELLORDAMAGE_IMMUNE`,
//! `SMSG_PROCRESIST`, `SMSG_DISPEL_FAILED` and `SMSG_SPELLINSTAKILLLOG` write a
//! raw `uint64` (vmangos' `buffer << guid` with no `WriteAsPacked` on it).
//! `SMSG_SPELLENERGIZELOG`, `SMSG_PERIODICAURALOG` and `SMSG_SPELLDISPELLOG`
//! write packed guids. `SMSG_SPELLLOGEXECUTE` writes its caster packed and
//! every target raw. There is no pattern to it and no build guard on most of
//! them; each vmangos writer, such as an `AppendBodyTo`, chooses its own form.
//! A reader that assumes the wrong form reads the next field as part of a guid
//! and produces a plausible number for the wrong unit, and the resulting combat
//! log line does not look wrong.
//!
//! Source: vmangos `Server/Packets/Combat.cpp`, `Server/Packets/Spell.cpp`,
//! `Server/Packets/Misc.cpp`, `Spells/SpellEffects.cpp` and `Objects/Unit.cpp`
//! (`SendPeriodicAuraLog`).

use crate::bytes::Reader;

/// One combat event, queued to become a line of text.
///
/// The queue is [`crate::state::objects::ObjectManager::note_combat`]. It is a
/// queue because a combat log line is an event: two swings for the same damage
/// on the same creature are two lines, and state that keeps only the last value
/// on an entity cannot produce the second one.
///
/// `Swing`, `SpellDamage` and `SpellHeal` carry packets read in
/// [`super::action`], because those packets also move something on screen, and
/// `Enchantment` carries one read in [`super::items`]. Carrying them here lets
/// one drain compose every line in arrival order, which is the order a player
/// reads them in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CombatEvent {
    /// `SMSG_ATTACKERSTATEUPDATE` — the weapon swing, landed or not.
    Swing(super::action::AttackUpdate),
    /// `SMSG_SPELLNONMELEEDAMAGELOG` — a spell that did damage.
    SpellDamage(super::action::SpellDamage),
    /// `SMSG_SPELLHEALLOG`.
    SpellHeal(super::action::SpellHeal),
    /// One target of a `SMSG_SPELLLOGMISS`. The handler fans the packet out:
    /// the packet is one cast and a list, and the log has one line per target.
    SpellMissed {
        spell_id: u32,
        caster: u64,
        target: u64,
        miss: SpellMiss,
    },
    Energize(EnergizeLog),
    DamageShield(DamageShield),
    Environmental(EnvironmentalDamage),
    PartyKill(PartyKill),
    XpGain(XpGain),
    PeriodicAura(PeriodicAuraLog),
    /// One `SPELL_EFFECT_ADD_EXTRA_ATTACKS` entry of a `SMSG_SPELLLOGEXECUTE`.
    /// The handler fans the packet out, like [`CombatEvent::SpellMissed`]: one
    /// packet is a list of effects and each entry of each list is its own line.
    ExtraAttacks { target: u64, spell_id: u32, count: u32 },
    /// One `SPELL_EFFECT_INTERRUPT_CAST` entry of a `SMSG_SPELLLOGEXECUTE`.
    /// `spell_id` is the spell that was interrupted, which is what the sentence
    /// names; the interrupting spell does not appear in `SPELLINTERRUPT*`.
    Interrupt { caster: u64, target: u64, spell_id: u32 },
    /// One `SPELL_EFFECT_FEED_PET` entry of a `SMSG_SPELLLOGEXECUTE`. Its two
    /// keys name no unit and carry an item rather than a spell.
    FeedPet { caster: u64, item: u32 },
    /// One `SPELL_EFFECT_DURABILITY_DAMAGE` entry of a `SMSG_SPELLLOGEXECUTE`.
    /// A negative `item` means every item, which selects the `…ALL…` half of
    /// the four keys.
    DurabilityDamage {
        caster: u64,
        target: u64,
        spell_id: u32,
        item: i32,
    },
    /// `SMSG_SPELLDISPELLOG`, fanned out the same way: one cast, one line per
    /// aura it took off.
    Dispel { victim: u64, spell_id: u32 },
    /// `SMSG_ENCHANTMENTLOG`: an enchant applied to an item, or one fading off
    /// it. See [`super::items::EnchantmentLog`]: a zero caster is the fade,
    /// not a bad packet.
    ///
    /// It is a combat event rather than a player event, as in the 1.12.1
    /// client: the line goes to `CHAT_MSG_SPELL_ITEM_ENCHANTMENTS`, one of the
    /// six windows the combat log routes through, and its four
    /// `ITEMENCHANTMENTADD*` keys are the same perspective set every other
    /// family here takes.
    Enchantment(super::items::EnchantmentLog),
    /// `SMSG_SPELLORDAMAGE_IMMUNE`: a periodic tick or a damage shield that
    /// did nothing because `target` is immune. For a damage shield, `caster`
    /// is the shield's wearer and `target` the unit that struck it.
    Immune { caster: u64, target: u64, spell_id: u32 },
    /// `SMSG_PROCRESIST`: a proc's damage that `target` resisted.
    ProcResist { caster: u64, target: u64, spell_id: u32 },
    /// One aura of a `SMSG_DISPEL_FAILED`, fanned out at the handler like
    /// [`CombatEvent::Dispel`]: one line per aura that stayed.
    DispelFailed { caster: u64, victim: u64, spell_id: u32 },
    /// `SMSG_SPELLINSTAKILLLOG`: `victim` was killed outright by a spell. The
    /// packet names no caster.
    InstaKill { victim: u64, spell_id: u32 },
}

/// The spell effects `SMSG_SPELLLOGEXECUTE` is written against, by the numbers
/// in `Spell.dbc`'s `Effect` column.
///
/// Only the effects named by the `switch` in vmangos' `Spell::SendLogExecute`
/// are listed. They are kept here rather than in a general effect enum because
/// that switch decides the width of each entry, so an id this list does not
/// carry is a packet that cannot be read past that point. vmangos' `default:`
/// arm is `return`: it abandons the packet mid-write rather than send a body it
/// cannot describe, so stopping at an unknown id loses nothing that was sent.
pub mod spell_effects {
    pub const POWER_DRAIN: u32 = 8;
    pub const HEAL: u32 = 10;
    pub const ADD_EXTRA_ATTACKS: u32 = 19;
    pub const CREATE_ITEM: u32 = 24;
    pub const ENERGIZE: u32 = 30;
    pub const HEAL_MAX_HEALTH: u32 = 67;
    pub const INTERRUPT_CAST: u32 = 68;
    pub const FEED_PET: u32 = 101;
    pub const DURABILITY_DAMAGE: u32 = 111;
}

/// One entry of one effect's list in `SMSG_SPELLLOGEXECUTE`.
///
/// The packet is `caster, spell, effectCount` and then, per effect, the effect
/// id, a count, and that many entries. Each entry's width depends on the effect
/// id, so the body is parsed entry by entry rather than read as a fixed layout.
/// vmangos' `Spell::SendLogExecute` is the authority for every one of them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExecuteEntry {
    PowerDrain { target: u64, amount: u32, power: u32, multiplier: f32 },
    Heal { target: u64, amount: u32, critical: bool },
    Energize { target: u64, amount: u32, power: u32 },
    ExtraAttacks { target: u64, count: u32 },
    CreateItem { item: u32 },
    InterruptCast { target: u64, spell_id: u32 },
    FeedPet { item: u32 },
    DurabilityDamage { target: u64, item: i32, unknown: i32 },
    /// Every other effect the switch names: a bare target guid and nothing
    /// else. The effect id is kept because it is the only thing that
    /// distinguishes a summon from a resurrection once the guid is read.
    Targeted { effect: u32, target: u64 },
}

/// `SMSG_SPELLLOGEXECUTE`: what a cast's effects did, per effect and per
/// target.
#[derive(Debug, Clone, PartialEq)]
pub struct SpellExecuteLog {
    pub caster: u64,
    pub spell_id: u32,
    pub entries: Vec<ExecuteEntry>,
}

/// The caster is packed; every `targetGuid` inside is raw.
///
/// vmangos writes `data << m_caster->GetPackGUID()` for the caster and
/// `data << info.targetGuid` for each target, whose `ObjectGuid` operator
/// writes eight plain bytes. The two forms are four lines apart in one
/// function; the module header lists the form each packet uses.
///
/// A truncated tail ends the walk rather than failing the packet. Everything
/// read up to that point is valid, and an entry that ran off the end is one
/// this reader does not understand, which is the same outcome as `default:` on
/// the writing side.
pub fn parse_spell_execute_log(body: &[u8]) -> Option<SpellExecuteLog> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    let caster = r.packed_guid();
    if !r.has(8) {
        return None;
    }
    let spell_id = r.u32();
    let effects = r.u32();
    let mut entries = Vec::new();
    'effects: for _ in 0..effects {
        if !r.has(8) {
            break;
        }
        let effect = r.u32();
        let count = r.u32();
        for _ in 0..count {
            let entry = match effect {
                spell_effects::POWER_DRAIN if r.has(20) => ExecuteEntry::PowerDrain {
                    target: r.u64(),
                    amount: r.u32(),
                    power: r.u32(),
                    multiplier: r.f32(),
                },
                // The critical flag is one byte, not four. vmangos'
                // `heal.critical` is a `uint8` and `ByteBuffer` writes it at its
                // own width; reading four would swallow the next entry's guid.
                spell_effects::HEAL | spell_effects::HEAL_MAX_HEALTH if r.has(13) => {
                    ExecuteEntry::Heal {
                        target: r.u64(),
                        amount: r.u32(),
                        critical: r.u8() != 0,
                    }
                }
                spell_effects::ENERGIZE if r.has(16) => ExecuteEntry::Energize {
                    target: r.u64(),
                    amount: r.u32(),
                    power: r.u32(),
                },
                spell_effects::ADD_EXTRA_ATTACKS if r.has(12) => ExecuteEntry::ExtraAttacks {
                    target: r.u64(),
                    count: r.u32(),
                },
                // These two carry no guid. A reader that expects a guid in every
                // entry loses the rest of the packet here, as one that reads
                // `critical` as four bytes does above.
                spell_effects::CREATE_ITEM if r.has(4) => {
                    ExecuteEntry::CreateItem { item: r.u32() }
                }
                spell_effects::FEED_PET if r.has(4) => ExecuteEntry::FeedPet { item: r.u32() },
                spell_effects::INTERRUPT_CAST if r.has(12) => ExecuteEntry::InterruptCast {
                    target: r.u64(),
                    spell_id: r.u32(),
                },
                spell_effects::DURABILITY_DAMAGE if r.has(16) => {
                    ExecuteEntry::DurabilityDamage {
                        target: r.u64(),
                        item: r.u32() as i32,
                        unknown: r.u32() as i32,
                    }
                }
                // A named effect whose entry does not fit: the tail is short and
                // there is nothing further to read.
                spell_effects::POWER_DRAIN
                | spell_effects::HEAL
                | spell_effects::HEAL_MAX_HEALTH
                | spell_effects::ENERGIZE
                | spell_effects::ADD_EXTRA_ATTACKS
                | spell_effects::CREATE_ITEM
                | spell_effects::FEED_PET
                | spell_effects::INTERRUPT_CAST
                | spell_effects::DURABILITY_DAMAGE => break 'effects,
                // Every other effect in the switch is a bare guid.
                _ if r.has(8) => ExecuteEntry::Targeted {
                    effect,
                    target: r.u64(),
                },
                _ => break 'effects,
            };
            entries.push(entry);
        }
    }
    Some(SpellExecuteLog {
        caster,
        spell_id,
        entries,
    })
}

/// `SMSG_SPELLDISPELLOG`: what a dispel took off, and off whom.
///
/// The log has one line per aura removed (`AURADISPELSELF`/`…OTHER`), so the
/// list is kept here and the handler fans it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellDispelLog {
    pub victim: u64,
    pub caster: u64,
    /// The auras that were removed, named by the spell that applied them.
    pub spells: Vec<u32>,
}

/// Both guids are packed and the victim comes first. `SMSG_DISPEL_FAILED` is
/// the reverse: both guids are raw and the caster comes first. vmangos writes
/// the two packets twenty lines apart in one function.
///
/// The caster is read, but no line this client composes names it:
/// `AURADISPEL*` is a one-unit family that says only whose aura went.
pub fn parse_spell_dispel_log(body: &[u8]) -> Option<SpellDispelLog> {
    let mut r = Reader::new(body);
    if !r.has(2) {
        return None;
    }
    let victim = r.packed_guid();
    let caster = r.packed_guid();
    if !r.has(4) {
        return None;
    }
    let count = r.u32();
    let mut spells = Vec::new();
    for _ in 0..count {
        if !r.has(4) {
            break;
        }
        spells.push(r.u32());
    }
    Some(SpellDispelLog {
        victim,
        caster,
        spells,
    })
}

/// `SMSG_SPELLORDAMAGE_IMMUNE` and `SMSG_PROCRESIST`, which share one layout:
/// two raw guids, a spell and a byte.
///
/// ```text
/// u64 caster, u64 target, u32 spell, u8 flag
/// ```
///
/// vmangos writes the byte as 0 in both (`Server/Packets/Spell.cpp`). In
/// `SMSG_SPELLORDAMAGE_IMMUNE` the 1.12.1 client reads it as "this was a
/// periodic tick" and then drops the line when the `CombatLogPeriodicSpells`
/// CVar is 0, which a 0 byte never reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellNotTaken {
    pub caster: u64,
    pub target: u64,
    pub spell_id: u32,
    pub flag: u8,
}

/// Reads [`SpellNotTaken`]: 21 bytes.
pub fn parse_spell_not_taken(body: &[u8]) -> Option<SpellNotTaken> {
    if body.len() < 21 {
        return None;
    }
    let mut r = Reader::new(body);
    Some(SpellNotTaken {
        caster: r.u64(),
        target: r.u64(),
        spell_id: r.u32(),
        flag: r.u8(),
    })
}

/// `SMSG_DISPEL_FAILED`: a dispel that left these auras on the victim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispelFailed {
    pub caster: u64,
    pub victim: u64,
    /// The auras that stayed, named by the spell that applied them.
    pub spells: Vec<u32>,
}

/// Two raw guids, caster first, then spell ids to the end of the body with no
/// count (`Spell::EffectDispel` in vmangos' `Spells/SpellEffects.cpp`). A
/// trailing part of a spell id is dropped.
pub fn parse_dispel_failed(body: &[u8]) -> Option<DispelFailed> {
    if body.len() < 16 {
        return None;
    }
    let mut r = Reader::new(body);
    let caster = r.u64();
    let victim = r.u64();
    let mut spells = Vec::with_capacity(r.remaining() / 4);
    while r.has(4) {
        spells.push(r.u32());
    }
    Some(DispelFailed { caster, victim, spells })
}

/// `SMSG_SPELLINSTAKILLLOG`: a raw victim guid and the spell, and no caster
/// (`Spell::EffectInstaKill`). vmangos sends it only to clients after 1.11.2.
pub fn parse_instakill_log(body: &[u8]) -> Option<(u64, u32)> {
    if body.len() < 12 {
        return None;
    }
    let mut r = Reader::new(body);
    Some((r.u64(), r.u32()))
}

/// `SMSG_LOG_XPGAIN`: experience, and where it came from.
///
/// The only packet behind `COMBAT_XP_GAIN`. A client that does not read it has
/// no "Kobold Vermin dies, you gain 42 experience." line: the level and the
/// experience field both change in an update block, but neither says who died
/// or how much of the gain was rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XpGain {
    /// Who died. Zero for experience that was not a kill (a quest hand-in, an
    /// exploration bonus); `kind` also states this, and the `…_UNNAMED` keys
    /// are for this case.
    pub victim: u64,
    /// Everything gained, rested bonus included.
    pub total: u32,
    /// 0 for a kill, 1 for anything else.
    pub kind: u8,
    /// What the kill was worth before rest. Only present for a kill, and equal
    /// to `total` when there was no bonus.
    pub base: u32,
    /// The group factor, where 1.0 means no group bonus. vmangos' comment is
    /// "1=none 0=100% group bonus output", and it writes 1.0 unconditionally,
    /// so the group and raid variants of the key are unreachable against this
    /// server.
    pub group_bonus: f32,
}

/// `victimGuid` is a raw `uint64`, and the tail is present only for a kill.
pub fn parse_xp_gain(body: &[u8]) -> Option<XpGain> {
    let mut r = Reader::new(body);
    if !r.has(13) {
        return None;
    }
    let victim = r.u64();
    let total = r.u32();
    let kind = r.u8();
    let mut gain = XpGain {
        victim,
        total,
        kind,
        base: total,
        group_bonus: 1.0,
    };
    if kind == 0 && r.has(8) {
        gain.base = r.u32();
        gain.group_bonus = r.f32();
    }
    Some(gain)
}

/// `SMSG_PARTYKILLLOG`: a group member landed the killing blow.
///
/// Two raw guids and nothing else. It is the only statement the protocol makes
/// about who killed a unit; the death itself is a health field reaching zero,
/// which every observer sees and which names nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartyKill {
    pub killer: u64,
    pub victim: u64,
}

pub fn parse_party_kill(body: &[u8]) -> Option<PartyKill> {
    let mut r = Reader::new(body);
    if !r.has(16) {
        return None;
    }
    Some(PartyKill {
        killer: r.u64(),
        victim: r.u64(),
    })
}

/// `SMSG_ENVIRONMENTALDAMAGELOG`: damage from the environment, with no
/// attacker to name.
///
/// Fatigue, drowning, falling, lava, slime and fire, in that order: the
/// `EnvironmentalDamageType` values 0..5. vmangos rewrites its own seventh
/// value (`DAMAGE_FALL_TO_VOID`, a fall with no durability loss) to
/// `DAMAGE_FALL` before sending, so nothing above 5 should arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnvironmentalDamage {
    pub victim: u64,
    /// 0 fatigue, 1 drowning, 2 falling, 3 lava, 4 slime, 5 fire.
    pub damage_type: u8,
    pub damage: u32,
    pub absorbed: u32,
    pub resisted: i32,
}

/// The victim is a raw guid. The absorb and resist tail exists in 1.7.0 and
/// later, which includes 1.12, but it is read defensively for the same reason
/// as every other tail here.
pub fn parse_environmental_damage(body: &[u8]) -> Option<EnvironmentalDamage> {
    let mut r = Reader::new(body);
    if !r.has(13) {
        return None;
    }
    let mut log = EnvironmentalDamage {
        victim: r.u64(),
        damage_type: r.u8(),
        damage: r.u32(),
        absorbed: 0,
        resisted: 0,
    };
    if r.has(4) {
        log.absorbed = r.u32();
    }
    if r.has(4) {
        log.resisted = r.u32() as i32;
    }
    Some(log)
}

/// `SMSG_SPELLDAMAGESHIELD`: a damage shield dealt damage to the unit that
/// struck it.
///
/// Thorns, Fire Shield, a Retribution Aura. Both guids are raw. The shield's
/// wearer comes first and the unit that hit them second, the reverse of the
/// blow that triggered it. The sentence (`"%s reflects %d %s damage to %s."`)
/// names the wearer first, as the attacker of this line, because the wearer
/// deals the damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageShield {
    /// The unit wearing the shield, which the line is about.
    pub wearer: u64,
    /// The unit that struck the wearer, which takes the damage.
    pub struck_by: u64,
    pub damage: u32,
    pub school: u32,
}

pub fn parse_damage_shield(body: &[u8]) -> Option<DamageShield> {
    let mut r = Reader::new(body);
    if !r.has(24) {
        return None;
    }
    Some(DamageShield {
        wearer: r.u64(),
        struck_by: r.u64(),
        damage: r.u32(),
        school: r.u32(),
    })
}

/// How a spell failed to land: the miss byte of `SMSG_SPELLLOGMISS`.
///
/// `SpellMissInfo` (`SpellDefines.h`). `NONE` never arrives on this packet (a
/// spell that landed is `SMSG_SPELLNONMELEEDAMAGELOG`); it is listed so the
/// numbering matches the enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SpellMiss {
    None = 0,
    Miss = 1,
    Resist = 2,
    Dodge = 3,
    Parry = 4,
    Block = 5,
    Evade = 6,
    Immune = 7,
    Deflect = 8,
    Absorb = 9,
    Reflect = 10,
}

impl SpellMiss {
    pub fn from_code(code: u8) -> Option<SpellMiss> {
        Some(match code {
            0 => SpellMiss::None,
            1 => SpellMiss::Miss,
            2 => SpellMiss::Resist,
            3 => SpellMiss::Dodge,
            4 => SpellMiss::Parry,
            5 => SpellMiss::Block,
            6 => SpellMiss::Evade,
            7 => SpellMiss::Immune,
            8 => SpellMiss::Deflect,
            9 => SpellMiss::Absorb,
            10 => SpellMiss::Reflect,
            _ => return None,
        })
    }

    /// The `GlobalStrings` stem this failure takes.
    ///
    /// The stem is not the upper-cased variant name, so it is a table: a miss
    /// is `SPELLMISS`, a dodge is `SPELLDODGED` with a D, an immunity is
    /// `IMMUNESPELL` with the words the other way round, and an absorb is
    /// `SPELLLOGABSORB`. There are four different shapes in eleven values.
    pub fn stem(self) -> Option<&'static str> {
        Some(match self {
            SpellMiss::None => return None,
            SpellMiss::Miss => "SPELLMISS",
            SpellMiss::Resist => "SPELLRESIST",
            SpellMiss::Dodge => "SPELLDODGED",
            SpellMiss::Parry => "SPELLPARRIED",
            SpellMiss::Block => "SPELLBLOCKED",
            SpellMiss::Evade => "SPELLEVADED",
            SpellMiss::Immune => "IMMUNESPELL",
            SpellMiss::Deflect => "SPELLDEFLECTED",
            SpellMiss::Absorb => "SPELLLOGABSORB",
            SpellMiss::Reflect => "SPELLREFLECT",
        })
    }
}

/// `SMSG_SPELLLOGMISS`: one cast, and every target it did not land on.
///
/// A list rather than a packet per target, because an area spell resisted by
/// four of six targets is one cast: the caster is named once and the targets
/// follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellMissLog {
    pub spell_id: u32,
    pub caster: u64,
    pub targets: Vec<(u64, SpellMiss)>,
}

/// The caster and every target guid are raw. The server writes the
/// `useExtendedInfo` byte as `false`, with vmangos' note that it "seems unused
/// in client", so the two floats behind it never appear. If the byte is ever
/// set, this reader refuses the packet rather than guess at them.
pub fn parse_spell_miss_log(body: &[u8]) -> Option<SpellMissLog> {
    let mut r = Reader::new(body);
    if !r.has(17) {
        return None;
    }
    let spell_id = r.u32();
    let caster = r.u64();
    let extended = r.u8() != 0;
    if extended {
        return None;
    }
    let count = r.u32();
    let mut targets = Vec::new();
    for _ in 0..count {
        if !r.has(9) {
            break;
        }
        let guid = r.u64();
        let Some(miss) = SpellMiss::from_code(r.u8()) else {
            continue;
        };
        targets.push((guid, miss));
    }
    Some(SpellMissLog {
        spell_id,
        caster,
        targets,
    })
}

/// `SMSG_SPELLENERGIZELOG`: a spell restored power to a unit.
///
/// Evocation, a mana potion's own spell, Innervate. Both guids are packed,
/// unlike the raw-guid packets above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnergizeLog {
    pub target: u64,
    pub caster: u64,
    pub spell_id: u32,
    /// `Powers` — 0 mana, 1 rage, 2 focus, 3 energy, 4 happiness.
    pub power: u32,
    pub amount: u32,
}

pub fn parse_energize_log(body: &[u8]) -> Option<EnergizeLog> {
    let mut r = Reader::new(body);
    let target = r.packed_guid();
    let caster = r.packed_guid();
    if !r.has(12) {
        return None;
    }
    Some(EnergizeLog {
        target,
        caster,
        spell_id: r.u32(),
        power: r.u32(),
        amount: r.u32(),
    })
}

/// What a periodic tick did, which decides the rest of
/// `SMSG_PERIODICAURALOG`'s body.
///
/// Keyed by `AuraType` (`SpellAuraDefines.h`). Only the seven aura types
/// vmangos sends on this opcode are handled: `SendPeriodicAuraLog` has a
/// `default:` arm that logs an error and sends nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PeriodicEffect {
    /// A damage-over-time tick: damage, school, absorb, resist.
    Damage {
        damage: u32,
        school: u32,
        absorbed: u32,
        resisted: i32,
    },
    /// A heal-over-time tick, or a health regeneration aura.
    Heal { amount: u32 },
    /// A mana regeneration or energize aura.
    Energize { power: u32, amount: u32 },
    /// A mana leech: the same two, plus what the caster gains per point.
    Leech {
        power: u32,
        amount: u32,
        multiplier: f32,
    },
}

/// `SMSG_PERIODICAURALOG`: one tick of an aura.
///
/// A damage-over-time spell's damage arrives separately, on
/// `SMSG_SPELLNONMELEEDAMAGELOG` with `periodicLog` set, and the floating
/// number is drawn from that. This packet is the aura's own record of the tick
/// and feeds the `SPELL_PERIODIC_*` windows, which is why there are two
/// periodic routing families.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeriodicAuraLog {
    pub target: u64,
    pub caster: u64,
    pub spell_id: u32,
    /// The `AuraType` id, kept raw so an arm this client does not model is
    /// still identifiable in a report.
    pub aura_type: u32,
    pub effect: PeriodicEffect,
}

/// The seven `AuraType` values `SendPeriodicAuraLog` writes.
pub mod aura_type {
    pub const PERIODIC_DAMAGE: u32 = 3;
    pub const PERIODIC_HEAL: u32 = 8;
    pub const OBS_MOD_HEALTH: u32 = 20;
    pub const OBS_MOD_MANA: u32 = 21;
    pub const PERIODIC_ENERGIZE: u32 = 24;
    pub const PERIODIC_MANA_LEECH: u32 = 64;
    pub const PERIODIC_DAMAGE_PERCENT: u32 = 89;
}

/// Both guids are packed. The `count` word after the spell is written as a
/// literal 1 and is not a length: the body that follows is one effect, not
/// `count` of them, and reading it as a length walks off the end of every
/// packet.
pub fn parse_periodic_aura_log(body: &[u8]) -> Option<PeriodicAuraLog> {
    use aura_type::*;
    let mut r = Reader::new(body);
    let target = r.packed_guid();
    let caster = r.packed_guid();
    if !r.has(12) {
        return None;
    }
    let spell_id = r.u32();
    // vmangos writes `uint32(1)` here and calls it `count`; nothing varies.
    let _count = r.u32();
    let aura_type = r.u32();
    let effect = match aura_type {
        PERIODIC_DAMAGE | PERIODIC_DAMAGE_PERCENT => {
            if !r.has(12) {
                return None;
            }
            let damage = r.u32();
            let school = r.u32();
            let absorbed = r.u32();
            PeriodicEffect::Damage {
                damage,
                school,
                absorbed,
                // 1.5.1 and earlier have no resist word; 1.12 does, but a
                // truncated tail is still a tick worth reporting.
                resisted: if r.has(4) { r.u32() as i32 } else { 0 },
            }
        }
        PERIODIC_HEAL | OBS_MOD_HEALTH => {
            if !r.has(4) {
                return None;
            }
            PeriodicEffect::Heal { amount: r.u32() }
        }
        OBS_MOD_MANA | PERIODIC_ENERGIZE => {
            if !r.has(8) {
                return None;
            }
            PeriodicEffect::Energize {
                power: r.u32(),
                amount: r.u32(),
            }
        }
        PERIODIC_MANA_LEECH => {
            if !r.has(12) {
                return None;
            }
            PeriodicEffect::Leech {
                power: r.u32(),
                amount: r.u32(),
                multiplier: r.f32(),
            }
        }
        // `SendPeriodicAuraLog`'s `default:` sends nothing, so an unknown aura
        // type is a packet vmangos never writes, not a gap in this reader.
        _ => return None,
    };
    Some(PeriodicAuraLog {
        target,
        caster,
        spell_id,
        aura_type,
        effect,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immune_and_proc_resist_are_two_raw_guids_a_spell_and_a_byte() {
        let mut w = crate::bytes::Writer::new();
        w.u64(0xF130_0000_0000_0001).u64(5).u32(133).u8(0);
        assert_eq!(
            parse_spell_not_taken(&w.buf),
            Some(SpellNotTaken { caster: 0xF130_0000_0000_0001, target: 5, spell_id: 133, flag: 0 })
        );
        assert_eq!(parse_spell_not_taken(&w.buf[..20]), None);
    }

    /// The spell list has no count; it runs to the end of the body.
    #[test]
    fn a_failed_dispel_lists_its_spells_to_the_end() {
        let mut w = crate::bytes::Writer::new();
        w.u64(7).u64(9).u32(10).u32(11).u8(1);
        assert_eq!(
            parse_dispel_failed(&w.buf),
            Some(DispelFailed { caster: 7, victim: 9, spells: vec![10, 11] })
        );
        assert_eq!(parse_dispel_failed(&w.buf[..15]), None);
    }

    #[test]
    fn an_instakill_is_the_victim_and_the_spell() {
        let mut w = crate::bytes::Writer::new();
        w.u64(9).u32(5);
        assert_eq!(parse_instakill_log(&w.buf), Some((9, 5)));
        assert_eq!(parse_instakill_log(&w.buf[..11]), None);
    }
    use crate::bytes::Writer;

    #[test]
    fn a_kill_carries_its_base_and_its_bonus_and_a_quest_does_not() {
        let mut w = Writer::new();
        w.u64(0xF130_0000_0042).u32(120).u8(0).u32(100).f32(1.0);
        let kill = parse_xp_gain(&w.buf).unwrap();
        assert_eq!(kill.victim, 0xF130_0000_0042);
        assert_eq!((kill.total, kill.base, kill.kind), (120, 100, 0));
        assert_eq!(kill.group_bonus, 1.0);

        // A non-kill has no tail at all, and the base is the total.
        let mut w = Writer::new();
        w.u64(0).u32(350).u8(1);
        let quest = parse_xp_gain(&w.buf).unwrap();
        assert_eq!((quest.victim, quest.total, quest.base, quest.kind), (0, 350, 350, 1));
    }

    #[test]
    fn a_party_kill_is_two_raw_guids() {
        let mut w = Writer::new();
        w.u64(1).u64(2);
        assert_eq!(
            parse_party_kill(&w.buf),
            Some(PartyKill {
                killer: 1,
                victim: 2
            })
        );
        assert_eq!(parse_party_kill(&[0; 15]), None);
    }

    #[test]
    fn environmental_damage_keeps_its_type_and_its_tail() {
        let mut w = Writer::new();
        w.u64(7).u8(2).u32(430).u32(0).u32(0);
        let fall = parse_environmental_damage(&w.buf).unwrap();
        assert_eq!((fall.victim, fall.damage_type, fall.damage), (7, 2, 430));

        // The 1.6-and-earlier form has no absorb/resist, and is still a fall.
        let mut w = Writer::new();
        w.u64(7).u8(2).u32(430);
        let short = parse_environmental_damage(&w.buf).unwrap();
        assert_eq!(short.damage, 430);
        assert_eq!((short.absorbed, short.resisted), (0, 0));
    }

    /// The wearer comes first. A reader that swaps the two guids produces a
    /// line that looks just as valid.
    #[test]
    fn a_damage_shield_names_its_wearer_first() {
        let mut w = Writer::new();
        w.u64(0xAA).u64(0xBB).u32(12).u32(3);
        assert_eq!(
            parse_damage_shield(&w.buf),
            Some(DamageShield {
                wearer: 0xAA,
                struck_by: 0xBB,
                damage: 12,
                school: 3
            })
        );
    }

    #[test]
    fn a_spell_miss_log_is_one_cast_and_many_targets() {
        let mut w = Writer::new();
        w.u32(133).u64(0x11).u8(0).u32(2);
        w.u64(0xAA).u8(2); // resist
        w.u64(0xBB).u8(1); // miss
        let log = parse_spell_miss_log(&w.buf).unwrap();
        assert_eq!(log.spell_id, 133);
        assert_eq!(log.caster, 0x11);
        assert_eq!(
            log.targets,
            vec![(0xAA, SpellMiss::Resist), (0xBB, SpellMiss::Miss)]
        );

        // A count longer than the body keeps what arrived rather than failing.
        let mut w = Writer::new();
        w.u32(133).u64(0x11).u8(0).u32(9);
        w.u64(0xAA).u8(2);
        assert_eq!(parse_spell_miss_log(&w.buf).unwrap().targets.len(), 1);
    }

    /// The stems have four different shapes over eleven values. This test pins
    /// the ones that do not follow the common shape.
    #[test]
    fn the_miss_stems_are_not_a_upper_casing() {
        assert_eq!(SpellMiss::Dodge.stem(), Some("SPELLDODGED"));
        assert_eq!(SpellMiss::Immune.stem(), Some("IMMUNESPELL"));
        assert_eq!(SpellMiss::Absorb.stem(), Some("SPELLLOGABSORB"));
        assert_eq!(SpellMiss::None.stem(), None);
        assert_eq!(SpellMiss::from_code(11), None);
    }

    #[test]
    fn an_energize_log_is_packed_where_the_others_are_not() {
        let mut w = Writer::new();
        w.packed_guid(0x1234).packed_guid(0x5678).u32(12051).u32(0).u32(1500);
        assert_eq!(
            parse_energize_log(&w.buf),
            Some(EnergizeLog {
                target: 0x1234,
                caster: 0x5678,
                spell_id: 12051,
                power: 0,
                amount: 1500
            })
        );
    }

    /// Each effect's entry has its own width, and the packet cannot be walked
    /// without knowing them.
    ///
    /// The three entries in this packet are the three that break a fixed-width
    /// reader: a heal, whose `critical` is one byte and not four; a feed-pet,
    /// which carries no guid; and an extra-attacks entry after both, which is
    /// read from the right offset only if the first two were.
    #[test]
    fn a_spell_execute_log_is_walked_by_its_effect_widths() {
        use spell_effects::{ADD_EXTRA_ATTACKS, FEED_PET, HEAL};

        let mut w = Writer::new();
        w.packed_guid(0xAA).u32(7000).u32(3);
        w.u32(HEAL).u32(1).u64(0xB1).u32(500).u8(1);
        w.u32(FEED_PET).u32(1).u32(2287);
        w.u32(ADD_EXTRA_ATTACKS).u32(1).u64(0xC2).u32(3);

        let log = parse_spell_execute_log(&w.buf).expect("the packet reads");
        assert_eq!(log.caster, 0xAA);
        assert_eq!(log.spell_id, 7000);
        assert_eq!(
            log.entries,
            vec![
                ExecuteEntry::Heal {
                    target: 0xB1,
                    amount: 500,
                    critical: true
                },
                ExecuteEntry::FeedPet { item: 2287 },
                ExecuteEntry::ExtraAttacks {
                    target: 0xC2,
                    count: 3
                },
            ]
        );
    }

    /// A truncated tail ends the walk and keeps what was read.
    ///
    /// vmangos' `default:` arm abandons the packet mid-write rather than send a
    /// body it cannot describe, so nothing follows the point where a reader
    /// stops. Dropping the whole packet would lose entries that did arrive.
    #[test]
    fn a_short_spell_execute_log_keeps_what_it_read() {
        let mut w = Writer::new();
        w.packed_guid(0xAA).u32(7000).u32(2);
        w.u32(spell_effects::ADD_EXTRA_ATTACKS).u32(2).u64(0xC2).u32(3);
        // The second entry of the same run is four bytes short.
        w.u64(0xC3);

        let log = parse_spell_execute_log(&w.buf).expect("the packet reads");
        assert_eq!(
            log.entries,
            vec![ExecuteEntry::ExtraAttacks {
                target: 0xC2,
                count: 3
            }]
        );
    }

    /// The dispel log's two guids are packed and the victim comes first, the
    /// reverse of `SMSG_DISPEL_FAILED` twenty lines away in the same vmangos
    /// function.
    #[test]
    fn a_dispel_log_names_the_victim_first_and_lists_every_aura() {
        let mut w = Writer::new();
        w.packed_guid(0x51).packed_guid(0x99).u32(2).u32(118).u32(1459);
        let log = parse_spell_dispel_log(&w.buf).expect("the packet reads");
        assert_eq!(log.victim, 0x51);
        assert_eq!(log.caster, 0x99);
        assert_eq!(log.spells, vec![118, 1459]);
    }

    #[test]
    fn a_periodic_tick_branches_on_its_aura_type() {
        let dot = |aura: u32| {
            let mut w = Writer::new();
            w.packed_guid(0xAA).packed_guid(0xBB).u32(172).u32(1).u32(aura);
            w
        };

        let mut w = dot(aura_type::PERIODIC_DAMAGE);
        w.u32(30).u32(5).u32(0).u32(0);
        let tick = parse_periodic_aura_log(&w.buf).unwrap();
        assert_eq!(
            tick.effect,
            PeriodicEffect::Damage {
                damage: 30,
                school: 5,
                absorbed: 0,
                resisted: 0
            }
        );

        let mut w = dot(aura_type::PERIODIC_HEAL);
        w.u32(88);
        assert_eq!(
            parse_periodic_aura_log(&w.buf).unwrap().effect,
            PeriodicEffect::Heal { amount: 88 }
        );

        let mut w = dot(aura_type::PERIODIC_MANA_LEECH);
        w.u32(0).u32(40).f32(0.5);
        assert_eq!(
            parse_periodic_aura_log(&w.buf).unwrap().effect,
            PeriodicEffect::Leech {
                power: 0,
                amount: 40,
                multiplier: 0.5
            }
        );

        // An arm the server never writes is refused rather than half-read.
        let w = dot(999);
        assert_eq!(parse_periodic_aura_log(&w.buf), None);
    }

    /// The `count` word is a literal 1 and not a length — reading it as one
    /// would make this three-effect packet, which the server never sends.
    #[test]
    fn the_periodic_count_word_is_not_a_length() {
        let mut w = Writer::new();
        w.packed_guid(0xAA)
            .packed_guid(0xBB)
            .u32(172)
            .u32(3)
            .u32(aura_type::PERIODIC_HEAL)
            .u32(88);
        assert_eq!(
            parse_periodic_aura_log(&w.buf).unwrap().effect,
            PeriodicEffect::Heal { amount: 88 }
        );
    }
}
