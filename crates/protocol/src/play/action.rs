//! What a unit is visibly **doing** — the packets that drive an animation.
//!
//! **The server never sends an animation id.** Everything drawn on a character
//! beyond walking and standing is inferred from statements about the world, and
//! these are those statements: one melee swing and what the victim did about it
//! (`SMSG_ATTACKERSTATEUPDATE`), an emote, and the two halves of a spell cast.
//! What each one *means* on screen is the renderer's rule, not this module's;
//! the emote id in particular is an `Emotes.dbc` row and reaches an animation
//! only through `vale_assets::tables::dbc`.
//!
//! They sat in `movement.rs` because the swing arrived first and that is where
//! the packet parsers were. None of them is movement.
//!
//! Source: vmangos `Objects/Unit.cpp` (`SendAttackStateUpdate`,
//! `HandleEmoteCommand`) and `Spells/Spell.cpp` (`SendSpellStart`,
//! `SendSpellGo`).

use crate::bytes::Reader;

/// `SMSG_ATTACKERSTATEUPDATE`: one melee swing has landed, missed or been
/// parried.
///
/// This is the only thing the server ever says about a swing. There is no
/// "start attacking" packet and no animation id anywhere in the protocol — a
/// unit in combat simply generates one of these per swing timer, and the client
/// is what turns that into a raised weapon and a flinch. Without it a fight is
/// two creatures standing in their idle loops while their health bars empty,
/// which is what this client has shown so far.
///
/// Layout is `Unit::SendAttackStateUpdate`, for builds after 1.8.4: `u32
/// hitInfo`, **packed** attacker guid, **packed** victim guid, `u32
/// totalDamage`, `u8 subDamageCount`, `subDamageCount` × 20 bytes, `u32
/// victimState`, `u32 attackerState`, `u32 meleeSpellId`, `i32 blockedAmount`.
///
/// **The array used to be skipped and the tail never read**, which was right
/// while the only consumer was an animation: none of it moves anything. It is
/// all read now because the *combat log* is made of it — the school picks the
/// key, the absorb and resist columns are two of the five trailing clauses, and
/// the blocked amount is a third.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AttackUpdate {
    pub attacker: u64,
    pub victim: u64,
    pub damage: u32,
    pub hit_info: u32,
    /// `VictimState` — **what the victim did about it**, and the only statement
    /// the protocol makes about a dodge, a parry or a block.
    ///
    /// [`victim_state::NORMAL`] when the blow simply landed, and
    /// [`victim_state::UNAFFECTED`] when the body was too short to reach the
    /// field — a swing that is only known to have happened, which is what this
    /// client had for every swing until now.
    pub victim_state: u32,
    /// **The first sub-damage entry's school**, and the whole of what decides
    /// whether the combat log says "for 12." or "for 12 Fire damage.". 0 is
    /// physical, which is nearly every swing in the game.
    pub school: u32,
    /// The absorb column of the sub-damage array, summed — the ` (%d absorbed)`
    /// clause.
    pub absorbed: u32,
    /// …and the resist column, which is **signed**: a negative sum is a
    /// vulnerability rather than a resistance and takes a different clause
    /// entirely. See `vale_assets::interface::combatlog::Trailers`.
    pub resisted: i32,
    /// What the shield stopped, from the word behind the victim state — the
    /// ` (%d blocked)` clause. Distinct from
    /// [`victim_state::BLOCKS`], which is a *full* block with no damage at all.
    pub blocked: u32,
    /// The ability this swing was, or 0 for a plain one. Read for completeness:
    /// nothing in this client uses it yet, and the reference logs an ability's
    /// swing through the spell family rather than this one.
    pub melee_spell_id: u32,
}

