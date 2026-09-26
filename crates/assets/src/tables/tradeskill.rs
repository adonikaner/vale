//! **The trade-skill and craft windows as the panels index them**: which
//! recipes a profession lists, in what order, in what colour, and how many the
//! bags could make.
//!
//! `Blizzard_TradeSkillUI` and `Blizzard_CraftUI` never ask about a spell id.
//! They ask `GetNumTradeSkills()` and then `GetTradeSkillInfo(i)` for each
//! row's `(name, type, numAvailable, isExpanded)` — so, exactly as
//! [`crate::tables::book`] and [`crate::tables::trainer`] are, **the whole
//! panel is arithmetic over one ordered array**, and none of that order crosses
//! the wire. The recipes are ordinary known spells, the thresholds are
//! `SkillLineAbility.dbc`, and everything else here follows the client.
//!
//! ## The rules
//!
//! ```text
//! build        collect the known spells of the open skill line, one row
//!              each — spell id, difficulty index, group (item class,
//!              subclass), inv-slot mask, item level, numAvailable, visible
//! thresholds   min_value == 0 -> max(max_value - 25, 0), then the three
//!              thresholds below
//! numAvailable min over the 8 reagent slots of have/need, clamped to >= 0
//! created item (EffectItemType[0], spell record +0x19c) is asked of the
//!              item cache; while any is missing NO header groups are built
//!              at all
//! inv-slot     a table by inventoryType, with 18 (bag) -> 0x80000,
//!              11 (finger) -> 0x400, 12 (trinket) -> 0x1000, and 0 left
//!              -> 0x800000
//! row order    by group position, a header before its group's
//!              recipes, hidden rows last; within a group by difficulty index
//!              ascending, then item level DESCENDING,
//!              then spell name, case-insensitively
//! group order  by item class id ascending, ties by the subclass's
//!              own ItemSubClass.dbc name, case-insensitively
//! recount      three masks — expanded, inv-slot filter, subclass filter —
//!              hide rows, and the visible count is what GetNumTradeSkills
//!              answers
//! ```
//!
//! One stated difference from that recount: the reference re-sorts hidden rows
//! to the tail of its array and this module leaves the array alone and filters
//! **on read**. The reason is a rule one layer up: `TradeSkillFrame_Update`
//! re-reads the whole list *inside the click handler* that collapsed a header,
//! so a collapse has to apply through a shared reference — the same forced
//! shape as [`crate::tables::trainer::Board::select`], and the masks are
//! atomics for the same stated non-reason (a `Resource` must be `Sync`; no
//! second thread ever touches them). What the panel observes is identical.
//!
//! ## The difficulty formula
//!
//! Against the character's raw skill *value* (the low
//! word of `PLAYER_SKILL_INFO`'s second field — not the rank with the permanent
//! bonus in it):
//!
//! ```text
//! value <  min_value            optimal   orange
//! value <  (min + max) / 2      medium    yellow
//! value <  max_value            easy      green
//! otherwise                     trivial   grey
//! ```
//!
//! with `min_value = max(max_value - 25, 0)` where the row states 0. The words
//! are a literal table in the client and the addon's own
//! `TradeSkillTypeColor` keys them; the craft window's table has
//! two more (`none` at 0, `used` at 5) that only Beast Training rows take.
//!
//! ## Which window a cast opens
//!
//! The opening spells (Alchemy 2259, Enchanting 7411, Beast Training 5149…)
//! all carry `Effect[0] = 47` (`SPELL_EFFECT_TRADE_SKILL`). What separates
//! them is **`EffectMiscValue[0]`**, pinned three ways rather than at one
//! branch: the professions read 0 there and Enchanting reads 3, which is the
//! index `GetCraftDisplaySkillLine` tests (`kind == 3`);
//! the craft button token is indexed by kind — `USE`, `TRAIN`,
//! `DISGUISE`, `ENSCRIBE` — and Beast Training's 1 lands on `TRAIN`; and the
//! client keeps four per-kind spell lists
//! where the professions keep per-skill-line ones. So 0 opens
//! the trade-skill window on the skill line, and 1..3 open the craft window
//! on the kind. The skill line itself is the `Effect = 118`
//! (`SPELL_EFFECT_SKILL`) slot's misc value — 171 for Alchemy, 333 for
//! Enchanting — and Beast Training has no such slot, which is why the craft
//! window only shows a rank bar for Enchanting.
//!
//! ## What is here and what is one layer up
//!
//! This module holds the **rule**: given a skill line, the known spells, the
//! character's skill value and the bag counts, what the rows are and in what
//! order and colour. It needs no session and no window, so `vale
//! tradeskill` checks the same copy the client runs. The *player's* windows —
//! which line is open, the repeat counter, the rebuild latches — are
//! `crates/client/src/game/character/tradeskill.rs`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::tables::dbc::Dbc;
use crate::tables::spellbook::{SpellInfo, Spells};

/// `SPELL_EFFECT_TRADE_SKILL` — the whole content of an opening spell.
pub const TRADE_SKILL_EFFECT: u32 = 47;
/// `SPELL_EFFECT_SKILL` — the slot whose misc value is the skill line.
pub const SKILL_EFFECT: u32 = 118;
/// `SPELL_EFFECT_CREATE_ITEM` — the slot whose item type is what a recipe
/// makes and whose rolled value is how many.
pub const CREATE_ITEM_EFFECT: u32 = 24;

/// The craft kinds, by `EffectMiscValue[0]`, and the `GlobalStrings.lua` key
/// each puts on the create button — the client's literal table. Kind 0 is
/// not a craft at all (it is the trade-skill window); it is in the table
/// because the table is.
pub const CRAFT_BUTTON_TOKENS: [&str; 4] = ["USE", "TRAIN", "DISGUISE", "ENSCRIBE"];

/// The craft kind whose window shows a rank bar — Enchanting, the client's own
/// `kind == 3` test. Kind 2 (`DISGUISE`) is unused in 5875's
/// data.
pub const ENCHANTING_KIND: u32 = 3;

