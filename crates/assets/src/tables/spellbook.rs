//! **What a spell _is_** — its name, its icon, what it costs, how long it takes,
//! how far it reaches, and *what it may be aimed at*.
//!
//! [`crate::tables::spell`] is the other half of the same table: what a cast **looks
//! like** on the caster. This is what the player needs to press a button — and
//! the last of it, the aiming rule, is the one piece that is a *decision* rather
//! than a lookup, which is why it is here and not in the renderer: it can be
//! decided with no window open, so it can be unit-tested and the CLI can check
//! it against the same copy the client runs.
//!
//! ## The columns
//!
//! `Spell.dbc` is 22,360 rows of 173 fields, and every index below was measured
//! against the table rather than taken from a reference — Fireball (133) reads
//! back name "Fireball", rank "Rank 1", 30 mana, cast-time index 16 (1500 ms),
//! range index 35 (35 yards), icon 185, GCD category 133 at 1500 ms.
//!
//! ```text
//!   [  2] category                shared recovery bucket
//!   [  6] attributes
//!   [ 13] targets                 the aiming mask's seed
//!   [ 18] castingTimeIndex        -> SpellCastTimes.dbc [1] base ms
//!   [ 19] recoveryTime            the spell's own cooldown, ms
//!   [ 20] categoryRecoveryTime
//!   [ 31] powerType               mana / rage / focus / energy
//!   [ 32] manaCost
//!   [ 36] rangeIndex              -> SpellRange.dbc [2] max yards
//!   [ 82] EffectImplicitTargetA[0]  the aiming mask's adjustment
//!   [117] spellIconID             -> SpellIcon.dbc [1] a BLP path
//!   [120] SpellName               8 locale columns; [120] is enUS
//!   [129] Rank                    likewise, "Rank 1"
//!   [157] startRecoveryCategory   the global cooldown's bucket…
//!   [158] startRecoveryTime       …and its length, usually 1500
//! ```
//!
//! Two of those are cross-checks rather than reads. Field 13 is `0x34` bytes
//! into the record and field 82 is `0x148` — which are exactly the two offsets
//! the client's own cast-arming code loads (`SpellRec+0x34` and
//! `SpellRec+0x148`), so the layout above is pinned from both ends.
//!
//! ## The aiming rule
//!
//! **This is the part that cannot be guessed, and the part that is most visibly
//! wrong when it is.** A cast's target block is not "whatever is selected": the
//! client builds a flag word out of the spell's own two columns and then tries
//! to satisfy every bit of it against a candidate. Four outcomes, and each is a
//! different packet:
//!
//! * **word 0** — the spell says who it hits. Send `TARGET_FLAG_SELF`, which
//!   carries no guid at all. Every self-buff in the game: Ice Armor, Battle
//!   Shout, Feign Death. *Sending the current selection instead is exactly how
//!   Ice Armor comes back "Invalid target"* — the failure this rule exists to
//!   prevent, and one that looks like a server bug from the client's side.
//! * **word naming a destination** — a *place*. Put the targeting cursor up and
//!   send `TARGET_FLAG_DEST_LOCATION` and three floats at the click. Blizzard,
//!   Flamestrike, Rain of Fire; **tested before the selection is**, for the
//!   reason in [`CastAim::WantsGround`].
//! * **word satisfied by the selection** — send `TARGET_FLAG_UNIT` and its guid.
//! * **word satisfied by nobody** — three answers rather than one, and only one
//!   of them is a message; see [`resolve_aim`], where the branch is transcribed.
//!
//! The seed-and-adjust itself follows the client's cast arming and target
//! binding; the arm map is in
//! [`aim_mask`] and the per-bit satisfaction in [`resolve_aim`].
//! What is deliberately **not** modelled, and refused rather than approximated:
//! casts aimed at an item, a gameobject or a string, and the party/raid bits,
//! which want groups to exist.

use crate::tables::dbc::Dbc;
use crate::tables::faction::Reaction;
use crate::AssetError;
use std::collections::HashMap;

/// `Spell.dbc` field indices — see the module comment for how each was pinned.
pub mod spell_fields {
    pub const CATEGORY: usize = 2;
    /// `Dispel` -> `SpellDispelType.dbc`. **What colour a debuff's border is**,
    /// and the third answer `UnitDebuff` gives. Measured against the table:
    /// Polymorph (118) reads 1, Curse of Agony (980) and Curse of Weakness
    /// (702) read 2, Deadly Poison (2818) reads 4, and Fireball (133) and
    /// Battle Stance (2457) read 0.
    pub const DISPEL: usize = 4;
    /// **`castUI`, field 3 — which craft window lists the spell.** Zero for
    /// every spell the book shows; 1 for the hunter's pet-training spells
    /// (Beast Training's rows) and 3 for every Enchanting recipe. The book's
    /// third filter reads it at `+0xc` and the craft window's
    /// per-kind lists are indexed by it. See
    /// [`SpellInfo::craft_kind`].
    pub const CAST_UI: usize = 3;
    pub const ATTRIBUTES: usize = 6;
    /// `AttributesEx`, read for one bit — [`spell_attributes::NO_AURA_ICON`].
    pub const ATTRIBUTES_EX: usize = 7;
    /// **`AttributesEx2`, and the reason the parse stopped at 7 for so long is
    /// that nothing had asked it a question yet.**
    ///
    /// One bit — [`spell_attributes::EX2_AUTO_REPEAT`] — and it is half of the
    /// only thing in the game that says a press starts a *repeating* ranged
    /// attack rather than a cast. The layout is pinned from both ends: the
    /// column reads 0x20 for Auto Shot (75) and the wand's Shoot (5019) and 0
    /// for every other spell a player can press, and the client's own record
    /// offsets agree — `SpellRec+0x18` is `Attributes` (the bit-7 spellbook
    /// filter reads it there) and the client's auto-repeat test is the pair
    /// `[SpellRec+0x18] & 0x2` / `[SpellRec+0x20] & 0x20`, always together.
    /// Four dwords apart is fields 6 and 8.
    pub const ATTRIBUTES_EX2: usize = 8;
    /// **`AttributesEx3`, read for one bit** —
    /// [`spell_attributes::EX3_ONLY_ON_GHOSTS`], which is one of the three
    /// inputs to [`super::SpellInfo::death_only`].
    ///
    /// **Pinned from both sides rather than counted from one.** [`ATTRIBUTES_EX2`]
    /// above is pinned twice over (0x20 on exactly Auto Shot and the wand's
    /// Shoot, and 0x1 — `EX2_ALLOW_DEAD_TARGET` — on exactly spell 2584,
    /// *Waiting to Resurrect*, whose target is definitionally a corpse), and
    /// field **11** is pinned by Overpower (7384) reading `0x10000` there, which
    /// is `1 << (17 - 1)` for form 17, Battle Stance — a stance mask and nothing
    /// else. That leaves exactly two columns between them, and the format names
    /// them `AttributesEx3, AttributesEx4` in that order. `TARGETS` at 13, two
    /// past `StancesNot`, closes the run.
    pub const ATTRIBUTES_EX3: usize = 9;
    /// The aiming mask's seed. `SpellRec+0x34` in the client's own record.
    /// **`Stances` / `StancesNot`** — the shapeshift forms a spell may and may
    /// not be cast in, as masks of `1 << (form - 1)`. Between them they are why
    /// a druid's cat abilities grey out in bear form and why Ambush greys out
    /// of stealth.
    pub const STANCES: usize = 11;
    pub const STANCES_NOT: usize = 12;
    pub const TARGETS: usize = 13;
    /// **`CasterAuraState` / `TargetAuraState`** — the `AURA_STATE_*` the
    /// caster, or the target, must be in. Zero for "no condition", which is all
    /// but a few hundred rows.
    ///
    /// This is the whole mechanism behind "Judgement only works with a Seal up"
    /// (`AURA_STATE_JUDGEMENT`, 5), "Revenge only after a block"
    /// (`AURA_STATE_DEFENSE`, 1) and "Execute only under a fifth"
    /// (`AURA_STATE_HEALTHLESS_20_PERCENT`, 2, on the *target* side). The
    /// client never learns what a Seal is: the server sets a bit in
    /// `UNIT_FIELD_AURASTATE` and the button follows it. See
    /// [`vale_protocol::state::objects::Entity::aura_state`].
    pub const CASTER_AURA_STATE: usize = 16;
    pub const TARGET_AURA_STATE: usize = 17;
    pub const CASTING_TIME_INDEX: usize = 18;
    pub const RECOVERY_TIME: usize = 19;
    pub const CATEGORY_RECOVERY_TIME: usize = 20;
    /// `spellLevel`. Read for one thing only: it is the client's own fallback
    /// sort key for a spell whose `Rank` string carries no digits — see
    /// [`super::SpellInfo::rank_order`].
    /// `ProcChance` — the per-cent behind `$h`, and the largest single token
    /// this crate's tooltip substituter could not answer (360 of the corpus's
    /// 775 dropped ones). **Not read off a wiki**: it is boxed in on both sides
    /// by indices this map already pinned — `CategoryRecoveryTime` at 20 and
    /// `MaxLevel` at 27 — with `InterruptFlags`, `AuraInterruptFlags`,
    /// `ChannelInterruptFlags`, `ProcFlags`, this, and `ProcCharges` filling
    /// the six between them in the client's own order.
    /// **`InterruptFlags` — what cuts a cast short**, and the one bit of it the
    /// client reads before it sends anything: `SPELL_INTERRUPT_FLAG_MOVEMENT`
    /// (`0x1`). See [`super::check_cast`], which is the whole of why these three
    /// are read.
    ///
    /// **Pinned by the record's own offsets rather than counted.** The client
    /// tests `[SpellRec+0x54] & 1`, `[SpellRec+0x58] & 0x18` and
    /// `[SpellRec+0x5c] & 0x18` together, and a DBC row
    /// is its columns four bytes apart from the id — so `0x54` is column 21 and
    /// its two neighbours are 22 and 23. That agrees with the six columns
    /// [`PROC_CHANCE`]'s note already boxed in between 20 and 27.
    pub const INTERRUPT_FLAGS: usize = 21;
    /// `AuraInterruptFlags` — `SpellRec+0x58`.
    pub const AURA_INTERRUPT_FLAGS: usize = 22;
    /// `ChannelInterruptFlags` — `SpellRec+0x5c`.
    pub const CHANNEL_INTERRUPT_FLAGS: usize = 23;
    pub const PROC_CHANCE: usize = 25;
    pub const SPELL_LEVEL: usize = 29;
    pub const POWER_TYPE: usize = 31;
    pub const MANA_COST: usize = 32;
    pub const RANGE_INDEX: usize = 36;
    /// `EffectImplicitTargetA[0]`, the mask's adjustment. `SpellRec+0x148`.
    pub const IMPLICIT_TARGET_A: usize = 82;
    /// `EffectImplicitTargetB[3]`, fields 85..87 — read with the A column
    /// per effect by the craft window's `used` test.
    pub const IMPLICIT_TARGET_B: usize = 85;
    pub const ICON_ID: usize = 117;
    /// **`activeIconID`** — `SpellRec+0x1d8`, the picture a form button wears
    /// *while the form is on*. Zero for almost every spell in the game; the
    /// shapeshift and stealth spells are what carry it, and
    /// `GetShapeshiftFormInfo` is the only reader.
    pub const ACTIVE_ICON_ID: usize = 118;
    pub const NAME: usize = 120;
    pub const RANK: usize = 129;
    pub const START_RECOVERY_CATEGORY: usize = 157;
    pub const START_RECOVERY_TIME: usize = 158;
    /// **Where a form sits on the stance bar** — `SpellRec+0x298`, and the only
    /// thing the shapeshift list is sorted by.
    ///
    /// Pinned against the shipped file rather than counted off a header: the
    /// three warrior stances read 0, 1, 2 in Battle/Defensive/Berserker order,
    /// and the druid's five read Bear 0, Aquatic 1, Cat 2, Travel 3, Moonkin 4
    /// — which is the order the bar draws them in, and a column merely
    /// correlated with the row id could not produce it. **`-1` means "no
    /// position"** and sorts last; Stealth and Prowl are the two
    /// that carry it.
    pub const SHAPESHIFT_ORDER: usize = 166;

    /// `PreventionType` — **which of the two silencing words shuts this spell
    /// up**: 1 is silence, 2 is pacify, 0 is neither. Each is a plain compare
    /// against the constant, and each gating a
    /// different `UNIT_FIELD_FLAGS` bit — so a silenced caster may still swing
    /// an ability and a pacified one may still cast.
    ///
    /// **Pinned against the shipped file**: the column holds only 0, 1 and 2,
    /// 4,811 rows carrying 1 and 1,052 carrying 2, which no neighbouring column
    /// does.
    pub const PREVENTION_TYPE: usize = 165;
    /// **`SpellFamilyName` and `SpellFamilyFlags`** — which class's spell book a
    /// row belongs to, and which of that family's 64 bits it is.
    ///
    /// Read for one thing: a talent's `SMSG_SET_*_SPELL_MODIFIER` names a *bit*
    /// and nothing else, so these are the only columns that can say which spells
    /// it is about. See `crate::game::combat::spellmods` on the client side.
    ///
    /// **Pinned against the shipped file** rather than counted off a header:
    /// Fireball (133) reads family 3 and bit 0, Shadow Bolt (686) family 5 and
    /// bit 0, Corruption (172) family 5 and bit 1, Heroic Strike (78) family 4
    /// and bit 6, Eviscerate (2098) family 8. The flags are a `u64` across two
    /// columns, low first.
    pub const SPELL_FAMILY_NAME: usize = 160;
    pub const SPELL_FAMILY_FLAGS_LOW: usize = 161;
    pub const SPELL_FAMILY_FLAGS_HIGH: usize = 162;
    /// `MinFactionId` and `MinReputation` — a faction and the standing rank
    /// (`0..7`, Hated to Exalted) the caster must have reached with it, read
    /// against [`crate::tables::reputation::RANK_FLOORS`]'s own table.
    ///
    /// **The shipped table has exactly one row in it** — spell 6994, `zzOLD`
    /// Feinting Strike, faction 369 at rank 4 — so this refusal cannot fire in
    /// 1.12. It is read because the rule is three lines and a column that is
    /// zero everywhere is the cheapest kind to get wrong later.
    pub const MIN_FACTION_ID: usize = 170;
    pub const MIN_REPUTATION: usize = 171;

    // --- what the *tooltip* reads, which is a different half of the record ---

    /// `maxLevel` and `baseLevel`, the two ends of the clamp the effect value's
    /// level term is taken over. Zero `maxLevel` means "no cap".
    pub const MAX_LEVEL: usize = 27;
    pub const BASE_LEVEL: usize = 28;
    /// `DurationIndex` -> `SpellDuration.dbc` [1] base ms. `$d`.
    pub const DURATION_INDEX: usize = 30;
    /// `Reagent[8]` and `ReagentCount[8]` — item entries, not names: `Item.dbc`
    /// is not in the archives, so what a rune is *called* is a
    /// `CMSG_ITEM_QUERY_SINGLE` away. Pinned on Teleport: Ironforge (3562),
    /// which reads 17031 x1 and whose tooltip says "Rune of Teleportation".
    /// **`EquippedItemClass` and its two masks** — `-1` for "no requirement",
    /// otherwise an item class the character must have equipped. It is why a
    /// warrior's Sunder Armor greys with no weapon out and why a hunter's
    /// Auto Shot greys with no ranged weapon.
    ///
    /// Their place is pinned by their neighbours rather than by memory:
    /// [`Self::REAGENT_COUNT`] is eight wide from 50 and [`Self::EFFECT`] is
    /// three wide from 61, which leaves exactly 58..60 for these.
    pub const EQUIPPED_ITEM_CLASS: usize = 58;
    pub const EQUIPPED_ITEM_SUBCLASS_MASK: usize = 59;
    pub const EQUIPPED_ITEM_INVENTORY_TYPE_MASK: usize = 60;
    pub const REAGENT: usize = 42;
    pub const REAGENT_COUNT: usize = 50;
    /// `RequiresSpellFocus` -> `SpellFocusObject.dbc` — the anvil, the fire.
    ///
    /// **This was 17 and 17 is [`Self::TARGET_AURA_STATE`].** The two columns
    /// are four apart and both hold small numbers that are valid focus ids, so
    /// nothing looked wrong: the trade-skill panel simply never said *Requires
    /// Anvil*. Pinned from both ends over all 22,360 rows instead of from one
    /// spell: **field 15 is non-zero on 695 of them and every one of those 129
    /// distinct values is a real `SpellFocusObject` id** — Copper Chain Belt
    /// (2661) reads 1 (Anvil), every cooked-food recipe reads 4 (Fire) — where
    /// field 17 is non-zero on 16 rows with one distinct value between them,
    /// which is what a `TargetAuraState` column looks like.
    pub const REQUIRES_SPELL_FOCUS: usize = 15;
    /// `Totem[2]` — the tool an enchant is worked with, as an item entry.
    /// Pinned on Enchant Bracer - Minor Health (7418), which reads 6218
    /// (Runed Copper Rod) at [40], and on Runed Copper Rod's own craft
    /// (7421), which reads 0 there and carries the plain rod among its
    /// reagents at [42] instead.
    pub const TOTEM: usize = 40;
    /// `Effect[3]` and the six per-effect columns the description substitutes
    /// from. Pinned on Fireball (133): effect 0 is base 13, dice 1, sides 9 —
    /// "14 to 22 Fire damage" at level 1, exactly what the retail tooltip says.
    pub const EFFECT: usize = 61;
    pub const EFFECT_DIE_SIDES: usize = 64;
    pub const EFFECT_BASE_DICE: usize = 67;
    pub const EFFECT_DICE_PER_LEVEL: usize = 70;
    pub const EFFECT_REAL_POINTS_PER_LEVEL: usize = 73;
    pub const EFFECT_BASE_POINTS: usize = 76;
    /// `EffectRadiusIndex[3]` -> `SpellRadius.dbc` [1] yards. `$a`/`$A`.
    pub const EFFECT_RADIUS_INDEX: usize = 88;
    /// `EffectAmplitude[3]`, the tick interval in ms. `$t`, and the divisor
    /// that turns a per-tick value into `$o`'s total.
    pub const EFFECT_AMPLITUDE: usize = 94;
    /// `EffectChainTarget[3]`. `$n`.
    pub const EFFECT_CHAIN_TARGET: usize = 100;
    /// **`EffectItemType[3]` — what a `CREATE_ITEM` effect makes.** Pinned by
    /// measurement: Minor Mana Potion (2331) reads item 2455 at [103], and the
    /// client's own trade-skill build reads the created item at spell record
    /// `+0x19c`, which is 103 dwords in. See
    /// [`crate::tables::tradeskill`].
    pub const EFFECT_ITEM_TYPE: usize = 103;
    /// **`EffectMiscValue[3]`**, which for an open-lock effect is the
    /// `EffectApplyAuraName[3]`, fields 91..93 — `SpellRec+0x16c`, which the
    /// stance-bar test walks three of looking for 36. Pinned by that use and by
    /// the shipped file: the five druid forms and the three warrior stances all
    /// carry 36 in one of the three, and nothing else the bar draws does.
    pub const EFFECT_APPLY_AURA: usize = 91;
    /// `LockType.dbc` row it opens: Mining reads 3, Herb Gathering 2, Pick Lock
    /// 1 and Opening 5. Measured against the shipped file rather than counted
    /// off a header — see the test that pins all four.
    pub const EFFECT_MISC_VALUE: usize = 106;
    /// `EffectTriggerSpell[3]` — **what a teaching spell actually teaches**, and
    /// the only way from a trainer's service to the ability it grants. Pinned by
    /// the client rather than by a value: it reads `[SpellRec+0x1b4]` off the
    /// `Spell.dbc` row it has just checked for effect 36, and
    /// `0x1b4 / 4` is 109. See [`crate::tables::trainer`].
    pub const EFFECT_TRIGGER_SPELL: usize = 109;
    /// `Description[8]`; `[138]` is enUS. Pinned on Teleport: Ironforge, which
    /// reads "Teleports the caster to Ironforge."
    pub const DESCRIPTION: usize = 138;
    /// `AuraDescription[8]`; `[147]` is enUS — **a different string from
    /// [`DESCRIPTION`] and the one a *buff* shows.**
    ///
    /// `Description` is what the spell does when you press it and
    /// `AuraDescription` is what having it on you means, and the two are
    /// worded from opposite ends: Power Word: Fortitude reads "Power infuses
    /// the target, increasing their Stamina by $s1 for $d." at 138 and
    /// "Increases Stamina by $s1." at 147. Showing the first on a buff icon
    /// is a tooltip that talks about casting a spell you are not casting —
    /// reported exactly that way.
    ///
    /// Pinned by position rather than by search: the eight-plus-flags
    /// localised blocks run `Name` 120, `Rank` 129, `Description` 138, so 147
    /// is the next one, and 1243's two strings above confirm it. Both carry
    /// the same `$` variables and go through the same
    /// [`crate::tables::spelltext::describe`].
    pub const AURA_DESCRIPTION: usize = 147;
    /// `MaxAffectedTargets`. `$i`.
    pub const MAX_AFFECTED_TARGETS: usize = 163;
}

/// `SpellCastTimes.dbc`: id, **base ms**, per level, minimum.
/// `SPELL_EFFECT_OPEN_LOCK`, and its item-flavoured twin. See
/// [`Spells::open_lock_spells`], which reads both.
const EFFECT_OPEN_LOCK: u32 = 3;
const EFFECT_OPEN_LOCK_ITEM: u32 = 33;

const CAST_TIME_BASE: usize = 1;
/// `SpellRange.dbc`: id, **min**, **max yards** as `f32`s, then names.
const RANGE_MIN: usize = 1;
const RANGE_MAX: usize = 2;

/// **The two `SpellRange.dbc` rows the range check is not allowed to treat as
/// numbers**, named as vmangos names them (`SPELL_RANGE_IDX_*`).
///
/// Row 1 is `0.0` and means *self only* — there is no distance to be wrong
/// about. Row 2 is the melee row, and its stated 5 yards is a placeholder: the
/// server's `Spell::CheckRange` branches out of the numeric path entirely for
/// it and asks `CanReachWithMeleeSpellAttack`, which is both units' combat
/// reaches and a leeway term this client cannot compute. Heroic Strike is on
/// row 2, so a client that took the 5 literally would refuse the warrior's
/// commonest ability while standing on top of its target.
const RANGE_IDX_SELF_ONLY: u32 = 1;
const RANGE_IDX_COMBAT: u32 = 2;
/// `SpellIcon.dbc`: id, **path**. Two fields and nothing else.
const ICON_PATH: usize = 1;
/// `SpellDuration.dbc`: id, **base ms**, per level, maximum. Four fields; row 1
/// is 10000/0/10000. All three are signed: row 427, Resurrection Sickness's,
/// is -600000 / 60000 / 600000, which is how "none below 11, a minute a level
/// to ten minutes" is stated as arithmetic.
const DURATION_BASE: usize = 1;
const DURATION_PER_LEVEL: usize = 2;
const DURATION_MAX: usize = 3;
/// `SpellRadius.dbc`: id, **yards** as an `f32`, per level, maximum. Row 13 is
/// 10.0, which is Arcane Explosion's "within 10 yards".
const RADIUS_BASE: usize = 1;