/// `HitInfo`, the post-1.9.4 table — which is this client's.
pub mod hit_info {
    /// "no being hit animation on victim without it", says the header.
    pub const AFFECTS_VICTIM: u32 = 0x0000_0002;
    pub const LEFT_SWING: u32 = 0x0000_0004;
    pub const MISS: u32 = 0x0000_0010;
    /// `HITINFO_ABSORB`, whose comment in `UnitDefines.h` is **"plays absorb
    /// sound"** — a flag whose entire documented meaning is a noise, and one of
    /// the six entries the client caches by name.
    pub const ABSORB: u32 = 0x0000_0020;
    /// `HITINFO_CRITICALHIT`. **0x80 in this table and 0x08 in the pre-1.9.4
    /// one**, which is the reason the module note names the build: taking the
    /// wrong constant here reads a critical off every eighth ordinary swing.
    pub const CRITICAL_HIT: u32 = 0x0000_0080;
    /// **`HITINFO_RESIST`** — the whole blow was resisted.
    ///
    /// Named nowhere in `UnitDefines.h`; taken from the client instead, where
    /// it is the third arm of the swing composer's cascade and selects
    /// `VSRESIST`. Its neighbour [`ABSORB`] selects `VSABSORB` one
    /// test earlier, which is the pairing that makes the reading safe: two
    /// adjacent bits, two adjacent keys, in the order the two arms are written.
    pub const RESIST: u32 = 0x0000_0040;
    /// `HITINFO_CRUSHING` — the third of the victim's three wound voices, and
    /// the only one with no pose of its own.
    pub const CRUSHING: u32 = 0x0000_8000;
    /// `HITINFO_SWINGNOHITSOUND` — **the server telling the client to keep
    /// quiet about this swing**. vmangos sets it in exactly one place, on an
    /// evade, together with `MISS`; the name is the whole specification.
    pub const SWING_NO_HIT_SOUND: u32 = 0x0008_0000;
}

/// `enum VictimState` (`UnitDefines.h`) — how the blow was received.
///
/// The client turns three of these into an animation on the *victim*: a dodge
/// is a whole-body sidestep, a parry is made with whatever is in the hands, and
/// a block is the shield coming up. The rest either mean "it landed" or mean
/// something with no pose of its own.
pub mod victim_state {
    /// Seen with `HITINFO_MISS`: nothing happened to the victim at all.
    pub const UNAFFECTED: u32 = 0;
    pub const NORMAL: u32 = 1;
    pub const DODGE: u32 = 2;
    pub const PARRY: u32 = 3;
    pub const INTERRUPT: u32 = 4;
    pub const BLOCKS: u32 = 5;
    pub const EVADES: u32 = 6;
    pub const IS_IMMUNE: u32 = 7;
    pub const DEFLECTS: u32 = 8;
}

impl AttackUpdate {
    /// Did the blow land? A miss still swings the weapon; it just does not make
    /// the victim flinch.
    pub fn hit_the_victim(&self) -> bool {
        self.hit_info & hit_info::MISS == 0 && self.hit_info & hit_info::AFFECTS_VICTIM != 0
    }
}

/// One sub-damage description: school, damage as a float, damage again as an
/// integer, absorb, resist. Five words, and `subDamageCount` of them stand
/// between the header and the victim state.
const SUB_DAMAGE_BYTES: usize = 4 * 5;

/// A sanity bound on `subDamageCount`, which is a byte off the wire and is the
/// one number here that can make the parser walk off the end.
///
/// `m_weaponDamageCount` is per weapon and is 1 in vanilla for everything the
/// server sends; anything above a handful means the packet is being read at the
/// wrong offset, and the honest answer is then to report the swing without a
/// victim state rather than to invent one out of whatever follows.
const MAX_SUB_DAMAGES: u8 = 8;

/// **A spell landing on somebody** — `SMSG_SPELLNONMELEEDAMAGELOG` (592), and
/// the periodic tick of a damage-over-time on the same opcode.
///
/// The one packet that says a spell did damage. `SMSG_ATTACKERSTATEUPDATE`
/// covers the weapon swing and nothing else, which is why a client reading only
/// that draws a number for every auto-attack and nothing at all for a Fireball.
///
/// **A tick comes through here too**, with [`Self::periodic`] set: vmangos'
/// `Aura::PeriodicTick` builds a `SpellNonMeleeDamage` with `periodicLog = true`
/// and sends it down the same opcode (`SpellAuras.cpp:6279`), so there is one
/// parser rather than two. `SMSG_PERIODICAURALOG` (590) carries the *aura*
/// bookkeeping — which aura, on whom, how much of what — and is a different
/// subject: this is the number that floats, that is the line in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellDamage {
    pub victim: u64,
    pub caster: u64,
    pub spell_id: u32,
    /// What got through, after absorb, resist and block.
    pub damage: u32,
    /// `SpellSchools` — 0 physical, 1 holy, 2 fire, 3 nature, 4 frost,
    /// 5 shadow, 6 arcane. Carried because the reference's floating number
    /// takes a colour from it and this client draws it white; see
    /// `vale_assets::look::worldtext`.
    pub school: u8,
    pub absorbed: u32,
    pub resisted: i32,
    pub blocked: u32,
    /// A tick of a damage-over-time rather than a direct hit. The reference
    /// shows the spell's *name* in the combat log for one of these and not for
    /// a direct hit, which is the whole of what the flag is for.
    pub periodic: bool,
    /// `SpellHitType` — [`spell_hit::CRIT`] is the only bit this client reads.
    pub hit_info: u32,
}

