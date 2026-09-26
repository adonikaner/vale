//! **The talent tree** — three tabs a class may spend in, and what is in each.
//!
//! `Talent.dbc` and `TalentTab.dbc`, plus the six rules `Blizzard_TalentUI` is
//! written against. None of it needs a session or a window: the tabs are decided
//! by a class mask, the tiers by a row index, and a rank by which of a talent's
//! rank *spells* the character knows — so all of it is here, checked by
//! `vale talent`, and the client crate holds only the plumbing.
//!
//! ## The two file layouts
//!
//! `TalentTab.dbc` — **27 records, 15 fields**, three per class:
//!
//! ```text
//! [0]      id                  261
//! [1..8]   name, per locale    "Elemental"    (see LOCALES; [9] is the mask)
//! [10]     spellIconID         1137  -> SpellIcon.dbc [1], a BLP path
//! [11]     raceMask            511
//! [12]     classMask           64    -> shaman
//! [13]     orderIndex          0
//! [14]     backgroundFile      "ShamanElementalCombat"
//! ```
//!
//! `Talent.dbc` — **432 records, 21 fields**, and the field list is vmangos'
//! `TalentEntry` (`Database/DBCStructure.h:660`) confirmed against the file:
//!
//! ```text
//! [0]      id
//! [1]      talentTab           -> TalentTab.dbc
//! [2]      row                 0..6, the tier, zero-based
//! [3]      column              0..3, zero-based
//! [4..12]  rankSpell[9]        the spell each rank teaches; 5 is the most any
//!                              1.12 talent uses and slots 5..8 are all zero
//! [13..15] prereqTalent[3]     -> Talent.dbc; no 1.12 talent uses more than one
//! [16..18] prereqRank[3]       …and which rank of it, **zero-based**
//! [19]     flags               bit 0 -> isExceptional, the gold border
//! [20]     requiredSpell       one row in the whole file uses it (339)
//! ```
//!
//! ## Four rules that are the client's own, and none of them is in either file
//!
//! All four follow the client rather than being inferred, and each is one this
//! project could plausibly have got wrong:
//!
//! * **The tabs are DBC *record* order, not `orderIndex`.** The builder
//!   walks `TalentTab.dbc` from record 0 and appends every row whose
//!   race and class masks admit the character (the same test as
//!   [`crate::tables::skills`]' own `covers`). `orderIndex` is
//!   never read. That is not academic: **the mage's Arcane and Fire tabs both
//!   carry `orderIndex` 0**, so a sort on that column has a tie and one of the
//!   two orders it can pick is wrong.
//! * **A tab's talents are a contiguous slice of `Talent.dbc`, in record
//!   order.** The client walks the talent store once and starts a new group each
//!   time the tab id *changes* — so the index `GetTalentInfo(tab, i)` takes is
//!   the file's own order within that tab, not a sort by tier and column. The
//!   file really is grouped that way: 432 records in 27 runs, one per tab.
//! * **A talent's rank is which rank spell the character knows**, found by
//!   walking the nine slots **downwards** from 8 and stopping at
//!   the first known one. Nothing on the wire says "you have 3 points in
//!   Impale"; the server sends spell 12286 and the rank is read back out of it.
//! * **`maxRank` is the highest *populated* slot**, not the count of them. The
//!   same downward walk sets it on the first non-zero slot it sees, before the
//!   known-spell test. Identical for every 1.12 talent, since none of them has a
//!   hole, and stated because the two readings diverge the day one does.
//!
//! ## What a click sends, and what it does *not* check
//!
//! `LearnTalent(tab, index)` builds `CMSG_LEARN_TALENT` with the
//! talent's id and **the next rank index**, zero-based — so a talent with no
//! points sends rank 0. Its one refusal is `rank >= maxRank`: a maxed talent
//! sends nothing.
//!
//! It checks **nothing else**. Not the talent points, not the prereq, not the
//! five-per-tier gate — those are all `Player::LearnTalent`'s
//! (`Objects/Player.cpp:20786`), and the client simply asks. So
//! [`TalentTree::learn`] has exactly one gate too, and a refusal that looks like
//! silence is the reference's own silence.
//!
//! ## …and what the *panel* greys out, which is a different question
//!
//! `meetsPrereq` — `GetTalentInfo`'s eighth return — is only about field 20, the
//! required *spell*, and answers true whenever that field is zero. Everything
//! else the panel desaturates for is computed in Lua from what
//! [`TalentTree::build`] hands it: `TalentFrame_Update` does the five-points-per
//! tier arithmetic itself, and `GetTalentPrereqs`' `isLearnable` flags are the
//! per-prereq half. Those are the addon's, not this file's — see
//! `Blizzard_TalentUI.lua`.