/// **Beast Training's kind.** Its rows are the hunter's own spells whose
/// `castUI` is 1 — see [`crate::tables::spellbook::SpellInfo::craft_kind`] —
/// and its cost column is the pet's training points rather than a reagent.
/// See [`List::build_training`].
pub const TRAINING_KIND: u32 = 1;

/// Which window a spell opens, decided from its own row — see the module
/// comment for the three-way pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opens {
    /// `EffectMiscValue[0] == 0`: the trade-skill window, on this skill line.
    TradeSkill { skill: u32 },
    /// `EffectMiscValue[0] != 0`: the craft window. `skill` is 333 for
    /// Enchanting and 0 for Beast Training, whose opener has no skill slot.
    Craft { kind: u32, skill: u32 },
}

/// What `info` opens, or `None` for every spell that is not an opener.
///
/// **The line is the opener's own `SkillLineAbility` row**, with the `SKILL`
/// slot's misc value as the fallback. A reading corroborated from the data
/// rather than from the client: every opener in 5875 carries an
/// ability row, the two columns agree on every opener that has both (Alchemy
/// 171, Enchanting 333, …) — and Smelting (186), Poisons (40) and the three
/// specialty-smith openers have **no** `SKILL` effect at all, so a client
/// reading only the misc value would open those five on line 0, empty.
pub fn opens(info: &SpellInfo, tradeskills: &TradeSkills) -> Option<Opens> {
    if info.effects[0].kind != TRADE_SKILL_EFFECT {
        return None;
    }
    let kind = info.effects[0].misc_value.max(0) as u32;
    let skill = tradeskills
        .thresholds(info.id)
        .map(|row| row.skill)
        .filter(|skill| *skill != 0)
        .or_else(|| {
            info.effects
                .iter()
                .find(|effect| effect.kind == SKILL_EFFECT)
                .map(|effect| effect.misc_value.max(0) as u32)
        })
        .unwrap_or(0);
    Some(if kind == 0 {
        Opens::TradeSkill { skill }
    } else {
        Opens::Craft { kind, skill }
    })
}

/// The four words `GetTradeSkillInfo` answers, in the client's own index
/// order. The craft table shifts them one up
/// behind `none`, but the crafts this client lists only ever take these four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Difficulty {
    /// The craft table's index 0 — a Beast Training row the pet has not
    /// learned. Never produced for a recipe.
    None,
    Optimal,
    Medium,
    Easy,
    Trivial,
    /// The craft table's index 5 — a Beast Training row the pet already knows,
    /// or knows a later rank of. `Blizzard_CraftUI.lua` greys it and disables
    /// the train button on it.
    Used,
}

impl Difficulty {
    pub fn word(self) -> &'static str {
        match self {
            Difficulty::None => "none",
            Difficulty::Optimal => "optimal",
            Difficulty::Medium => "medium",
            Difficulty::Easy => "easy",
            Difficulty::Trivial => "trivial",
            Difficulty::Used => "used",
        }
    }
}

/// `SkillLineAbility.dbc`, the columns this window reads — vmangos'
/// `SkillLineAbilityEntry` names them, and the client's own build corroborates
/// the two thresholds: it loads `max_value` at `+0x28` (field 10) and
/// `min_value` at `+0x2c` (field 11) off the row it found.
mod ability_fields {
    pub const SKILL_ID: usize = 1;
    pub const SPELL_ID: usize = 2;
    /// `req_skill_value`, field 7 — the rank a trainer wants before teaching
    /// it. Carried for the CLI's census; the window itself never reads it.
    pub const REQ_SKILL_VALUE: usize = 7;
    /// `max_value` — the rank the recipe goes grey at.
    pub const MAX_VALUE: usize = 10;
    /// `min_value` — the rank it stops being orange at.
    pub const MIN_VALUE: usize = 11;
    /// `reqtrainpoints`, field 14 (`+0x38`) — what a pet spell costs in
    /// training points. `GetCraftInfo` reads it off the row
    /// found under the pet family's skill lines, and vmangos'
    /// `Pet::GetTPForSpell` charges from the same column.
    pub const REQ_TRAIN_POINTS: usize = 14;
    /// `forward_spellid`, field 8 (`+0x20`) — the next rank. The client
    /// walks it from a pet spell to decide whether a later rank is already
    /// known, which greys the earlier one.
    pub const FORWARD_SPELL: usize = 8;
}

/// `SPELL_EFFECT_LEARN_SPELL` — what a Beast Training row's effect is; its
/// trigger is the pet's own spell.
pub const LEARN_SPELL_EFFECT: u32 = 36;
/// `TARGET_PET` — the implicit target that says the learn effect is aimed at
/// the pet rather than the caster.
pub const TARGET_PET: u32 = 5;

/// One recipe's thresholds, as the difficulty formula wants them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Thresholds {
    /// The skill line the recipe belongs to.
    pub skill: u32,
    /// The trainer's requirement — field 7, unread by the window.
    pub req_rank: u32,
    /// The grey point — `max_value`.
    pub grey_at: u32,
    /// The orange point's end — `min_value`, **as the file states it**: the
    /// zero-means-`grey_at - 25` fixup is applied in [`Thresholds::difficulty`]
    /// rather than at parse, so the CLI can report how many rows state 0.
    pub yellow_at: u32,
    /// `reqtrainpoints` — a pet spell's training-point cost, 0 for every
    /// recipe. See [`ability_fields::REQ_TRAIN_POINTS`].
    pub train_points: u32,
    /// `forward_spellid` — the next rank, or 0. See
    /// [`ability_fields::FORWARD_SPELL`].
    pub forward: u32,
}

impl Thresholds {
    /// The colour at `value` — the client's formula, against the raw
    /// skill value. See the module comment.
    pub fn difficulty(&self, value: u32) -> Difficulty {
        let grey = self.grey_at as i64;
        let yellow = if self.yellow_at == 0 {
            (grey - 25).max(0)
        } else {
            self.yellow_at as i64
        };
        let green = (grey + yellow) / 2; // (min + max) >> 1, signed
        let value = i64::from(value);
        if value < yellow {
            Difficulty::Optimal
        } else if value < green {
            Difficulty::Medium
        } else if value < grey {
            Difficulty::Easy
        } else {
            Difficulty::Trivial
        }
    }
}