/// `SpellHitType` (`SpellDefines.h:188`), which is **not** the melee
/// `HitInfo` table: a spell's crit is `0x02` where a swing's is `0x80`.
pub mod spell_hit {
    /// `SPELL_HIT_TYPE_CRIT_DEBUG`, and it is not the crit.
    pub const CRIT_DEBUG: u32 = 0x01;
    pub const CRIT: u32 = 0x02;
    pub const MISS: u32 = 0x04;
    pub const ABSORB: u32 = 0x08;
    pub const RESIST: u32 = 0x10;
    pub const SPLIT: u32 = 0x40;
}

/// `SMSG_SPELLNONMELEEDAMAGELOG`, whose field order is
/// `WorldPackets::Spell::SpellNonMeleeDamageLog::AppendBodyTo`
/// (`Spell.cpp:124`).
///
/// **Both guids are packed.** `WriteAsPackedClientBuildAware` is packed for any
/// build over 1.8.4, and 1.12 is one — the `#else` branch that writes a raw
/// `uint64` is for the 1.5-era clients and would be a two-guid offset error
/// here, which is exactly the class this project reads the source to avoid.
///
/// A body that stops early is reported with whatever it carried: the tail from
/// `absorbed` on is bookkeeping the number does not need, and a truncated packet
/// that still names a victim and an amount is worth drawing.
pub fn parse_spell_damage(body: &[u8]) -> Option<SpellDamage> {
    let mut r = Reader::new(body);
    let victim = r.packed_guid();
    let caster = r.packed_guid();
    if !r.has(9) {
        return None;
    }
    let mut log = SpellDamage {
        victim,
        caster,
        spell_id: r.u32(),
        damage: r.u32(),
        school: r.u8(),
        absorbed: 0,
        resisted: 0,
        blocked: 0,
        periodic: false,
        hit_info: 0,
    };
    if r.has(4) {
        log.absorbed = r.u32();
    }
    if r.has(4) {
        log.resisted = r.u32() as i32;
    }
    if r.has(2) {
        log.periodic = r.u8() != 0;
        // `unused`, which vmangos names that and writes as false.
        let _ = r.u8();
    }
    if r.has(4) {
        log.blocked = r.u32();
    }
    if r.has(4) {
        log.hit_info = r.u32();
    }
    Some(log)
}

/// **A heal landing on somebody** — `SMSG_SPELLHEALLOG` (336).
///
/// `WorldPackets::Spell::SpellHealLog::AppendBodyTo` (`Spell.cpp:105`): two
/// packed guids, the spell, the amount, and one byte of crit. Sent only for
/// builds over 1.9.4, which 1.12 is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpellHeal {
    pub victim: u64,
    pub healer: u64,
    pub spell_id: u32,
    pub amount: u32,
    pub critical: bool,
}

pub fn parse_spell_heal(body: &[u8]) -> Option<SpellHeal> {
    let mut r = Reader::new(body);
    let victim = r.packed_guid();
    let healer = r.packed_guid();
    if !r.has(8) {
        return None;
    }
    Some(SpellHeal {
        victim,
        healer,
        spell_id: r.u32(),
        amount: r.u32(),
        // The byte is optional in the same sense the tail above is: an amount
        // with no crit flag is a heal drawn plain.
        critical: r.has(1) && r.u8() != 0,
    })
}