/// `Spell.dbc`'s `Attributes`, the three bits this client reads.
pub mod spell_attributes {
    /// `SPELL_ATTR_PASSIVE`. A passive is never cast — `HandleCastSpellOpcode`
    /// refuses one outright — so it must not reach the action bar even though it
    /// is in the spellbook. Half of a character's known spells are these.
    pub const PASSIVE: u32 = 0x0000_0040;
    /// **`SPELL_ATTR_DO_NOT_DISPLAY`: the client never puts this spell in the
    /// book at all.**
    ///
    /// The client's own "add this spell to the spellbook" reads the record's
    /// `Attributes` at `+0x18` and returns without adding it if bit 7 is set.
    ///
    /// The same test guards the *castable* list. vmangos names the
    /// bit `SPELL_ATTR_DO_NOT_DISPLAY` and comments it "not visible in spellbook
    /// or aura bar", which is the two sides agreeing.
    ///
    /// Without it a character who has been given the game's GM and world-buff
    /// spells opens a book full of `Interface\Icons\Temp` — "Hostile Intent",
    /// "Mass Heal", "Rallying Cry of the Dragonslayer" — under skill lines like
    /// `GENERIC (DND)` that the real client has no tab for. That is what a
    /// screenshot reported.
    pub const DO_NOT_DISPLAY: u32 = 0x0000_0080;
    /// `SPELL_ATTR_COOLDOWN_ON_EVENT`: the recovery starts when the effect
    /// *breaks* rather than when the spell is cast. Stealth, Feign Death — the
    /// cooldown record is inserted parked and released by
    /// `SMSG_COOLDOWN_EVENT`.
    pub const COOLDOWN_ON_EVENT: u32 = 0x0200_0000;

    /// **The two bits that make a press a *queued swing* rather than a cast.**
    ///
    /// vmangos' `SpellEntry::IsNextMeleeSwingSpell()` is
    /// `Attributes & (ON_NEXT_SWING_NO_DAMAGE | ON_NEXT_SWING)` and nothing
    /// else, and both halves of the server treat the result as a different
    /// *container* — `Spell::GetCurrentContainer` answers `CURRENT_MELEE_SPELL`,
    /// and `Spell::update`'s `SPELL_STATE_PREPARING` arm explicitly refuses to
    /// `cast()` one when its timer runs out. It is discharged by the next
    /// weapon swing and by nothing else.
    ///
    /// So the *animation* of one of these does not belong at the press: what a
    /// player sees is an ordinary attack, at whatever moment their swing timer
    /// comes round, and the spell's own art on top of it. Heroic Strike is the
    /// measured case — `Attributes 0x00050014`, which carries
    /// [`ON_NEXT_SWING_NO_DAMAGE`] — and it is the one nearly every warrior
    /// presses first.
    pub const ON_NEXT_SWING_NO_DAMAGE: u32 = 0x0000_0004;
    pub const ON_NEXT_SWING: u32 = 0x0000_0400;

    /// `SPELL_ATTR_ALLOW_CAST_WHILE_DEAD` — the handful a corpse may press.
    /// Read only so that the dead-caster refusal below does not swallow them.
    pub const ALLOW_CAST_WHILE_DEAD: u32 = 0x0080_0000;

    /// **The two bits that make a press an *auto-repeating ranged attack***,
    /// and both are needed: vmangos' `SpellEntry::IsAutoRepeatRangedSpell()` is
    /// `(Attributes & SPELL_ATTR_USES_RANGED_SLOT) && (AttributesEx2 &
    /// SPELL_ATTR_EX2_AUTO_REPEAT)` and nothing else, and the client asks it as
    /// the same pair of tests, always together.
    ///
    /// The first bit alone is *every* ranged ability — vmangos' own comment on
    /// it is "All ranged abilites have this flag" — so Aimed Shot, Multi-Shot
    /// and the one-off Throw (2764) carry it and are not this. Testing it alone
    /// would turn every hunter shot into a loop the player cannot stop.
    ///
    /// The pair is the whole of what makes Auto Shot different from a cast:
    /// `Spell::prepare` files it in `CURRENT_AUTOREPEAT_SPELL` instead of
    /// `CURRENT_GENERIC_SPELL`, `Spell::update`'s `PREPARING` arm refuses to
    /// `cast()` it when its timer runs out, and `Unit::_UpdateAutoRepeatSpell`
    /// then fires a *triggered* copy of it on the ranged attack timer for as
    /// long as it stands. So one `CMSG_CAST_SPELL` buys an indefinite stream of
    /// `SMSG_SPELL_GO`s, and the only way to stop it is
    /// `CMSG_CANCEL_AUTO_REPEAT_SPELL`.
    pub const USES_RANGED_SLOT: u32 = 0x0000_0002;
    pub const EX2_AUTO_REPEAT: u32 = 0x0000_0020;

    /// **`SPELL_ATTR_EX2_ALLOW_DEAD_TARGET`: this one may be cast at a corpse.**
    ///
    /// The exception half of [`super::SpellInfo::can_target_alive_state`]. It is
    /// also the second pin on [`super::spell_fields::ATTRIBUTES_EX2`]'s index:
    /// the column reads exactly `1` for spell **2584**, *Waiting to Resurrect* —
    /// the one spell in the game whose target is a corpse by definition, and the
    /// one vmangos hard-codes into `IsDeathOnlySpell` by id.
    pub const EX2_ALLOW_DEAD_TARGET: u32 = 0x0000_0001;

    /// **`SPELL_ATTR_EX3_ONLY_ON_GHOSTS`: …and this one may be cast at *nothing
    /// else*.** The third input to [`super::SpellInfo::death_only`], beside the
    /// corpse bits in `Targets` and spell 2584's id.
    pub const EX3_ONLY_ON_GHOSTS: u32 = 0x0000_1000;

    /// **`SPELL_ATTR_HELD_ITEM_ONLY`: the cast binds the main-hand item.**
    ///
    /// Poisons, sharpening stones, enchanting a weapon — the client picks the
    /// item out of the main-hand slot itself rather than asking. It is read here
    /// for exactly one thing: it is the *first* branch the client takes when the
    /// arm fails, and the answer is "Your weapon hand is
    /// empty" rather than the targeting cursor. vmangos names the same bit and
    /// comments it "Client automatically selects item from mainhand slot as a
    /// cast target", which is the two readings agreeing.
    pub const HELD_ITEM_ONLY: u32 = 0x0000_0200;

    /// **`SPELL_ATTR_TRADESPELL`: what a quest reward *says* it will teach.**
    ///
    /// Read for one sentence and nothing else. `QuestFrameItems_Update` chooses
    /// between `REWARD_SPELL` ("You will learn:") and `REWARD_TRADESKILL_SPELL`
    /// ("You will be able to craft:") on `GetRewardSpell`'s third answer, and
    /// that answer is this bit: `[SpellRec+0x18] & 0x20`,
    /// where `SpellRec+0x18` is `Attributes` — the same offset the auto-repeat
    /// pair above reads. vmangos names the bit `SPELL_ATTR_TRADESPELL`.
    pub const TRADESPELL: u32 = 0x0000_0020;

    /// **The two `AttributesEx` bits that make a cast a *channel*.**
    ///
    /// vmangos' `SpellEntry::IsChanneledSpell()` is
    /// `AttributesEx & (IS_CHANNELED | IS_SELF_CHANNELED)`. Both are needed and
    /// the second is the commoner: Evocation reads `AttributesEx 0x00000040`,
    /// which is `IS_SELF_CHANNELED` alone, so a test against the first bit only
    /// would answer "not a channel" for the game's own example of one.
    pub const IS_CHANNELED: u32 = 0x0000_0004;
    pub const IS_SELF_CHANNELED: u32 = 0x0000_0040;
    /// The pair as one mask, which is how the client's cast check reads it:
    /// `[SpellRec+0x1c] & 0x44`, and the answer is then carried through five
    /// of the refusals below.
    pub const CHANNELED: u32 = IS_CHANNELED | IS_SELF_CHANNELED;

    // --- the `Attributes` bits the caster-state chain reads, in bit order ---

    /// `SPELL_ATTR_DAYTIME_ONLY`. Refuses with
    /// `SPELL_FAILED_ONLY_DAYTIME` outside the window at [`super::DAY_BEGINS`].
    pub const DAYTIME_ONLY: u32 = 0x0000_1000;
    /// `SPELL_ATTR_NIGHT_ONLY` — the same window, the other way up.
    pub const NIGHT_ONLY: u32 = 0x0000_2000;
    /// `SPELL_ATTR_INDOOR_ONLY` and `SPELL_ATTR_OUTDOOR_ONLY`, tested together
    /// and then apart. The outdoor half has a
    /// second message — see [`SpellInfo::is_mount`].
    pub const INDOOR_ONLY: u32 = 0x0000_4000;
    pub const OUTDOOR_ONLY: u32 = 0x0000_8000;
    /// `SPELL_ATTR_ONLY_STEALTHED` — Pick Pocket, Ambush, Cheap
    /// Shot, Garrote.
    pub const ONLY_STEALTHED: u32 = 0x0002_0000;
    /// `SPELL_ATTR_CASTABLE_WHILE_MOUNTED`. Its **absence** is what
    /// refuses a cast on a mount, which is why nearly every spell in the game
    /// has to name it to be castable there and almost none do.
    pub const CASTABLE_WHILE_MOUNTED: u32 = 0x0100_0000;
    /// `SPELL_ATTR_CASTABLE_WHILE_SITTING` — likewise for a
    /// character on a chair, a stool or the ground.
    pub const CASTABLE_WHILE_SITTING: u32 = 0x0800_0000;
    /// `SPELL_ATTR_NOT_IN_COMBAT` — refuses with
    /// `SPELL_FAILED_AFFECTING_COMBAT` while `UNIT_FLAG_IN_COMBAT` stands.
    pub const NOT_IN_COMBAT: u32 = 0x1000_0000;

    /// **`AttributesEx` bit 28: never draw this aura's icon**, on anybody.
    ///
    /// A different column from the three above and a different question from
    /// [`DO_NOT_DISPLAY`]: that one is about the *spellbook*, this one is about
    /// the buff bar. The warrior stances are the case that makes it matter —
    /// Battle Stance (2457) reads `Attributes 0x09050010` with bit 7 **clear**,
    /// so the spellbook filter does not touch it, and `AttributesEx 0x90000000`
    /// with this bit set. Defensive Stance (71) is `0x10000000`, this bit
    /// alone. Without it every warrior in the game carries a stance icon in
    /// their buff bar and on their target frame, which the real client shows
    /// nowhere.
    ///
    /// vmangos names it `SPELL_ATTR_EX_NO_AURA_ICON` and comments it "Client
    /// doesn't display these spells in aura bar", which is the file and the
    /// server agreeing. It applies to **every** aura display rather than only
    /// to one's own bar — the target frame hides a target's stance too.
    pub const NO_AURA_ICON: u32 = 0x1000_0000;
}

/// `SpellDispelType.dbc`: id, then the name, then a second, sparser string.
///
/// **The second one is the rule and not a duplicate.** Field 11 is populated
/// for exactly rows 1..4 — Magic, Curse, Disease, Poison — and empty for the
/// other seven (None, Stealth, Invisibility, `All(M+C+D+P)`, `Special - npc
/// only`, Frenzy, `ZG Trinkets`). Those four are precisely the keys
/// `BuffFrame.lua`'s `DebuffTypeColor` table has, and `RefreshBuffs` indexes
/// that table with whatever `UnitDebuff` returns:
///
/// ```lua
/// if ( debuffType ) then debuffColor = DebuffTypeColor[debuffType];
/// else debuffColor = DebuffTypeColor["none"]; end
/// ```
///
/// — so returning "Stealth" there would index the table to `nil` and take the
/// panel down on the next line. The file states which four are sayable; this
/// client does not have to decide.
const DISPEL_TYPE_NAME: usize = 11;

/// `SpellShapeshiftForm.dbc`: **which action bar a form puts on the screen**.
///
/// Thirty-two rows, and the only column of them this client reads is field
/// **1**, `bonusActionBar` — the number `GetBonusBarOffset()` answers with.
/// `ActionButton_GetPagedID` turns it into a slot:
///
/// ```lua
/// return (button:GetID() + ((NUM_ACTIONBAR_PAGES + offset - 1) * NUM_ACTIONBAR_BUTTONS));
/// ```
///
/// — so offset 1 is slots 73..84, offset 2 is 85..96, and so on past the six
/// ordinary pages into the 120 the server sends. That is not a corner of the
/// protocol: it is where a **warrior's whole bar lives**. Measured on this
/// project's own test character, whose `SMSG_ACTION_BUTTONS` puts seventeen
/// buttons at slots 25, 63..66, 73..84 and 97..105 and **nothing at all in
/// 1..12** — page one of a stance-using class is empty by design, and a client
/// that only knows about pages draws an empty bar and looks broken.
///
/// The column is pinned by the file against a fact known independently: Battle
/// Stance (row 17) reads 1, Defensive (18) reads 2, Berserker (19) reads 3, Cat
/// Form (1) reads 1, and Travel Form (3) reads **0** — a form with no bar of its
/// own, which is exactly right for it. Those are also the two ends of the range
/// the character's own buttons occupy.
///
/// A form the file does not carry answers 0, which is the ordinary bar: the
/// degradation is "the page you were already on" rather than a wrong bar.
#[derive(Debug, Clone, Default)]
pub struct ShapeshiftForms {
    /// form id -> `bonusActionBar`. Zero entries are not stored, so a miss and
    /// a zero are the same answer.
    bars: HashMap<u32, u8>,
    /// form id -> the flags column. See [`ShapeshiftForms::cannot_be_cancelled`].
    flags: HashMap<u32, u32>,
}

/// **The flags bit that says a form cannot be shrugged off** —
/// `SpellShapeshiftForm.dbc` field 11, bit 1.
///
/// Set on the three warrior stances and clear on the five druid forms, which is
/// exactly the difference a player feels: pressing Bear Form while in bear form
/// drops it, and pressing Battle Stance while in Battle Stance does nothing at
/// all.
const FORM_CANNOT_BE_CANCELLED: u32 = 0x0000_0002;

impl ShapeshiftForms {
    /// `SpellShapeshiftForm.dbc` fields 1 and 11, per row. An unreadable or
    /// absent file gives the empty table, which answers 0 for everything.
    pub fn parse(raw: &[u8]) -> ShapeshiftForms {
        const BONUS_ACTION_BAR: usize = 1;
        const FLAGS: usize = 11;
        let mut bars = HashMap::new();
        let mut flags = HashMap::new();
        if let Ok(dbc) = Dbc::parse(raw) {
            for record in 0..dbc.record_count {
                let Some(id) = dbc.u32_at(record, 0) else {
                    continue;
                };
                if let Some(bar) = dbc.u32_at(record, BONUS_ACTION_BAR) {
                    if bar != 0 && bar <= u32::from(u8::MAX) {
                        bars.insert(id, bar as u8);
                    }
                }
                if let Some(value) = dbc.u32_at(record, FLAGS).filter(|f| *f != 0) {
                    flags.insert(id, value);
                }
            }
        }
        ShapeshiftForms { bars, flags }
    }

    /// `GetBonusBarOffset()` for a unit in `form`. Zero for no form, for a form
    /// with no bar of its own, and for a chain with no table.
    pub fn bonus_bar(&self, form: u8) -> u8 {
        self.bars.get(&u32::from(form)).copied().unwrap_or(0)
    }

    /// **Does pressing this form's own button while it is on do nothing?** —
    /// see [`FORM_CANNOT_BE_CANCELLED`]; the client returns before either the
    /// cancel or the cast when it is set.
    pub fn cannot_be_cancelled(&self, form: u32) -> bool {
        self.flags.get(&form).copied().unwrap_or(0) & FORM_CANNOT_BE_CANCELLED != 0
    }

    /// How many forms carry a bar — the census `vale spellbook` prints, and
    /// the number that would read 0 if the column were wrong.
    pub fn with_a_bar(&self) -> usize {
        self.bars.len()
    }
}

/// One spell, as a button needs it.
///
/// `Default` is for the tests and the CLI's own stand-ins: this has grown past
/// twenty fields and most of them are irrelevant to any one check, so a test
/// that cares about the rank order should say so and not fill in a radius.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpellInfo {
    pub id: u32,
    pub name: String,
    /// "Rank 1", or empty. Shown under the name in the game's own tooltip, and
    /// the only way to tell six Fireballs apart.
    pub rank: String,
    /// `Interface\Icons\...`, already the path the archive holds. Empty when the
    /// icon table is absent or the id is not in it.
    pub icon: String,
    /// How long the cast bar runs, in milliseconds. 0 for an instant.
    pub cast_time_ms: u32,
    /// `enum Powers`: 0 mana, 1 rage, 2 focus, 3 energy.
    pub power_type: u32,
    pub power_cost: u32,
    /// Maximum range in yards, 0 for a self-only spell.
    pub range_yards: f32,
    /// …and the minimum, which is 0 for all but the ranged weapon abilities and
    /// a handful of hunter shots. `SpellRange.dbc` field 1, read for exactly one
    /// answer: `SPELL_FAILED_TOO_CLOSE`.
    pub min_range_yards: f32,
    /// **Which `SpellRange.dbc` row the two above came from**, carried because
    /// two of the rows are not distances at all — see [`RANGE_IDX_SELF_ONLY`]
    /// and [`RANGE_IDX_COMBAT`]. Without it a melee ability's placeholder 5 is
    /// indistinguishable from a real five-yard reach.
    pub range_index: u32,
    /// The spell's own cooldown, in milliseconds.
    pub recovery_ms: u32,
    /// …and its category's, with the category it shares.
    pub category: u32,
    pub category_recovery_ms: u32,
    /// The global cooldown: which bucket, and how long. Usually 133 / 1500 for
    /// anything that has one; 0 for the abilities that are off the GCD.
    pub gcd_category: u32,
    pub gcd_ms: u32,
    /// **`InterruptFlags`** — bit 0 is `SPELL_INTERRUPT_FLAG_MOVEMENT`, and it
    /// is what says a cast may not be *begun* on the move. See
    /// [`check_cast`]'s moving rule, which is where all three of these are read
    /// and where the addresses are.
    pub interrupt_flags: u32,
    /// `AuraInterruptFlags` — read for `MOVING_CANCELS | TURNING_CANCELS`
    /// (`0x18`), the pair that makes a *zero-cast-time* spell refuse on the
    /// move as well: eating, drinking, Stealth's own sit.
    pub aura_interrupt_flags: u32,
    /// `ChannelInterruptFlags` — the same `0x18`, for a channel.
    pub channel_interrupt_flags: u32,
    pub attributes: u32,
    /// `AttributesEx` — see [`spell_attributes::NO_AURA_ICON`], the one bit of
    /// it this client reads.
    pub attributes_ex: u32,
    /// `AttributesEx2` — see [`spell_attributes::EX2_AUTO_REPEAT`], likewise
    /// one bit, and the half of the auto-repeat test that is not in
    /// [`Self::attributes`].
    pub attributes_ex2: u32,
    /// `AttributesEx3` — see [`spell_attributes::EX3_ONLY_ON_GHOSTS`], likewise
    /// one bit, and one of the three inputs to [`Self::death_only`].
    pub attributes_ex3: u32,
    /// **What a debuff's border is coloured by**, already resolved to the
    /// game's own word — `"Magic"`, `"Curse"`, `"Disease"`, `"Poison"` — or
    /// empty for the seven dispel classes the interface has no colour for. See
    /// [`DISPEL_TYPE_NAME`], which is where the file states which four those
    /// are.
    pub dispel_type: String,
    /// Field 13, the aiming mask's seed.
    pub targets: u32,
    /// **The conditions a spell may only be cast under**, straight off
    /// `Spell.dbc` — see [`spell_fields::CASTER_AURA_STATE`] and its
    /// neighbours for what each is and which spell it is famous for.
    ///
    /// Zero, zero, zero, zero and `-1` are "no condition", which is the
    /// overwhelming majority of the table; they are carried as read so that
    /// [`SpellInfo::castable_now`] is one function rather than five lookups at
    /// the call site.
    pub caster_aura_state: u32,
    pub target_aura_state: u32,
    pub stances: u32,
    pub stances_not: u32,
    /// **Which silencing word stops this spell** — see
    /// [`spell_fields::PREVENTION_TYPE`]. 1 is silence, 2 is pacify, 0 neither,
    /// and it is the gate on two of the six [`caster_flags`].
    pub prevention_type: u32,
    /// **Which class's book this row is in, and which of its 64 bits it is** —
    /// see [`spell_fields::SPELL_FAMILY_NAME`]. The pair is what a talent's
    /// modifier packet is matched against, and it is the only thing that can be:
    /// the packet names a bit and no family at all.
    pub spell_family: u32,
    pub spell_family_flags: u64,
    /// **A faction and the standing rank required with it** — see
    /// [`spell_fields::MIN_FACTION_ID`], which also records that the shipped
    /// table has one row and it is disabled.
    pub min_faction_id: u32,
    pub min_reputation: u32,
    /// `-1` for "no equipped-item requirement", which is why this is signed.
    pub equipped_item_class: i32,
    pub equipped_item_subclass_mask: u32,
    pub equipped_item_inventory_type_mask: u32,
    /// Field 82, its adjustment.
    pub implicit_target_a: u32,
    /// `castUI`, field 3 — see [`spell_fields::CAST_UI`] and
    /// [`SpellInfo::craft_kind`].
    pub cast_ui: u32,
    /// **The picture a form button wears while its form is on**, already
    /// resolved to a path — see [`spell_fields::ACTIVE_ICON_ID`]. Empty for
    /// every spell that does not carry one, which is nearly all of them.
    pub active_icon: String,
    /// …and where the form sits on the stance bar, `-1` for no position — see
    /// [`spell_fields::SHAPESHIFT_ORDER`].
    pub shapeshift_order: i32,
    /// `spellLevel`, field 29 — see [`spell_fields::SPELL_LEVEL`]. Carried for
    /// [`SpellInfo::rank_order`] and nothing else.
    pub spell_level: u32,

    // --- what the tooltip prints under the numbers ---
    /// **The sentence**, straight out of the file and still carrying its `$`
    /// variables: "Hurls a fiery ball that causes $s1 Fire damage and an
    /// additional $o2 Fire damage over $d."
    ///
    /// Unsubstituted on purpose. Turning it into words needs the *caster's*
    /// level and, for a `$<id>s1`, another spell's row — neither of which a
    /// single record knows. [`crate::tables::spelltext::describe`] is where that
    /// happens, and it takes this plus the level.
    pub description: String,
    /// **…and the sentence a *buff icon* shows**, which is a different string
    /// in the same row — see [`spell_fields::AURA_DESCRIPTION`]. Empty for
    /// everything that is not an aura, which is most of the table, and empty
    /// is what the tooltip then falls back to [`Self::description`] on.
    pub aura_description: String,
    /// `Reagent[8]` paired with its count, zeroes dropped. **Item entries**:
    /// `Item.dbc` is not in the archives, so the name is a server query away.
    pub reagents: Vec<(u32, u32)>,
    /// `Totem[2]` — the tool a craft is worked with (the runed rod), as item
    /// entries, zeroes kept in place. See [`spell_fields::TOTEM`].
    pub totems: [u32; 2],
    /// `RequiresSpellFocus` — the `SpellFocusObject.dbc` row the caster must
    /// stand near (the anvil, the forge), 0 for none.
    pub focus_object: u32,
    /// How long the aura lasts, in milliseconds — `$d`, and the divisor behind
    /// `$o`'s tick count. 0 for a spell with no duration at all.
    pub duration_ms: u32,
    /// The two ends of the clamp the level term is taken over — see
    /// [`SpellEffect::value_at`]. `max_level` 0 means "no cap".
    pub max_level: u32,
    pub base_level: u32,
    /// `MaxAffectedTargets` — `$i`.
    pub max_affected_targets: u32,
    /// `ProcChance` — `$h`, a per-cent. 101 for a spell with no proc at all,
    /// which is the file's own "always" and is why the token reads as a
    /// percentage without a divisor.
    pub proc_chance: u32,
    /// The three effect slots, in the order the `$s1`/`$s2`/`$s3` suffixes
    /// index them (one-based in the string, zero-based here).
    pub effects: [SpellEffect; 3],
}

