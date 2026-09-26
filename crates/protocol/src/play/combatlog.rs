//! **The nine packets a fight is *narrated* by**, as opposed to fought with.
//!
//! `SMSG_ATTACKERSTATEUPDATE`, `SMSG_SPELLNONMELEEDAMAGELOG` and
//! `SMSG_SPELLHEALLOG` are in [`super::action`], because each of them also
//! moves something on screen — a swing, a flinch, a floating number. What is
//! here is the rest of the log: the packets whose *only* consumer is a line of
//! text, which is why they had no reader at all until the combat log did.
//!
//! The rule that turns any of these into a sentence is
//! `vale_assets::interface::combatlog`; this module is bytes and nothing
//! else.
//!
//! ## Packed and unpacked guids sit side by side here, and it matters
//!
//! Most of this protocol writes a guid packed. **Four of these nine do not.**
//! `SMSG_PARTYKILLLOG`, `SMSG_ENVIRONMENTALDAMAGELOG`, `SMSG_SPELLDAMAGESHIELD`
//! and `SMSG_SPELLLOGMISS` write a raw `uint64` — vmangos' `buffer << guid`
//! with no `WriteAsPacked` on it — while `SMSG_SPELLENERGIZELOG` and
//! `SMSG_PERIODICAURALOG` write packed. There is no pattern to it and no build
//! guard on most of them; it is simply what each `AppendBodyTo` does. A reader
//! that assumes one form reads the next field as part of a guid and produces a
//! plausible number for the wrong unit, which in a combat log is a line nobody
//! can tell is wrong.
//!
//! Source: vmangos `Server/Packets/Combat.cpp`, `Server/Packets/Spell.cpp`,
//! `Server/Packets/Misc.cpp` and `Objects/Unit.cpp`
//! (`SendPeriodicAuraLog`).

use crate::bytes::Reader;

/// **One thing that happened in a fight, on its way to a line of text.**
///
/// The queue this travels on is [`crate::state::objects::ObjectManager::note_combat`],
/// and it exists because a combat log line is an *event*: two swings for the
/// same damage on the same creature are two lines, and nothing that latches the
/// last value on an entity can produce the second one.
///
/// Three of the ten come from [`super::action`] rather than from this module,
/// because those packets move something on screen as well; carrying them here
/// is what lets one drain compose every kind of line in arrival order, which is
/// the order a player reads them in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CombatEvent {
    /// `SMSG_ATTACKERSTATEUPDATE` — the weapon swing, landed or not.
    Swing(super::action::AttackUpdate),
    /// `SMSG_SPELLNONMELEEDAMAGELOG` — a spell that did damage.
    SpellDamage(super::action::SpellDamage),
    /// `SMSG_SPELLHEALLOG`.
    SpellHeal(super::action::SpellHeal),
    /// One target of a `SMSG_SPELLLOGMISS`. **Fanned out at the handler**: the
    /// packet is one cast and a list, and the log is one line per target.
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
    /// `SMSG_SPELLLOGEXECUTE`'s `SPELL_EFFECT_ADD_EXTRA_ATTACKS`. **Fanned out
    /// at the handler**, like [`CombatEvent::SpellMissed`]: one packet is a list
    /// of effects and each entry of each list is its own line.
    ExtraAttacks { target: u64, spell_id: u32, count: u32 },
    /// …its `SPELL_EFFECT_INTERRUPT_CAST`. `spell_id` is the spell that was
    /// **interrupted**, which is what the sentence names — the interrupting
    /// spell does not appear in `SPELLINTERRUPT*` at all.
    Interrupt { caster: u64, target: u64, spell_id: u32 },
    /// …its `SPELL_EFFECT_FEED_PET`, whose two keys name no unit at all and
    /// carry an *item* rather than a spell.
    FeedPet { caster: u64, item: u32 },
    /// …and its `SPELL_EFFECT_DURABILITY_DAMAGE`. A negative `item` is **every**
    /// item, which is the `…ALL…` half of the four keys.
    DurabilityDamage {
        caster: u64,
        target: u64,
        spell_id: u32,
        item: i32,
    },
    /// `SMSG_SPELLDISPELLOG`, fanned out the same way: one cast, one line per
    /// aura it took off.
    Dispel { victim: u64, spell_id: u32 },
    /// `SMSG_ENCHANTMENTLOG` — an enchant applied to an item, or one fading off
    /// it. See [`super::items::EnchantmentLog`], whose note is that a zero
    /// caster is the fade rather than a bad packet.
    ///
    /// **A combat event rather than a player one**, which is where the reference
    /// puts it too: the line goes to `CHAT_MSG_SPELL_ITEM_ENCHANTMENTS`, one of
    /// the six windows the combat log routes through, and its four
    /// `ITEMENCHANTMENTADD*` keys are the same perspective set every other
    /// family here takes.
    Enchantment(super::items::EnchantmentLog),
}