pub fn parse_attack_update(body: &[u8]) -> Option<AttackUpdate> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let hit_info = r.u32();
    let attacker = r.packed_guid();
    let victim = r.packed_guid();
    if !r.has(4) {
        return None;
    }
    let mut update = AttackUpdate {
        attacker,
        victim,
        damage: r.u32(),
        hit_info,
        victim_state: victim_state::UNAFFECTED,
        school: 0,
        absorbed: 0,
        resisted: 0,
        blocked: 0,
        melee_spell_id: 0,
    };
    // **The victim state sits behind a variable-length array**, which is why
    // this client stopped at the damage for so long. The array is one entry in
    // practice and the count is stated, so crossing it is arithmetic rather
    // than guesswork — but a short body costs the state and nothing else, since
    // everything above it is what says a swing happened at all.
    if !r.has(1) {
        return Some(update);
    }
    let sub_damages = r.u8();
    if sub_damages > MAX_SUB_DAMAGES {
        return Some(update);
    }
    if !r.has(SUB_DAMAGE_BYTES * usize::from(sub_damages) + 4) {
        return Some(update);
    }
    // **The array is no longer skipped**, because the combat log's trailers are
    // in it: the client sums the absorb and resist columns over five entries
    // before the line is composed, and without them
    // every swing reads as full damage with nothing shrugged off. The first
    // entry's school is also what decides the `…SCHOOL…` key.
    for i in 0..sub_damages {
        let school = r.u32() as i32;
        // The float copy of the damage, which the client never reads.
        let _ = r.f32();
        let _damage = r.u32();
        let absorbed = r.u32();
        let resisted = r.u32() as i32;
        // The reference's loops stop at the first negative school rather than
        // running to the count, so a padded array cannot inflate either sum.
        if school < 0 {
            break;
        }
        if i == 0 {
            update.school = school as u32;
        }
        update.absorbed = update.absorbed.saturating_add(absorbed);
        update.resisted = update.resisted.saturating_add(resisted);
    }
    update.victim_state = r.u32();
    // …and the three words behind it, which the composer's last argument and
    // its block trailer come from. `attackerState` is read by nothing here.
    if !r.has(12) {
        return Some(update);
    }
    let _attacker_state = r.u32();
    update.melee_spell_id = r.u32();
    update.blocked = r.u32();
    Some(update)
}


/// `SMSG_EMOTE`: a unit has played a one-shot emote.
///
/// **The closest the protocol ever comes to naming an animation**, and it still
/// does not: the id is an `Emotes.dbc` row, and that row's third column is the
/// `AnimationData.dbc` id. One hop, and it is the shortest one in the game.
///
/// The body is `u32 emoteId` then a **plain** `ObjectGuid` —
/// `Unit::HandleEmoteCommand` writes `data << GetObjectGuid()`, which streams
/// the raw 64-bit value. This is one of the places a guid is not packed, and
/// reading it as packed would attribute every emote to the wrong unit.
///
/// Distinct from `UNIT_NPC_EMOTESTATE`, which `HandleEmoteState` writes into an
/// update field and which is a *state* rather than an event: an innkeeper stuck
/// permanently in a work animation. This is the one-shot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Emote {
    pub guid: u64,
    /// An `Emotes.dbc` id, not an animation id.
    pub emote_id: u32,
}

pub fn parse_emote(body: &[u8]) -> Option<Emote> {
    let mut r = Reader::new(body);
    if !r.has(4 + 8) {
        return None;
    }
    let emote_id = r.u32();
    Some(Emote {
        guid: r.u64(),
        emote_id,
    })
}

/// `SMSG_AI_REACTION`: a creature has noticed you, or has decided to fight you.
///
/// **The one packet in the protocol whose entire purpose is a sound.** vmangos'
/// own comment on the enum says so — `AI_REACTION_HOSTILE` is "sent on every
/// attack, triggers aggro sound (used in client packet handler)" — and the
/// client's handler agrees to the instruction: it dispatches on exactly
/// the two values [`ai_reaction::ALERT`] and [`ai_reaction::HOSTILE`], plays a
/// voice for each and does **nothing else at all** — no animation, no message,
/// no interface. The other three reactions fall out of the bottom of the same
/// function without a branch.
///
/// Without it a creature charges you in silence: `CreatureSoundData`'s aggro
/// column is stated by 4,636 of the game's display ids and nothing else in the
/// protocol ever says "this unit has just engaged".
///
/// Body is `Creature::SendAIReaction`: a **plain** `ObjectGuid` and a `u32`,
/// twelve bytes. Broadcast to everyone in sight (`SendObjectMessageToSet`), so
/// the yell belongs to the creature and not to whoever it is yelling at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiReaction {
    pub guid: u64,
    /// One of [`ai_reaction`]'s five; the client voices two of them.
    pub reaction: u32,
}