use std::collections::HashSet;

use crate::tables::dbc::Dbc;

/// How many rank slots a `Talent.dbc` row carries. **Nine, and five are used** —
/// the client's own walk runs from slot 8 down to 0, and vmangos'
/// `MAX_TALENT_RANK` is 5 because that is all any 1.12 row populates. Reading
/// all nine costs nothing and cannot be wrong; reading five would silently drop
/// a rank if a patched archive ever had one.
pub const RANKS: usize = 9;

/// How many prerequisites a row may name. Three in the file; **no 1.12 talent
/// uses more than one**, and `GetTalentPrereqs` returns three values per
/// populated slot regardless.
pub const PREREQS: usize = 3;

/// The points a tier costs before the next one opens — `Row * 5` in
/// `Player::LearnTalent`, and `(tier - 1) * 5` in `TalentFrame_Update`. Both
/// sides of the same rule, which is why it is here and not in either.
pub const POINTS_PER_TIER: u32 = 5;

/// Where the pictures behind a tab live. `TalentFrame_Update` builds
/// `"Interface\\TalentFrame\\"..fileName.."-TopLeft"` and its three siblings.
pub const BACKGROUND_DIR: &str = r"Interface\TalentFrame\";

mod tab_fields {
    /// `[1..8]`, one per locale; the client indexes with its locale ordinal
    /// and this reads the first, as every other name column
    /// in this crate does.
    pub const NAME: usize = 1;
    pub const ICON: usize = 10;
    pub const RACE_MASK: usize = 11;
    pub const CLASS_MASK: usize = 12;
    pub const ORDER: usize = 13;
    pub const BACKGROUND: usize = 14;
}

mod talent_fields {
    pub const TAB: usize = 1;
    pub const ROW: usize = 2;
    pub const COLUMN: usize = 3;
    pub const RANK: usize = 4;
    pub const PREREQ_TALENT: usize = 13;
    pub const PREREQ_RANK: usize = 16;
    pub const FLAGS: usize = 19;
    pub const REQUIRED_SPELL: usize = 20;
}

/// `flags` bit 0 — the gold-bordered capstone.
const FLAG_EXCEPTIONAL: u32 = 0x1;

/// One row of `TalentTab.dbc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabRow {
    pub id: u32,
    pub name: String,
    /// `SpellIcon.dbc`'s id, resolved to a path by [`TalentTree::build`].
    pub icon: u32,
    pub race_mask: u32,
    pub class_mask: u32,
    /// **Read, reported and never sorted on** — see the module comment, where
    /// the mage's tie is.
    pub order: u32,
    /// The stem of the four background quarters, without [`BACKGROUND_DIR`].
    pub background: String,
}

/// One row of `Talent.dbc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalentRow {
    pub id: u32,
    pub tab: u32,
    /// Zero-based, as the file has it. The panel's `tier` is this plus one.
    pub row: u32,
    /// Zero-based likewise.
    pub column: u32,
    pub ranks: [u32; RANKS],
    pub prereq_talent: [u32; PREREQS],
    /// **Zero-based**, and that is load-bearing: `Player::LearnTalent` loops
    /// `for (i = DependsOnRank; i < MAX_TALENT_RANK; ++i)`, so a stored 4 means
    /// "five points in it".
    pub prereq_rank: [u32; PREREQS],
    pub flags: u32,
    pub required_spell: u32,
}

impl TalentRow {
    /// The highest populated rank slot, one-based — `GetTalentInfo`'s sixth
    /// return. Zero for a row with no rank spells at all, which no shipped row
    /// is.
    pub fn max_rank(&self) -> u32 {
        (0..RANKS)
            .rev()
            .find(|slot| self.ranks[*slot] != 0)
            .map_or(0, |slot| slot as u32 + 1)
    }

    /// **Which rank of this the character has**, one-based, 0 for none — the
    /// client's downward walk.
    pub fn rank(&self, known: &HashSet<u32>) -> u32 {
        (0..RANKS)
            .rev()
            .find(|slot| self.ranks[*slot] != 0 && known.contains(&self.ranks[*slot]))
            .map_or(0, |slot| slot as u32 + 1)
    }