/// **The spell effects `SMSG_SPELLLOGEXECUTE` is written against**, by the
/// numbers `Spell.dbc`'s `Effect` column carries.
///
/// Only the ones the packet's own `switch` names, and they are here rather than
/// in a general effect enum because that switch is the whole of what this
/// module needs to know about effects: **the width of each entry is decided by
/// it**, so an id this list does not carry is a packet that cannot be walked
/// past that point. vmangos' own `default:` arm is `return` — it abandons the
/// packet mid-write rather than sending a body it cannot describe — so stopping
/// at an unknown id loses nothing that was ever sent.
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

/// **One entry of one effect's list in `SMSG_SPELLLOGEXECUTE`.**
///
/// The packet is `caster, spell, effectCount` and then, per effect, the effect
/// id, a count, and that many entries — and each entry's width depends on the
/// effect id, which is what makes this a parser rather than a cast.
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

/// `SMSG_SPELLLOGEXECUTE` — **what a cast's effects actually did**, per effect
/// and per target.
#[derive(Debug, Clone, PartialEq)]
pub struct SpellExecuteLog {
    pub caster: u64,
    pub spell_id: u32,
    pub entries: Vec<ExecuteEntry>,
}

/// The caster is **packed**; every `targetGuid` inside is **raw**.
///
/// `data << m_caster->GetPackGUID()` against `data << info.targetGuid`, whose
/// `ObjectGuid` operator writes eight plain bytes. The two forms sit four lines
/// apart in one function, which is what this module's header is about.
///
/// **A truncated tail ends the walk rather than failing the packet.** Everything
/// read up to that point is real, and an entry that ran off the end is one this
/// reader does not understand — the same answer `default:` gives on the writing
/// side.
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
                // **The critical flag is one byte, not four.** vmangos'
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
                // …and these two carry **no guid at all**, which is the other
                // way a fixed-width reader loses the rest of a packet.
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

/// `SMSG_SPELLDISPELLOG` — **what a dispel took off, and off whom.**
///
/// One line per aura removed (`AURADISPELSELF`/`…OTHER`), which is why the list
/// is here and the fan-out is at the handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellDispelLog {
    pub victim: u64,
    pub caster: u64,
    /// The auras that were removed, named by the spell that applied them.
    pub spells: Vec<u32>,
}

/// **Both guids are packed and the victim comes first** — which is the reverse
/// of `SMSG_DISPEL_FAILED` beside it, where both are raw and the caster leads.
/// vmangos writes the two packets twenty lines apart in one function.
///
/// The caster is read and named by no line this client composes: `AURADISPEL*`
/// is a one-unit family that says only whose aura went.
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

/// `SMSG_LOG_XPGAIN` — **experience, and where it came from.**
///
/// The one packet behind `COMBAT_XP_GAIN`, and it is the reason a client that
/// does not read it has no "Kobold Vermin dies, you gain 42 experience." line
/// at all — the level and the experience *field* both move in an update block,
/// but neither says who died or how much of it was rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XpGain {
    /// Who died. **Zero for experience that was not a kill** — a quest hand-in,
    /// an exploration bonus — which is what `kind` says in its own right and
    /// what the `…_UNNAMED` keys are for.
    pub victim: u64,
    /// Everything gained, rested bonus included.
    pub total: u32,
    /// 0 for a kill, 1 for anything else.
    pub kind: u8,
    /// What the kill was worth before rest. Only present for a kill, and equal
    /// to `total` when there was no bonus.
    pub base: u32,
    /// The group factor, where **1.0 means no group bonus** — vmangos' own
    /// comment is "1=none 0=100% group bonus output", and it writes 1.0
    /// unconditionally, so the group and raid variants of the key are
    /// unreachable against this server.
    pub group_bonus: f32,
}

/// `victimGuid` is a **raw** `uint64`, and the tail is present only for a kill.
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

/// `SMSG_PARTYKILLLOG` — **somebody in the group landed the killing blow.**
///
/// Two raw guids and nothing else. It is the only statement the protocol makes
/// about *who* killed a unit; the death itself is a health field reaching zero,
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

/// `SMSG_ENVIRONMENTALDAMAGELOG` — **the world hurt somebody**, with no
/// attacker to name.
///
/// Falling, drowning, fatigue, lava, slime and fire, in that order — the
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

/// The victim is a **raw** guid; the absorb and resist tail is 1.7.0 and later,
/// which 1.12 is, but it is read defensively for the same reason every other
/// tail here is.
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

/// `SMSG_SPELLDAMAGESHIELD` — **the blow answered itself.**
///
/// Thorns, Fire Shield, a Retribution Aura. Both guids raw, and note the order:
/// the **shield's wearer comes first** and the unit that hit them second, which
/// is the opposite of the way the sentence reads
/// (`"%s reflects %d %s damage to %s."` names the wearer first as the
/// *attacker* of this line, because they are the one dealing the damage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DamageShield {
    /// The unit wearing the shield — the one the line is *about*.
    pub wearer: u64,
    /// …and the one that struck them, who takes the damage.
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