/// `enum AiReaction` (`SharedDefines.h`), with vmangos' own notes on which ones
/// the client's handler acts on.
pub mod ai_reaction {
    /// Pre-aggro: the creature has noticed something. Voiced.
    pub const ALERT: u32 = 0;
    /// Not voiced.
    pub const FRIENDLY: u32 = 1;
    /// "Sent on every attack, triggers aggro sound." Voiced.
    pub const HOSTILE: u32 = 2;
    /// Seen for polymorph. Not voiced.
    pub const AFRAID: u32 = 3;
    /// Sent on object destroy. Not voiced.
    pub const DESTROY: u32 = 4;
}

pub fn parse_ai_reaction(body: &[u8]) -> Option<AiReaction> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    Some(AiReaction {
        guid: r.u64(),
        reaction: r.u32(),
    })
}

/// `SMSG_SPELL_START` / `SMSG_SPELL_GO`: a unit is casting, and a unit has
/// cast.
///
/// The two are the wind-up and the release, and the client needs both because
/// they drive different animations: `SpellPrecast` is held for as long as the
/// cast bar runs and `SpellCast` is the one-shot at the end. An instant spell
/// sends only the `GO`.
///
/// **The first guid is not the caster.** Both packets lead with the *cast
/// item's* guid, falling back to the caster's when there is no item — so a
/// wand's cast is attributed to the wand unless the second guid is the one
/// read. Both are packed.
///
/// ```text
/// packedguid castItem (or caster)
/// packedguid caster
/// u32 spellId
/// u16 castFlags
/// u32 castTimeMs      SMSG_SPELL_START only, and it is `m_timer`
/// u8  hitCount        SMSG_SPELL_GO only  \  `Spell::WriteSpellGoTargets`
/// u64 hit[hitCount]                       |  — **plain** guids, not packed
/// u8  missCount                           |
/// (u64, u8[, u8]) miss[missCount]         /
/// ...                 targets, and ammo when CAST_FLAG_AMMO is set
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpellCast {
    pub caster: u64,
    pub spell_id: u32,
    /// How long the cast bar runs, in milliseconds. Zero for `SMSG_SPELL_GO`,
    /// which is the release rather than the wind-up, and zero for an instant.
    pub cast_time_ms: u32,
    /// **Everything the spell landed on**, in the order the server listed it,
    /// and empty for a cast that hit nothing the client can name — a self-buff,
    /// a ground-targeted spell, a spell that missed outright.
    ///
    /// `SMSG_SPELL_GO` only, and it is the *hit* list rather than anything out
    /// of `SpellCastTargets`: `Spell::WriteSpellGoTargets` writes the guids the
    /// spell actually connected with, which is what a projectile has to fly at
    /// and what a burst has to burst on.
    ///
    /// **The whole list, not the first entry.** It was the first for four
    /// rounds, on the reasoning that one missile per cast is what the 1.12
    /// client throws — which is true of the missile and false of everything
    /// else a cast draws. An Arcane Explosion catching five creatures writes
    /// five guids here and the impact kit belongs on all five; keeping one
    /// gave four of them nothing at all, which is the report exactly.
    /// [`Self::target`] is still the missile's, and it is the first of these.
    ///
    /// **The guids here are not packed**, alone among the guids in these two
    /// packets: `*data << ihit.targetGUID` streams an `ObjectGuid` as a plain
    /// `uint64`, where the two in the header go through `GetPackGUID()`.
    /// Reading them packed consumes one byte where eight were written and
    /// every field after it is nonsense.
    pub hits: Vec<u64>,
}

impl SpellCast {
    /// **What a missile flies at**: the first thing the cast hit, or 0.
    ///
    /// One target rather than the list because a projectile is one object with
    /// one destination — the client throws a single missile per cast — where
    /// the impact art is per victim. See [`Self::hits`].
    pub fn target(&self) -> u64 {
        self.hits.first().copied().unwrap_or(0)
    }
}