    /// The spell whose name and icon the panel draws: **the rank the character
    /// has**, or rank 1 when they have none.
    ///
    /// Not cosmetic — a talent's ranks are separate `Spell.dbc` rows and the
    /// later ones carry the numbers the tooltip prints, so a panel that always
    /// showed rank 1 would quote the wrong figures at every rank above it.
    pub fn face_spell(&self, known: &HashSet<u32>) -> u32 {
        match self.rank(known) {
            0 => self.ranks[0],
            rank => self.ranks[rank as usize - 1],
        }
    }

    /// Bit 0 of the flags.
    pub fn exceptional(&self) -> bool {
        self.flags & FLAG_EXCEPTIONAL != 0
    }
}

/// Both files, parsed. **Both or neither** — a tree with tabs and no talents is
/// not a degradation, it is a panel of empty parchment.
#[derive(Debug, Clone, Default)]
pub struct Talents {
    /// In `TalentTab.dbc` record order — see the module comment.
    tabs: Vec<TabRow>,
    /// In `Talent.dbc` record order, which is grouped by tab.
    talents: Vec<TalentRow>,
}

impl Talents {
    /// Parse both, or `None` if either is missing or unreadable.
    ///
    /// `None` is what a client with no `Talent.dbc` should show: `GetNumTalentTabs`
    /// answers 0, `TalentFrame_Update` finds no tab name and falls back to the
    /// `MageFire` parchment with no buttons on it — which is the reference's own
    /// "temporary default for classes without talents poor guys" branch.
    pub fn parse(talent: &[u8], talent_tab: &[u8]) -> Option<Talents> {
        let tabs = Dbc::parse(talent_tab).ok()?;
        let talents = Dbc::parse(talent).ok()?;
        let tabs = (0..tabs.record_count)
            .filter_map(|r| {
                Some(TabRow {
                    id: tabs.u32_at(r, 0)?,
                    name: tabs.string_at(r, tab_fields::NAME).unwrap_or_default(),
                    icon: tabs.u32_at(r, tab_fields::ICON).unwrap_or(0),
                    race_mask: tabs.u32_at(r, tab_fields::RACE_MASK).unwrap_or(0),
                    class_mask: tabs.u32_at(r, tab_fields::CLASS_MASK).unwrap_or(0),
                    order: tabs.u32_at(r, tab_fields::ORDER).unwrap_or(0),
                    background: tabs
                        .string_at(r, tab_fields::BACKGROUND)
                        .unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        let talents = (0..talents.record_count)
            .filter_map(|r| {
                let quad = |base: usize, n: usize| {
                    let mut out = [0u32; RANKS];
                    for (slot, cell) in out.iter_mut().take(n).enumerate() {
                        *cell = talents.u32_at(r, base + slot).unwrap_or(0);
                    }
                    out
                };
                let prereq = |base: usize| {
                    let mut out = [0u32; PREREQS];
                    for (slot, cell) in out.iter_mut().enumerate() {
                        *cell = talents.u32_at(r, base + slot).unwrap_or(0);
                    }
                    out
                };
                Some(TalentRow {
                    id: talents.u32_at(r, 0)?,
                    tab: talents.u32_at(r, talent_fields::TAB)?,
                    row: talents.u32_at(r, talent_fields::ROW).unwrap_or(0),
                    column: talents.u32_at(r, talent_fields::COLUMN).unwrap_or(0),
                    ranks: quad(talent_fields::RANK, RANKS),
                    prereq_talent: prereq(talent_fields::PREREQ_TALENT),
                    prereq_rank: prereq(talent_fields::PREREQ_RANK),
                    flags: talents.u32_at(r, talent_fields::FLAGS).unwrap_or(0),
                    required_spell: talents
                        .u32_at(r, talent_fields::REQUIRED_SPELL)
                        .unwrap_or(0),
                })
            })
            .collect::<Vec<_>>();
        if tabs.is_empty() || talents.is_empty() {
            return None;
        }
        Some(Talents { tabs, talents })
    }

    /// Every tab, in file order.
    pub fn tabs(&self) -> &[TabRow] {
        &self.tabs
    }

    /// Every talent, in file order.
    pub fn talents(&self) -> &[TalentRow] {
        &self.talents
    }

    /// **The tabs this class may spend in**, in file order — the builder's own
    /// walk.
    ///
    /// `race` is passed because the mask column exists and the client tests it;
    /// every shipped row has `raceMask` 511, so it has never yet excluded
    /// anything.
    pub fn tabs_for(&self, race: u8, class: u8) -> Vec<&TabRow> {
        self.tabs
            .iter()
            .filter(|tab| covers(tab.race_mask, tab.class_mask, race, class))
            .collect()
    }

    /// …and the talents on one of them, in file order.
    pub fn talents_in(&self, tab: u32) -> impl Iterator<Item = &TalentRow> {
        self.talents.iter().filter(move |t| t.tab == tab)
    }

    /// One row by id, for a prereq to point at.
    pub fn talent(&self, id: u32) -> Option<&TalentRow> {
        self.talents.iter().find(|t| t.id == id)
    }
}

/// **A zero mask means everybody**, and the bit is `1 << (id - 1)`.
///
/// The same function [`crate::tables::skills`] has, because it is the same
/// test in the client, with a zero-mask short-circuit on each mask.
fn covers(race_mask: u32, class_mask: u32, race: u8, class: u8) -> bool {
    let bit = |id: u8| if id == 0 { 0 } else { 1u32 << (id - 1) };
    (race_mask == 0 || race_mask & bit(race) != 0)
        && (class_mask == 0 || class_mask & bit(class) != 0)
}

/// One prerequisite, as `GetTalentPrereqs` hands it over: a **one-based** cell
/// and whether the character has enough points in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prereq {
    pub tier: u32,
    pub column: u32,
    /// Whether any rank at or above `prereqRank` is known — the client's loop.
    /// The addon greys the arrow and everything downstream of it when false.
    pub learnable: bool,
}

/// One talent, in the shape `GetTalentInfo` answers in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Talent {
    pub id: u32,
    pub name: String,
    pub icon: String,
    /// **The spell the name and icon came from** — the held rank's, or rank 1's
    /// when there is none. See [`TalentRow::face_spell`]; the tooltip is built
    /// off it, so a talent at rank 3 quotes rank 3's numbers.
    pub face_spell: u32,
    /// **One-based**, as the panel wants it — `[ebx+8] + 1`.
    pub tier: u32,
    /// One-based likewise.
    pub column: u32,
    pub rank: u32,
    pub max_rank: u32,
    pub exceptional: bool,
    /// Field 20 only — see the module comment's last section.
    pub meets_prereq: bool,
    pub prereqs: Vec<Prereq>,
}

/// One tab, in the shape `GetTalentTabInfo` answers in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: u32,
    pub name: String,
    /// Already a BLP path, or empty when `SpellIcon.dbc` has no such row.
    pub icon: String,
    /// The stem, without [`BACKGROUND_DIR`] — `GetTalentTabInfo`'s fourth
    /// return, which the addon concatenates itself.
    pub background: String,
    pub points_spent: u32,
    pub talents: Vec<Talent>,
}

/// **What the panel draws**, built once per change rather than per read.
///
/// Building it means a rank walk over ~50 talents and a `Spell.dbc` lookup per
/// talent — nothing once, and unacceptable at the rate `TalentFrame_Update`
/// calls `GetTalentInfo` (twenty buttons × three tabs on every tab click). The
/// client crate latches it on the spellbook's own version counter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TalentTree {
    pub tabs: Vec<Tab>,
}