/// One of a spell's three effect slots, as far as the **description** needs it.
///
/// Nothing here decides what an effect *does* — that is the server's business
/// and this client never computes it. What these columns are for is the
/// sentence: `$s1` is a number the client prints, and it prints it from the
/// same fields the server rolls the real value from.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpellEffect {
    /// `Effect[i]` — the effect id. Read only so that an empty slot (0) can be
    /// told from one that happens to roll zero.
    pub kind: u32,
    pub base_points: i32,
    pub die_sides: i32,
    pub base_dice: i32,
    pub dice_per_level: f32,
    pub real_points_per_level: f32,
    /// Yards, off `SpellRadius.dbc` — `$a`/`$A`.
    pub radius_yards: f32,
    /// The tick interval in ms — `$t`, and what divides the duration into the
    /// tick count `$o` multiplies by.
    pub amplitude_ms: u32,
    /// `$n`.
    pub chain_targets: u32,
    /// `EffectTriggerSpell[i]` — the spell this effect casts, or **teaches**.
    /// Zero for the great majority of effects; read because a trainer's service
    /// is a teaching spell whose whole content is this field. See
    /// [`crate::tables::trainer`].
    pub trigger_spell: u32,
    /// `EffectApplyAuraName[i]` — **which aura this effect applies**, 0 for an
    /// effect that applies none. Read for one value: 36, `MOD_SHAPESHIFT`,
    /// which is what makes a spell a stance-bar button. See
    /// [`SpellInfo::is_shapeshift_button`].
    pub aura: u32,
    /// `EffectMiscValue[i]` — what the effect id makes of it: a lock type for
    /// an open-lock effect, a skill line for a `SKILL` effect, and the craft
    /// kind for a `TRADE_SKILL` one. Signed because the file stores -1 in a
    /// few rows. See [`crate::tables::tradeskill::opens`].
    pub misc_value: i32,
    /// `EffectItemType[i]` — what a `CREATE_ITEM` effect makes, as an item
    /// entry. See [`crate::tables::tradeskill`].
    pub item_type: u32,
    /// `EffectImplicitTargetA[i]` and `B[i]` — where the effect lands. Read
    /// for one test: a learn effect aimed at `TARGET_PET` (5) is a Beast
    /// Training row. See [`crate::tables::tradeskill::List::build_training`].
    pub target_a: u32,
    pub target_b: u32,
}

impl SpellEffect {
    /// **The two ends of the value this effect rolls at `level`** — the number
    /// `$s`, `$m` and `$M` print.
    ///
    /// This is vmangos' `SpellCaster::CalculateSpellEffectValue`
    /// (`Objects/SpellCaster.cpp`), which is an authority on the arithmetic and
    /// not on the presentation: the level is clamped into
    /// `[baseLevel, maxLevel]` and then measured from `spellLevel`, the base
    /// grows by `realPointsPerLevel` and the die by `dicePerLevel`, and the
    /// roll runs from `baseDice` to `dieSides`.
    ///
    /// Checked against the retail client rather than only against the source:
    /// Fireball (133) at level 1 comes out **14 to 22**, which is what 1.12's
    /// own tooltip says.
    ///
    /// **What is deliberately not applied is every modifier the caster wears** —
    /// talents, auras, spell power. The real client folds its own
    /// `SPELLMOD_ALL_EFFECTS` in and this one has no talent data at all, so a
    /// character with points in a damage talent reads a few points low here.
    /// That is a stated shortfall rather than a hidden one: it is the same
    /// direction for every spell and it is the number the file states.
    pub fn value_at(&self, level: u32, spell_level: u32, base_level: u32, max_level: u32) -> (i32, i32) {
        let mut level = level as i32;
        if max_level > 0 && level > max_level as i32 {
            level = max_level as i32;
        } else if level < base_level as i32 {
            level = base_level as i32;
        }
        let level = (level - spell_level as i32).max(0);
        let base = self.base_points as f32 + level as f32 * self.real_points_per_level;
        let sides = (self.die_sides as f32 + level as f32 * self.dice_per_level) as i32;
        let base = base as i32;
        // The server's own ordering: `randomPoints` of 0 or 1 is not a roll at
        // all and contributes `baseDice` flat, which is why a one-sided effect
        // prints a single number rather than "n to n".
        if sides <= 1 {
            let flat = base + self.base_dice;
            return (flat, flat);
        }
        let (low, high) = if self.base_dice >= sides {
            (sides, self.base_dice)
        } else {
            (self.base_dice, sides)
        };
        (base + low, base + high)
    }
}

/// The widest the ground-target circle is ever drawn, in yards — the client's
/// own clamp, against the literal 20.0.
pub const GROUND_CIRCLE_MAX_YARDS: f32 = 20.0;

/// …and the width it falls back to when the spell states no radius at all, and
/// the width the **red** circle always takes.
///
/// `0x3fb1c71c`, used when the radius the aiming tick computes is
/// exactly zero — which the refusing branch sets it to on purpose.
/// So the two circles differ in size as well as in colour, and the small one is
/// not a degradation: a refused placement has no area to show.
///
/// Written as its own bits rather than as a decimal: the literal is the
/// four bytes the client carries, and a rounded 1.388889 is a *different* f32.
pub const GROUND_CIRCLE_MIN_YARDS: f32 = f32::from_bits(0x3fb1_c71c);

impl SpellInfo {
    /// **Is this spell castable *right now*?** — the conditions a `Spell.dbc`
    /// row states, against what the character and the target are.
    ///
    /// This is the half of `IsUsableAction` that is not about mana, and it is
    /// the half that answers every "why is this greyed out" a player asks:
    ///
    /// ```text
    /// Judgement          casterAuraState = 5   a Seal is up
    /// Revenge            casterAuraState = 1   you blocked, parried or dodged
    /// Riposte            casterAuraState = 7
    /// Execute            targetAuraState = 2   the target is under a fifth
    /// Eviscerate         AttributesEx bit 20   combo points on the target
    /// Ambush             stancesNot            …and not out of stealth
    /// Sunder Armor       equippedItemClass 2   a weapon in hand
    /// ```
    ///
    /// **The client never learns what a Seal is**, and that is the point of
    /// doing it this way: the server sets a bit in `UNIT_FIELD_AURASTATE` when
    /// the aura lands and clears it when it goes, and the button follows the
    /// bit. The same three fields cover a dozen abilities across five classes
    /// with no per-spell knowledge anywhere.
    ///
    /// ## What each argument is, and what `None` means
    ///
    /// `caster_state` and `target_state` are `UNIT_FIELD_AURASTATE`;
    /// `combo_points` is `PLAYER_FIELD_BYTES` byte 1; `form` is the shapeshift
    /// form byte. A `None` target is **no target selected**, and a target-side
    /// condition then reads as unmet — which is the reference's own answer, and
    /// is why Execute is grey with nothing selected.
    ///
    /// ## What is deliberately not here
    ///
    /// The **equipped-item** test takes the three columns and this signature
    /// does not: answering it needs the character's weapons, which is the
    /// inventory's business and not this table's. It is stated in
    /// [`Self::equipped_item_class`] and applied by the caller when there is
    /// one. Nor is the *range* test, which has its own function, nor the
    /// cooldown, which is the swirl rather than the fade.
    pub fn castable_now(
        &self,
        caster_state: u32,
        target_state: Option<u32>,
        combo_points: u8,
        form: u8,
    ) -> bool {
        // The bit for state *n* is `1 << (n - 1)` — vmangos' `ModifyAuraState`
        // writes exactly that, and state 0 is "no condition" rather than bit 0.
        let has = |state: u32, mask: u32| state == 0 || mask & (1 << (state - 1)) != 0;
        if !has(self.caster_aura_state, caster_state) {
            return false;
        }
        // **No target is an unmet target condition**, not an absent one: a
        // spell that asks something of its target and has none cannot be cast,
        // which is why Execute greys with nothing selected.
        if self.target_aura_state != 0 && !target_state.is_some_and(|s| has(self.target_aura_state, s)) {
            return false;
        }
        if self.needs_combo_points() && combo_points == 0 {
            return false;
        }
        // The form masks are `1 << (form - 1)` on the same terms, and form 0 —
        // no form at all — is in neither mask: a spell that names *any* stance
        // cannot be cast unshifted, and one that excludes stances can.
        let in_form = |mask: u32| form != 0 && mask & (1 << (form - 1)) != 0;
        if self.stances != 0 && !in_form(self.stances) {
            return false;
        }
        if self.stances_not != 0 && in_form(self.stances_not) {
            return false;
        }
        true
    }

    /// **Does this spell spend combo points?** — `AttributesEx` bits 20 and 22,
    /// which is vmangos' own `SpellEntry::NeedsComboPoints`:
    ///
    /// ```cpp
    /// return (AttributesEx & (SPELL_ATTR_EX_FINISHING_MOVE_DAMAGE
    ///                       | SPELL_ATTR_EX_FINISHING_MOVE_DURATION));
    /// ```
    ///
    /// Two bits rather than one because the game splits finishers by what the
    /// points scale — Eviscerate's damage against Rupture's duration — and a
    /// client that read only the first greys half the rogue's bar correctly and
    /// half of it not at all.
    pub fn needs_combo_points(&self) -> bool {
        const FINISHING_MOVE_DAMAGE: u32 = 0x0010_0000;
        const FINISHING_MOVE_DURATION: u32 = 0x0040_0000;
        self.attributes_ex & (FINISHING_MOVE_DAMAGE | FINISHING_MOVE_DURATION) != 0
    }

    /// A passive is in the spellbook and cannot be cast — see
    /// [`spell_attributes::PASSIVE`].
    pub fn is_passive(&self) -> bool {
        self.attributes & spell_attributes::PASSIVE != 0
    }

    /// **Does this spell get a button on the stance bar?** — the whole of the
    /// test the client applies to every spell it learns.
    ///
    /// ```text
    /// 4b27fa   test  $0x2,%dl          ; AttributesEx2 bit 1 -> never a button
    /// 4b27fd   jne   ...
    /// 4b2810   cmpl  $0x24,(%ecx)      ; EffectApplyAuraName[i] == 36
    /// 4b2813   je    ...               ;   MOD_SHAPESHIFT -> a button
    /// 4b2815   test  $0x10,%dl         ; …or AttributesEx2 bit 4 forces one
    /// ```
    ///
    /// **Both halves are load-bearing against the shipped file.** The forcing
    /// bit puts 44 rows on the bar that apply no shapeshift aura at all — every
    /// paladin aura, which is what the class's bar is made of — and the
    /// excluding bit takes five rows off it that do apply one, Ghost Wolf and
    /// Shadowform among them. A reader with only the aura test would give a
    /// paladin an empty bar and a shaman a button that should not be there.
    pub fn is_shapeshift_button(&self) -> bool {
        const EX2_NEVER_A_FORM_BUTTON: u32 = 0x0000_0002;
        const EX2_ALWAYS_A_FORM_BUTTON: u32 = 0x0000_0010;
        if self.attributes_ex2 & EX2_NEVER_A_FORM_BUTTON != 0 {
            return false;
        }
        self.attributes_ex2 & EX2_ALWAYS_A_FORM_BUTTON != 0 || self.shapeshift_form() != 0
    }

    /// **Which form this spell puts the character into**, or 0 for a button that
    /// is not a form at all — a paladin's aura, or Stealth.
    ///
    /// The `EffectMiscValue` of the first effect applying aura 36, and it is
    /// what `GetShapeshiftFormInfo` compares
    /// against `UNIT_FIELD_BYTES_1`'s form byte to decide which button is
    /// pressed in.
    pub fn shapeshift_form(&self) -> u32 {
        const MOD_SHAPESHIFT: u32 = 36;
        self.effects
            .iter()
            .find(|effect| effect.aura == MOD_SHAPESHIFT)
            .map_or(0, |effect| effect.misc_value.max(0) as u32)
    }

    /// **How wide the green circle is drawn for a placed cast**, in yards.
    ///
    /// The shape of the reference's computation is three
    /// things this client would otherwise have guessed:
    ///
    /// * it is **`EffectRadiusIndex` through `SpellRadius.dbc`**, not the
    ///   spell's range and not the model's own extent;
    /// * it is the **larger of effects 0 and 1** and effect 2 is not consulted
    ///   at all;
    /// * it is **clamped to [`GROUND_CIRCLE_MAX_YARDS`]**, so a 100-yard row
    ///   (`SpellRadius` id 12) draws a 20-yard circle rather than covering the
    ///   screen.
    ///
    /// The reference's own term is `Radius + casterLevel * RadiusPerLevel`.
    /// **That column is 0.0 in all 24 shipped rows** (`vale spellbook`), so
    /// the level never enters and the caster is not threaded through here.
    /// Stated rather than silently dropped: a table where it were not zero would
    /// need the level, and this is where it would go.
    ///
    /// Zero for a spell whose two rows are both zero, which the caller reads as
    /// [`GROUND_CIRCLE_MIN_YARDS`] — the same branch the refusal takes.
    pub fn ground_circle_yards(&self) -> f32 {
        self.effects[0]
            .radius_yards
            .max(self.effects[1].radius_yards)
            .clamp(0.0, GROUND_CIRCLE_MAX_YARDS)
    }

    /// **Is this spell's range a number this client may measure against?**
    ///
    /// Two questions in the game wear this answer and neither may be asked
    /// without it: whether to refuse a press locally ([`check_cast`]) and
    /// whether the action button draws a range indicator
    /// (`game::api::action_has_range`). Both used to spell the predicate out —
    /// or, in the second case, answer `nil` for everything.
    ///
    /// False for four kinds of row and each for its own reason:
    ///
    /// * [`RANGE_IDX_SELF_ONLY`], where there is no distance to be wrong about;
    /// * [`RANGE_IDX_COMBAT`], the melee row, whose stated 5 yards is a
    ///   placeholder for a reach the server computes from both units' bulk plus
    ///   a leeway term that depends on how fast they are *both moving* — see
    ///   the note on that constant. Heroic Strike is on it;
    /// * a next-swing ability, which `Spell::CheckRange` returns OK for before
    ///   it looks at any distance at all;
    /// * a row this build's `SpellRange.dbc` did not carry, which reads 0.
    pub fn checks_range(&self) -> bool {
        self.range_index != RANGE_IDX_SELF_ONLY
            && self.range_index != RANGE_IDX_COMBAT
            && !self.on_next_swing()
            && self.range_yards > 0.0
    }

    /// **…and this one is not in the spellbook at all** — see
    /// [`spell_attributes::DO_NOT_DISPLAY`], which is the client's own filter
    /// rather than a rule invented here.
    ///
    /// **Not the whole of the book's filter.** A recipe is kept out by a second
    /// bit and a different mechanism — see [`SpellInfo::recipe`] and
    /// [`SpellInfo::in_book`], which is the one to ask. This bit is the one the
    /// *aura* display shares, which is why it is still its own predicate.
    pub fn hidden(&self) -> bool {
        self.attributes & spell_attributes::DO_NOT_DISPLAY != 0
    }

    /// **…and this one is a recipe**, which is not in the spellbook either —
    /// see [`spell_attributes::TRADESPELL`].
    ///
    /// It is the same bit the quest reward reads to say "You will be able to
    /// craft:" rather than "You will learn:", which is what makes it the recipe
    /// test rather than an approximation of one.
    pub fn recipe(&self) -> bool {
        self.attributes & spell_attributes::TRADESPELL != 0
    }

    /// **The whole of the client's spellbook filter, in one door** — the first
    /// two things "add this spell to the book" does with the record it was
    /// handed:
    ///
    /// ```text
    /// Attributes bit 7                -> not added
    /// Attributes bit 5, TRADESPELL    clear -> the ordinary path
    ///   …set: was it *just* learned?  no  -> not added
    ///                                 yes -> announced, and not added
    /// ```
    ///
    /// So a recipe **never** reaches the book: at a login it is dropped outright,
    /// and when one is learned mid-session the function's whole business is to
    /// say so in the chat and return. The trade-skill window is where it is
    /// listed, off the same known-set — see
    /// [`crate::tables::tradeskill`], which reads `world.spellbook.known`
    /// directly and is unaffected by this.
    ///
    /// Both bits together are what keeps *Honorless Target* (2479,
    /// `Attributes 0x09000120`) out of a book this client used to show it in:
    /// bit 7 is clear on that row, so the older filter passed it.
    ///
    /// **The third test is `castUI`**, field 3, read at `+0xc` after the two
    /// bit tests and the learned-spell announcement: above zero, and the spell
    /// is not added.
    ///
    /// The hunter's pet-training spells carry 1 there (Great Stamina Rank 1,
    /// 4195: `Attributes 0x00040100`, neither bit set, `castUI 1`) and every
    /// Enchanting recipe carries 3, and the number is the craft window's kind
    /// — see [`Self::craft_kind`]. Without this test a hunter who has visited
    /// a pet trainer has seven ranks of Great Stamina and four of Frost
    /// Resistance on the General tab.
    ///
    /// **None of the three reaches the other two lists.** The client's flat
    /// "spells you know" array tests bit 7 and nothing
    /// else, so a recipe stays castable and stays draggable to an action bar,
    /// which is what a player expects of a cooking recipe.
    pub fn in_book(&self) -> bool {
        !self.hidden() && !self.recipe() && self.cast_ui == 0
    }

    /// **Which craft window lists this spell**, or `None` for a spell that
    /// belongs in the book: `castUI` above zero. 1 is Beast Training's list
    /// (`TRAIN`) and 3 is Enchanting's (`ENSCRIBE`); the player object keeps
    /// one list per kind at `+0x1cd0` and the craft build walks
    /// the one the opening spell's `EffectMiscValue[0]` names, so the two
    /// numbers are the same table read from both ends. Pinned from the data:
    /// 7418 (Enchant Bracer - Minor Health) reads 3, 24533 and 4195 (the
    /// trainer's row and the hunter's own Great Stamina) read 1, and the
    /// openers 2259, 7411 and 5149 read 0.
    pub fn craft_kind(&self) -> Option<u32> {
        (self.cast_ui != 0).then_some(self.cast_ui)
    }

    /// **…and this one's aura is never drawn as an icon**, which is a different
    /// column and a different display — see [`spell_attributes::NO_AURA_ICON`].
    ///
    /// Either bit hides an aura: `DO_NOT_DISPLAY`'s own comment in vmangos is
    /// "not visible in spellbook **or aura bar**", so the spellbook filter is a
    /// subset of this one rather than an alternative to it.
    pub fn no_aura_icon(&self) -> bool {
        self.hidden() || self.attributes_ex & spell_attributes::NO_AURA_ICON != 0
    }

    /// Whether its cooldown starts on the effect breaking rather than on the
    /// cast.
    pub fn cooldown_on_event(&self) -> bool {
        self.attributes & spell_attributes::COOLDOWN_ON_EVENT != 0
    }

    /// **Is this press a queued swing rather than a cast?** — see
    /// [`spell_attributes::ON_NEXT_SWING`], which is where the consequence is
    /// written down. Heroic Strike, Cleave, Raptor Strike, Maul.
    pub fn on_next_swing(&self) -> bool {
        self.attributes
            & (spell_attributes::ON_NEXT_SWING | spell_attributes::ON_NEXT_SWING_NO_DAMAGE)
            != 0
    }

    /// **Is this a channel?** — see [`spell_attributes::IS_CHANNELED`].
    ///
    /// A channel is an *instant* on the wire: `SMSG_SPELL_START` is not sent
    /// and `SMSG_SPELL_GO` fires at once, and the only packet that says how long
    /// it runs is `MSG_CHANNEL_START`, which arrives afterwards. So this bit is
    /// what lets the press be told apart from a genuine instant before either
    /// packet has been seen.
    pub fn is_channelled(&self) -> bool {
        self.attributes_ex
            & (spell_attributes::IS_CHANNELED | spell_attributes::IS_SELF_CHANNELED)
            != 0
    }

    /// **Is this press a *ranged weapon* attack?** — `SPELL_ATTR_USES_RANGED_SLOT`
    /// on its own, which is every hunter shot and every wand.
    ///
    /// Read for one thing: what the caster is drawn doing. A ranged ability is
    /// fired from the ranged slot, so it is the bow that comes out and the bow
    /// that is drawn back, whatever the spell's own visual says about the
    /// missile. See [`Self::is_auto_repeat_ranged`] for the narrower question.
    pub fn uses_ranged_slot(&self) -> bool {
        self.attributes & spell_attributes::USES_RANGED_SLOT != 0
    }

    /// **…and is it the *repeating* one?** — see
    /// [`spell_attributes::EX2_AUTO_REPEAT`], where the consequence is written
    /// down. Auto Shot and the wand's Shoot, and in 5875's data those two and
    /// nothing else a player can press.
    ///
    /// The pair rather than either half: the first bit is every ranged ability
    /// and the second appears on a handful of creature spells with no ranged
    /// slot at all.
    pub fn is_auto_repeat_ranged(&self) -> bool {
        self.uses_ranged_slot() && self.attributes_ex2 & spell_attributes::EX2_AUTO_REPEAT != 0
    }

    /// **Is this spell aimed at the dead and only at the dead?** — vmangos'
    /// `SpellEntry::IsDeathOnlySpell`, all three of its inputs.
    ///
    /// A resurrection, a corpse retrieval, *Waiting to Resurrect*. The two
    /// corpse bits in `Targets` are the same ones [`aim_mask`] feeds to the
    /// binder, so for those spells this and [`unsatisfied`]'s corpse arms are
    /// two readings of one column and cannot disagree.
    pub fn death_only(&self) -> bool {
        const CORPSE_BITS: u32 = 0x0200 | 0x8000;
        self.attributes_ex3 & spell_attributes::EX3_ONLY_ON_GHOSTS != 0
            || self.targets & CORPSE_BITS != 0
            // `Waiting to Resurrect`, which vmangos names by id because its own
            // columns do not say so.
            || self.id == 2584
    }

    /// **May this spell be cast at a unit in that state?** — vmangos'
    /// `SpellEntry::CanTargetAliveState`, verbatim.
    ///
    /// A death-only spell wants a corpse and nothing else; everything else wants
    /// a living target unless it carries
    /// [`spell_attributes::EX2_ALLOW_DEAD_TARGET`].
    pub fn can_target_alive_state(&self, alive: bool) -> bool {
        if self.death_only() {
            return !alive;
        }
        alive || self.attributes_ex2 & spell_attributes::EX2_ALLOW_DEAD_TARGET != 0
    }