/// `SkillLineAbility.dbc` reduced to what the two windows need: every spell's
/// thresholds, by spell id.
///
/// **No race or class mask is applied**, deliberately: membership in a window
/// is "the character knows the spell", and the known set is already the
/// server's answer for this character. A recipe has one row; the multi-row
/// spells (`Shoot Bow` and its kind) are weapon abilities no profession lists.
#[derive(Debug, Clone, Default)]
pub struct TradeSkills {
    /// Every row of a spell, in file order. A recipe has one; a pet spell
    /// has one per family line it is taught under.
    by_spell: HashMap<u32, Vec<Thresholds>>,
}

impl TradeSkills {
    pub fn parse(ability_dbc: &[u8]) -> Option<TradeSkills> {
        let table = Dbc::parse(ability_dbc).ok()?;
        let mut by_spell: HashMap<u32, Vec<Thresholds>> = HashMap::new();
        for record in 0..table.record_count {
            let field = |f: usize| table.u32_at(record, f).unwrap_or(0);
            let spell = field(ability_fields::SPELL_ID);
            if spell == 0 {
                continue;
            }
            by_spell.entry(spell).or_default().push(Thresholds {
                skill: field(ability_fields::SKILL_ID),
                req_rank: field(ability_fields::REQ_SKILL_VALUE),
                grey_at: field(ability_fields::MAX_VALUE),
                yellow_at: field(ability_fields::MIN_VALUE),
                train_points: field(ability_fields::REQ_TRAIN_POINTS),
                forward: field(ability_fields::FORWARD_SPELL),
            });
        }
        Some(TradeSkills { by_spell })
    }

    /// The thresholds for one spell, or `None` for a spell no skill line owns.
    ///
    /// **The first row in file order**, matching the client's
    /// lookup, which walks its hash chain in file order.
    pub fn thresholds(&self, spell: u32) -> Option<&Thresholds> {
        self.by_spell.get(&spell)?.first()
    }

    /// **The spell's row under one of `lines`**, tried in order — the client's
    /// lookup for a pet spell, where `lines` are the pet family's two skill
    /// lines from `CreatureFamily.dbc`. `None` when no line carries the spell,
    /// which `GetCraftInfo` answers as a cost of 0.
    pub fn row_for_lines(&self, spell: u32, lines: &[u32]) -> Option<&Thresholds> {
        let rows = self.by_spell.get(&spell)?;
        lines
            .iter()
            .filter(|line| **line != 0)
            .find_map(|line| rows.iter().find(|row| row.skill == *line))
    }

    /// **Is a later rank of `spell` known?** — from the spell's
    /// row under `lines`, follow `forward_spellid` and ask `knows` at each
    /// step, stopping at the first known rank or at a chain's end. A spell
    /// with no row, or whose row states no forward spell, answers `false`.
    pub fn later_rank_known(
        &self,
        spell: u32,
        lines: &[u32],
        knows: &dyn Fn(u32) -> bool,
    ) -> bool {
        let mut current = spell;
        // Bounded: a chain in the file cannot be longer than the table.
        for _ in 0..64 {
            let Some(row) = self.row_for_lines(current, lines) else {
                return false;
            };
            if row.forward == 0 {
                return false;
            }
            if knows(row.forward) {
                return true;
            }
            current = row.forward;
        }
        false
    }

    /// Every spell of one skill line that is in `known`, unordered — the
    /// build's intake.
    pub fn known_of_line(&self, skill: u32, known: &[u32]) -> Vec<u32> {
        known
            .iter()
            .copied()
            .filter(|spell| {
                self.by_spell
                    .get(spell)
                    .is_some_and(|rows| rows.iter().any(|row| row.skill == skill))
            })
            .collect()
    }

    /// …and every spell of the line at all, known or not — the CLI's census.
    pub fn all_of_line(&self, skill: u32) -> Vec<u32> {
        let mut spells: Vec<u32> = self
            .by_spell
            .iter()
            .filter(|(_, rows)| rows.iter().any(|row| row.skill == skill))
            .map(|(spell, _)| *spell)
            .collect();
        spells.sort_unstable();
        spells
    }

    /// How many spells have a row, for the census.
    pub fn len(&self) -> usize {
        self.by_spell.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_spell.is_empty()
    }
}

/// What the item cache knows about a created item — the four fields the build
/// reads off the cached record (`+0` class, `+4` subclass, `+0x2c`
/// inventoryType, `+0x38` itemLevel).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemHead {
    pub class: u32,
    pub subclass: u32,
    pub inventory_type: u32,
    pub item_level: u32,
}

/// The inv-slot mask for one inventory type — the client's table with the
/// build's own three overrides and its "other" fallback.
pub fn inv_slot_mask(inventory_type: u32) -> u32 {
    let mask = match inventory_type {
        18 => 0x80000,          // a bag
        11 => 0x400,            // finger
        12 => 0x1000,           // trinket
        1..=28 => 1u32 << inventory_type,
        _ => 0,
    };
    if mask == 0 { 0x800000 } else { mask }
}

/// **The trigger spell a Beast Training row teaches** — the first learn
/// effect's `EffectTriggerSpell`, which is the pet's own spell, or `None` for a
/// row with no learn effect. `GetCraftInfo`'s cost and level walk the three
/// effects for the first that is `LEARN_SPELL` at the pet or
/// `LEARN_PET_SPELL` (57).
pub fn training_trigger(info: &SpellInfo) -> Option<u32> {
    info.effects
        .iter()
        .find(|effect| {
            effect.trigger_spell != 0
                && ((effect.kind == LEARN_SPELL_EFFECT && effect.target_a == TARGET_PET)
                    || effect.kind == LEARN_PET_SPELL_EFFECT)
        })
        .map(|effect| effect.trigger_spell)
}