impl TalentTree {
    /// Build the tree for a character.
    ///
    /// `known` is every spell the server said this character has —
    /// `SMSG_INITIAL_SPELLS` and its deltas. `catalog` supplies the name and
    /// icon; without it every talent draws as an unnamed button, which is the
    /// same degradation the spellbook takes.
    pub fn build(
        talents: &Talents,
        race: u8,
        class: u8,
        known: &[u32],
        catalog: Option<&crate::tables::spellbook::Spells>,
    ) -> TalentTree {
        let known: HashSet<u32> = known.iter().copied().collect();
        let tabs = talents
            .tabs_for(race, class)
            .into_iter()
            .map(|tab| {
                let rows: Vec<&TalentRow> = talents.talents_in(tab.id).collect();
                let points_spent = rows.iter().map(|row| row.rank(&known)).sum();
                let talents_in_tab = rows
                    .iter()
                    .map(|row| {
                        let face = row.face_spell(&known);
                        let info = catalog.and_then(|c| c.info(face));
                        Talent {
                            id: row.id,
                            name: info.as_ref().map(|i| i.name.clone()).unwrap_or_default(),
                            icon: info.map(|i| i.icon).unwrap_or_default(),
                            face_spell: face,
                            tier: row.row + 1,
                            column: row.column + 1,
                            rank: row.rank(&known),
                            max_rank: row.max_rank(),
                            exceptional: row.exceptional(),
                            // **Field 20 alone**, and true when it is zero.
                            meets_prereq: row.required_spell == 0
                                || known.contains(&row.required_spell),
                            prereqs: prereqs_of(talents, row, &known),
                        }
                    })
                    .collect();
                Tab {
                    id: tab.id,
                    name: tab.name.clone(),
                    icon: catalog
                        .and_then(|c| c.icon_path(tab.icon))
                        .unwrap_or_default(),
                    background: tab.background.clone(),
                    points_spent,
                    talents: talents_in_tab,
                }
            })
            .collect();
        TalentTree { tabs }
    }