    /// **Would the client refuse to *begin* this cast at these movement
    /// flags?** — the client's rule, and the missing half of the
    /// "spells begin casting and are then interrupted" report.
    ///
    /// ```text
    /// InterruptFlags & MOVEMENT                clear -> ok
    /// movement flags & 0x200f                  clear -> ok   (forward|back|strafe|jumping)
    /// AttributesEx2 & AUTO_REPEAT              set   -> ok
    /// a cast time                              yes   -> fail
    /// AuraInterruptFlags & MOVING|TURNING      set   -> fail
    /// ChannelInterruptFlags & MOVING|TURNING   clear -> ok
    /// fail: SPELL_FAILED_MOVING
    /// ```
    ///
    /// Three things in it are easy to get wrong and each is load-bearing:
    ///
    /// * **Turning is not moving.** The mask stops at `0xf` and the two turn
    ///   flags are `0x10`/`0x20`, so a character spinning on the spot may start
    ///   a cast. Reaching for `MOVEFLAG_MASK_MOVING` instead would refuse them.
    /// * **An auto-repeat is exempt**, which is the same exception vmangos'
    ///   `IsAcceptableAutorepeatError` makes from the other side: a hunter may
    ///   arm Auto Shot on the run and it fires when they stop.
    /// * **A zero-cast-time spell can still be refused**, through the two
    ///   interrupt words — which is what makes eating and drinking refuse on
    ///   the move where an instant Shadow Word: Pain does not.
    ///
    /// Brown Horse (458) is the worked example and the report: `InterruptFlags`
    /// 31, a 3,000 ms cast, and no auto-repeat bit — so pressing a mount at a
    /// run is refused here, where before it went out, drew its wind-up, and
    /// came back interrupted.
    pub fn refused_while_moving(&self, move_flags: u32) -> bool {
        const MOVEMENT_INTERRUPTS: u32 = 0x0000_0001;
        if self.interrupt_flags & MOVEMENT_INTERRUPTS == 0 {
            return false;
        }
        if move_flags & REFUSES_A_CAST == 0 {
            return false;
        }
        if self.attributes_ex2 & spell_attributes::EX2_AUTO_REPEAT != 0 {
            return false;
        }
        self.cast_time_ms != 0
            || (self.aura_interrupt_flags | self.channel_interrupt_flags) & CANCELLED_BY_MOVING != 0
    }

    /// **Is this spell a channel?** — [`spell_attributes::CHANNELED`], which is
    /// both bits and not just the first.
    pub fn channeled(&self) -> bool {
        self.attributes_ex & spell_attributes::CHANNELED != 0
    }

    /// **Does this spell carry one of the four [`interrupt_when`] words?**
    ///
    /// The `AuraInterruptFlags` column always, and `ChannelInterruptFlags` only
    /// when the spell is a channel — which is the pair of tests the client's
    /// cast check makes at each of the four, reporting the same
    /// refusal from both. Reading the channel column unconditionally would
    /// refuse casts on words that apply to a channel this spell never runs.
    pub fn interrupted_when(&self, word: u32) -> bool {
        self.aura_interrupt_flags & word != 0
            || (self.channeled() && self.channel_interrupt_flags & word != 0)
    }

    /// **Is this spell the thing you sit on?** — an `APPLY_AURA` effect whose
    /// aura is `MOUNTED`, which is the client's own three-slot loop.
    ///
    /// One consumer, and it is a *message* rather than a rule: an outdoor-only
    /// spell refused indoors says `SPELL_FAILED_ONLY_OUTDOORS` unless it is a
    /// mount, and then it says `SPELL_FAILED_NO_MOUNTS_ALLOWED` — "You can't
    /// mount here", which is the sentence a player standing in an inn expects.
    pub fn is_mount(&self) -> bool {
        self.effects
            .iter()
            .any(|effect| effect.kind == EFFECT_APPLY_AURA && effect.aura == AURA_MOUNTED)
    }

    /// Whether a corpse may press it — read only so the dead-caster refusal in
    /// [`check_cast`] does not swallow the few that are legal.
    pub fn castable_while_dead(&self) -> bool {
        self.attributes & spell_attributes::ALLOW_CAST_WHILE_DEAD != 0
    }

    /// **Which of six Fireballs this is**, as the client's own spellbook sort
    /// asks it.
    ///
    /// The digits in the `Rank` string, run together — so "Rank 1" is 1 and
    /// "Rank 12" is 12 — scanning from the left and stopping at the first
    /// non-digit *after* a digit has been seen. A rank with no digits in it at
    /// all ("Racial", "Passive", the empty string) falls back to
    /// [`SpellInfo::spell_level`], which is what the client does at its own last
    /// line.
    ///
    /// It is the third key of three and it only ever separates two spells that
    /// already share a page and a name, so the fallback is doing very little
    /// work — but it is the client's, and inventing a different one puts Rank 10
    /// between Rank 1 and Rank 2.
    pub fn rank_order(&self) -> u32 {
        let mut value: u32 = 0;
        let mut seen = false;
        for byte in self.rank.bytes() {
            if byte.is_ascii_digit() {
                value = value.saturating_mul(10) + u32::from(byte - b'0');
                seen = true;
            } else if seen {
                break;
            }
        }
        if seen {
            value
        } else {
            self.spell_level
        }
    }

    /// "Fireball (Rank 1)", or just the name when there is no rank.
    pub fn label(&self) -> String {
        if self.rank.is_empty() {
            self.name.clone()
        } else {
            format!("{} ({})", self.name, self.rank)
        }
    }
}

/// `Spell.dbc` and the three small tables it points into.
///
/// The rows are **not** unpacked up front, and the trade is worth stating
/// plainly rather than implying it is free. Unpacking would mean 22,360
/// `SpellInfo`s and 45,000 string allocations at load so that a twelve-slot
/// action bar can ask about twelve of them; keeping the `Dbc` instead costs the
/// record block staying resident — about 16 MB for `Spell.dbc`'s 22,360 × 173
/// words — and makes [`Self::info`] build one row on demand.
///
/// So this is a load-time-and-allocation trade, not a memory saving. A caller
/// that asks every frame should cache what it gets, which is what the client's
/// bar does (keyed on the spellbook's version).
pub struct Spells {
    spell: Dbc,
    /// Spell id -> record index. `Spell.dbc` is *nearly* ordered by id and is
    /// not exactly, so a binary search would be subtly wrong.
    index: HashMap<u32, usize>,
    cast_times: HashMap<u32, u32>,
    ranges: HashMap<u32, f32>,
    /// …and the same table's minimum column, which only the ranged abilities
    /// populate. Kept apart rather than made a pair, because every other reader
    /// of `ranges` wants the maximum alone.
    min_ranges: HashMap<u32, f32>,
    icons: HashMap<u32, String>,
    /// The two the *description* points into. Both degrade to nothing, and each
    /// absence costs one substitution: no `SpellDuration` reads every `$d` as
    /// no duration, no `SpellRadius` reads every `$a` as zero yards.
    /// `(base, per level, max)`, signed — see [`DURATION_BASE`].
    durations: HashMap<u32, (i32, i32, i32)>,
    radii: HashMap<u32, f32>,
    /// `Dispel` -> the word a debuff border is coloured by. Absent for the seven
    /// classes the file leaves unnamed — see [`DISPEL_TYPE_NAME`].
    dispel_types: HashMap<u32, String>,
    /// **`LockType` -> every spell that opens a lock of that kind**, built at
    /// parse time by scanning all 22,360 rows once.
    ///
    /// The only consumer is [`crate::look::object::opener`] and the reason it
    /// exists is that a chest, an ore vein and a herb are **not opened by
    /// `CMSG_GAMEOBJ_USE` at all** — see that function. Built rather than asked
    /// per click because the question is "which spell has this misc value",
    /// which is a scan of the whole table in the direction the file is not
    /// indexed in.
    openers: HashMap<u32, Vec<u32>>,
}

impl Spells {
    /// `Spell.dbc` is required; the other three degrade.
    ///
    /// **Each absence is a documented degradation, not an error**, on the same
    /// terms as `DisplayTables::load`: with no `SpellIcon` the bar draws named
    /// buttons with no pictures, with no `SpellCastTimes` every spell reads
    /// instant (so the cast bar never appears), and with no `SpellRange` every
    /// spell reads zero yards — which the *server* still range-checks, so the
    /// cost is a local refusal that does not happen rather than a wrong cast.
    pub fn parse(
        spell: &[u8],
        cast_times: &[u8],
        ranges: &[u8],
        icons: &[u8],
        durations: &[u8],
        radii: &[u8],
        dispel_types: &[u8],
    ) -> Result<Spells, AssetError> {
        let spell = Dbc::parse(spell)?;
        let mut index = HashMap::with_capacity(spell.record_count);
        let mut openers: HashMap<u32, Vec<u32>> = HashMap::new();
        for record in 0..spell.record_count {
            let Some(id) = spell.u32_at(record, 0) else {
                continue;
            };
            index.insert(id, record);
            // **Both open-lock effects, and there really are two.** 3 is
            // `SPELL_EFFECT_OPEN_LOCK` and 33 is `SPELL_EFFECT_OPEN_LOCK_ITEM`;
            // the server routes both to the same `Spell::EffectOpenLock`, and
            // every spell that matters here — Herb Gathering, Mining, Pick Lock,
            // Opening — is a **33**. A reader that took only 3 would find none
            // of them and quietly answer "you cannot gather".
            for slot in 0..3 {
                let kind = spell.u32_at(record, spell_fields::EFFECT + slot).unwrap_or(0);
                if kind != EFFECT_OPEN_LOCK && kind != EFFECT_OPEN_LOCK_ITEM {
                    continue;
                }
                let lock_type = spell
                    .u32_at(record, spell_fields::EFFECT_MISC_VALUE + slot)
                    .unwrap_or(0);
                openers.entry(lock_type).or_default().push(id);
            }
        }
        for ids in openers.values_mut() {
            ids.sort_unstable();
        }
        Ok(Spells {
            spell,
            index,
            openers,
            cast_times: number_table(cast_times, CAST_TIME_BASE),
            ranges: float_table(ranges, RANGE_MAX),
            min_ranges: float_table(ranges, RANGE_MIN),
            icons: string_table(icons, ICON_PATH),
            durations: duration_table(durations),
            radii: float_table(radii, RADIUS_BASE),
            // Empty strings dropped, so "is there a name" and "is there a row"
            // are the same question — which is what the sayable-four rule is.
            dispel_types: string_table(dispel_types, DISPEL_TYPE_NAME)
                .into_iter()
                .filter(|(_, name)| !name.is_empty())
                .collect(),
        })
    }

    /// **Every spell that opens a lock of this kind**, by ascending id.
    ///
    /// The rank does not matter and that is worth stating, because there are
    /// five Herb Gathering spells and picking the wrong one would be an easy
    /// bug: `Spell::CanOpenLock` compares the lock's required rank against
    /// `player->GetSkillValue(skillId)` — **the character's own skill**, never
    /// the spell's — so any rank the character knows opens anything their skill
    /// is high enough for. Ascending order is for determinism, not preference.
    pub fn open_lock_spells(&self, lock_type: u32) -> &[u32] {
        self.openers.get(&lock_type).map_or(&[][..], Vec::as_slice)
    }

    /// **A `SpellIcon.dbc` path by its own id**, rather than by a spell's.
    ///
    /// Every other reader here reaches an icon through a spell — [`SpellInfo`]
    /// resolves field 117 itself — but two tables index the same file directly:
    /// `SkillLine`'s tab pictures and `TalentTab`'s. This is the second of them
    /// (see [`crate::tables::talent`]); the skills panel predates the accessor
    /// and reads its own copy of the table.
    pub fn icon_path(&self, icon_id: u32) -> Option<String> {
        self.icons.get(&icon_id).cloned()
    }

    /// How many lock types have an opener at all, for `vale objects` to
    /// report — 8 in the shipped file, which is every row of `LockType.dbc`
    /// that any 1.12 lock names.
    pub fn opener_kinds(&self) -> usize {
        self.openers.len()
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Every spell id the table has a row for, unordered — the walk
    /// `vale tradeskill` finds the opening spells with.
    pub fn ids(&self) -> Vec<u32> {
        self.index.keys().copied().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// **A spell's duration at a level, in milliseconds** — `SpellDuration`'s
    /// three columns as `min(base + per_level * level, max)`, which is the
    /// arithmetic in `GetResSicknessDuration` and
    /// the same one `Spell::GetSpellDuration` runs on the server. `None` for a
    /// spell with no row; a negative answer is a real one and means "none at
    /// this level" — Resurrection Sickness at level 10 is -600000 + 600000 =
    /// 0, and at level 9 it is under zero.
    pub fn duration_at(&self, spell_id: u32, level: u32) -> Option<i32> {
        let record = *self.index.get(&spell_id)?;
        let index = self.spell.u32_at(record, spell_fields::DURATION_INDEX)?;
        let (base, per_level, max) = *self.durations.get(&index)?;
        let level = i32::try_from(level).unwrap_or(i32::MAX);
        let scaled = base.saturating_add(per_level.saturating_mul(level));
        Some(if max > 0 { scaled.min(max) } else { scaled })
    }

    /// Everything about one spell, or `None` if `Spell.dbc` has no such row.
    pub fn info(&self, spell_id: u32) -> Option<SpellInfo> {
        let record = *self.index.get(&spell_id)?;
        let field = |f: usize| self.spell.u32_at(record, f).unwrap_or(0);
        let signed = |f: usize| self.spell.u32_at(record, f).unwrap_or(0) as i32;
        let float = |f: usize| self.spell.f32_at(record, f).unwrap_or(0.0);
        let string = |f: usize| self.spell.string_at(record, f).unwrap_or_default();
        // The three effect slots are three parallel arrays, so slot `i` is
        // `COLUMN + i` in every one of them.
        let mut effects = [SpellEffect::default(); 3];
        for (slot, effect) in effects.iter_mut().enumerate() {
            *effect = SpellEffect {
                kind: field(spell_fields::EFFECT + slot),
                base_points: signed(spell_fields::EFFECT_BASE_POINTS + slot),
                die_sides: signed(spell_fields::EFFECT_DIE_SIDES + slot),
                base_dice: signed(spell_fields::EFFECT_BASE_DICE + slot),
                dice_per_level: float(spell_fields::EFFECT_DICE_PER_LEVEL + slot),
                real_points_per_level: float(spell_fields::EFFECT_REAL_POINTS_PER_LEVEL + slot),
                radius_yards: self
                    .radii
                    .get(&field(spell_fields::EFFECT_RADIUS_INDEX + slot))
                    .copied()
                    .unwrap_or(0.0),
                amplitude_ms: field(spell_fields::EFFECT_AMPLITUDE + slot),
                chain_targets: field(spell_fields::EFFECT_CHAIN_TARGET + slot),
                trigger_spell: field(spell_fields::EFFECT_TRIGGER_SPELL + slot),
                aura: field(spell_fields::EFFECT_APPLY_AURA + slot),
                misc_value: signed(spell_fields::EFFECT_MISC_VALUE + slot),
                item_type: field(spell_fields::EFFECT_ITEM_TYPE + slot),
                target_a: field(spell_fields::IMPLICIT_TARGET_A + slot),
                target_b: field(spell_fields::IMPLICIT_TARGET_B + slot),
            };
        }
        // Eight slots, the empty ones dropped — nearly every spell has none and
        // the ones that have any have one.
        let reagents = (0..8)
            .filter_map(|slot| {
                let entry = field(spell_fields::REAGENT + slot);
                (entry != 0)
                    .then(|| (entry, field(spell_fields::REAGENT_COUNT + slot).max(1)))
            })
            .collect();
        Some(SpellInfo {
            description: string(spell_fields::DESCRIPTION),
            aura_description: string(spell_fields::AURA_DESCRIPTION),
            reagents,
            totems: [
                field(spell_fields::TOTEM),
                field(spell_fields::TOTEM + 1),
            ],
            focus_object: field(spell_fields::REQUIRES_SPELL_FOCUS),
            duration_ms: self
                .durations
                .get(&field(spell_fields::DURATION_INDEX))
                .map(|(base, _, _)| u32::try_from(*base).unwrap_or(0))
                .unwrap_or(0),
            max_level: field(spell_fields::MAX_LEVEL),
            base_level: field(spell_fields::BASE_LEVEL),
            max_affected_targets: field(spell_fields::MAX_AFFECTED_TARGETS),
            proc_chance: field(spell_fields::PROC_CHANCE),
            effects,
            id: spell_id,
            name: string(spell_fields::NAME),
            rank: string(spell_fields::RANK),
            icon: self
                .icons
                .get(&field(spell_fields::ICON_ID))
                .cloned()
                .unwrap_or_default(),
            active_icon: self
                .icons
                .get(&field(spell_fields::ACTIVE_ICON_ID))
                .cloned()
                .unwrap_or_default(),
            shapeshift_order: signed(spell_fields::SHAPESHIFT_ORDER),
            cast_time_ms: self
                .cast_times
                .get(&field(spell_fields::CASTING_TIME_INDEX))
                .copied()
                .unwrap_or(0),
            power_type: field(spell_fields::POWER_TYPE),
            power_cost: field(spell_fields::MANA_COST),
            range_yards: self
                .ranges
                .get(&field(spell_fields::RANGE_INDEX))
                .copied()
                .unwrap_or(0.0),
            min_range_yards: self
                .min_ranges
                .get(&field(spell_fields::RANGE_INDEX))
                .copied()
                .unwrap_or(0.0),
            range_index: field(spell_fields::RANGE_INDEX),
            recovery_ms: field(spell_fields::RECOVERY_TIME),
            category: field(spell_fields::CATEGORY),
            category_recovery_ms: field(spell_fields::CATEGORY_RECOVERY_TIME),
            gcd_category: field(spell_fields::START_RECOVERY_CATEGORY),
            gcd_ms: field(spell_fields::START_RECOVERY_TIME),
            interrupt_flags: field(spell_fields::INTERRUPT_FLAGS),
            aura_interrupt_flags: field(spell_fields::AURA_INTERRUPT_FLAGS),
            channel_interrupt_flags: field(spell_fields::CHANNEL_INTERRUPT_FLAGS),
            attributes: field(spell_fields::ATTRIBUTES),
            attributes_ex: field(spell_fields::ATTRIBUTES_EX),
            attributes_ex2: field(spell_fields::ATTRIBUTES_EX2),
            attributes_ex3: field(spell_fields::ATTRIBUTES_EX3),
            dispel_type: self
                .dispel_types
                .get(&field(spell_fields::DISPEL))
                .cloned()
                .unwrap_or_default(),
            targets: field(spell_fields::TARGETS),
            caster_aura_state: field(spell_fields::CASTER_AURA_STATE),
            target_aura_state: field(spell_fields::TARGET_AURA_STATE),
            prevention_type: field(spell_fields::PREVENTION_TYPE),
            spell_family: field(spell_fields::SPELL_FAMILY_NAME),
            spell_family_flags: u64::from(field(spell_fields::SPELL_FAMILY_FLAGS_LOW))
                | (u64::from(field(spell_fields::SPELL_FAMILY_FLAGS_HIGH)) << 32),
            min_faction_id: field(spell_fields::MIN_FACTION_ID),
            min_reputation: field(spell_fields::MIN_REPUTATION),
            stances: field(spell_fields::STANCES),
            stances_not: field(spell_fields::STANCES_NOT),
            equipped_item_class: field(spell_fields::EQUIPPED_ITEM_CLASS) as i32,
            equipped_item_subclass_mask: field(spell_fields::EQUIPPED_ITEM_SUBCLASS_MASK),
            equipped_item_inventory_type_mask: field(
                spell_fields::EQUIPPED_ITEM_INVENTORY_TYPE_MASK,
            ),
            implicit_target_a: field(spell_fields::IMPLICIT_TARGET_A),
            cast_ui: field(spell_fields::CAST_UI),
            spell_level: field(spell_fields::SPELL_LEVEL),
        })
    }

    /// A spell's name alone, which is what the chat log and a tooltip want.
    pub fn name(&self, spell_id: u32) -> Option<String> {
        let record = *self.index.get(&spell_id)?;
        self.spell.string_at(record, spell_fields::NAME)
    }

    /// **The highest known rank that supersedes `stale`**, or `None` if nothing
    /// does — the repair for a bar that went stale before this client read
    /// `SMSG_SUPERCEDED_SPELL`.
    ///
    /// **This is a departure from the reference and is marked as one.** The 1.12
    /// client never needs it: it reads the packet, so its bar is corrected the
    /// moment the rank is learned and a stale row cannot accumulate. This
    /// project's bars *did* accumulate them, for as long as it has been played,
    /// and a packet that has already been sent cannot fix a row it was sent
    /// about. So the rule here is a reconstruction of the client's *grouping*
    /// rather than of any code it runs.
    ///
    /// The grouping is the one the spellbook's own sort uses and is not invented:
    /// two rows are ranks of one spell when their `Name` strings match, and which
    /// is higher is [`SpellInfo::rank_order`], the client's digit scan. So
    /// "Heroic Strike" / "Rank 8" is superseded by "Heroic Strike" / "Rank 9",
    /// and a spell whose name nothing else shares is left exactly where it is.
    ///
    /// **`known` is the character's own book**, so a rank that was *unlearned*
    /// rather than superseded — a talent reset — finds no higher rank and is
    /// left alone, which is the honest answer: the button really does point at
    /// nothing and the player has to decide what goes there.
    ///
    /// The name is compared before any `SpellInfo` is built, because the caller's
    /// `known` is ~200 rows for a level-60 character and the full read allocates
    /// several strings a row. The whole scan runs only for a button that is
    /// *already* stale, which after one corrected login is none of them.
    pub fn superseding_rank(&self, stale: u32, known: &[u32]) -> Option<u32> {
        let gone = self.info(stale)?;
        let mut best: Option<(u32, u32)> = None;
        for id in known {
            if *id == stale {
                continue;
            }
            let Some(record) = self.index.get(id) else {
                continue;
            };
            if self.spell.string_at(*record, spell_fields::NAME).as_deref() != Some(&*gone.name) {
                continue;
            }
            let Some(info) = self.info(*id) else {
                continue;
            };
            let order = info.rank_order();
            if order <= gone.rank_order() {
                continue;
            }
            if best.is_none_or(|(best_order, _)| order > best_order) {
                best = Some((order, *id));
            }
        }
        best.map(|(_, id)| id)
    }

    /// **Every spell that repeats itself once pressed**, in id order — the
    /// census `vale spellbook` prints.
    ///
    /// A count rather than a spot check because the read is a *pair* of bits in
    /// two adjacent columns and a wrong index for either would answer a
    /// plausible number: the ranged-slot bit alone is 373 rows and the
    /// auto-repeat bit alone is a different set again, so the intersection being
    /// small, named and recognisable — Auto Shot and Shoot — is what says both
    /// columns were found.
    pub fn auto_repeat_ranged(&self) -> Vec<(u32, String)> {
        let mut found: Vec<(u32, String)> = self
            .index
            .keys()
            .filter_map(|id| {
                let info = self.info(*id)?;
                info.is_auto_repeat_ranged()
                    .then(|| (info.id, info.name.clone()))
            })
            .collect();
        found.sort_unstable();
        found
    }

    /// …and how many carry the ranged-slot bit alone, which is the number the
    /// pair has to be measured against — see [`SpellInfo::uses_ranged_slot`].
    pub fn ranged_slot_count(&self) -> usize {
        self.index
            .keys()
            .filter(|id| self.info(**id).is_some_and(|i| i.uses_ranged_slot()))
            .count()
    }

    /// How many rows have a name, an icon and a cast time — the three counts
    /// `vale spellbook` reports, since each is a table that can be absent.
    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.index.len(),
            self.icons.len(),
            self.cast_times.len(),
        )
    }
}

/// `id -> u32 at field` over a whole table.
fn number_table(raw: &[u8], field: usize) -> HashMap<u32, u32> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return HashMap::new();
    };
    (0..dbc.record_count)
        .filter_map(|r| Some((dbc.u32_at(r, 0)?, dbc.u32_at(r, field)?)))
        .collect()
}