/// Parse `SMSG_SPELL_START` (`with_timer`) or `SMSG_SPELL_GO` (without).
///
/// One function for two opcodes because they share a header; the caller says
/// which, since the only part past the flags this client reads is the timer and
/// only one of them has it.
pub fn parse_spell_cast(body: &[u8], with_timer: bool) -> Option<SpellCast> {
    let mut r = Reader::new(body);
    if !r.has(1) {
        return None;
    }
    let _cast_item = r.packed_guid();
    if !r.has(1) {
        return None;
    }
    let caster = r.packed_guid();
    if !r.has(4 + 2) {
        return None;
    }
    let spell_id = r.u32();
    let _cast_flags = r.u16();
    let cast_time_ms = if with_timer && r.has(4) { r.u32() } else { 0 };
    // The hit list, which only the release carries. A truncated one costs the
    // targets it did not reach and nothing else — the caster and the spell are
    // already read, and an unfollowable missile is a better answer than a
    // dropped cast, so the loop stops on the bytes rather than on the count.
    let mut hits = Vec::new();
    if !with_timer && r.has(1) {
        let count = r.u8();
        for _ in 0..count {
            if !r.has(8) {
                break;
            }
            hits.push(r.u64());
        }
    }
    Some(SpellCast {
        caster,
        spell_id,
        cast_time_ms,
        hits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;

    /// `SMSG_ATTACKERSTATEUPDATE`, built as `Unit::SendAttackStateUpdate`
    /// writes it: `u32 hitInfo`, **packed** attacker, **packed** victim, then
    /// the damage.
    ///
    /// Both guids are packed and neither is the sender's, which is the trap: a
    /// plain read of either one puts the swing on some other creature entirely
    /// and still parses.
    #[test]
    fn an_attack_names_both_ends_of_the_swing() {
        let mut w = crate::bytes::Writer::new();
        w.u32(hit_info::AFFECTS_VICTIM);
        w.packed_guid(0xF130_0000_0001_2345);
        w.packed_guid(0x0000_0000_0000_0007);
        w.u32(142);

        let attack = parse_attack_update(&w.buf).expect("an attack");
        assert_eq!(attack.attacker, 0xF130_0000_0001_2345);
        assert_eq!(attack.victim, 7);
        assert_eq!(attack.damage, 142);
        assert!(attack.hit_the_victim());
    }

    /// A miss still swings the weapon and still does not make the victim
    /// flinch. `HITINFO_AFFECTS_VICTIM`'s own comment in `UnitDefines.h` is "no
    /// being hit animation on victim without it".
    #[test]
    fn a_miss_swings_but_does_not_land() {
        let mut w = crate::bytes::Writer::new();
        w.u32(hit_info::MISS);
        w.packed_guid(1);
        w.packed_guid(2);
        w.u32(0);
        assert!(!parse_attack_update(&w.buf).expect("an attack").hit_the_victim());

        // Truncated is refused rather than read as a swing by nobody at nobody.
        assert!(parse_attack_update(&[0u8; 3]).is_none());
    }

    /// **The whole body, with everything the combat log reads out of it.**
    ///
    /// Two sub-damage entries, because the absorb and resist clauses are sums
    /// over the array rather than the first entry's — and the school is the
    /// first entry's rather than the sum, which is the pair of rules easiest to
    /// get the wrong way round.
    #[test]
    fn the_trailing_clauses_come_out_of_the_sub_damage_array_and_the_tail() {
        let mut w = Writer::new();
        w.u32(hit_info::AFFECTS_VICTIM | hit_info::CRITICAL_HIT);
        w.packed_guid(0xF130_0000_0001_2345);
        w.packed_guid(7);
        w.u32(96);
        w.u8(2);
        // school, damage (float), damage, absorb, resist
        w.u32(2).f32(60.0).u32(60).u32(5).u32(3);
        w.u32(0).f32(36.0).u32(36).u32(2).u32(4);
        w.u32(victim_state::NORMAL);
        w.u32(0); // attackerState, which nothing reads
        w.u32(0); // meleeSpellId
        w.u32(11); // blockedAmount

        let attack = parse_attack_update(&w.buf).expect("an attack");
        assert_eq!(attack.damage, 96);
        assert_eq!(attack.school, 2, "the *first* entry's school, not the last");
        assert_eq!(attack.absorbed, 7, "summed over the array");
        assert_eq!(attack.resisted, 7);
        assert_eq!(attack.blocked, 11);
        assert_eq!(attack.victim_state, victim_state::NORMAL);
    }

    /// A negative school stops the walk where the reference's own loops stop,
    /// so a padded array cannot inflate either sum.
    #[test]
    fn a_negative_school_ends_the_sub_damage_walk() {
        let mut w = Writer::new();
        w.u32(hit_info::AFFECTS_VICTIM);
        w.packed_guid(1);
        w.packed_guid(2);
        w.u32(10);
        w.u8(2);
        w.u32(0).f32(10.0).u32(10).u32(4).u32(0);
        w.u32(u32::MAX).f32(0.0).u32(0).u32(999).u32(999);
        w.u32(victim_state::NORMAL);

        let attack = parse_attack_update(&w.buf).expect("an attack");
        assert_eq!(attack.absorbed, 4, "the padded entry contributes nothing");
        assert_eq!(attack.resisted, 0);
    }

    /// The victim state sits behind the sub-damage array, and crossing it is
    /// the only arithmetic in this parser.
    ///
    /// **A miscount does not fail** - it reads some other field as the victim
    /// state and makes a character parry a blow that landed. So the body is
    /// built exactly as `SendAttackStateUpdate` writes it, with a sub-damage
    /// entry present, and the assertion is the value on the far side.
    #[test]
    fn the_victim_state_is_read_across_the_sub_damage_array() {
        let mut w = Writer::new();
        w.u32(hit_info::AFFECTS_VICTIM);
        w.packed_guid(0xF130_0000_0001_2345);
        w.packed_guid(7);
        w.u32(0); // total damage - a parry does none
        w.u8(1); // one sub-damage
        w.u32(1).f32(0.0).u32(0).u32(0).u32(0); // school, damage, damage, absorb, resist
        w.u32(victim_state::PARRY);

        let attack = parse_attack_update(&w.buf).expect("an attack");
        assert_eq!(attack.victim_state, victim_state::PARRY);
        assert_eq!(attack.attacker, 0xF130_0000_0001_2345);
    }

    /// Two sub-damages, which the array's own count is there to allow for.
    #[test]
    fn more_than_one_sub_damage_is_stepped_over() {
        let mut w = Writer::new();
        w.u32(hit_info::AFFECTS_VICTIM);
        w.packed_guid(1);
        w.packed_guid(2);
        w.u32(50);
        w.u8(2);
        for _ in 0..2 {
            w.u32(1).f32(25.0).u32(25).u32(0).u32(0);
        }
        w.u32(victim_state::BLOCKS);
        assert_eq!(
            parse_attack_update(&w.buf).expect("an attack").victim_state,
            victim_state::BLOCKS
        );
    }

    /// A body that stops before the victim state still reports the swing.
    ///
    /// The swing is what makes the attacker move; the victim state only decides
    /// what the *victim* does. Losing the whole packet over the shorter half
    /// would take the animation this client already had.
    #[test]
    fn a_swing_with_no_victim_state_is_still_a_swing() {
        let mut w = Writer::new();
        w.u32(hit_info::AFFECTS_VICTIM);
        w.packed_guid(1);
        w.packed_guid(2);
        w.u32(9);
        let attack = parse_attack_update(&w.buf).expect("an attack");
        assert_eq!(attack.damage, 9);
        assert_eq!(attack.victim_state, victim_state::UNAFFECTED);
    }

    /// An implausible sub-damage count is refused rather than followed.
    ///
    /// The count is a byte off the wire and it is the one number here that can
    /// walk the reader off the end. A packet read at the wrong offset produces
    /// a large one, and the honest answer is the swing without a victim state.
    #[test]
    fn an_absurd_sub_damage_count_costs_only_the_victim_state() {
        let mut w = Writer::new();
        w.u32(hit_info::AFFECTS_VICTIM);
        w.packed_guid(1);
        w.packed_guid(2);
        w.u32(9);
        w.u8(200);
        let attack = parse_attack_update(&w.buf).expect("an attack");
        assert_eq!(attack.victim_state, victim_state::UNAFFECTED);
    }

    /// **`SMSG_AI_REACTION`'s guid is not packed either** — `SendAIReaction`
    /// streams `GetObjectGuid()` straight out, the same as the emote above.
    #[test]
    fn an_ai_reaction_names_a_unit_and_a_reason() {
        let mut w = Writer::new();
        w.u64(0xF130_0000_0001_2345);
        w.u32(ai_reaction::HOSTILE);
        let reaction = parse_ai_reaction(&w.buf).expect("a reaction");
        assert_eq!(reaction.guid, 0xF130_0000_0001_2345);
        assert_eq!(reaction.reaction, ai_reaction::HOSTILE);
        // A short body is refused rather than read as a reaction of 0, which is
        // `ALERT` and would put a bark on every truncated packet.
        assert!(parse_ai_reaction(&w.buf[..11]).is_none());
    }

    /// **`SMSG_EMOTE`'s guid is not packed**, and reading it as though it were
    /// puts the emote on a unit that may not exist.
    #[test]
    fn an_emote_names_a_row_and_a_plain_guid() {
        let mut w = Writer::new();
        w.u32(11); // ONESHOT_WAVE
        w.u64(0xF130_0000_0001_2345);
        let emote = parse_emote(&w.buf).expect("an emote");
        assert_eq!(emote.emote_id, 11);
        assert_eq!(emote.guid, 0xF130_0000_0001_2345);
        assert!(parse_emote(&w.buf[..8]).is_none());
    }

    /// **The caster is the second guid.** The first is the cast item's, and
    /// falls back to the caster only when there is no item - so a wand or a
    /// trinket cast reads as coming from the object unless both are consumed.
    #[test]
    fn a_cast_is_attributed_to_the_second_guid() {
        let mut w = Writer::new();
        w.packed_guid(0x4000_0000_0000_0009); // the cast item
        w.packed_guid(0xF130_0000_0001_2345); // the caster
        w.u32(133); // Fireball
        w.u16(0x02); // CAST_FLAG_UNKNOWN2
        w.u32(3500); // m_timer

        let cast = parse_spell_cast(&w.buf, true).expect("a cast");
        assert_eq!(cast.caster, 0xF130_0000_0001_2345);
        assert_eq!(cast.spell_id, 133);
        assert_eq!(cast.cast_time_ms, 3500);

        // `SMSG_SPELL_GO` is the same header with no timer behind it, and the
        // bytes that follow are targets rather than a duration.
        let go = parse_spell_cast(&w.buf, false).expect("a cast");
        assert_eq!(go.caster, 0xF130_0000_0001_2345);
        assert_eq!(go.cast_time_ms, 0, "the release was given the wind-up's clock");
    }

    /// **What the spell landed on, and its guids are not packed.**
    /// `Spell::WriteSpellGoTargets` streams each hit target as a plain
    /// `uint64`, where the two guids in the header above it go through
    /// `GetPackGUID()` — so this is the one place in the packet where the
    /// habit of reading a guid packed consumes one byte of eight and turns
    /// every field after it into rubbish.
    ///
    /// It is the release's list and only the release's: the same bytes after
    /// `SMSG_SPELL_START`'s header are its `m_timer`, which is why the caller
    /// says which packet it is holding.
    #[test]
    fn the_release_names_what_it_hit() {
        let mut w = Writer::new();
        w.packed_guid(0xF130_0000_0001_2345); // no cast item: the caster again
        w.packed_guid(0xF130_0000_0001_2345);
        w.u32(133);
        w.u16(0x102); // CAST_FLAG_UNKNOWN9
        w.u8(2); // two targets hit
        w.u64(0xF130_0000_0000_0042);
        w.u64(0xF130_0000_0000_0043);
        w.u8(0); // and none missed

        let go = parse_spell_cast(&w.buf, false).expect("a cast");
        assert_eq!(
            go.hits,
            vec![0xF130_0000_0000_0042, 0xF130_0000_0000_0043],
            "**both** of them, read as plain guids"
        );
        assert_eq!(
            go.target(),
            0xF130_0000_0000_0042,
            "and the missile flies at the first"
        );

        // A cast that hit nothing nameable — a self-buff, a ground target —
        // says so with an empty list rather than by omitting it.
        let mut none = Writer::new();
        none.packed_guid(0xF130_0000_0001_2345);
        none.packed_guid(0xF130_0000_0001_2345);
        none.u32(1459);
        none.u16(0x100);
        none.u8(0);
        none.u8(0);
        let none = parse_spell_cast(&none.buf, false).expect("a cast");
        assert!(none.hits.is_empty());
        assert_eq!(none.target(), 0);

        // …and the wind-up's own bytes are a duration, not a count. Reading the
        // list off `SMSG_SPELL_START` would take the low byte of `m_timer` as a
        // target count and eight bytes of nothing as a guid.
        assert!(parse_spell_cast(&w.buf, true).expect("a cast").hits.is_empty());

        // **A count the body cannot back is not a dropped cast.** The list stops
        // where the bytes stop: what a truncated tail costs is the targets past
        // the cut, and the caster and the spell are already read.
        let mut short = Writer::new();
        short.packed_guid(0xF130_0000_0001_2345);
        short.packed_guid(0xF130_0000_0001_2345);
        short.u32(133);
        short.u16(0x102);
        short.u8(3); // …says three
        short.u64(0xF130_0000_0000_0042); // …and carries one
        let short = parse_spell_cast(&short.buf, false).expect("a cast");
        assert_eq!(short.hits, vec![0xF130_0000_0000_0042]);
    }
}