    /// `GetNumTalentTabs()`.
    pub fn num_tabs(&self) -> usize {
        self.tabs.len()
    }

    /// A tab by **one-based** index, the way every one of the six C functions
    /// takes it.
    pub fn tab(&self, index: usize) -> Option<&Tab> {
        self.tabs.get(index.checked_sub(1)?)
    }

    /// …and a talent inside one, both indices one-based.
    pub fn talent(&self, tab: usize, index: usize) -> Option<&Talent> {
        self.tab(tab)?.talents.get(index.checked_sub(1)?)
    }

    /// **What pressing a talent button sends**, or `None` for the one refusal
    /// the client makes itself.
    ///
    /// `(talentId, requestedRank)` with the rank **zero-based** — a talent with
    /// no points asks for rank 0. The gate is `rank >= maxRank` and there is no
    /// other: see the module comment, which is where the argument for not adding
    /// one is.
    pub fn learn(&self, tab: usize, index: usize) -> Option<(u32, u32)> {
        let talent = self.talent(tab, index)?;
        (talent.rank < talent.max_rank).then_some((talent.id, talent.rank))
    }

    /// Every talent point this character has spent, across all tabs — what
    /// `vale talent` reports and what a talent-wipe check would compare.
    pub fn points_spent(&self) -> u32 {
        self.tabs.iter().map(|tab| tab.points_spent).sum()
    }
}