/// `SpellDuration.dbc` whole: the three signed columns, by row id.
fn duration_table(raw: &[u8]) -> HashMap<u32, (i32, i32, i32)> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return HashMap::new();
    };
    (0..dbc.record_count)
        .filter_map(|r| {
            let signed = |field: usize| dbc.u32_at(r, field).map(|v| v as i32);
            Some((
                dbc.u32_at(r, 0)?,
                (
                    signed(DURATION_BASE)?,
                    signed(DURATION_PER_LEVEL)?,
                    signed(DURATION_MAX)?,
                ),
            ))
        })
        .collect()
}

fn float_table(raw: &[u8], field: usize) -> HashMap<u32, f32> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return HashMap::new();
    };
    (0..dbc.record_count)
        .filter_map(|r| Some((dbc.u32_at(r, 0)?, dbc.f32_at(r, field)?)))
        .collect()
}

fn string_table(raw: &[u8], field: usize) -> HashMap<u32, String> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return HashMap::new();
    };
    (0..dbc.record_count)
        .filter_map(|r| Some((dbc.u32_at(r, 0)?, dbc.string_at(r, field)?)))
        .filter(|(_, path)| !path.is_empty())
        .collect()
}

/// **A cast failure is displayed in two layers, and the second one replaces the
/// first.**
///
/// The wire reason resolves to a `SPELL_FAILED_*` key (that half is
/// `vale_protocol::play::spells::cast_failure_key`), and then a per-reason error id
/// decides how to show it. Most are a pure passthrough, but about a dozen carry
/// an `ERR_*` string of their own with no `%s` in it — which **replaces** the
/// message entirely. That is why the game says "Spell is not ready yet." on
/// screen while `SPELL_FAILED_NOT_READY` reads "Not yet recovered", and why
/// `SPELL_FAILED_NO_POWER` has no string in `GlobalStrings.lua` at all: it is
/// always shown as one of the four `ERR_OUT_OF_*`, chosen by the spell's own
/// power type.
///
/// Two of the dozen are modelled here, and they are the two a player meets
/// hourly. The rest fall through to their own key, which is the correct English
/// message in every case — only less specific than the one the client shows.
pub fn failure_override(reason_key: &str, power_type: u32) -> Option<&'static str> {
    Some(match reason_key {
        "SPELL_FAILED_NO_POWER" => match power_type {
            1 => "ERR_OUT_OF_RAGE",
            2 => "ERR_OUT_OF_FOCUS",
            3 => "ERR_OUT_OF_ENERGY",
            _ => "ERR_OUT_OF_MANA",
        },
        "SPELL_FAILED_NOT_READY" => "ERR_SPELL_COOLDOWN",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// The aiming rule
// ---------------------------------------------------------------------------

/// The flag-word bits the binder consumes. `TARGET_FLAG_*`, as the client's own
/// targeting word holds them.
mod aim_bits {
    pub const UNIT: u16 = 0x0002;
    pub const UNIT_RAID: u16 = 0x0004;
    pub const UNIT_PARTY: u16 = 0x0008;
    pub const UNIT_ENEMY: u16 = 0x0080;
    pub const UNIT_ASSIST: u16 = 0x0100;
    pub const CORPSE_ENEMY: u16 = 0x0200;
    /// "there is an explicit selection", which carries no guid of its own and is
    /// discharged by any real candidate.
    pub const EXPLICIT: u16 = 0x0400;
    pub const CORPSE_ALLY: u16 = 0x8000;
    pub const UNK_23: u16 = 0x0800;
    pub const UNK_26: u16 = 0x4000;

    /// **The bit that names a *place* rather than a thing** — the ground-target
    /// machine, which is Blizzard, Flamestrike, Rain of Fire, Volley, Hurricane
    /// and everything else you click a patch of floor with.
    ///
    /// It is one of only two flags in the word the server answers with bare
    /// coordinates rather than a guid: `SpellCastTargets::read` reads three
    /// floats for it and three more for `TARGET_FLAG_SOURCE_LOCATION` (`0x0020`)
    /// beside it (vmangos `Spells/Spell.cpp`). The source is deliberately not
    /// named here, because nothing in this client can produce one and a word
    /// carrying it alone stays in the refused leg — where it has always been.
    /// See [`CastAim::WantsGround`].
    ///
    /// **The reference tests both bits and the difference costs nothing here.**
    /// Its "is this cursor waiting for a place" predicate is `targetMask & 0x60`,
    /// so a source-only word puts the circle up there where it is
    /// refused here. Measured before leaving it that way: **205 spells carry the
    /// destination alone, 13 carry the source alone and 0 carry both** — and all
    /// 13 are NPC and test rows (`Word of Recall Other`, `Area Death (TEST)`,
    /// `Firegut Fear Storm`), none of them in any class's book. Binding the bit
    /// would mean sending a block this client cannot fill, so it stays out.
    pub const DEST_LOCATION: u16 = 0x0040;

    /// **`TARGET_FLAG_ITEM` — the cast is aimed at a thing you are carrying**,
    /// and it is the reason Rockbiter Weapon came back *"Invalid target"*.
    ///
    /// `Spell.dbc`'s `Targets` column is `0x0010` and nothing else for it
    /// (`vale spellbook 8017`), so [`super::aim_mask`] answers `0x0010`,
    /// which is outside [`UNIT_FAMILY`] and fell into the refusal that names
    /// the report. vmangos reads it as a packed guid in
    /// `SpellCastTargets::read` and checks for it in
    /// `Spell::ValidateExplicitTargetMask`, whose `expectedTargetMask` *is*
    /// this column — so a cast that sends no item for a spell that wants one is
    /// refused on both sides of the wire.
    ///
    /// Almost every one of them picks the item itself: see
    /// [`super::spell_attributes::HELD_ITEM_ONLY`], which travels with this bit
    /// on the weapon imbues, the poisons and the sharpening stones.
    pub const ITEM: u16 = 0x0010;

    /// Every bit a *unit* can satisfy. A word with anything outside this is
    /// aimed at an item, a gameobject, a patch of ground or a string, of which
    /// only the ground is one this client can bind (see [`CastAim::WantsGround`]).
    pub const UNIT_FAMILY: u16 = UNIT
        | UNIT_RAID
        | UNIT_PARTY
        | UNIT_ENEMY
        | UNIT_ASSIST
        | CORPSE_ENEMY
        | EXPLICIT
        | CORPSE_ALLY;
}

/// Where a cast should be aimed, and what to send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastAim {
    /// `TARGET_FLAG_SELF`: no guid on the wire. The spell names its own targets.
    SelfImplicit,
    /// `TARGET_FLAG_UNIT` and this guid — possibly the caster's own.
    Unit(u64),
    /// Do not send. The payload is a `GlobalStrings.lua` key: the *client's own*
    /// message for the case, so the player reads "You have no target." exactly
    /// as the real client says it.
    Refused(&'static str),
    /// **Do not send, and do not complain: ask.** This is 1.12's targeting
    /// cursor — the hand-and-sparkle pointer you click a target with, and the
    /// answer for every friendly spell pressed with nothing suitable selected.
    ///
    /// It is a *third* outcome rather than a flavour of [`Self::Refused`]
    /// because the client draws the line on one bit and draws it hard. See
    /// [`resolve_aim`]'s last three paragraphs, where the branch is transcribed.
    WantsTarget,
    /// **…and the same cursor's other job: point at a patch of *ground*.**
    ///
    /// The spell's word carries [`aim_bits::DEST_LOCATION`], which is the only
    /// thing in the game that says "this cast is placed rather than aimed" —
    /// Blizzard's `Targets` column is `0x0040` and nothing else. What the click
    /// produces is three floats rather than a guid
    /// ([`crate::tables::spellbook::CastAim`]'s only non-unit answer), and the caller
    /// sends them as `TARGET_FLAG_DEST_LOCATION`.
    ///
    /// **It outranks the selection deliberately**, which is the one ordering
    /// decision in [`resolve_aim`] that is not the client's: a ground spell asks
    /// even with a perfectly good unit selected. The argument is what the server
    /// does with the alternative — vmangos' `Spell::SetTargetMap` falls back to
    /// `m_targets.setDestination(caster's own position)` when the flag is
    /// absent (`Spells/Spell.cpp`, the `TARGET_LOCATION_CASTER_DEST` arms), so a
    /// Blizzard "helpfully" sent at the selection lands **on the caster's feet**.
    /// That is the plausibly-wrong outcome this project keeps recording.
    WantsGround,
    /// **`TARGET_FLAG_ITEM` and this item's guid** — the weapon an imbue goes
    /// on, the blade a sharpening stone is dragged down.
    ///
    /// The client picks it rather than asking, which is what
    /// [`spell_attributes::HELD_ITEM_ONLY`] means: vmangos comments the same
    /// bit *"Client automatically selects item from mainhand slot as a cast
    /// target"*, and the reference puts no cursor up for it. So this carries a
    /// guid the way [`Self::Unit`] does and the caller sends
    /// `TARGET_FLAG_ITEM`.
    Item(u64),
    /// **…and the residue: an item cast that has to be *pointed* at one.**
    ///
    /// A spell whose word wants an item and which does not carry the held-item
    /// bit — the enchanting formulas. In 1.12 those are aimed by clicking a bag
    /// slot, and the interface's whole share of it is `UseContainerItem`
    /// (`ContainerFrame.lua:596`): there is no global of their own to answer.
    /// Named here so the rule is complete and the census is honest; a caller
    /// with no item cursor may treat it exactly as [`Self::WantsTarget`].
    WantsItem,
}

/// The two local refusals, which are the client's own two cast-failure reasons
/// re-used: `SPELL_FAILED_BAD_IMPLICIT_TARGETS` (0x09, "You have no target.")
/// and `SPELL_FAILED_BAD_TARGETS` (0x0a, "Invalid target").
pub const NO_TARGET: &str = "SPELL_FAILED_BAD_IMPLICIT_TARGETS";
pub const INVALID_TARGET: &str = "SPELL_FAILED_BAD_TARGETS";
/// …and the third, which is `0x2d` — "Your weapon hand is empty", the answer to
/// a poison or a sharpening stone pressed with nothing to put it on. See
/// [`spell_attributes::HELD_ITEM_ONLY`].
pub const MAINHAND_EMPTY: &str = "SPELL_FAILED_MAINHAND_EMPTY";

/// The spell's aiming word: `targets`, adjusted by the implicit-target arm.
///
/// Transcribed from the client's own switch: the word starts as `Spell.dbc`'s
/// `Targets` column and exactly one arm, keyed on `EffectImplicitTargetA[0]`,
/// adjusts it. Every enum not listed is the default no-op arm.
///
/// The important consequence is that **most spells state no `Targets` at all**
/// and are decided entirely here: Fireball's column is 0 and its implicit target
/// is 6, which sets the enemy bit and makes it a cast that requires a hostile
/// unit. A self-buff's implicit target is 1, which *clears* a bit and leaves the
/// word empty.
pub fn aim_mask(spell: &SpellInfo) -> u16 {
    let mut word = spell.targets as u16;
    match spell.implicit_target_a {
        1 => word &= !aim_bits::EXPLICIT,
        5 => word &= !aim_bits::CORPSE_ALLY,
        6 | 53 => word |= aim_bits::UNIT_ENEMY,
        // 16 is the ground-target arm: it sets the client's cursor mode rather
        // than a word bit, and the location bits arrive through `Targets`
        // itself — which is measured rather than assumed, and `vale
        // spellbook` prints the count of rows where the two disagree.
        21 | 45 => word |= aim_bits::UNIT_ASSIST,
        23 => word |= aim_bits::UNK_23,
        25 | 63 => word |= aim_bits::UNIT,
        26 => word |= aim_bits::UNK_26,
        35 => word |= aim_bits::UNIT_PARTY,
        57 | 61 => word |= aim_bits::UNIT_RAID,
        _ => {}
    }
    word
}

/// What a candidate is, as far as the binder cares.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    pub guid: u64,
    /// Whether this candidate is the caster.
    pub is_self: bool,
    /// How the caster stands towards it.
    pub reaction: Reaction,
    /// `UNIT_FIELD_FLAGS`, for [`crate::tables::faction::can_attack`].
    pub unit_flags: u32,
    pub dead: bool,
}

/// Which bits of `word` this candidate leaves unsatisfied.
///
/// The client's target binding, unit branch: each bit is its own relation test and a
/// cast commits only when every one of them is cleared. The approximations are
/// named — party and raid accept only the caster until groups exist, and
/// "assist" is friendly reaction rather than the client's full `CanAssist`.
fn unsatisfied(word: u16, who: &Candidate) -> u16 {
    let mut word = word;
    let assist = who.is_self || who.reaction == Reaction::Friendly;
    let attackable = !who.is_self && crate::tables::faction::can_attack(who.unit_flags, who.reaction);
    if who.is_self {
        word &= !(aim_bits::UNIT_PARTY | aim_bits::UNIT_RAID);
    }
    if assist {
        word &= !aim_bits::UNIT_ASSIST;
    }
    if attackable {
        word &= !aim_bits::UNIT_ENEMY;
    }
    // The generic unit bit has no relation test at all — any resolved unit
    // satisfies it — and the explicit-selection gate is discharged by any real
    // candidate other than the implicit self.
    word &= !aim_bits::UNIT;
    if !who.is_self {
        word &= !aim_bits::EXPLICIT;
    }
    if assist && who.dead {
        word &= !aim_bits::CORPSE_ALLY;
    }
    if attackable && who.dead {
        word &= !aim_bits::CORPSE_ENEMY;
    }
    word
}

/// **The CVar name `auto_self_cast` is fed from**, beside the rule that reads
/// it rather than spelled out at the call site.
///
/// The client registers it with the string `"0"`, and
/// `OptionsFrame.xml` gives it a checkbox — "Auto self cast" — so it is a
/// setting a player can actually reach rather than a console-only one. See
/// [`resolve_aim`], whose fifth step is the whole of what it does.
pub const AUTO_SELF_CAST: &str = "autoSelfCast";

/// **Where to aim a cast**, given the spell and who is selected.
///
/// The whole rule, in the client's own order:
///
/// 1. an empty word commits with no target at all;
/// 2. a word naming a **destination** asks for a patch of ground — see
///    [`CastAim::WantsGround`], and note that this comes *before* the selection
///    rather than after it;
/// 3. a word wanting something that is still not a unit is refused, because the
///    machines that bind an item, a gameobject or a string are not modelled here
///    and guessing produces a cast the server refuses;
/// 4. the **selection** is tried first;
/// 5. then the **caster**, behind `auto_self_cast` — the classic "buffing with
///    an enemy targeted casts it on yourself";
/// 6. and failing all of that, the client's **three-way** answer below.
///
/// `auto_self_cast` is the game's own CVar and its engine default is **off** —
/// the client registers `"autoSelfCast"` with the string `"0"`,
/// and the only other thing that reads it
/// is a branch inside the arm. Passing `true` is the
/// `SELFACTIONBUTTON` binding, which is what that flag is for.
///
/// ## What happens when nothing binds, which is three things and not one
///
/// Transcribed from the arm's **caller**, where the failure is
/// handled — the arm itself only answers yes or no:
///
/// ```text
/// did the arm bind anything?                    yes -> cast
/// Attributes & SPELL_ATTR_HELD_ITEM_ONLY (0x200)
///                                               set -> SPELL_FAILED_MAINHAND_EMPTY
/// the aiming word's UNIT_ENEMY bit              clear -> the cursor
///                                               set -> error 0x09 or 0x0a, on
///                                                      whether anything was selected
/// ```
///
/// So **the enemy bit is the whole of it**: a spell that wants a hostile unit
/// says "You have no target."/"Invalid target", and a spell that does not asks
/// the player to point at something. Fireball's word is `0x0080` and Lesser
/// Heal's is `0x0100`, which is the case the report was about — this client
/// printed "No target" where the reference shows the cursor.
///
/// **One case is deliberately still a refusal**: a word wanting an **item**, a
/// **gameobject** or a **string** (step 3 above) reaches the same cursor in the
/// reference, and this client has no way to bind any of those — so refusing is
/// the stated deviation rather than promising a mode with nothing behind it.
/// The *ground*, which was in that list for the life of this function, is not
/// any more: see [`CastAim::WantsGround`].
pub fn resolve_aim(
    spell: &SpellInfo,
    selection: Option<Candidate>,
    caster: Option<Candidate>,
    auto_self_cast: bool,
    held_item: Option<u64>,
) -> CastAim {
    let word = aim_mask(spell);
    if word == 0 {
        return CastAim::SelfImplicit;
    }
    // **A place, before anything that is a thing.** See [`CastAim::WantsGround`]
    // for why this outranks the selection instead of falling in behind it.
    if word & aim_bits::DEST_LOCATION != 0 {
        return CastAim::WantsGround;
    }
    // **…and a *thing you own*, before anything you could point at.** The
    // report is Rockbiter Weapon: its `Targets` column is `0x0010` alone, which
    // was outside `UNIT_FAMILY` and fell into the refusal one line down —
    // "Invalid target", for a spell with nothing wrong with it. The order is
    // the client's: [`spell_attributes::HELD_ITEM_ONLY`] is tested *first* when
    // the arm fails, so an imbue with an empty hand says "Your
    // weapon hand is empty" rather than putting a cursor up, and it says it
    // whether or not anything is selected.
    if word & aim_bits::ITEM != 0 {
        if spell.attributes & spell_attributes::HELD_ITEM_ONLY == 0 {
            return CastAim::WantsItem;
        }
        return match held_item {
            Some(guid) => CastAim::Item(guid),
            None => CastAim::Refused(MAINHAND_EMPTY),
        };
    }
    if word & !aim_bits::UNIT_FAMILY != 0 {
        return CastAim::Refused(INVALID_TARGET);
    }
    if let Some(who) = selection {
        if unsatisfied(word, &who) == 0 {
            return CastAim::Unit(who.guid);
        }
    }
    if auto_self_cast {
        if let Some(mut me) = caster {
            me.is_self = true;
            if unsatisfied(word, &me) == 0 {
                return CastAim::Unit(me.guid);
            }
        }
    }
    // The client's own three-way answer — see the doc comment, where the
    // branch is transcribed. The held-item test comes first because it is
    // first in the client and because it is about the *caster's hands* rather
    // than about anything the player could point at: an empty main hand is not
    // a question to ask.
    if spell.attributes & spell_attributes::HELD_ITEM_ONLY != 0 {
        return CastAim::Refused(MAINHAND_EMPTY);
    }
    if word & aim_bits::UNIT_ENEMY == 0 {
        return CastAim::WantsTarget;
    }
    // "You have no target." when there was nothing to try, "Invalid target"
    // when there was and it did not fit — which is the distinction the client's
    // own two messages draw.
    CastAim::Refused(if selection.is_none() {
        NO_TARGET
    } else {
        INVALID_TARGET
    })
}

// ---------------------------------------------------------------------------
// …and the conditions the aiming rule does not cover
// ---------------------------------------------------------------------------

/// **What the world has to be like for a cast to go out**, measured by the
/// caller and judged here.
///
/// [`resolve_aim`] answers "*who* does this hit" and stops there. Everything
/// else the server checks in `Spell::CheckCast` — am I alive, can I pay for it,
/// is the target within reach — this client used to send and be refused for,
/// which costs a round trip and, because our own cast is drawn at the press, a
/// wind-up and a release that are then taken back off. That is the "it starts
/// casting and then fails" report.
///
/// **Every field is something the client already knows**, which is the rule for
/// what may be checked here at all: a condition that needs the server's own
/// state (line of sight, immunities, reagents in the bag, a stance requirement)
/// is deliberately absent, because a *wrong* local refusal is a button that does
/// nothing with no explanation — strictly worse than the round trip.
#[derive(Debug, Clone, Copy, Default)]
pub struct CastConditions {
    /// The caster's power in the spell's own currency, **in the wire's own
    /// units** — which is what [`SpellInfo::power_cost`] is in too, so the two
    /// compare directly and rage's factor of ten cancels on both sides rather
    /// than being applied to one (see `power_type::display`, which is a
    /// *presentation* rule and belongs nowhere near a cost comparison).
    ///
    /// `None` when the caster has no pool of that kind at all, which is not the
    /// same as having none of it: a mage's rage is not zero rage, it is not a
    /// question, and the server is left to answer it.
    pub power: Option<u32>,
    pub caster_dead: bool,
    /// **Whether the unit this cast bound is dead** — `None` when it bound no
    /// unit at all (a self-implicit cast, a placed one, the pointer's own range
    /// preview).
    ///
    /// `Some` *is* the "explicitly selected unit target" gate vmangos puts on
    /// its own alive-state check: [`resolve_aim`] answering [`CastAim::Unit`] is
    /// precisely the case where the aiming word named a unit and one was bound,
    /// so the caller has nothing to decide. See [`check_cast`].
    pub target_dead: Option<bool>,
    /// **Centre-to-centre yards to the resolved unit target, less both combat
    /// reaches** — vmangos' `GetCombatDistance`, which is the distance its own
    /// range check is written against. `None` for a cast with no unit target
    /// (`TARGET_FLAG_SELF`), which has no distance to be wrong about, and for a
    /// cast aimed at the caster.
    ///
    /// **A ground cast puts its own distance here and it is a different
    /// measurement**: `CheckRange`'s destination branch is a plain 3D distance
    /// from the caster to the point (`IsWithinDist3d`), with no reach subtracted
    /// at either end. It is also the *more permissive* of the two on the
    /// server's side — `SizeFactor::BoundingRadius` adds the caster's own radius
    /// before comparing — so measuring it centre-to-point here is stricter by
    /// about a third of a yard, which [`PLAYER_RANGE_SLACK`] already covers four
    /// times over.
    pub distance: Option<f32>,
    /// **The caster's movement flags, verbatim** — `MovementInfo::flags`, which
    /// is what decides whether a cast may be *begun* at all. See the moving
    /// rule in [`check_cast`]; zero is "standing still", which is what a caller
    /// with no opinion should pass.
    pub move_flags: u32,
    /// **The caster's own `UNIT_FIELD_FLAGS`** — read for the six bits in
    /// [`caster_flags`] and nothing else. Zero is "nothing is wrong with me",
    /// which is what a caller with no opinion should pass and what errs towards
    /// sending.
    pub unit_flags: u32,
    /// **Somebody else is driving this body** — `UNIT_FIELD_CHARMEDBY` holding a
    /// guid that is not the caster's own (the client compares the field
    /// against the player's guid before it refuses).
    pub charmed: bool,
    /// `UNIT_FIELD_MOUNTDISPLAYID` is set. Both directions are refusals: a
    /// spell that is not `CASTABLE_WHILE_MOUNTED` is refused while this stands,
    /// and one carrying [`interrupt_when::NOT_MOUNTED`] is refused while it
    /// does not.
    pub mounted: bool,
    /// The caster is on a chair, a stool or the ground — `UNIT_FIELD_BYTES_1`
    /// byte 0, anything but standing.
    pub sitting: bool,
    /// The caster is sneaking — `UNIT_FIELD_BYTES_1` byte 3 bit 1, the `CREEP`
    /// bit.
    pub stealthed: bool,
    /// **No weapon is drawn** — the sheath state is `0`. Refuses the spells
    /// carrying [`interrupt_when::NOT_SHEATHED`]; `false` is "a weapon is out",
    /// which is the permissive default.
    pub sheathed: bool,
    /// `UNIT_FIELD_AURASTATE`, against [`SpellInfo::caster_aura_state`] — a
    /// one-based state number, so the bit tested is `1 << (state - 1)`
    /// as the client tests it.
    pub aura_state: u32,
    /// Whether the world's clock is inside [`is_daytime`]'s window. `None` is
    /// "the caller has no clock", which skips both of the two refusals it
    /// decides rather than guessing at one.
    pub daytime: Option<bool>,
    /// Whether the caster is under open sky. `None` likewise skips the pair.
    pub outdoors: Option<bool>,
    /// **The cost and the range this cast will really be charged and measured
    /// at**, when the caller has put the character's talents through them.
    ///
    /// `None` is "use the row's own", which is what a caller with no modifiers
    /// — the CLI checks, the tests, a character with no talents — should pass
    /// and what this function did before there was anywhere to get them from.
    /// See `crate::game::combat::spellmods`, whose whole subject is that these
    /// two numbers are on the wire and in no file.
    ///
    /// They are overrides rather than a modified [`SpellInfo`] because a
    /// `SpellInfo` carries five `String`s and this is asked once a frame per
    /// action button by the range indicator.
    pub cost_override: Option<u32>,
    pub range_override: Option<f32>,
    /// **The caster's standing rank with [`SpellInfo::min_faction_id`]**, `0`
    /// Hated to `7` Exalted, when the spell names a faction and the caller can
    /// answer. `None` skips the check, which on the shipped table is every
    /// spell — see [`spell_fields::MIN_FACTION_ID`].
    ///
    /// **The rank rather than the raw value, and the two are the same test.**
    /// The client compares the reputation *value* against
    /// `RANK_FLOORS[min_reputation]`; the floors rise with the rank, so
    /// `value >= floors[r]` and `rank_of(value) >= r` are one condition. The
    /// rank is what the wire hands this client — `SMSG_INITIALIZE_FACTIONS`
    /// resolves it before it reaches [`FactionState`] — so asking for the value
    /// would mean carrying a number nothing here has.
    ///
    /// [`FactionState`]: crate::tables::faction::FactionState
    pub reputation_rank: Option<u8>,
}

/// **The two values [`SpellInfo::prevention_type`] takes**, each gating one of
/// the [`caster_flags`] — plain compares against 1 and 2.
pub const PREVENTION_SILENCE: u32 = 1;
pub const PREVENTION_PACIFY: u32 = 2;

/// **The `UNIT_FIELD_FLAGS` bits the client's cast check reads.**
///
/// Each is tested on its own and each has its own word, and the order is the
/// client's: a stunned caster is told they are stunned even if they are also
/// silenced. The two in the middle are the ones that are *not* unconditional —
/// see [`SpellInfo::prevention_type`], which decides whether a spell is the kind
/// either word stops.
///
/// **A separate module from `faction::unit_flags`** on that module's own terms:
/// it names the bits *it* reads, and these six are a different question asked of
/// the same field.
pub mod caster_flags {
    /// `UNIT_FLAG_SILENCED`.
    pub const SILENCED: u32 = 0x0000_2000;
    /// `UNIT_FLAG_PACIFIED`.
    pub const PACIFIED: u32 = 0x0002_0000;
    /// `UNIT_FLAG_STUNNED`.
    pub const STUNNED: u32 = 0x0004_0000;
    /// `UNIT_FLAG_IN_COMBAT`.
    pub const IN_COMBAT: u32 = 0x0008_0000;
    /// `UNIT_FLAG_CONFUSED`.
    pub const CONFUSED: u32 = 0x0040_0000;
    /// `UNIT_FLAG_FLEEING`.
    pub const FLEEING: u32 = 0x0080_0000;
}

/// **The four `AuraInterruptFlags` bits that are cast *requirements* rather than
/// interruptions**, and they are read from two columns.
///
/// The columns are `AuraInterruptFlags` (22) and `ChannelInterruptFlags` (23),
/// and the client's cast check asks each of these four of the first always and
/// of the second only when the spell is [`spell_attributes::CHANNELED`], each
/// pair reporting the same word. See
/// [`SpellInfo::interrupted_when`], which is that pair as one call.
///
/// The sense is inverted from the name, and that is the reference's: a spell
/// whose aura is cancelled by *not* being mounted may only be cast **while**
/// mounted.
pub mod interrupt_when {
    /// `AURA_INTERRUPT_FLAG_NOT_MOUNTED` -> `SPELL_FAILED_ONLY_MOUNTED`.
    pub const NOT_MOUNTED: u32 = 0x0000_0040;
    /// `AURA_INTERRUPT_FLAG_NOT_ABOVEWATER` -> `SPELL_FAILED_ONLY_ABOVEWATER`,
    /// which is refused while [`SWIMMING`] stands.
    pub const NOT_ABOVEWATER: u32 = 0x0000_0080;
    /// `AURA_INTERRUPT_FLAG_NOT_UNDERWATER` -> `SPELL_FAILED_ONLY_UNDERWATER`.
    pub const NOT_UNDERWATER: u32 = 0x0000_0100;
    /// `AURA_INTERRUPT_FLAG_NOT_SHEATHED` -> `SPELL_FAILED_NOT_UNSHEATHED`.
    pub const NOT_SHEATHED: u32 = 0x0000_0200;
}

/// **`MOVEFLAG_SWIMMING`**, which is the whole of what the client means by
/// underwater: the client's four tests check exactly this bit of
/// `MovementInfo::flags` and consult neither the liquid nor the camera.
pub const SWIMMING: u32 = 0x0020_0000;

/// **`SPELL_EFFECT_APPLY_AURA` and `SPELL_AURA_MOUNTED`**, the pair that turns
/// one refusal into another — see [`SpellInfo::is_mount`].
const EFFECT_APPLY_AURA: u32 = 6;
const AURA_MOUNTED: u32 = 78;

/// **When the client's day begins and ends**, in whole minutes past midnight on
/// the world's own clock — the client builds two times and asks whether
/// now is between them.
///
/// The two are `05:30` and `21:00`. Everything either side of
/// that window is night, which is what `SPELL_FAILED_ONLY_DAYTIME` and
/// `SPELL_FAILED_ONLY_NIGHTTIME` are measured against.
pub const DAY_BEGINS: u32 = 5 * 60 + 30;
pub const DAY_ENDS: u32 = 21 * 60;

/// Is this minute-of-day inside the client's own daylight window?
pub fn is_daytime(minute_of_day: u32) -> bool {
    (DAY_BEGINS..DAY_ENDS).contains(&minute_of_day)
}

/// **The movement this client refuses a cast for** — the client's own mask
/// over the movement flags.
///
/// The four translation keys and a jump. **Turning is deliberately not in it**
/// (`TURN_LEFT`/`TURN_RIGHT` are `0x10`/`0x20` and the mask stops at `0xf`), so
/// a character spinning on the spot may still start a cast — which is the
/// reference's behaviour and would be easy to get wrong by reaching for
/// `MOVEFLAG_MASK_MOVING` instead.
pub const REFUSES_A_CAST: u32 = 0x0000_200F;

/// **The two aura-interrupt bits that make a spell refuse on the move even with
/// no cast time** — `AURA_INTERRUPT_MOVING_CANCELS | AURA_INTERRUPT_TURNING_CANCELS`,
/// tested as one byte.
const CANCELLED_BY_MOVING: u32 = 0x0000_0018;

/// **The player's own slack on a strict range check** — `Spell::CheckRange`'s
/// `range_mod`, which is `1.25` yards for a player casting and more again for
/// the landing check.
///
/// Taken rather than dropped because the check here is the *strict* one and the
/// error is one-sided: a client that refuses at exactly `max_range` refuses
/// casts the server would have accepted, which is the failure this whole
/// function exists to avoid.
const PLAYER_RANGE_SLACK: f32 = 1.25;

/// **Would this cast be refused, and with which of the game's own words?**
///
/// `None` means send it. `Some(key)` is a `GlobalStrings.lua`-resolvable
/// `SPELL_FAILED_*` key, on exactly the terms [`CastAim::Refused`] carries one —
/// so a local refusal and a server one read identically on screen, which is the
/// property that makes this safe to add. Pass the answer through
/// [`failure_override`] as the wire's own reasons already are.
///
/// The order is `Spell::CheckCast`'s: the caster's own state first, then the
/// target's, then range, then cost — the alive-state loop sits at
/// `Spell.cpp:5951`, well above `CheckRange` at 6089 and `CheckPower` at 6103.
pub fn check_cast(spell: &SpellInfo, at: &CastConditions) -> Option<&'static str> {
    if at.caster_dead && !spell.castable_while_dead() {
        return Some("SPELL_FAILED_CASTER_DEAD");
    }

    // **The caster's own state, in the client's order.** Every one
    // of these is a refusal the reference makes *before the packet*, and every
    // one of them costs a round trip without it: the cast goes out, the wind-up
    // is drawn, and it comes back refused with the same word a quarter of a
    // second later. The whole chain is one straight line of tests with no state
    // between them, which is why it reads as a list.
    //
    // The mechanic refinement is deliberately **not** here. The reference walks
    // the caster's 48 aura slots for the aura doing the stopping and, when the
    // spell may be cast through that particular one, lets the cast go — and when
    // it may not, says `SPELL_FAILED_PREVENTED_BY_MECHANIC` with the aura's own
    // mechanic instead of the plain word. What is here is the plain word, which is what the reference
    // itself falls back to when the flag is set and no slot explains it. The
    // difference is visible on the handful of spells that are castable while
    // stunned; the cost of getting it wrong is one refused press that the server
    // would have accepted, which is the same failure this whole function has.
    if at.charmed {
        return Some("SPELL_FAILED_CHARMED");
    }
    if at.unit_flags & caster_flags::STUNNED != 0 {
        return Some("SPELL_FAILED_STUNNED");
    }
    // **The two that are not unconditional**: a silence stops a spell and a
    // pacify stops an ability, and which a spell is is its own column.
    if spell.prevention_type == PREVENTION_SILENCE && at.unit_flags & caster_flags::SILENCED != 0 {
        return Some("SPELL_FAILED_SILENCED");
    }
    if spell.prevention_type == PREVENTION_PACIFY && at.unit_flags & caster_flags::PACIFIED != 0 {
        return Some("SPELL_FAILED_PACIFIED");
    }
    if at.unit_flags & caster_flags::FLEEING != 0 {
        return Some("SPELL_FAILED_FLEEING");
    }
    if at.unit_flags & caster_flags::CONFUSED != 0 {
        return Some("SPELL_FAILED_CONFUSED");
    }
    if spell.interrupted_when(interrupt_when::NOT_SHEATHED) && at.sheathed {
        return Some("SPELL_FAILED_NOT_UNSHEATHED");
    }
    if spell.interrupted_when(interrupt_when::NOT_MOUNTED) && !at.mounted {
        return Some("SPELL_FAILED_ONLY_MOUNTED");
    }
    if at.mounted && spell.attributes & spell_attributes::CASTABLE_WHILE_MOUNTED == 0 {
        return Some("SPELL_FAILED_NOT_MOUNTED");
    }
    if at.sitting && spell.attributes & spell_attributes::CASTABLE_WHILE_SITTING == 0 {
        return Some("SPELL_FAILED_NOT_STANDING");
    }
    if let Some(daytime) = at.daytime {
        if !daytime && spell.attributes & spell_attributes::DAYTIME_ONLY != 0 {
            return Some("SPELL_FAILED_ONLY_DAYTIME");
        }
        if daytime && spell.attributes & spell_attributes::NIGHT_ONLY != 0 {
            return Some("SPELL_FAILED_ONLY_NIGHTTIME");
        }
    }
    // **Underwater is `MOVEFLAG_SWIMMING` and nothing else** — see [`SWIMMING`],
    // which is read off the same word the moving refusal below reads.
    let swimming = at.move_flags & SWIMMING != 0;
    if !swimming && spell.interrupted_when(interrupt_when::NOT_UNDERWATER) {
        return Some("SPELL_FAILED_ONLY_UNDERWATER");
    }
    if swimming && spell.interrupted_when(interrupt_when::NOT_ABOVEWATER) {
        return Some("SPELL_FAILED_ONLY_ABOVEWATER");
    }

    if spell.refused_while_moving(at.move_flags) {
        return Some("SPELL_FAILED_MOVING");
    }

    // …and the five the reference asks **after** the moving one, in its order.
    // The stance check that sits between them is `crate::tables::shapeshift`'s
    // and is asked by the caller, because a form is the one condition here that
    // this client answers from something other than the caster's snapshot.
    if !at.stealthed && spell.attributes & spell_attributes::ONLY_STEALTHED != 0 {
        return Some("SPELL_FAILED_ONLY_STEALTHED");
    }
    if at.unit_flags & caster_flags::IN_COMBAT != 0
        && spell.attributes & spell_attributes::NOT_IN_COMBAT != 0
    {
        return Some("SPELL_FAILED_AFFECTING_COMBAT");
    }
    // **One-based**, which is the whole of the arithmetic: `Enrage` is state 1
    // and its bit is `1 << 0`. A state of 0 is "no requirement".
    if spell.caster_aura_state != 0
        && at.aura_state & (1u32 << (spell.caster_aura_state - 1)) == 0
    {
        return Some("SPELL_FAILED_CASTER_AURASTATE");
    }
    if spell.min_faction_id != 0 {
        if let Some(rank) = at.reputation_rank {
            if u32::from(rank) < spell.min_reputation {
                return Some("SPELL_FAILED_REPUTATION");
            }
        }
    }
    if let Some(outdoors) = at.outdoors {
        if outdoors && spell.attributes & spell_attributes::INDOOR_ONLY != 0 {
            return Some("SPELL_FAILED_ONLY_INDOORS");
        }
        if !outdoors && spell.attributes & spell_attributes::OUTDOOR_ONLY != 0 {
            // …and the one place a refusal has two words, on whether the thing
            // being refused is a mount. See [`SpellInfo::is_mount`].
            return Some(if spell.is_mount() {
                "SPELL_FAILED_NO_MOUNTS_ALLOWED"
            } else {
                "SPELL_FAILED_ONLY_OUTDOORS"
            });
        }
    }

    // **The target's own aliveness, which is the last thing `resolve_aim` does
    // not ask.** The binder tests *relations* — friendly, attackable, in the
    // party — and a corpse is still hostile and still attackable, so Fireball
    // bound cleanly to a dead mob, went out, and came back
    // `SPELL_FAILED_BAD_TARGETS` with the wind-up already drawn and then taken
    // off again. That is the "spells begin casting on invalid targets before
    // cancelling" report.
    //
    // The rule is `SpellEntry::CanTargetAliveState` and the message is
    // `Spell::CheckCast`'s own — vmangos returns `SPELL_FAILED_BAD_TARGETS`
    // there rather than `TARGETS_DEAD`, which is reserved for a dead *pet*.
    if let Some(dead) = at.target_dead {
        if !spell.can_target_alive_state(!dead) {
            return Some("SPELL_FAILED_BAD_TARGETS");
        }
    }

    // **The range check, and the four kinds of row it must not run on** — see
    // [`SpellInfo::checks_range`], which the action bar's range indicator asks
    // the same question of, so the two cannot come apart.
    if let (true, Some(distance)) = (spell.checks_range(), at.distance) {
        // **The talented range if the caller measured one**, because the server
        // will use it and a client testing the row's own refuses casts the
        // server would have taken. See [`CastConditions::range_override`].
        let max_range = at.range_override.unwrap_or(spell.range_yards);
        if distance > max_range + PLAYER_RANGE_SLACK {
            return Some("SPELL_FAILED_OUT_OF_RANGE");
        }
        // The minimum is a bare `<`, with no slack: the server's own test is
        // `if (min_range && dist < min_range)` and the slack term is added to
        // the maximum only.
        if spell.min_range_yards > 0.0 && distance < spell.min_range_yards {
            return Some("SPELL_FAILED_TOO_CLOSE");
        }
    }

    // **The cost, in the currency the spell states.** A pool the caster does not
    // have is not a refusal here: `power` is `None` for it and the server is
    // left to say so, because "a warrior pressing a mana spell" is a case this
    // client would rather get wrong towards sending.
    // …and the talented cost, on the same terms as the range above.
    let cost = at.cost_override.unwrap_or(spell.power_cost);
    if cost > 0 {
        if let Some(power) = at.power {
            if power < cost {
                return Some("SPELL_FAILED_NO_POWER");
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// **The six `UNIT_FIELD_FLAGS` refusals**, including the two that are not
    /// unconditional. See [`caster_flags`] and [`SpellInfo::prevention_type`].
    #[test]
    fn the_caster_state_flags_each_refuse_with_their_own_word() {
        let spell = SpellInfo::default();
        let at = |flags: u32| CastConditions { unit_flags: flags, ..CastConditions::default() };
        assert_eq!(check_cast(&spell, &at(caster_flags::STUNNED)), Some("SPELL_FAILED_STUNNED"));
        assert_eq!(check_cast(&spell, &at(caster_flags::FLEEING)), Some("SPELL_FAILED_FLEEING"));
        assert_eq!(check_cast(&spell, &at(caster_flags::CONFUSED)), Some("SPELL_FAILED_CONFUSED"));
        // …and a charm by somebody else, which is a guid rather than a flag.
        let charmed = CastConditions { charmed: true, ..CastConditions::default() };
        assert_eq!(check_cast(&spell, &charmed), Some("SPELL_FAILED_CHARMED"));

        // **Silence stops a spell, pacify stops an ability**, and which a row is
        // is `PreventionType`. A silenced warrior may still Heroic Strike.
        let cast = SpellInfo { prevention_type: PREVENTION_SILENCE, ..SpellInfo::default() };
        let ability = SpellInfo { prevention_type: PREVENTION_PACIFY, ..SpellInfo::default() };
        assert_eq!(check_cast(&cast, &at(caster_flags::SILENCED)), Some("SPELL_FAILED_SILENCED"));
        assert_eq!(check_cast(&ability, &at(caster_flags::SILENCED)), None);
        assert_eq!(check_cast(&ability, &at(caster_flags::PACIFIED)), Some("SPELL_FAILED_PACIFIED"));
        assert_eq!(check_cast(&cast, &at(caster_flags::PACIFIED)), None);
        // A row with neither is stopped by neither, which is most of the table.
        assert_eq!(check_cast(&spell, &at(caster_flags::SILENCED)), None);
        assert_eq!(check_cast(&spell, &at(caster_flags::PACIFIED)), None);

        // **In combat is a refusal only for the rows that say so**, which is the
        // other direction from the five above.
        let peaceful =
            SpellInfo { attributes: spell_attributes::NOT_IN_COMBAT, ..SpellInfo::default() };
        assert_eq!(
            check_cast(&peaceful, &at(caster_flags::IN_COMBAT)),
            Some("SPELL_FAILED_AFFECTING_COMBAT")
        );
        assert_eq!(check_cast(&spell, &at(caster_flags::IN_COMBAT)), None);
    }

    /// **The four [`interrupt_when`] words read two columns**, and the second
    /// only for a channel — see [`SpellInfo::interrupted_when`].
    #[test]
    fn a_requirement_word_is_read_off_the_channel_column_only_for_a_channel() {
        let aura = SpellInfo {
            aura_interrupt_flags: interrupt_when::NOT_SHEATHED,
            ..SpellInfo::default()
        };
        let channel_word = SpellInfo {
            channel_interrupt_flags: interrupt_when::NOT_SHEATHED,
            ..SpellInfo::default()
        };
        let channelled = SpellInfo {
            attributes_ex: spell_attributes::IS_SELF_CHANNELED,
            ..channel_word.clone()
        };
        let sheathed = CastConditions { sheathed: true, ..CastConditions::default() };
        assert_eq!(check_cast(&aura, &sheathed), Some("SPELL_FAILED_NOT_UNSHEATHED"));
        // The channel column alone says nothing about a spell that never channels.
        assert_eq!(check_cast(&channel_word, &sheathed), None);
        assert_eq!(check_cast(&channelled, &sheathed), Some("SPELL_FAILED_NOT_UNSHEATHED"));
        // …and a drawn weapon is no refusal either way.
        assert_eq!(check_cast(&aura, &CastConditions::default()), None);
    }

    /// **Both mount refusals, which point opposite ways** — see
    /// [`spell_attributes::CASTABLE_WHILE_MOUNTED`].
    #[test]
    fn a_mount_refuses_a_cast_and_a_cast_can_refuse_a_dismounted_caster() {
        let ordinary = SpellInfo::default();
        let mounted = CastConditions { mounted: true, ..CastConditions::default() };
        assert_eq!(check_cast(&ordinary, &mounted), Some("SPELL_FAILED_NOT_MOUNTED"));
        let allowed = SpellInfo {
            attributes: spell_attributes::CASTABLE_WHILE_MOUNTED,
            ..SpellInfo::default()
        };
        assert_eq!(check_cast(&allowed, &mounted), None);
        // …and the other way: a word that only stands while mounted. **It has
        // to carry the attribute as well**, and that is the reference's own
        // arithmetic rather than the test's convenience: the two refusals are
        // eleven instructions apart in the one direction, so a row demanding a
        // mount and not permitting one could never be cast at all.
        let riding = SpellInfo {
            aura_interrupt_flags: interrupt_when::NOT_MOUNTED,
            attributes: spell_attributes::CASTABLE_WHILE_MOUNTED,
            ..SpellInfo::default()
        };
        assert_eq!(
            check_cast(&riding, &CastConditions::default()),
            Some("SPELL_FAILED_ONLY_MOUNTED")
        );
        assert_eq!(check_cast(&riding, &mounted), None);
    }

    /// **The window is 05:30 to 21:00** — [`DAY_BEGINS`].
    #[test]
    fn the_clients_own_daylight_window_decides_the_two_time_refusals() {
        assert!(!is_daytime(5 * 60 + 29));
        assert!(is_daytime(DAY_BEGINS));
        assert!(is_daytime(20 * 60 + 59));
        assert!(!is_daytime(DAY_ENDS));
        assert!(!is_daytime(0));

        let day = SpellInfo { attributes: spell_attributes::DAYTIME_ONLY, ..SpellInfo::default() };
        let night = SpellInfo { attributes: spell_attributes::NIGHT_ONLY, ..SpellInfo::default() };
        let at = |daytime| CastConditions { daytime: Some(daytime), ..CastConditions::default() };
        assert_eq!(check_cast(&day, &at(false)), Some("SPELL_FAILED_ONLY_DAYTIME"));
        assert_eq!(check_cast(&day, &at(true)), None);
        assert_eq!(check_cast(&night, &at(true)), Some("SPELL_FAILED_ONLY_NIGHTTIME"));
        assert_eq!(check_cast(&night, &at(false)), None);
        // **No clock asks nothing**, which is the safe half of the `Option`.
        assert_eq!(check_cast(&day, &CastConditions::default()), None);
        assert_eq!(check_cast(&night, &CastConditions::default()), None);
    }

    /// **Underwater is the swimming move flag** — see [`SWIMMING`], and note
    /// that it is the same word the moving refusal reads.
    #[test]
    fn the_water_refusals_are_read_off_the_movement_flags() {
        let under = SpellInfo {
            aura_interrupt_flags: interrupt_when::NOT_UNDERWATER,
            ..SpellInfo::default()
        };
        let above = SpellInfo {
            aura_interrupt_flags: interrupt_when::NOT_ABOVEWATER,
            ..SpellInfo::default()
        };
        let swimming = CastConditions { move_flags: SWIMMING, ..CastConditions::default() };
        assert_eq!(
            check_cast(&under, &CastConditions::default()),
            Some("SPELL_FAILED_ONLY_UNDERWATER")
        );
        assert_eq!(check_cast(&under, &swimming), None);
        assert_eq!(check_cast(&above, &swimming), Some("SPELL_FAILED_ONLY_ABOVEWATER"));
        assert_eq!(check_cast(&above, &CastConditions::default()), None);
        // Swimming is not one of the four translations the moving mask holds,
        // so it never reads as a moving refusal on its own.
        assert_eq!(SWIMMING & REFUSES_A_CAST, 0);
    }

    /// **`SPELL_FAILED_NO_MOUNTS_ALLOWED` is the outdoor refusal wearing a
    /// second word** — see [`SpellInfo::is_mount`].
    #[test]
    fn an_outdoor_only_spell_indoors_says_no_mounts_when_it_is_one() {
        let outdoor =
            SpellInfo { attributes: spell_attributes::OUTDOOR_ONLY, ..SpellInfo::default() };
        let mut mount = outdoor.clone();
        mount.effects[1] = SpellEffect { kind: 6, aura: 78, ..SpellEffect::default() };
        let inside = CastConditions { outdoors: Some(false), ..CastConditions::default() };
        let outside = CastConditions { outdoors: Some(true), ..CastConditions::default() };
        assert_eq!(check_cast(&outdoor, &inside), Some("SPELL_FAILED_ONLY_OUTDOORS"));
        assert_eq!(check_cast(&mount, &inside), Some("SPELL_FAILED_NO_MOUNTS_ALLOWED"));
        assert_eq!(check_cast(&mount, &outside), None);
        // …and the other half of the pair.
        let indoor =
            SpellInfo { attributes: spell_attributes::INDOOR_ONLY, ..SpellInfo::default() };
        assert_eq!(check_cast(&indoor, &outside), Some("SPELL_FAILED_ONLY_INDOORS"));
        assert_eq!(check_cast(&indoor, &inside), None);
        // No sky overhead to report asks neither.
        assert_eq!(check_cast(&outdoor, &CastConditions::default()), None);
    }

    /// **`CasterAuraState` is one-based**, which is the whole of the arithmetic
    /// and the easiest thing here to get wrong by one.
    #[test]
    fn the_caster_aura_state_bit_is_the_state_less_one() {
        let enrage = SpellInfo { caster_aura_state: 1, ..SpellInfo::default() };
        let none = CastConditions::default();
        assert_eq!(check_cast(&enrage, &none), Some("SPELL_FAILED_CASTER_AURASTATE"));
        let held = CastConditions { aura_state: 1 << 0, ..CastConditions::default() };
        assert_eq!(check_cast(&enrage, &held), None);
        // State 8 is bit 7, not bit 8.
        let eighth = SpellInfo { caster_aura_state: 8, ..SpellInfo::default() };
        let seven = CastConditions { aura_state: 1 << 7, ..CastConditions::default() };
        assert_eq!(check_cast(&eighth, &seven), None);
        // A spell with no requirement is never refused for it.
        assert_eq!(check_cast(&SpellInfo::default(), &none), None);
    }

    /// **Stealth, and the reputation rank whose one row is disabled** — see
    /// [`spell_fields::MIN_FACTION_ID`].
    #[test]
    fn the_two_refusals_after_the_stance_check() {
        let ambush =
            SpellInfo { attributes: spell_attributes::ONLY_STEALTHED, ..SpellInfo::default() };
        assert_eq!(
            check_cast(&ambush, &CastConditions::default()),
            Some("SPELL_FAILED_ONLY_STEALTHED")
        );
        let sneaking = CastConditions { stealthed: true, ..CastConditions::default() };
        assert_eq!(check_cast(&ambush, &sneaking), None);

        let owed = SpellInfo { min_faction_id: 369, min_reputation: 4, ..SpellInfo::default() };
        let unfriendly = CastConditions { reputation_rank: Some(2), ..CastConditions::default() };
        let friendly = CastConditions { reputation_rank: Some(4), ..CastConditions::default() };
        assert_eq!(check_cast(&owed, &unfriendly), Some("SPELL_FAILED_REPUTATION"));
        assert_eq!(check_cast(&owed, &friendly), None);
        // No answer asks nothing, which is what every one of the 22,359 other
        // rows gets.
        assert_eq!(check_cast(&owed, &CastConditions::default()), None);
    }

    /// **The moving refusal, with its three easy-to-get-wrong details.** See
    /// [`SpellInfo::refused_while_moving`], which states the client's rule.
    #[test]
    fn a_cast_is_refused_on_the_move_by_the_clients_own_rule() {
        use crate::tables::spellbook::REFUSES_A_CAST;
        const FORWARD: u32 = 0x1;
        const TURN_LEFT: u32 = 0x10;
        const JUMPING: u32 = 0x2000;
        // Brown Horse (458): InterruptFlags 31, a 3,000 ms cast, no auto-repeat.
        let mount = SpellInfo { interrupt_flags: 31, cast_time_ms: 3000, ..SpellInfo::default() };
        assert!(mount.refused_while_moving(FORWARD));
        assert!(mount.refused_while_moving(JUMPING), "a jump is in the mask");
        assert!(!mount.refused_while_moving(TURN_LEFT), "turning is not moving");
        assert!(!mount.refused_while_moving(0));
        assert_eq!(REFUSES_A_CAST & TURN_LEFT, 0);
        // An instant with no interrupt words goes out on the run.
        let instant = SpellInfo { interrupt_flags: 31, cast_time_ms: 0, ..SpellInfo::default() };
        assert!(!instant.refused_while_moving(FORWARD));
        // …unless its aura word says moving cancels it — food and drink.
        let drink = SpellInfo { interrupt_flags: 1, aura_interrupt_flags: 0x8, ..SpellInfo::default() };
        assert!(drink.refused_while_moving(FORWARD));
        // An auto-repeat is exempt whatever its cast time.
        let volley = SpellInfo {
            interrupt_flags: 31,
            cast_time_ms: 500,
            attributes_ex2: spell_attributes::EX2_AUTO_REPEAT,
            ..SpellInfo::default()
        };
        assert!(!volley.refused_while_moving(FORWARD));
        // A spell movement does not interrupt is never refused for it.
        let cast = SpellInfo { interrupt_flags: 0, cast_time_ms: 3000, ..SpellInfo::default() };
        assert!(!cast.refused_while_moving(FORWARD));
        // …and it is the first thing `check_cast` says after the caster's death.
        let at = CastConditions { move_flags: FORWARD, ..CastConditions::default() };
        assert_eq!(check_cast(&mount, &at), Some("SPELL_FAILED_MOVING"));
    }

    fn spell(targets: u32, implicit: u32) -> SpellInfo {
        SpellInfo {
            id: 1,
            name: "Test".into(),
            range_yards: 30.0,
            gcd_category: 133,
            gcd_ms: 1500,
            targets,
            implicit_target_a: implicit,
            ..SpellInfo::default()
        }
    }

    fn candidate(guid: u64, reaction: Reaction) -> Candidate {
        Candidate {
            guid,
            is_self: false,
            reaction,
            unit_flags: 0,
            dead: false,
        }
    }

    /// The catalog reads the columns the module comment pins, through the two
    /// small tables the indices point into.
    #[test]
    fn a_spell_row_resolves_its_name_cast_time_range_and_icon() {
        // One Spell.dbc row shaped like Fireball's: name at 120, rank at 129,
        // icon 185, cast-time index 16, range index 35, 30 mana.
        let mut row = vec![0u32; 173];
        row[0] = 133;
        row[spell_fields::CASTING_TIME_INDEX] = 16;
        row[spell_fields::MANA_COST] = 30;
        row[spell_fields::RANGE_INDEX] = 35;
        row[spell_fields::ICON_ID] = 185;
        row[spell_fields::NAME] = 1;
        row[spell_fields::RANK] = 10;
        row[spell_fields::START_RECOVERY_CATEGORY] = 133;
        row[spell_fields::START_RECOVERY_TIME] = 1500;
        row[spell_fields::IMPLICIT_TARGET_A] = 6;
        // …and the tooltip half of the same record, pinned in the same test:
        // the sentence, the reagent, the duration and effect 0's four value
        // columns, which are the ones `spelltext` reads.
        row[spell_fields::DESCRIPTION] = 17;
        row[spell_fields::REAGENT] = 17031;
        row[spell_fields::REAGENT_COUNT] = 2;
        row[spell_fields::DURATION_INDEX] = 35;
        row[spell_fields::EFFECT] = 2;
        row[spell_fields::EFFECT_BASE_POINTS] = 13;
        row[spell_fields::EFFECT_DIE_SIDES] = 9;
        row[spell_fields::EFFECT_BASE_DICE] = 1;
        row[spell_fields::EFFECT_RADIUS_INDEX] = 13;
        let strings = b"\0Fireball\0Rank 1\0Hurls a fiery ball.\0";

        let spells = Spells::parse(
            &dbc(&[row], 173, strings),
            &dbc(&[vec![16, 1500, 0, 1500]], 4, b"\0"),
            &dbc(&[vec![35, 0, 35.0f32.to_bits()]], 3, b"\0"),
            &dbc(&[vec![185, 1]], 2, b"\0Interface\\Icons\\Spell_Fire_FlameBolt\0"),
            &dbc(&[vec![35, 4000, 0, 4000]], 4, b"\0"),
            &dbc(&[vec![13, 10.0f32.to_bits(), 0, 10.0f32.to_bits()]], 4, b"\0"),
            &[],
        )
        .expect("a catalog");

        let info = spells.info(133).expect("Fireball");
        assert_eq!(info.name, "Fireball");
        assert_eq!(info.rank, "Rank 1");
        assert_eq!(info.description, "Hurls a fiery ball.");
        assert_eq!(info.reagents, vec![(17031, 2)], "entry and count, zeroes dropped");
        assert_eq!(info.duration_ms, 4000);
        assert_eq!(info.effects[0].base_points, 13);
        assert_eq!(info.effects[0].die_sides, 9);
        assert_eq!(info.effects[0].radius_yards, 10.0);
        assert_eq!(info.effects[1], crate::tables::spellbook::SpellEffect::default());
        assert_eq!(info.label(), "Fireball (Rank 1)");
        assert_eq!(info.cast_time_ms, 1500);
        assert_eq!(info.power_cost, 30);
        assert_eq!(info.range_yards, 35.0);
        assert_eq!(info.icon, "Interface\\Icons\\Spell_Fire_FlameBolt");
        assert_eq!(info.gcd_ms, 1500);
        assert!(!info.is_passive());
        assert_eq!(spells.info(999), None);
    }

    /// **A stale rank is found by name and beaten by rank order** — the repair
    /// for a bar that went stale before `SMSG_SUPERCEDED_SPELL` was read.
    ///
    /// Three rows share a name and one does not, which is the whole rule: a
    /// spell nothing outranks stays where it is, and a spell that merely shares
    /// a *page* is not a rank of anything.
    #[test]
    fn a_stale_rank_is_superseded_by_the_highest_known_one_of_the_same_name() {
        let row = |id: u32, name: u32, rank: u32| {
            let mut row = vec![0u32; 173];
            row[0] = id;
            row[spell_fields::NAME] = name;
            row[spell_fields::RANK] = rank;
            row
        };
        // "\0Heroic Strike\0Rank 8\0Rank 9\0Rank 10\0Rend\0Rank 1\0"
        //   1              15       22       29        37    42
        let strings = b"\0Heroic Strike\0Rank 8\0Rank 9\0Rank 10\0Rend\0Rank 1\0";
        let spells = Spells::parse(
            &dbc(
                &[
                    row(11566, 1, 15), // Heroic Strike, Rank 8
                    row(11567, 1, 22), // …Rank 9
                    row(11568, 1, 29), // …Rank 10
                    row(772, 37, 42),  // Rend, Rank 1 — a different name
                ],
                173,
                strings,
            ),
            &dbc(&[vec![0u32, 0, 0, 0]], 4, b"\0"),
            &dbc(&[vec![0u32, 0, 0]], 3, b"\0"),
            &dbc(&[vec![0u32, 0]], 2, b"\0"),
            &dbc(&[vec![0u32, 0, 0, 0]], 4, b"\0"),
            &dbc(&[vec![0u32, 0, 0, 0]], 4, b"\0"),
            &[],
        )
        .expect("a catalog");

        // The book holds ranks 9 and 10; the bar holds 8. The *highest* wins,
        // not the first one found — "Rank 10" is 10 and not 1, which is the
        // digit scan the fallback would get wrong.
        let known = vec![11567, 11568, 772];
        assert_eq!(spells.superseding_rank(11566, &known), Some(11568));

        // …the one the bar already holds is not superseded by itself.
        assert_eq!(spells.superseding_rank(11568, &known), None);
        // …a spell nothing outranks is left alone.
        assert_eq!(spells.superseding_rank(772, &known), None);
        // …and a book that has only lower ranks does nothing.
        assert_eq!(spells.superseding_rank(11568, &[11566, 11567]), None);
        // A spell the table does not carry cannot be reconciled at all.
        assert_eq!(spells.superseding_rank(999_999, &known), None);
    }

    /// **The ground circle is the larger of the first two effects, clamped at
    /// 20, and effect 2 is not consulted at all.**
    ///
    /// Three separate claims and each fails invisibly: taking effect 0 alone
    /// draws Volley's circle at nothing, taking the max over all three reads a
    /// column the client's eleven instructions never touch, and leaving the
    /// clamp off draws `SpellRadius` id 12 — 100 yards — across the screen.
    #[test]
    fn the_ground_circle_is_the_wider_of_the_first_two_effects() {
        let mut row = vec![0u32; 173];
        row[0] = 133;
        row[spell_fields::NAME] = 1;
        // effect 0 narrow, effect 1 wide, effect 2 wider than the clamp.
        row[spell_fields::EFFECT_RADIUS_INDEX] = 7;
        row[spell_fields::EFFECT_RADIUS_INDEX + 1] = 8;
        row[spell_fields::EFFECT_RADIUS_INDEX + 2] = 12;
        let radii = dbc(
            &[
                vec![7, 2.0f32.to_bits(), 0, 2.0f32.to_bits()],
                vec![8, 5.0f32.to_bits(), 0, 5.0f32.to_bits()],
                vec![12, 100.0f32.to_bits(), 0, 100.0f32.to_bits()],
            ],
            4,
            b"\0",
        );
        let spells = Spells::parse(
            &dbc(&[row.clone()], 173, b"\0Blizzard\0"),
            &[],
            &[],
            &[],
            &[],
            &radii,
            &[],
        )
        .expect("a catalog");
        assert_eq!(
            spells.info(133).expect("the spell").ground_circle_yards(),
            5.0,
            "effect 1's five yards, not effect 0's two and not effect 2's hundred"
        );

        // …and the clamp, with a hundred-yard row in a slot that *is* read.
        row[spell_fields::EFFECT_RADIUS_INDEX] = 12;
        let spells = Spells::parse(
            &dbc(&[row], 173, b"\0Blizzard\0"),
            &[],
            &[],
            &[],
            &[],
            &radii,
            &[],
        )
        .expect("a catalog");
        assert_eq!(
            spells.info(133).expect("the spell").ground_circle_yards(),
            GROUND_CIRCLE_MAX_YARDS
        );
    }

    /// **The three small tables degrade one at a time.** Without them the bar
    /// still names its buttons, which is the whole point of them being separate.
    #[test]
    fn the_supporting_tables_are_optional() {
        let mut row = vec![0u32; 173];
        row[0] = 133;
        row[spell_fields::NAME] = 1;
        row[spell_fields::CASTING_TIME_INDEX] = 16;
        let spells =
            Spells::parse(&dbc(&[row], 173, b"\0Fireball\0"), &[], &[], &[], &[], &[], &[])
                .expect("a catalog with no supporting tables");
        let info = spells.info(133).expect("Fireball");
        assert_eq!(info.name, "Fireball");
        assert_eq!(info.cast_time_ms, 0, "no cast-time table means instant");
        assert_eq!(info.range_yards, 0.0);
        assert!(info.icon.is_empty());
        // …and the two the description points into degrade the same way: the
        // sentence loses a number rather than the spell losing its button.
        assert_eq!(info.duration_ms, 0, "no duration table means no `$d`");
        assert_eq!(info.effects[0].radius_yards, 0.0, "and no `$a`");
        // …and the buff bar's: a debuff with no dispel table draws the "none"
        // border rather than losing its icon.
        assert!(info.dispel_type.is_empty(), "no dispel table means no colour");
    }

    /// **Both bits or neither**, which is the whole of `IsAutoRepeatRangedSpell`
    /// and the half of it a careful guess gets wrong.
    ///
    /// The ranged-slot bit alone is every ranged ability in the game — Aimed
    /// Shot, Multi-Shot, and the one-off Throw (2764), whose real
    /// `AttributesEx2` is 0 — so a test against it would put the player in an
    /// unstoppable loop the moment they used one. The auto-repeat bit alone is
    /// carried by rows with no ranged slot at all. The three rows here are the
    /// shipped values: Auto Shot's `0x00050012` / `0x20`, Throw's `0x00410012` /
    /// `0`, and Fireball's neither.
    #[test]
    fn an_auto_repeat_is_the_two_bits_together() {
        let spell = |id: u32, attributes: u32, ex2: u32| {
            let mut row = vec![0u32; 173];
            row[0] = id;
            row[spell_fields::ATTRIBUTES] = attributes;
            row[spell_fields::ATTRIBUTES_EX2] = ex2;
            row
        };
        let rows = [
            spell(75, 0x0005_0012, 0x20),   // Auto Shot
            spell(5019, 0x0000_0012, 0x20), // Shoot, a wand
            spell(2764, 0x0041_0012, 0),    // Throw — ranged, and one throw only
            spell(133, 0x0001_0000, 0),     // Fireball — neither
        ];
        let spells = Spells::parse(&dbc(&rows, 173, b"\0"), &[], &[], &[], &[], &[], &[])
            .expect("a catalog");
        let of = |id: u32| spells.info(id).expect("a row");
        assert!(of(75).is_auto_repeat_ranged() && of(5019).is_auto_repeat_ranged());
        assert!(
            of(2764).uses_ranged_slot() && !of(2764).is_auto_repeat_ranged(),
            "Throw is a ranged ability and is thrown once"
        );
        assert!(!of(133).uses_ranged_slot() && !of(133).is_auto_repeat_ranged());
        // …and the census, which is what `vale spellbook` prints: three of
        // the four carry the slot bit and two of those repeat.
        assert_eq!(spells.ranged_slot_count(), 3);
        assert_eq!(
            spells.auto_repeat_ranged().iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![75, 5019]
        );
    }

    /// **Only four of the eleven dispel classes have a word**, and the file is
    /// what says which — see [`DISPEL_TYPE_NAME`]. `RefreshBuffs` indexes
    /// `DebuffTypeColor` with whatever comes back, so a fifth name would take
    /// the panel down rather than draw an odd colour.
    #[test]
    fn only_the_dispel_classes_the_file_names_are_sayable() {
        let spell = |id: u32, dispel: u32| {
            let mut row = vec![0u32; 173];
            row[0] = id;
            row[spell_fields::DISPEL] = dispel;
            row
        };
        // `SpellDispelType.dbc`'s own shape: id, the display name, seven empty
        // locales, the mask, a flag, and the sparse second name at 11.
        let dispel_row = |id: u32, name: u32, sayable: u32| {
            let mut row = vec![0u32; 12];
            row[0] = id;
            row[1] = name;
            row[10] = sayable;
            row[DISPEL_TYPE_NAME] = if sayable == 0 { 0 } else { name };
            row
        };
        let spells = Spells::parse(
            &dbc(&[spell(118, 1), spell(1784, 5), spell(133, 0)], 173, b"\0"),
            &[],
            &[],
            &[],
            &[],
            &[],
            &dbc(
                &[
                    dispel_row(0, 1, 0),
                    dispel_row(1, 6, 1),
                    dispel_row(5, 12, 0),
                ],
                12,
                b"\0None\0Magic\0Stealth\0",
            ),
        )
        .expect("a catalog");
        assert_eq!(spells.info(118).expect("Polymorph").dispel_type, "Magic");
        assert_eq!(
            spells.info(1784).expect("Stealth").dispel_type,
            "",
            "Stealth is a dispel class with no colour, and DebuffTypeColor has no key for it"
        );
        assert_eq!(spells.info(133).expect("Fireball").dispel_type, "");
    }

    /// **A warrior's stance is an aura with no icon**, and the bit that says so
    /// is in `AttributesEx` rather than in the column the spellbook filters on —
    /// see [`spell_attributes::NO_AURA_ICON`]. Battle Stance's own two words.
    #[test]
    fn a_stance_is_hidden_from_the_buff_bar_and_not_from_the_book() {
        let mut row = vec![0u32; 173];
        row[0] = 2457;
        row[spell_fields::ATTRIBUTES] = 0x0905_0010;
        row[spell_fields::ATTRIBUTES_EX] = 0x9000_0000;
        let spells = Spells::parse(&dbc(&[row], 173, b"\0"), &[], &[], &[], &[], &[], &[])
            .expect("a catalog");
        let info = spells.info(2457).expect("Battle Stance");
        assert!(!info.hidden(), "bit 7 of Attributes is clear, so it is in the book");
        assert!(info.no_aura_icon(), "…and bit 28 of AttributesEx hides its aura");
    }

    /// **`SpellShapeshiftForm.dbc` field 1 is the bonus action bar**, and the
    /// four values here are the file's own — see [`ShapeshiftForms`].
    ///
    /// The check that matters is Travel Form reading **0** beside the three
    /// stances reading 1, 2 and 3: a column that were merely correlated with the
    /// row id would not have a zero in the middle of it, and a zero is what
    /// keeps a form with no bar of its own on the ordinary page.
    #[test]
    fn the_form_says_which_bonus_bar_is_showing() {
        let form = |id: u32, bar: u32| {
            let mut row = vec![0u32; 14];
            row[0] = id;
            row[1] = bar;
            row
        };
        let forms = ShapeshiftForms::parse(&dbc(
            &[
                form(1, 1),   // Cat Form
                form(3, 0),   // Travel Form — no bar of its own
                form(17, 1),  // Battle Stance
                form(18, 2),  // Defensive Stance
                form(19, 3),  // Berserker Stance
            ],
            14,
            b"\0",
        ));
        assert_eq!(forms.bonus_bar(0), 0, "no form is the ordinary bar");
        assert_eq!(forms.bonus_bar(17), 1);
        assert_eq!(forms.bonus_bar(18), 2);
        assert_eq!(forms.bonus_bar(19), 3);
        assert_eq!(forms.bonus_bar(3), 0, "a form with no bar stays on the page");
        assert_eq!(forms.bonus_bar(200), 0, "…and so does a form the file lacks");
        assert_eq!(forms.with_a_bar(), 4);

        // An absent table answers the ordinary bar for everything, which is the
        // degradation rather than a wrong bar.
        assert_eq!(ShapeshiftForms::parse(&[]).bonus_bar(17), 0);
    }

    /// **A self-buff sends no target.** This is the rule's whole reason for
    /// existing: shipping the current selection here is how Ice Armor comes back
    /// "Invalid target" while looking, from the client's side, like a server
    /// bug.
    #[test]
    fn a_self_buff_commits_with_no_target_even_with_an_enemy_selected() {
        // Implicit target 1 (TARGET_SELF) clears the explicit bit and leaves an
        // empty word.
        let ice_armor = spell(0, 1);
        assert_eq!(aim_mask(&ice_armor), 0);
        assert_eq!(
            resolve_aim(
                &ice_armor,
                Some(candidate(42, Reaction::Hostile)),
                Some(candidate(7, Reaction::Friendly)),
                false, None),
            CastAim::SelfImplicit
        );
    }

    /// Fireball: no `Targets` column at all, implicit target 6, which is what
    /// makes it a cast that needs a hostile unit.
    #[test]
    fn an_attack_spell_binds_the_selection_and_refuses_a_friend() {
        let fireball = spell(0, 6);
        assert_eq!(aim_mask(&fireball), aim_bits::UNIT_ENEMY);
        assert_eq!(
            resolve_aim(&fireball, Some(candidate(42, Reaction::Hostile)), None, false, None),
            CastAim::Unit(42)
        );
        // A neutral critter is attackable — "not friendly" is the test.
        assert_eq!(
            resolve_aim(&fireball, Some(candidate(42, Reaction::Neutral)), None, false, None),
            CastAim::Unit(42)
        );
        // A friend is not, and the message is the client's own.
        assert_eq!(
            resolve_aim(&fireball, Some(candidate(42, Reaction::Friendly)), None, false, None),
            CastAim::Refused(INVALID_TARGET)
        );
        // Nothing selected at all is the *other* message.
        assert_eq!(
            resolve_aim(&fireball, None, Some(candidate(7, Reaction::Friendly)), true, None),
            CastAim::Refused(NO_TARGET)
        );
    }

    /// A flagged unit is refused whatever its faction says — a quest giver, a
    /// flight master mid-flight, a creature still spawning.
    #[test]
    fn an_unattackable_flag_refuses_an_attack_spell() {
        let fireball = spell(0, 6);
        let mut hostile = candidate(42, Reaction::Hostile);
        hostile.unit_flags = crate::tables::faction::UNATTACKABLE_FLAGS;
        assert_eq!(
            resolve_aim(&fireball, Some(hostile), None, false, None),
            CastAim::Refused(INVALID_TARGET)
        );
    }

    /// **The auto-self-cast fallback**, which is the classic "buffing with an
    /// enemy targeted heals you instead" — and it is off by default in the game.
    #[test]
    fn a_friendly_cast_falls_back_to_the_caster_only_when_asked() {
        // Implicit target 21 sets the assist bit: a heal.
        let heal = spell(0, 21);
        assert_eq!(aim_mask(&heal), aim_bits::UNIT_ASSIST);
        let enemy = Some(candidate(42, Reaction::Hostile));
        let me = Some(candidate(7, Reaction::Friendly));
        assert_eq!(resolve_aim(&heal, enemy, me, true, None), CastAim::Unit(7));
        // …and with a friend selected it binds the friend either way.
        assert_eq!(
            resolve_aim(&heal, Some(candidate(9, Reaction::Friendly)), me, true, None),
            CastAim::Unit(9)
        );
        // **…and with nothing selected at all it binds the caster**, which is
        // the half of the setting a player actually notices: a buff pressed on
        // an empty screen lands rather than putting the targeting cursor up.
        // Compare `a_friendly_cast_with_nothing_to_bind_puts_the_cursor_up`,
        // which is the same press with the CVar at its shipped `"0"`.
        assert_eq!(resolve_aim(&heal, None, me, true, None), CastAim::Unit(7));
    }

    /// **The CVar's name is the one the interface writes**, which is the whole
    /// of what [`AUTO_SELF_CAST`] is for: `OptionsFrame.xml`'s checkbox does
    /// `SetCVar(this.cvar, ...)` with that spelling, and a client reading a
    /// different one reads the default for ever.
    #[test]
    fn the_auto_self_cast_cvar_is_one_the_client_registers() {
        assert!(crate::interface::cvars::DEFAULTS
            .iter()
            .any(|(name, value)| *name == AUTO_SELF_CAST && *value == "0"));
    }

    /// **A friendly spell that binds nothing asks rather than refuses**, which
    /// is the whole of the "positive spells say 'No target'" report.
    ///
    /// The client's own branch turns on one bit of the aiming word
    /// (`UNIT_ENEMY`), and the two spells here are the game's
    /// own example of each side of it — Fireball's word is `UNIT_ENEMY` (0x0080)
    /// and Lesser Heal's is `UNIT_ASSIST` (0x0100). So a Fireball pressed at
    /// nothing says "You have no target." and a heal pressed at nothing puts the
    /// targeting cursor up, with `autoSelfCast` off in both cases because that
    /// is the shipped default (the client registers it with `"0"`).
    #[test]
    fn a_friendly_cast_with_nothing_to_bind_puts_the_cursor_up() {
        let heal = spell(0, 21);
        let me = Some(candidate(7, Reaction::Friendly));
        assert_eq!(resolve_aim(&heal, None, me, false, None), CastAim::WantsTarget);
        // …and with an enemy selected too: the selection did not fit and the
        // caster was not offered, so there is still nothing bound.
        assert_eq!(
            resolve_aim(&heal, Some(candidate(42, Reaction::Hostile)), me, false, None),
            CastAim::WantsTarget
        );

        // The enemy bit is what makes the difference, and nothing else is.
        let fireball = spell(0, 6);
        assert_eq!(aim_mask(&fireball), aim_bits::UNIT_ENEMY);
        assert_eq!(
            resolve_aim(&fireball, None, me, false, None),
            CastAim::Refused(NO_TARGET)
        );

        // **And a spell that binds the main hand is refused before either**, on
        // the client's own first branch: an empty weapon hand is not a question
        // to ask the player. A poison's word is friendly, so without this test's
        // own subject it would reach the cursor.
        let mut poison = spell(0, 21);
        poison.attributes |= spell_attributes::HELD_ITEM_ONLY;
        assert_eq!(
            resolve_aim(&poison, None, me, false, None),
            CastAim::Refused(MAINHAND_EMPTY)
        );
    }

    /// **The one failure with no string of its own.** `SPELL_FAILED_NO_POWER`
    /// is absent from `GlobalStrings.lua` — verified against the shipped file —
    /// because the client always replaces it with one of the four
    /// `ERR_OUT_OF_*`, chosen by the spell's power type. A client that only
    /// looked up the reason key would show the commonest failure in the game as
    /// a blank line.
    #[test]
    fn the_power_failure_is_shown_as_the_power_it_wanted() {
        assert_eq!(failure_override("SPELL_FAILED_NO_POWER", 0), Some("ERR_OUT_OF_MANA"));
        assert_eq!(failure_override("SPELL_FAILED_NO_POWER", 1), Some("ERR_OUT_OF_RAGE"));
        assert_eq!(failure_override("SPELL_FAILED_NO_POWER", 3), Some("ERR_OUT_OF_ENERGY"));
        assert_eq!(
            failure_override("SPELL_FAILED_NOT_READY", 0),
            Some("ERR_SPELL_COOLDOWN")
        );
        // Everything else passes through to its own key.
        assert_eq!(failure_override("SPELL_FAILED_OUT_OF_RANGE", 0), None);
    }

    /// A word wanting something that is not a unit, not a place **and not an
    /// item** is refused rather than sent. The gameobject and string machines
    /// are not modelled, and a cast the server will reject is worse than a
    /// local message.
    #[test]
    fn a_target_this_client_cannot_bind_is_refused_locally() {
        // Bit 13 is a string — `TARGET_FLAG_STRING`, and nothing produces one.
        let on_a_string = spell(0x2000, 0);
        assert_eq!(
            resolve_aim(&on_a_string, Some(candidate(42, Reaction::Hostile)), None, true, None),
            CastAim::Refused(INVALID_TARGET)
        );
    }

    /// **Rockbiter Weapon, as a unit test** — `vale spellbook 8017` says its
    /// `Targets` column is `0x0010` alone and its attributes carry
    /// `0x00050200`, so the held-item bit is set. Before the item bit was read,
    /// the word fell past `UNIT_FAMILY` into the local refusal and the report
    /// was *"Enchant-type spells say Invalid Target"*.
    ///
    /// The two assertions that matter are the *selection* ones: an imbue binds
    /// the main hand whatever is selected and whatever it is, because the
    /// client picks the item rather than asking.
    #[test]
    fn an_imbue_binds_the_main_hand_rather_than_the_selection() {
        let mut rockbiter = spell(0x0010, 0);
        rockbiter.attributes = spell_attributes::HELD_ITEM_ONLY;
        let weapon = 0xF001_0000_0000_0042;

        for selected in [None, Some(candidate(42, Reaction::Hostile)), Some(candidate(9, Reaction::Friendly))] {
            assert_eq!(
                resolve_aim(&rockbiter, selected, None, true, Some(weapon)),
                CastAim::Item(weapon),
                "the selection reached an imbue"
            );
        }
    }

    /// **…and with an empty hand it is the client's own sentence**, not the
    /// targeting cursor: the client tests the held-item bit first when the arm
    /// fails, and vmangos comments the same bit *"Client automatically selects
    /// item from mainhand slot as a cast target"*.
    #[test]
    fn an_imbue_with_nothing_in_the_hand_says_so() {
        let mut rockbiter = spell(0x0010, 0);
        rockbiter.attributes = spell_attributes::HELD_ITEM_ONLY;
        assert_eq!(
            resolve_aim(&rockbiter, None, None, true, None),
            CastAim::Refused(MAINHAND_EMPTY)
        );
    }

    /// **An item cast *without* the held-item bit has to be pointed at one** —
    /// the enchanting formulas. Named apart from `WantsTarget` so the census
    /// can count what is still owed rather than reporting it as answered.
    #[test]
    fn an_item_cast_that_picks_nothing_asks_for_one() {
        let formula = spell(0x0010, 0);
        assert_eq!(
            resolve_aim(&formula, Some(candidate(42, Reaction::Hostile)), None, true, Some(7)),
            CastAim::WantsItem
        );
    }

    /// **…and a word naming a *destination* asks for a patch of floor**, which
    /// is the whole of "click AOE spells say invalid target".
    ///
    /// The assertion that matters is the second one: it asks **with a perfectly
    /// good hostile unit selected** and still gets the cursor. Binding the
    /// selection instead is not a harmless shortcut — the server has no unit
    /// target to use and falls back to the *caster's own position*, so a
    /// Blizzard sent that way lands on the mage's feet.
    #[test]
    fn a_ground_targeted_spell_asks_for_a_place_rather_than_a_unit() {
        // Blizzard's own column, measured: `targets 0x0040, implicit target A
        // 28 -> aiming word 0x0040` (`vale spellbook 10`).
        let blizzard = spell(0x0040, 28);
        assert_eq!(resolve_aim(&blizzard, None, None, false, None), CastAim::WantsGround);
        assert_eq!(
            resolve_aim(&blizzard, Some(candidate(42, Reaction::Hostile)), None, true, None),
            CastAim::WantsGround
        );
        // …and the self-cast binding does not redirect one onto the caster
        // either, for the same reason: there is no unit in this cast at all.
        assert_eq!(
            resolve_aim(&blizzard, None, Some(candidate(7, Reaction::Friendly)), true, None),
            CastAim::WantsGround
        );
    }

    /// **The three refusals the client can make for itself**, so a press that
    /// cannot work never draws a wind-up and never costs a round trip.
    #[test]
    fn a_cast_that_cannot_work_is_refused_before_the_socket() {
        let mut fireball = spell(0, 6);
        fireball.range_index = 35;
        fireball.power_cost = 30;

        // Everything in order: it goes.
        let ok = CastConditions {
            power: Some(100),
            distance: Some(20.0),
            ..CastConditions::default()
        };
        assert_eq!(check_cast(&fireball, &ok), None);

        // Out of range — and **not** at exactly the stated maximum, because the
        // server's own strict check adds a yard and a quarter first.
        let at_max = CastConditions { distance: Some(30.0), ..ok };
        assert_eq!(check_cast(&fireball, &at_max), None);
        let inside_slack = CastConditions { distance: Some(31.0), ..ok };
        assert_eq!(
            check_cast(&fireball, &inside_slack),
            None,
            "the player's own 1.25 yards of slack is taken, not dropped"
        );
        let far = CastConditions { distance: Some(40.0), ..ok };
        assert_eq!(check_cast(&fireball, &far), Some("SPELL_FAILED_OUT_OF_RANGE"));

        // Not enough mana.
        let poor = CastConditions { power: Some(10), ..ok };
        assert_eq!(check_cast(&fireball, &poor), Some("SPELL_FAILED_NO_POWER"));
        // …but a power the caster does not have at all is the server's to
        // answer, not this client's: `None` is "not a question here".
        let no_pool = CastConditions { power: None, ..ok };
        assert_eq!(check_cast(&fireball, &no_pool), None);

        // Dead.
        let dead = CastConditions { caster_dead: true, ..ok };
        assert_eq!(check_cast(&fireball, &dead), Some("SPELL_FAILED_CASTER_DEAD"));
        let mut ankh = fireball.clone();
        ankh.attributes |= spell_attributes::ALLOW_CAST_WHILE_DEAD;
        assert_eq!(check_cast(&ankh, &dead), None);
    }

    /// **A corpse is still hostile, which is why the aiming rule lets one
    /// through and this has to stop it.**
    ///
    /// `unsatisfied` tests relations — friendly, attackable, in the party — and
    /// nothing about a dead mob fails any of them, so Fireball bound to it
    /// cleanly, went out, and came back `SPELL_FAILED_BAD_TARGETS` with the
    /// wind-up already drawn and then taken off again. That is the report.
    #[test]
    fn a_cast_bound_to_a_corpse_is_refused_before_the_socket() {
        let mut fireball = spell(0, 6);
        fireball.range_index = 35;
        let alive = CastConditions {
            target_dead: Some(false),
            ..CastConditions::default()
        };
        let corpse = CastConditions {
            target_dead: Some(true),
            ..CastConditions::default()
        };
        assert_eq!(check_cast(&fireball, &alive), None);
        assert_eq!(
            check_cast(&fireball, &corpse),
            Some("SPELL_FAILED_BAD_TARGETS")
        );

        // **A cast that bound no unit asks nothing at all** — a self-implicit,
        // a placed spell, the pointer's own range preview. `None` is not
        // `Some(false)`, and conflating them would make every one of those
        // answer a question about a target that does not exist.
        assert_eq!(check_cast(&fireball, &CastConditions::default()), None);

        // …and the exception column: `EX2_ALLOW_DEAD_TARGET` is what makes a
        // spell legal on a corpse without making it *require* one.
        let mut soulstone = fireball.clone();
        soulstone.attributes_ex2 |= spell_attributes::EX2_ALLOW_DEAD_TARGET;
        assert_eq!(check_cast(&soulstone, &corpse), None);
        assert_eq!(check_cast(&soulstone, &alive), None);
    }

    /// **…and the rule runs the other way for the thirty-five that want a
    /// corpse**, which is `CanTargetAliveState`'s first branch: a resurrection
    /// aimed at somebody who is standing up is refused with the same message.
    ///
    /// All three of `IsDeathOnlySpell`'s inputs, each on its own: the corpse
    /// bits in `Targets` (Resurrection's 0x8000), the `AttributesEx3` ghost bit
    /// (Spirit Heal, Graveyard Teleport) and spell **2584**'s bare id, which
    /// vmangos hard-codes because its own columns do not say so.
    #[test]
    fn a_resurrection_is_refused_on_somebody_who_is_alive() {
        let alive = CastConditions {
            target_dead: Some(false),
            ..CastConditions::default()
        };
        let corpse = CastConditions {
            target_dead: Some(true),
            ..CastConditions::default()
        };

        let mut resurrection = spell(0x8000, 0);
        resurrection.range_index = 35;
        assert!(resurrection.death_only(), "the corpse bit in Targets");
        assert_eq!(
            check_cast(&resurrection, &alive),
            Some("SPELL_FAILED_BAD_TARGETS")
        );
        assert_eq!(check_cast(&resurrection, &corpse), None);

        let mut spirit_heal = spell(0, 0);
        spirit_heal.attributes_ex3 |= spell_attributes::EX3_ONLY_ON_GHOSTS;
        assert!(spirit_heal.death_only(), "the AttributesEx3 ghost bit");
        assert_eq!(
            check_cast(&spirit_heal, &alive),
            Some("SPELL_FAILED_BAD_TARGETS")
        );

        let mut waiting = spell(0, 0);
        waiting.id = 2584;
        assert!(waiting.death_only(), "Waiting to Resurrect, by id");
        assert_eq!(
            check_cast(&waiting, &alive),
            Some("SPELL_FAILED_BAD_TARGETS")
        );
        // …and an ordinary row with none of the three is not death-only, which
        // is the half that would refuse every cast in the game if it were.
        assert!(!spell(0, 6).death_only());
    }

    /// **The melee row is not five yards and the self row is not zero yards** —
    /// both are `SpellRange.dbc` rows whose numbers mean something other than a
    /// distance, and reading either literally refuses a cast that works.
    ///
    /// Heroic Strike is the measured case: row 2, a stated 5, and a warrior
    /// standing on top of the thing they are hitting is routinely further than
    /// 5 yards centre-to-centre from a large creature.
    #[test]
    fn the_two_special_range_rows_are_not_distances() {
        let mut heroic_strike = spell(0, 6);
        heroic_strike.range_index = RANGE_IDX_COMBAT;
        heroic_strike.range_yards = 5.0;
        heroic_strike.attributes = spell_attributes::ON_NEXT_SWING_NO_DAMAGE;
        assert!(heroic_strike.on_next_swing());
        let miles_off = CastConditions {
            distance: Some(200.0),
            ..CastConditions::default()
        };
        assert_eq!(check_cast(&heroic_strike, &miles_off), None);

        let mut evocation = spell(0, 1);
        evocation.range_index = RANGE_IDX_SELF_ONLY;
        evocation.range_yards = 0.0;
        assert_eq!(check_cast(&evocation, &miles_off), None);

        // **The same predicate the action bar's range indicator asks**, which is
        // why it is a method rather than four lines inside `check_cast`: a
        // button that lit red for a range this function refuses to enforce would
        // contradict its own press. See [`SpellInfo::checks_range`].
        assert!(!heroic_strike.checks_range());
        assert!(!evocation.checks_range());
        let mut unlisted = spell(0, 6);
        unlisted.range_index = 9;
        unlisted.range_yards = 0.0;
        assert!(!unlisted.checks_range(), "a row this build did not carry");
        let mut fireball = spell(0, 6);
        fireball.range_index = 4;
        fireball.range_yards = 35.0;
        assert!(fireball.checks_range());
    }

    /// A minimum range is a real refusal for the ranged abilities that carry
    /// one, and it takes **no** slack — the server adds its `range_mod` to the
    /// maximum alone.
    #[test]
    fn a_minimum_range_refuses_a_target_stood_on_top_of_you() {
        let mut shoot = spell(0, 6);
        shoot.range_index = 5;
        shoot.range_yards = 30.0;
        shoot.min_range_yards = 8.0;
        let close = CastConditions {
            distance: Some(3.0),
            ..CastConditions::default()
        };
        assert_eq!(check_cast(&shoot, &close), Some("SPELL_FAILED_TOO_CLOSE"));
        let clear = CastConditions {
            distance: Some(9.0),
            ..CastConditions::default()
        };
        assert_eq!(check_cast(&shoot, &clear), None);
    }

    /// **The two attribute tests that decide what a press *is***, against the
    /// values the files actually carry — both were read off `vale spellbook`
    /// rather than chosen, and both would answer "no" against the wrong bit.
    #[test]
    fn the_next_swing_and_channel_bits_are_the_ones_the_files_set() {
        // Heroic Strike (78): Attributes 0x00050014 — ON_NEXT_SWING_NO_DAMAGE,
        // *not* ON_NEXT_SWING, which is why the test is against both.
        let mut heroic_strike = spell(0, 6);
        heroic_strike.attributes = 0x0005_0014;
        assert!(heroic_strike.on_next_swing());
        assert!(!heroic_strike.is_channelled());

        // Evocation (12051): AttributesEx 0x00000040 — IS_SELF_CHANNELED alone.
        let mut evocation = spell(0, 1);
        evocation.attributes = 0x0001_0000;
        evocation.attributes_ex = 0x0000_0040;
        assert!(evocation.is_channelled());
        assert!(!evocation.on_next_swing());

        // …and an ordinary cast is neither.
        let fireball = spell(0, 6);
        assert!(!fireball.on_next_swing());
        assert!(!fireball.is_channelled());
    }
    /// **Judgement needs a Seal**, and the client never learns what a Seal is:
    /// the row says `casterAuraState = 5` and the server sets the bit.
    #[test]
    fn a_caster_aura_state_is_the_whole_of_the_seal_rule() {
        const JUDGEMENT: u32 = 5;
        let judgement = SpellInfo { caster_aura_state: JUDGEMENT, ..SpellInfo::default() };
        let bit = 1 << (JUDGEMENT - 1);

        assert!(!judgement.castable_now(0, None, 0, 0), "no Seal, no Judgement");
        assert!(judgement.castable_now(bit, None, 0, 0), "…and with one, yes");
        // A *different* state up is not this one — the mask is per state.
        assert!(!judgement.castable_now(1 << (1 - 1), None, 0, 0));

        // …and a row with no condition is castable whatever the state is.
        let plain = SpellInfo::default();
        assert!(plain.castable_now(0, None, 0, 0));
    }

    /// **A target-side condition with no target is unmet**, which is why
    /// Execute is grey with nothing selected rather than white.
    #[test]
    fn a_target_condition_with_no_target_is_not_met() {
        const HEALTHLESS_20: u32 = 2;
        let execute = SpellInfo { target_aura_state: HEALTHLESS_20, ..SpellInfo::default() };
        let bit = 1 << (HEALTHLESS_20 - 1);

        assert!(!execute.castable_now(0, None, 0, 0), "nothing selected");
        assert!(!execute.castable_now(0, Some(0), 0, 0), "a healthy target");
        assert!(execute.castable_now(0, Some(bit), 0, 0), "…and a hurt one");
    }

    /// **Both finisher bits count**, which is the difference between greying
    /// half a rogue's bar and all of it — see [`SpellInfo::needs_combo_points`].
    #[test]
    fn a_finisher_needs_a_combo_point_under_either_bit() {
        let damage = SpellInfo { attributes_ex: 0x0010_0000, ..SpellInfo::default() };
        let duration = SpellInfo { attributes_ex: 0x0040_0000, ..SpellInfo::default() };
        for finisher in [&damage, &duration] {
            assert!(finisher.needs_combo_points());
            assert!(!finisher.castable_now(0, None, 0, 0), "no points, no finisher");
            assert!(finisher.castable_now(0, None, 1, 0), "…and one is enough");
        }
        assert!(!SpellInfo::default().needs_combo_points());
    }

    /// **The two stance masks are opposites and form 0 is in neither**: a
    /// spell that names a stance cannot be cast unshifted, and one that
    /// excludes stances can.
    #[test]
    fn the_stance_masks_gate_opposite_ways() {
        let bear = 1u32;
        let only_in_bear = SpellInfo { stances: 1 << (bear - 1), ..SpellInfo::default() };
        let never_in_bear = SpellInfo { stances_not: 1 << (bear - 1), ..SpellInfo::default() };

        assert!(!only_in_bear.castable_now(0, None, 0, 0), "unshifted");
        assert!(only_in_bear.castable_now(0, None, 0, bear as u8));

        assert!(never_in_bear.castable_now(0, None, 0, 0), "unshifted is fine");
        assert!(!never_in_bear.castable_now(0, None, 0, bear as u8));
    }

    /// **The columns are where the neighbours say they are.** The three
    /// equipped-item fields are pinned by the two arrays either side of them —
    /// eight reagent counts from 50 and three effects from 61 — so a layout
    /// that had drifted would collide rather than read a plausible number.
    #[test]
    fn the_new_columns_do_not_collide_with_their_neighbours() {
        use spell_fields as f;
        assert!(f::REAGENT_COUNT + 8 == f::EQUIPPED_ITEM_CLASS);
        assert!(f::EQUIPPED_ITEM_INVENTORY_TYPE_MASK + 1 == f::EFFECT);
        // …and the four before `Targets`, likewise.
        assert!(f::STANCES_NOT + 1 == f::TARGETS);
        assert!(f::TARGET_AURA_STATE + 1 == f::CASTING_TIME_INDEX);
    }

}