/// `SPELL_EFFECT_LEARN_PET_SPELL` — the other learn effect `GetCraftInfo`
/// accepts, which no 5875 hunter spell carries.
pub const LEARN_PET_SPELL_EFFECT: u32 = 57;

/// **`trainingPointCost`** — `reqtrainpoints` of the trigger spell's row under
/// the pet family's skill lines (`[row + 0x38]`), or 0 for no pet,
/// no trigger or no row. The raw column: the reference shows it, and the
/// server charges the difference from the rank already known
/// (`Pet::GetTPForSpell`), which is not this number.
pub fn training_cost(
    info: &SpellInfo,
    tradeskills: &TradeSkills,
    pet_lines: &[u32],
) -> u32 {
    training_trigger(info)
        .and_then(|trigger| tradeskills.row_for_lines(trigger, pet_lines))
        .map_or(0, |row| row.train_points)
}

/// **`requiredLevel`** — the trigger spell's `spellLevel`, raised to the
/// character's `SkillRaceClassInfo` `minLevel` for the row's skill line when
/// such a row exists. `race_class_level` answers that lookup, or
/// `None`. Zero for no trigger or no row, which the panel takes as no
/// requirement line.
pub fn training_level(
    info: &SpellInfo,
    tradeskills: &TradeSkills,
    catalog: &Spells,
    pet_lines: &[u32],
    race_class_level: &dyn Fn(u32) -> Option<u32>,
) -> u32 {
    let Some(trigger) = training_trigger(info) else {
        return 0;
    };
    let Some(row) = tradeskills.row_for_lines(trigger, pet_lines) else {
        return 0;
    };
    let spell_level = catalog.info(trigger).map_or(0, |t| t.spell_level);
    match race_class_level(row.skill) {
        Some(min_level) => spell_level.max(min_level),
        None => spell_level,
    }
}

/// One row of the window — a header or a recipe, exactly the fields
/// `GetTradeSkillInfo` hands back plus the two the sort used.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// 0 for a header — the client stores -1; what matters is the test.
    pub spell: u32,
    /// The row's own text: a recipe's spell name (field 120 alone — the
    /// reference does not append the rank), a header's subclass
    /// name out of `ItemSubClass.dbc`.
    pub name: String,
    pub difficulty: Difficulty,
    /// min over the reagents of have/need, clamped to zero.
    pub num_available: u32,
    /// The reagents themselves, `(entry, count)` — carried on the row so a
    /// reader can re-take the bag counts without a `Spell.dbc` lookup per
    /// frame; the rebuild latch one crate up keys on exactly this.
    pub reagents: Vec<(u32, u32)>,
    /// The group's position in [`List::subclasses`]' order, `None` for an
    /// ungrouped row — an enchant, or any recipe while a created item's
    /// template is still a round trip away.
    pub group: Option<usize>,
    /// The created item's entry — 0 for an enchant, which makes nothing.
    pub creates: u32,
    /// …and its level and inv-slot mask, for the sort and the filter.
    pub item_level: u32,
    pub inv_slot_mask: u32,
}

impl Row {
    pub fn is_header(&self) -> bool {
        self.spell == 0
    }
}

/// **The built window**: the rows in the client's own order, the masks, and
/// the selection.
///
/// The masks and the selection are atomics because
/// `TradeSkillFrame_SetSelection` collapses a header and the same click's
/// `TradeSkillFrame_Update` re-reads the whole list — a shared-reference
/// write, on [`crate::tables::trainer::Board::select`]'s own terms. No thread
/// is involved; a `Resource` merely has to be `Sync`.
#[derive(Debug, Default)]
pub struct List {
    /// Every row, headers included, in the sorted order above. Visibility is
    /// computed on read from the masks — see the module comment for why this
    /// deliberately differs from the reference's own array shuffle.
    rows: Vec<Row>,
    /// The groups in sorted order: class, subclass, the header's name.
    groups: Vec<(u32, u32, String)>,
    /// Expanded bits by group position — set = expanded.
    expanded: AtomicU32,
    /// Subclass filter bits by group position — set = shown.
    subclass_filter: AtomicU32,
    /// The selected recipe's **spell id**, 0 for none — the client keeps the
    /// id rather than the index, which is what lets a selection survive a
    /// collapse that moves every position under it.
    selection: AtomicU32,
}

impl Clone for List {
    fn clone(&self) -> List {
        List {
            rows: self.rows.clone(),
            groups: self.groups.clone(),
            expanded: AtomicU32::new(self.expanded.load(Ordering::Relaxed)),
            subclass_filter: AtomicU32::new(self.subclass_filter.load(Ordering::Relaxed)),
            selection: AtomicU32::new(self.selection.load(Ordering::Relaxed)),
        }
    }
}