/// The three prereq slots, resolved — `GetTalentPrereqs`' whole body.
fn prereqs_of(talents: &Talents, row: &TalentRow, known: &HashSet<u32>) -> Vec<Prereq> {
    (0..PREREQS)
        .filter(|slot| row.prereq_talent[*slot] != 0)
        .filter_map(|slot| {
            let dep = talents.talent(row.prereq_talent[slot])?;
            let need = row.prereq_rank[slot] as usize;
            Some(Prereq {
                tier: dep.row + 1,
                column: dep.column + 1,
                // **At or above**, which is why this is a scan and not a
                // comparison against `dep.rank(known)`: the two agree today
                // because only one rank of a talent is ever known at once, and
                // the client's own loop is the scan.
                learnable: (need..RANKS)
                    .any(|slot| dep.ranks[slot] != 0 && known.contains(&dep.ranks[slot])),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Talent.dbc`'s own shape, one row at a time.
    fn talent_dbc(rows: &[TalentRow]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&21u32.to_le_bytes());
        out.extend_from_slice(&(21u32 * 4).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        for row in rows {
            let mut fields = [0u32; 21];
            fields[0] = row.id;
            fields[1] = row.tab;
            fields[2] = row.row;
            fields[3] = row.column;
            fields[4..13].copy_from_slice(&row.ranks);
            fields[13..16].copy_from_slice(&row.prereq_talent);
            fields[16..19].copy_from_slice(&row.prereq_rank);
            fields[19] = row.flags;
            fields[20] = row.required_spell;
            for field in fields {
                out.extend_from_slice(&field.to_le_bytes());
            }
        }
        out.push(0);
        out
    }

    /// …and `TalentTab.dbc`'s, whose two string columns share one block.
    fn tab_dbc(rows: &[TabRow]) -> Vec<u8> {
        let mut strings = vec![0u8];
        let mut intern = |text: &str| {
            let at = strings.len() as u32;
            strings.extend_from_slice(text.as_bytes());
            strings.push(0);
            at
        };
        let mut records = Vec::new();
        for row in rows {
            let mut fields = [0u32; 15];
            fields[0] = row.id;
            fields[tab_fields::NAME] = intern(&row.name);
            fields[tab_fields::ICON] = row.icon;
            fields[tab_fields::RACE_MASK] = row.race_mask;
            fields[tab_fields::CLASS_MASK] = row.class_mask;
            fields[tab_fields::ORDER] = row.order;
            fields[tab_fields::BACKGROUND] = intern(&row.background);
            for field in fields {
                records.extend_from_slice(&field.to_le_bytes());
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&15u32.to_le_bytes());
        out.extend_from_slice(&(15u32 * 4).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        out.extend_from_slice(&records);
        out.extend_from_slice(&strings);
        out
    }

    fn row(id: u32, tab: u32, tier: u32, column: u32, ranks: &[u32]) -> TalentRow {
        let mut slots = [0u32; RANKS];
        slots[..ranks.len()].copy_from_slice(ranks);
        TalentRow {
            id,
            tab,
            row: tier,
            column,
            ranks: slots,
            prereq_talent: [0; PREREQS],
            prereq_rank: [0; PREREQS],
            flags: 0,
            required_spell: 0,
        }
    }

    fn tab(id: u32, name: &str, class_mask: u32, order: u32, background: &str) -> TabRow {
        TabRow {
            id,
            name: name.to_string(),
            icon: 0,
            race_mask: 0,
            class_mask,
            order,
            background: background.to_string(),
        }
    }

    /// The arms tree's first three rows, as the file has them.
    fn arms() -> (Vec<TabRow>, Vec<TalentRow>) {
        let tabs = vec![
            tab(161, "Arms", 1, 0, "WarriorArms"),
            tab(164, "Fury", 1, 1, "WarriorFury"),
            tab(81, "Arcane", 128, 0, "MageArcane"),
        ];
        let talents = vec![
            row(124, 161, 0, 0, &[12282, 12663, 12664]),
            row(130, 161, 0, 1, &[16462, 16463, 16464, 16465, 16466]),
            row(641, 164, 1, 1, &[12295, 12676, 12677, 12678, 12679]),
        ];
        (tabs, talents)
    }

    fn parsed() -> Talents {
        let (tabs, talents) = arms();
        Talents::parse(&talent_dbc(&talents), &tab_dbc(&tabs)).expect("both parse")
    }

    #[test]
    fn a_row_reads_its_tier_column_and_ranks() {
        let talents = parsed();
        let impale = talents.talent(124).expect("row 124");
        assert_eq!((impale.tab, impale.row, impale.column), (161, 0, 0));
        assert_eq!(impale.max_rank(), 3);
        assert_eq!(impale.ranks[0], 12282);
    }

    /// **The rank is read back out of the known spells**, and the face spell
    /// follows it — see the module comment's third rule.
    #[test]
    fn the_rank_is_which_rank_spell_is_known() {
        let talents = parsed();
        let impale = talents.talent(124).expect("row 124");
        let none: HashSet<u32> = HashSet::new();
        assert_eq!(impale.rank(&none), 0);
        assert_eq!(impale.face_spell(&none), 12282, "rank 1's spell when unspent");

        let two: HashSet<u32> = [12663].into_iter().collect();
        assert_eq!(impale.rank(&two), 2);
        assert_eq!(impale.face_spell(&two), 12663, "…and the held rank's above it");
    }

    /// A class sees three tabs and never anybody else's.
    #[test]
    fn a_class_sees_only_its_own_tabs() {
        let talents = parsed();
        let warrior = talents.tabs_for(1, 1);
        assert_eq!(
            warrior.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![161, 164]
        );
        let mage = talents.tabs_for(1, 8);
        assert_eq!(mage.iter().map(|t| t.id).collect::<Vec<_>>(), vec![81]);
    }

    /// **File order, not `orderIndex`** — the rule that the mage's tie is about.
    /// Fury is `orderIndex` 1 and sits *after* Arms in the file, so a sort would
    /// agree here; what this pins is that the tie-break is position.
    #[test]
    fn tabs_come_out_in_file_order() {
        let (mut tabs, talents) = arms();
        // Give both warrior tabs the same order index, which is exactly the
        // shape the mage's Arcane and Fire have in the shipped file.
        tabs[0].order = 0;
        tabs[1].order = 0;
        let talents = Talents::parse(&talent_dbc(&talents), &tab_dbc(&tabs)).expect("parses");
        let tree = TalentTree::build(&talents, 1, 1, &[], None);
        assert_eq!(
            tree.tabs.iter().map(|t| t.id).collect::<Vec<_>>(),
            vec![161, 164],
            "the file's order survives a tie no column can break"
        );
    }

    /// Points spent is the sum of the ranks, per tab and over the tree.
    #[test]
    fn points_spent_is_the_sum_of_the_ranks() {
        let talents = parsed();
        let tree = TalentTree::build(&talents, 1, 1, &[12663, 16464], None);
        assert_eq!(tree.tab(1).expect("arms").points_spent, 2 + 3);
        assert_eq!(tree.tab(2).expect("fury").points_spent, 0);
        assert_eq!(tree.points_spent(), 5);
    }

    /// **The click's whole gate**, and the rank it asks for.
    #[test]
    fn learning_asks_for_the_next_rank_and_stops_at_the_top() {
        let talents = parsed();
        let fresh = TalentTree::build(&talents, 1, 1, &[], None);
        assert_eq!(fresh.learn(1, 1), Some((124, 0)), "no points asks for rank 0");

        let two = TalentTree::build(&talents, 1, 1, &[12663], None);
        assert_eq!(two.learn(1, 1), Some((124, 2)), "two points asks for slot 2");

        let maxed = TalentTree::build(&talents, 1, 1, &[12664], None);
        assert_eq!(maxed.learn(1, 1), None, "a maxed talent sends nothing");
        assert_eq!(maxed.learn(1, 99), None, "and so does an index off the end");
    }

    /// A prereq resolves to the **cell** of the talent it names, and its
    /// learnability is at-or-above the stored rank.
    #[test]
    fn a_prereq_names_a_cell_and_a_rank_floor() {
        let (tabs, mut talents) = arms();
        // Deep Wounds' own shape: it hangs off talent 641 at its fifth rank.
        let mut deep = row(137, 164, 2, 1, &[12296]);
        deep.prereq_talent[0] = 641;
        deep.prereq_rank[0] = 4;
        deep.flags = FLAG_EXCEPTIONAL;
        talents.push(deep);
        let talents = Talents::parse(&talent_dbc(&talents), &tab_dbc(&tabs)).expect("parses");

        let four = TalentTree::build(&talents, 1, 1, &[12678], None);
        let deep = four.talent(2, 2).expect("deep wounds");
        assert!(deep.exceptional, "flags bit 0 is the gold border");
        assert_eq!(deep.prereqs.len(), 1);
        assert_eq!((deep.prereqs[0].tier, deep.prereqs[0].column), (2, 2));
        assert!(
            !deep.prereqs[0].learnable,
            "four points is below the stored floor of rank 5"
        );

        let five = TalentTree::build(&talents, 1, 1, &[12679], None);
        assert!(five.talent(2, 2).expect("deep wounds").prereqs[0].learnable);
    }

    /// **`meetsPrereq` is field 20 and nothing else** — the distinction the
    /// module comment's last section is about.
    #[test]
    fn meets_prereq_is_the_required_spell_alone() {
        let (tabs, mut talents) = arms();
        let mut gated = row(999, 161, 3, 0, &[555]);
        gated.required_spell = 339;
        gated.prereq_talent[0] = 124;
        gated.prereq_rank[0] = 0;
        talents.push(gated);
        let talents = Talents::parse(&talent_dbc(&talents), &tab_dbc(&tabs)).expect("parses");

        let without = TalentTree::build(&talents, 1, 1, &[], None);
        let row = without.talent(1, 3).expect("the gated one");
        assert!(!row.meets_prereq, "the required spell is not known");
        assert_eq!(row.prereqs.len(), 1, "…and the talent prereq is separate");

        let with = TalentTree::build(&talents, 1, 1, &[339], None);
        assert!(with.talent(1, 3).expect("the gated one").meets_prereq);
    }

    /// Both files or neither.
    #[test]
    fn one_file_alone_is_no_tree() {
        let (tabs, talents) = arms();
        assert!(Talents::parse(&[], &tab_dbc(&tabs)).is_none());
        assert!(Talents::parse(&talent_dbc(&talents), &[]).is_none());
    }
}