/// How a spell failed to land, on `SMSG_SPELLLOGMISS`'s own byte.
///
/// `SpellMissInfo` (`SpellDefines.h`). `NONE` never arrives on this packet —
/// a spell that landed is `SMSG_SPELLNONMELEEDAMAGELOG` — so it is here only
/// so the numbering is the enum's.
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
    /// **Not a mechanical upper-casing of the name**, which is why it is a
    /// table: a miss is `SPELLMISS`, a dodge is `SPELLDODGED` with a D, an
    /// immunity is `IMMUNESPELL` with the words the other way round, and an
    /// absorb is `SPELLLOGABSORB`. Four different shapes in eleven values.
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

/// `SMSG_SPELLLOGMISS` — **one cast, and everybody it did not land on.**
///
/// A list rather than a packet per target, because an area spell resisted by
/// four of six is one cast: the caster is named once and the targets follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellMissLog {
    pub spell_id: u32,
    pub caster: u64,
    pub targets: Vec<(u64, SpellMiss)>,
}

/// The caster and every target guid are **raw**, and the `useExtendedInfo`
/// byte is written `false` by the server with vmangos' own note that it "seems
/// unused in client" — so the two floats behind it never appear and this
/// reader refuses the packet rather than guessing at them if it is ever set.
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

/// `SMSG_SPELLENERGIZELOG` — **a spell gave somebody power back.**
///
/// Evocation, a mana potion's own spell, Innervate. Both guids **packed**,
/// unlike the four above.
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

/// What a periodic tick *did*, which decides the rest of
/// `SMSG_PERIODICAURALOG`'s body.
///
/// `AuraType` (`SpellAuraDefines.h`), and only the six vmangos will send on
/// this opcode are named — `SendPeriodicAuraLog` has a `default:` arm that logs
/// an error and sends nothing at all.
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

/// `SMSG_PERIODICAURALOG` — **one tick of an aura.**
///
/// Distinct from a damage-over-time's *damage*, which arrives on
/// `SMSG_SPELLNONMELEEDAMAGELOG` with `periodicLog` set and is what the
/// floating number is drawn from. This is the aura's own bookkeeping and it is
/// what the `SPELL_PERIODIC_*` windows are fed by — which is why the two
/// periodic routing families exist at all.
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

/// The six `AuraType` values `SendPeriodicAuraLog` will write.
pub mod aura_type {
    pub const PERIODIC_DAMAGE: u32 = 3;
    pub const PERIODIC_HEAL: u32 = 8;
    pub const OBS_MOD_HEALTH: u32 = 20;
    pub const OBS_MOD_MANA: u32 = 21;
    pub const PERIODIC_ENERGIZE: u32 = 24;
    pub const PERIODIC_MANA_LEECH: u32 = 64;
    pub const PERIODIC_DAMAGE_PERCENT: u32 = 89;
}

/// Both guids **packed**. The `count` word after the spell is written as a
/// literal 1 and is not a length — the body that follows is one effect, not
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
        // `SendPeriodicAuraLog`'s own `default:` sends nothing, so an arm this
        // does not know is a packet nobody wrote rather than a reader gap.
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

    /// The wearer comes first, which is the field order a reader is most
    /// likely to get backwards — and the two are indistinguishable in a line.
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

    /// The stems are four different shapes over eleven values, and the two that
    /// break the pattern are the ones worth pinning.
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

    /// **Each effect's entry is a different width, and the packet cannot be
    /// walked without knowing them.**
    ///
    /// The three in this packet are the three that catch a fixed-width reader:
    /// a heal, whose `critical` is **one byte** and not four; a feed-pet, which
    /// carries **no guid at all**; and an extra-attacks entry after both of
    /// them, which only lands in the right place if the first two did.
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

    /// **A truncated tail ends the walk and keeps what was read.**
    ///
    /// vmangos' own `default:` arm abandons the packet mid-write rather than
    /// sending a body it cannot describe, so there is never anything after the
    /// point a reader stops — dropping the whole packet would lose entries that
    /// really did arrive.
    #[test]
    fn a_short_spell_execute_log_keeps_what_it_read() {
        let mut w = Writer::new();
        w.packed_guid(0xAA).u32(7000).u32(2);
        w.u32(spell_effects::ADD_EXTRA_ATTACKS).u32(2).u64(0xC2).u32(3);
        // …and the second entry of that same run is four bytes short.
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

    /// **The dispel log's two guids are packed and the victim leads**, which is
    /// the reverse of `SMSG_DISPEL_FAILED` twenty lines away in the same
    /// vmangos function.
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