impl List {
    /// **The build** — for one skill line.
    ///
    /// `counts` answers how many of an item entry the character carries;
    /// `head` answers the cache's four fields for a created item, `None` while
    /// its template has not arrived; `subclass_name` is `ItemSubClass.dbc`.
    pub fn build(
        skill: u32,
        known: &[u32],
        skill_value: u32,
        tradeskills: &TradeSkills,
        catalog: &Spells,
        counts: &dyn Fn(u32) -> u32,
        head: &dyn Fn(u32) -> Option<ItemHead>,
        subclass_name: &dyn Fn(u32, u32) -> Option<String>,
    ) -> List {
        struct Built {
            row: Row,
            key: (u32, u32),
        }
        let mut built: Vec<Built> = Vec::new();
        let mut all_heads_known = true;
        for spell in tradeskills.known_of_line(skill, known) {
            let Some(info) = catalog.info(spell) else {
                continue;
            };
            // **The openers are on their own lines too** — Alchemy's four
            // rank spells all carry `SkillLineAbility` rows for line 171 —
            // and the window does not list them: a row whose first effect is
            // the window itself is the door, not a recipe.
            if info.effects[0].kind == TRADE_SKILL_EFFECT {
                continue;
            }
            let thresholds = tradeskills.thresholds(spell).copied().unwrap_or_default();
            // min over the reagents of have/need — a recipe with no reagents
            // clamps to 0 on the same `max(0)` the client's -1 does.
            let num_available = info
                .reagents
                .iter()
                .map(|(entry, need)| counts(*entry) / (*need).max(1))
                .min()
                .unwrap_or(0);
            let creates = info
                .effects
                .iter()
                .find(|effect| effect.kind == CREATE_ITEM_EFFECT)
                .map_or(0, |effect| effect.item_type);
            let cached = if creates == 0 { None } else { head(creates) };
            if creates != 0 && cached.is_none() {
                all_heads_known = false;
            }
            let cached = cached.unwrap_or_default();
            built.push(Built {
                key: (cached.class, cached.subclass),
                row: Row {
                    spell,
                    reagents: info.reagents.clone(),
                    name: info.name,
                    difficulty: thresholds.difficulty(skill_value),
                    num_available,
                    group: None,
                    creates,
                    item_level: cached.item_level,
                    inv_slot_mask: inv_slot_mask(cached.inventory_type),
                },
            });
        }
        // **No headers until every created item is cached** — the client's
        // own gate, and the honest state for a cold cache: the recipes
        // list ungrouped, and the headers arrive with `TRADE_SKILL_UPDATE`
        // when the last template does.
        let mut groups: Vec<(u32, u32, String)> = Vec::new();
        if all_heads_known {
            for entry in built.iter().filter(|entry| entry.row.creates != 0) {
                let (class, subclass) = entry.key;
                if !groups.iter().any(|(c, s, _)| (*c, *s) == (class, subclass)) {
                    let name = subclass_name(class, subclass).unwrap_or_default();
                    groups.push((class, subclass, name));
                }
            }
            // Group order: class id ascending, ties by subclass
            // name, case-insensitively.
            groups.sort_by(|a, b| {
                a.0.cmp(&b.0)
                    .then_with(|| a.2.to_ascii_lowercase().cmp(&b.2.to_ascii_lowercase()))
            });
        }
        let group_of = |key: (u32, u32), creates: u32| -> Option<usize> {
            if creates == 0 {
                return None;
            }
            groups.iter().position(|(c, s, _)| (*c, *s) == key)
        };
        let mut rows: Vec<Row> = built
            .iter()
            .map(|entry| Row {
                group: group_of(entry.key, entry.row.creates),
                ..entry.row.clone()
            })
            .collect();
        for (position, (_, _, name)) in groups.iter().enumerate() {
            rows.push(Row {
                spell: 0,
                reagents: Vec::new(),
                name: name.clone(),
                difficulty: Difficulty::Optimal,
                num_available: 0,
                group: Some(position),
                creates: 0,
                item_level: 0,
                inv_slot_mask: 0,
            });
        }
        // The one sort (the client's outcome, minus the hidden-rows-last shuffle
        // the module comment states): group position, a header before its
        // group, then difficulty ascending, item level descending, name.
        rows.sort_by(|a, b| {
            use std::cmp::Ordering as O;
            a.group
                .unwrap_or(0)
                .cmp(&b.group.unwrap_or(0))
                .then_with(|| match (a.is_header(), b.is_header()) {
                    (true, false) => O::Less,
                    (false, true) => O::Greater,
                    _ => O::Equal,
                })
                .then_with(|| a.difficulty.cmp(&b.difficulty))
                .then_with(|| b.item_level.cmp(&a.item_level))
                .then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
        });
        List {
            rows,
            groups,
            expanded: AtomicU32::new(u32::MAX),
            subclass_filter: AtomicU32::new(u32::MAX),
            selection: AtomicU32::new(0),
        }
    }

    /// **The Beast Training list** — the craft build over the
    /// kind-1 list, which is every spell the character knows whose `castUI`
    /// is 1.
    ///
    /// No headers, no reagents and no item: each row is one of the hunter's
    /// training spells, a `LEARN_SPELL` (36) aimed at the pet whose trigger is
    /// the pet's own spell. The row's word is `used` or `none`:
    ///
    /// * for each effect with `Effect[i] == 36`: if `ImplicitTargetA[i]` or
    ///   `ImplicitTargetB[i]` is 5 (`TARGET_PET`), the row is `used` when the
    ///   pet knows `EffectTriggerSpell[i]` or a later rank of it
    ///   (along `SkillLineAbility`'s
    ///   `forward_spellid` chain); otherwise it is `used` when the *player*
    ///   knows the trigger;
    /// * a row with no such effect, or whose trigger nobody knows, is `none`.
    ///
    /// `pet_knows` answers over the pet's own spell list from
    /// `SMSG_PET_SPELLS`, `player_knows` over the known set, and
    /// `pet_lines` are the pet family's skill lines, which is where the
    /// forward chain is looked up. With no pet (`pet_lines` empty) the chain
    /// walk finds no row and only the direct test applies, which is the
    /// reference's own path: the client skips both pet tests when the pet
    /// object does not resolve.
    ///
    /// The training-point cost and the required level are not on the row:
    /// both are read against the pet at `GetCraftInfo` time,
    /// because the pet can change while the window is
    /// open. See [`training_cost`] and [`training_level`].
    ///
    /// **Order** is name, then rank, then id — a reading rather than a
    /// measurement: the craft build's own sort comparator was not traced, and
    /// this is the order every rank list in the interface takes.
    pub fn build_training(
        known: &[u32],
        catalog: &Spells,
        tradeskills: &TradeSkills,
        pet_lines: &[u32],
        pet_knows: &dyn Fn(u32) -> bool,
        player_knows: &dyn Fn(u32) -> bool,
    ) -> List {
        let mut rows: Vec<(Row, u32, u32)> = Vec::new();
        for &spell in known {
            let Some(info) = catalog.info(spell) else {
                continue;
            };
            if info.craft_kind() != Some(TRAINING_KIND) {
                continue;
            }
            let used = info.effects.iter().any(|effect| {
                effect.kind == LEARN_SPELL_EFFECT
                    && effect.trigger_spell != 0
                    && if effect.target_a == TARGET_PET || effect.target_b == TARGET_PET {
                        pet_knows(effect.trigger_spell)
                            || tradeskills.later_rank_known(
                                effect.trigger_spell,
                                pet_lines,
                                pet_knows,
                            )
                    } else {
                        player_knows(effect.trigger_spell)
                    }
            });
            rows.push((
                Row {
                    spell,
                    name: info.name.clone(),
                    difficulty: if used { Difficulty::Used } else { Difficulty::None },
                    num_available: 0,
                    reagents: Vec::new(),
                    group: None,
                    creates: 0,
                    item_level: 0,
                    inv_slot_mask: 0,
                },
                info.rank_order(),
                spell,
            ));
        }
        rows.sort_by(|a, b| {
            a.0.name
                .to_ascii_lowercase()
                .cmp(&b.0.name.to_ascii_lowercase())
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        List {
            rows: rows.into_iter().map(|(row, _, _)| row).collect(),
            groups: Vec::new(),
            expanded: AtomicU32::new(u32::MAX),
            subclass_filter: AtomicU32::new(u32::MAX),
            selection: AtomicU32::new(0),
        }
    }

    /// Whether one row is visible under the current masks.
    fn shown(&self, row: &Row) -> bool {
        let Some(group) = row.group else {
            return !row.is_header();
        };
        let bit = 1u32 << (group.min(31));
        let filtered = self.subclass_filter.load(Ordering::Relaxed) & bit != 0;
        if row.is_header() {
            filtered
        } else {
            filtered && self.expanded.load(Ordering::Relaxed) & bit != 0
        }
    }

    /// The visible rows, in draw order.
    fn visible(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|row| self.shown(row))
    }

    /// `GetNumTradeSkills` — the visible count.
    pub fn len(&self) -> usize {
        self.visible().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// …and every row there is, for the CLI.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// One visible row, one-based, as the panel asks.
    pub fn row(&self, index: usize) -> Option<&Row> {
        if index == 0 {
            return None;
        }
        self.visible().nth(index - 1)
    }

    /// `GetFirstTradeSkill` — the first visible recipe, one-based, 0 for none
    /// (the client walks for the first row whose spell is positive).
    pub fn first(&self) -> usize {
        self.visible()
            .position(|row| !row.is_header())
            .map_or(0, |i| i + 1)
    }

    /// `GetTradeSkillSelectionIndex` — where the selected spell now sits,
    /// one-based, 0 for none. Re-found on every ask, which is what lets the
    /// selection survive a collapse (the client walks the same way).
    pub fn selection(&self) -> usize {
        let spell = self.selection.load(Ordering::Relaxed);
        if spell == 0 {
            return 0;
        }
        self.visible()
            .position(|row| row.spell == spell)
            .map_or(0, |i| i + 1)
    }

    /// `SelectTradeSkill(i)` — remember the row's spell (the client stores the
    /// id, not the index). A header or an out-of-range index clears it.
    pub fn select(&self, index: usize) {
        let spell = self
            .row(index)
            .filter(|row| !row.is_header())
            .map_or(0, |row| row.spell);
        self.selection.store(spell, Ordering::Relaxed);
    }

    /// The selected recipe's spell id, 0 for none.
    pub fn selected_spell(&self) -> u32 {
        self.selection.load(Ordering::Relaxed)
    }

    /// …and re-select a remembered spell outright, which is what a rebuild
    /// does to keep the highlight where it was —
    /// [`crate::tables::trainer::Board::select_spell`]'s own shape.
    pub fn select_spell(&self, spell: u32) {
        self.selection.store(spell, Ordering::Relaxed);
    }

    /// Expand or collapse one header (by its one-based visible index) or, with
    /// index 0, all of them — the argument `TradeSkillCollapseAllButton`
    /// passes.
    pub fn set_expanded(&self, index: usize, expanded: bool) {
        let mask = if index == 0 {
            u32::MAX
        } else {
            let Some(group) = self
                .row(index)
                .filter(|row| row.is_header())
                .and_then(|row| row.group)
            else {
                return;
            };
            1u32 << group.min(31)
        };
        let now = self.expanded.load(Ordering::Relaxed);
        self.expanded.store(
            if expanded { now | mask } else { now & !mask },
            Ordering::Relaxed,
        );
    }

    /// Whether one visible header is expanded.
    pub fn expanded(&self, index: usize) -> bool {
        self.row(index)
            .filter(|row| row.is_header())
            .and_then(|row| row.group)
            .is_none_or(|group| {
                self.expanded.load(Ordering::Relaxed) & (1 << group.min(31)) != 0
            })
    }

    /// The subclass dropdown's entries, in group order.
    pub fn subclasses(&self) -> Vec<String> {
        self.groups.iter().map(|(_, _, name)| name.clone()).collect()
    }

    /// `GetTradeSkillSubClassFilter(i)` — 0 asks "all shown?".
    pub fn subclass_filter(&self, index: usize) -> bool {
        let mask = self.subclass_filter.load(Ordering::Relaxed);
        if index == 0 {
            return (0..self.groups.len()).all(|g| mask & (1 << g.min(31)) != 0);
        }
        index <= self.groups.len() && mask & (1 << (index - 1).min(31)) != 0
    }

    /// `SetTradeSkillSubClassFilter(i, on, exclusive)` — 0 means all.
    pub fn set_subclass_filter(&self, index: usize, on: bool, exclusive: bool) {
        let mask = if index == 0 {
            u32::MAX
        } else if index <= self.groups.len() {
            1u32 << (index - 1).min(31)
        } else {
            return;
        };
        let now = if exclusive {
            0
        } else {
            self.subclass_filter.load(Ordering::Relaxed)
        };
        self.subclass_filter.store(
            if on { now | mask } else { now & !mask },
            Ordering::Relaxed,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_difficulty_formula_matches_the_client() {
        // Enchant Bracer - Minor Health's own row: grey 80, min 0 -> yellow 55.
        let row = Thresholds { skill: 333, req_rank: 1, grey_at: 80, yellow_at: 0, ..Default::default() };
        assert_eq!(row.difficulty(0), Difficulty::Optimal);
        assert_eq!(row.difficulty(54), Difficulty::Optimal);
        assert_eq!(row.difficulty(55), Difficulty::Medium, "yellow at max-25 when min is 0");
        assert_eq!(row.difficulty(66), Difficulty::Medium);
        assert_eq!(row.difficulty(67), Difficulty::Easy, "green at (55+80)/2");
        assert_eq!(row.difficulty(79), Difficulty::Easy);
        assert_eq!(row.difficulty(80), Difficulty::Trivial);
        // …and a stated pair: yellow 100, grey 150 -> green at 125.
        let row = Thresholds { skill: 171, req_rank: 0, grey_at: 150, yellow_at: 100, ..Default::default() };
        assert_eq!(row.difficulty(99), Difficulty::Optimal);
        assert_eq!(row.difficulty(100), Difficulty::Medium);
        assert_eq!(row.difficulty(124), Difficulty::Medium);
        assert_eq!(row.difficulty(125), Difficulty::Easy);
        assert_eq!(row.difficulty(150), Difficulty::Trivial);
    }

    /// A `Spell.dbc` of hunter training spells and the pet spells they teach,
    /// in the measured shape: the hunter's row is a `LEARN_SPELL` (36) at
    /// `TARGET_PET` (5) with `castUI 1`, its trigger the pet's own passive.
    fn training_catalog() -> crate::tables::spellbook::Spells {
        use crate::tables::dbc::testing::dbc;
        use crate::tables::spellbook::{spell_fields, Spells};
        // (id, name offset, rank offset, castUI, effect, target A, trigger, spellLevel)
        let row = |id: u32, name: u32, rank: u32, cast_ui: u32, effect: u32, target: u32, trigger: u32, level: u32| {
            let mut row = vec![0u32; 173];
            row[0] = id;
            row[spell_fields::NAME] = name;
            row[spell_fields::RANK] = rank;
            row[spell_fields::CAST_UI] = cast_ui;
            row[spell_fields::EFFECT] = effect;
            row[spell_fields::IMPLICIT_TARGET_A] = target;
            row[spell_fields::EFFECT_TRIGGER_SPELL] = trigger;
            row[spell_fields::SPELL_LEVEL] = level;
            row
        };
        // Strings: \0 Great Stamina \0 Rank 1 \0 Rank 2 \0 Bite \0 Beast Training \0
        let strings = b"\0Great Stamina\0Rank 1\0Rank 2\0Bite\0Beast Training\0";
        Spells::parse(
            &dbc(
                &[
                    // The hunter's two ranks of Great Stamina, and Bite.
                    row(4195, 1, 15, 1, 36, 5, 4187, 0),
                    row(4196, 1, 22, 1, 36, 5, 4188, 0),
                    row(2969, 29, 15, 1, 36, 5, 17253, 0),
                    // The pet's own spells.
                    row(4187, 1, 15, 0, 6, 1, 0, 1),
                    row(4188, 1, 22, 0, 6, 1, 0, 12),
                    row(17253, 29, 15, 0, 6, 1, 0, 1),
                    // The opener, which is not a row.
                    row(5149, 34, 0, 0, 47, 0, 0, 0),
                ],
                173,
                strings,
            ),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .expect("a catalog")
    }

    /// A `SkillLineAbility.dbc` with the pet spells under two family lines,
    /// with costs and a forward chain: 4187 -> 4188 on line 270.
    fn training_abilities() -> TradeSkills {
        let row = |id: u32, skill: u32, spell: u32, forward: u32, cost: u32| {
            let mut record = vec![0u32; 15];
            record[0] = id;
            record[ability_fields::SKILL_ID] = skill;
            record[ability_fields::SPELL_ID] = spell;
            record[ability_fields::FORWARD_SPELL] = forward;
            record[ability_fields::REQ_TRAIN_POINTS] = cost;
            record
        };
        TradeSkills::parse(&crate::tables::dbc::testing::dbc(
            &[
                row(1, 270, 4187, 4188, 10),
                row(2, 270, 4188, 0, 15),
                row(3, 208, 17253, 0, 5),
                // …and a recipe on Enchanting's line, with no cost.
                row(4, 333, 7418, 0, 0),
            ],
            15,
            b"\0",
        ))
        .expect("parses")
    }

    /// **`row_for_lines` reads the row under the pet family's lines** and
    /// nothing else, and the forward chain greys an earlier rank.
    #[test]
    fn a_pet_spells_row_is_found_under_its_family_lines() {
        let abilities = training_abilities();
        assert_eq!(abilities.row_for_lines(4187, &[208, 270]).map(|r| r.train_points), Some(10));
        assert_eq!(abilities.row_for_lines(17253, &[208, 270]).map(|r| r.train_points), Some(5));
        assert_eq!(abilities.row_for_lines(4187, &[208, 0]), None, "not on the wolf line");
        assert_eq!(abilities.row_for_lines(4187, &[]), None, "no pet, no row");
        // A later rank known greys the earlier one; nothing known does not.
        let knows_rank_2 = |spell: u32| spell == 4188;
        assert!(abilities.later_rank_known(4187, &[270], &knows_rank_2));
        assert!(!abilities.later_rank_known(4188, &[270], &knows_rank_2), "the chain ends");
        assert!(!abilities.later_rank_known(4187, &[270], &|_| false));
        assert!(!abilities.later_rank_known(4187, &[208], &knows_rank_2), "no row, no chain");
    }

    /// **The Beast Training list**: the kind-1 spells in name and rank order,
    /// `used` where the pet knows the trigger or a later rank of it, and the
    /// two columns read against the pet.
    #[test]
    fn the_training_list_is_the_kind_one_spells_greyed_by_the_pets_book() {
        let catalog = training_catalog();
        let abilities = training_abilities();
        let known = [5149, 2969, 4196, 4195, 7418];
        let lines = [208, 270];

        // The pet knows Rank 2 of Great Stamina and nothing else.
        let pet_knows = |spell: u32| spell == 4188;
        let player_knows = |spell: u32| known.contains(&spell);
        let list = List::build_training(&known, &catalog, &abilities, &lines, &pet_knows, &player_knows);
        let rows: Vec<(u32, &str, Difficulty)> = list
            .rows()
            .iter()
            .map(|row| (row.spell, row.name.as_str(), row.difficulty))
            .collect();
        assert_eq!(
            rows,
            vec![
                (2969, "Bite", Difficulty::None),
                (4195, "Great Stamina", Difficulty::Used),
                (4196, "Great Stamina", Difficulty::Used),
            ],
            "the opener and the enchant are not rows; rank 1 is used through the chain"
        );
        assert_eq!(list.len(), 3);

        // The cost and the level, read against the pet.
        let rank_1 = catalog.info(4195).unwrap();
        let rank_2 = catalog.info(4196).unwrap();
        let bite = catalog.info(2969).unwrap();
        assert_eq!(training_trigger(&rank_1), Some(4187));
        assert_eq!(training_cost(&rank_1, &abilities, &lines), 10);
        assert_eq!(training_cost(&rank_2, &abilities, &lines), 15);
        assert_eq!(training_cost(&bite, &abilities, &lines), 5);
        assert_eq!(training_cost(&rank_1, &abilities, &[]), 0, "no pet reads as no cost");
        let no_row = |_line: u32| -> Option<u32> { None };
        assert_eq!(training_level(&rank_2, &abilities, &catalog, &lines, &no_row), 12);
        assert_eq!(training_level(&rank_1, &abilities, &catalog, &lines, &no_row), 1);
        // A race-class row raises the level and never lowers it.
        let raises = |line: u32| -> Option<u32> { (line == 270).then_some(20) };
        assert_eq!(training_level(&rank_2, &abilities, &catalog, &lines, &raises), 20);
        let lowers = |line: u32| -> Option<u32> { (line == 270).then_some(3) };
        assert_eq!(training_level(&rank_2, &abilities, &catalog, &lines, &lowers), 12);

        // With no pet nothing is used and the list is the same rows.
        let none = List::build_training(&known, &catalog, &abilities, &[], &|_| false, &player_knows);
        assert!(none.rows().iter().all(|row| row.difficulty == Difficulty::None));
        assert_eq!(none.len(), 3);
    }

    /// A built list keeps a last-row selection findable and its text intact —
    /// the shape of the first live session's covered-title report, pinned as
    /// far as this layer can pin it.
    #[test]
    fn a_selection_on_the_last_row_survives_the_walk() {
        use crate::tables::spellbook::Spells;
        // Four one-effect spells, no created items — a cold cache's flat list.
        let mut records: Vec<Vec<u8>> = Vec::new();
        let mut strings: Vec<u8> = vec![0];
        let names = ["Copper Bracers", "Rough Copper Vest", "Rough Sharpening Stone", "Rough Weightstone"];
        for (i, name) in names.iter().enumerate() {
            let mut record = vec![0u32; 173];
            record[0] = 100 + i as u32;
            record[120] = strings.len() as u32;
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
            records.push(record.iter().flat_map(|w| w.to_le_bytes()).collect());
        }
        let mut spell_dbc = b"WDBC".to_vec();
        spell_dbc.extend_from_slice(&(records.len() as u32).to_le_bytes());
        spell_dbc.extend_from_slice(&173u32.to_le_bytes());
        spell_dbc.extend_from_slice(&(173u32 * 4).to_le_bytes());
        spell_dbc.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for record in &records {
            spell_dbc.extend_from_slice(record);
        }
        spell_dbc.extend_from_slice(&strings);
        let catalog = Spells::parse(&spell_dbc, &[], &[], &[], &[], &[], &[]).expect("parses");

        let mut ability: Vec<Vec<u8>> = Vec::new();
        for i in 0..4u32 {
            let mut record = vec![0u32; 15];
            record[0] = 1 + i;
            record[1] = 164;
            record[2] = 100 + i;
            record[10] = 95; // grey
            ability.push(record.iter().flat_map(|w| w.to_le_bytes()).collect());
        }
        let mut ability_dbc = b"WDBC".to_vec();
        ability_dbc.extend_from_slice(&4u32.to_le_bytes());
        ability_dbc.extend_from_slice(&15u32.to_le_bytes());
        ability_dbc.extend_from_slice(&60u32.to_le_bytes());
        ability_dbc.extend_from_slice(&1u32.to_le_bytes());
        for record in &ability {
            ability_dbc.extend_from_slice(record);
        }
        ability_dbc.push(0);
        let tradeskills = TradeSkills::parse(&ability_dbc).expect("parses");

        let known = [100, 101, 102, 103];
        let list = List::build(
            164, &known, 1, &tradeskills, &catalog, &|_| 0, &|_| None, &|_, _| None,
        );
        assert_eq!(list.len(), 4, "four recipes, no headers on a cold cache");
        // Name order at equal difficulty and item level.
        let drawn: Vec<&str> = (1..=4).map(|i| list.row(i).unwrap().name.as_str()).collect();
        assert_eq!(drawn, names, "the sort is by name here");
        list.select(4);
        assert_eq!(list.selection(), 4, "the last row is selectable");
        let row = list.row(list.selection()).expect("the selected row exists");
        assert_eq!(row.name, "Rough Weightstone");
        assert!(!row.is_header());
    }

    #[test]
    fn the_inv_slot_mask_keeps_the_three_overrides() {
        assert_eq!(inv_slot_mask(18), 0x80000, "a bag");
        assert_eq!(inv_slot_mask(11), 0x400, "finger");
        assert_eq!(inv_slot_mask(12), 0x1000, "trinket");
        assert_eq!(inv_slot_mask(1), 2, "head is 1 << 1");
        assert_eq!(inv_slot_mask(0), 0x800000, "no slot at all is the other bucket");
    }
}
