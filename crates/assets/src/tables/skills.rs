//! Which skill line a spell belongs to, and what that line is called; also the
//! skills panel's list ([`SkillList`]).
//!
//! The spellbook panel's tabs ask the first question. The spellbook shows a
//! character's spells on pages (General, Fire, Frost, Arcane), and nothing in
//! the protocol says which page a spell goes on. `SMSG_INITIAL_SPELLS` is a
//! list of ids; the client does the grouping, from two tables in the archives.
//!
//! [`crate::tables::book`] does the grouping. This module answers the two
//! lookups it needs.
//!
//! ## The tables
//!
//! ```text
//! SkillLineAbility.dbc     5,072 rows    [1] skillId  [2] spellId
//!                                        [3] racemask [4] classmask
//! SkillLine.dbc              123 rows    [0] id  [1] categoryId
//!                                        [3] name (8 locale columns, enUS first)
//!                                       [21] spellIconID -> SpellIcon.dbc [1]
//! SkillRaceClassInfo.dbc     201 rows    [1] skillId
//!                                        [2] racemask [3] classmask  [4] flags
//! ```
//!
//! All three layouts are vmangos's `DBCStructure.h` (`SkillLineEntry`,
//! `SkillLineAbilityEntry`, `SkillRaceClassInfoEntry`). The 1.12.1 client
//! takes a tab's name from `SkillLine` field 3 and its icon from field 21,
//! which it resolves through `SpellIcon.dbc` field 1. That icon path column is
//! the same one [`crate::tables::spellbook`] reads for a spell.
//!
//! ## How a spell's skill line is found
//!
//! A spell's skill line is the `SkillLineAbility` row that matches the spell
//! and the character's race and class. The client matches race and class
//! before it takes `skillId` from the row. The masks matter: `Shoot Bow` is one
//! spell with rows for several classes, and taking the first row puts a
//! hunter's ability on a warrior's page.
//!
//! A spell with no matching row is skill line 0, which is the General tab.
//! The client does not treat this as an error: skill line 0 is a real value,
//! and [`crate::tables::book`] sorts it first and takes its name from
//! `GlobalStrings.lua`.
//!
//! ### How `SkillRaceClassInfo` decides whether that line is a tab
//!
//! The `SkillLineAbility` record does not decide it. The gate is
//! `DBFilesClient\SkillRaceClassInfo.dbc`: 201 rows of eight fields, keyed by
//! `skillId` (field 1), matched on fields 2 and 3 the same way an ability is,
//! and tested on field 4, `flags`:
//!
//! ```text
//! SkillRaceClassInfo for (skillId, race, class)
//!   no row          -> skill line stays 0
//!   flags & 0x80    -> skill line stays 0
//!   otherwise       -> the tab is this skill line
//! ```
//!
//! [`NO_SPELLBOOK_TAB`] (0x80) keeps a page out of the book. In 5875's data it
//! is set on every line except the three class specialisations of each class,
//! `Survival` and `Riding`. A warrior's `SkillRaceClassInfo` rows give `Arms`,
//! `Fury` and `Protection` flags of `0x410`, and give `Bows`, `Guns`,
//! `Crossbows`, `Thrown`, `Defense`, `Dual Wield`, `Unarmed`, every weapon
//! skill, every language, every profession and every riding line flags of
//! `0x80` or above. This is why the 1.12 spellbook has four tabs. Without the
//! gate, this client drew nine, with `Shoot Bow` and `Parry` each on a page of
//! its own.
//!
//! The client matches a `SkillRaceClassInfo` row's fields 2 and 3 with the same
//! comparison it uses for an ability's masks, with no exclude masks. A row
//! therefore narrows by race and class and by nothing else, which is why the
//! gate is a second mask test and not a filter on the ability.

use crate::tables::dbc::Dbc;
use std::collections::HashMap;

/// `SkillLineAbility.dbc` — see the module comment.
mod ability_fields {
    pub const SKILL_ID: usize = 1;
    pub const SPELL_ID: usize = 2;
    pub const RACE_MASK: usize = 3;
    pub const CLASS_MASK: usize = 4;
}

/// `SkillLine.dbc`; see the module comment.
mod line_fields {
    pub const CATEGORY_ID: usize = 1;
    pub const NAME: usize = 3;
    /// Nine fields after the name: the name's eight locale columns and its flag
    /// word come between them. The skills panel's detail pane reads it.
    pub const DESCRIPTION: usize = 12;
    pub const ICON_ID: usize = 21;
}

/// `SkillLine.dbc`'s category for a class skill line: the three
/// specialisations. [`SkillLine::category`] says why this is a cross-check and
/// not the rule.
pub const CATEGORY_CLASS: u32 = 7;

/// `SkillRaceClassInfo.dbc`, the tab gate. See the module comment.
mod race_class_fields {
    pub const SKILL_ID: usize = 1;
    pub const RACE_MASK: usize = 2;
    pub const CLASS_MASK: usize = 3;
    pub const FLAGS: usize = 4;
    /// The level at which a line the character has not started is shown.
    pub const MIN_LEVEL: usize = 5;
}

/// The `SkillRaceClassInfo` flag that keeps a skill line out of the spellbook:
/// bit 7 of `flags`. A line carrying it, or one with no row for this character
/// at all, sends its spells to [`GENERAL`] instead of opening a tab of its own.
pub const NO_SPELLBOOK_TAB: u32 = 0x80;

/// The three `SkillRaceClassInfo` flags the skills panel reads when it builds
/// its list, and the proficiency flag.
///
/// They come from the same column as [`NO_SPELLBOOK_TAB`] and are different
/// bits. The book asks whether a line opens a tab; the panel asks whether the
/// character is shown the line at all, and the answers differ. A warrior's
/// `Defense` is not a spellbook tab and is on the skills panel.
pub mod panel_flags {
    /// Bit 0: listed whether or not the character has any value in it.
    pub const ALWAYS: u32 = 0x01;
    /// Bit 1: never listed, whatever the value. It is tested first, so it
    /// overrides [`Self::ALWAYS`].
    pub const NEVER: u32 = 0x02;
    /// Bit 2: listed once the character's level is at least the row's
    /// `minLevel`, even with no value.
    pub const AT_LEVEL: u32 = 0x04;
    /// Bit 10: a proficiency. Rank and max are both clamped to 1, which makes
    /// `SkillFrame_SetStatusBar` draw a grey full bar with no number rather
    /// than `1/1`.
    pub const PROFICIENCY: u32 = 0x400;
}

/// `SpellIcon.dbc` has two fields, id and path. This is the path, the same
/// column [`crate::tables::spellbook`] reads.
const ICON_PATH: usize = 1;

/// The General tab. It is not a table row: it is the id a spell with no
/// `SkillLineAbility` row of its own falls to, and the client gives it a fixed
/// name and icon. See [`crate::tables::book`].
pub const GENERAL: u32 = 0;

/// One skill line, as a tab needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillLine {
    pub id: u32,
    /// `SkillLine.dbc`'s own enUS name — "Fire", "Frost", "Arms". What the tab's
    /// tooltip says, and the key the tabs are ordered by.
    pub name: String,
    /// The tab's art, resolved through `SpellIcon.dbc`. `None` for a line whose
    /// icon id is absent from that table, which draws as no texture at all
    /// rather than as a placeholder.
    pub icon: Option<String>,
    /// `SkillLine.dbc`'s `categoryId`: 6 weapon skill, 7 class skill, 8
    /// armour proficiency, 9 secondary, 10 language, 11 primary profession,
    /// 12 generic.
    ///
    /// The spellbook does not use it. The tab gate is `SkillRaceClassInfo`'s
    /// flags and not this column, because the client reads the flags. It is
    /// carried so `vale book` can cross-check the gate against a column the
    /// gate does not come from. The two agree on 5875's data.
    ///
    /// The skills panel does use it: it is the heading a line goes under. See
    /// [`SkillList::build`].
    pub category: u32,
    /// `SkillLine.dbc` field 12: what the panel's detail pane prints under the
    /// bar. Empty for most lines, which draws the text frame with nothing in
    /// it, as the 1.12.1 client does.
    pub description: String,
}

/// A zero mask means "everybody". Every mask column in this game's tables uses
/// that convention, so a lookup cannot only test the bit. `race` and `class`
/// are the one-based ids the server sends; the mask bit is `1 << (id - 1)`.
///
/// One function serves both tables, because the client matches a
/// `SkillRaceClassInfo` row's two masks with the same comparison it uses for
/// the abilities, with no exclude masks.
fn covers(race_mask: u32, class_mask: u32, race: u8, class: u8) -> bool {
    let bit = |id: u8| if id == 0 { 0 } else { 1u32 << (id - 1) };
    (race_mask == 0 || race_mask & bit(race) != 0)
        && (class_mask == 0 || class_mask & bit(class) != 0)
}

/// One `SkillLineAbility` row, reduced to the three things the lookup needs.
#[derive(Debug, Clone, Copy)]
struct Ability {
    skill: u32,
    race_mask: u32,
    class_mask: u32,
}

impl Ability {
    fn covers(&self, race: u8, class: u8) -> bool {
        covers(self.race_mask, self.class_mask, race, class)
    }
}

/// One `SkillRaceClassInfo` row: the same two masks, and the flags the tab gate
/// tests. See [`NO_SPELLBOOK_TAB`].
#[derive(Debug, Clone, Copy)]
struct RaceClass {
    race_mask: u32,
    class_mask: u32,
    flags: u32,
    /// `SkillRaceClassInfo` field 5: the level at which a line the character
    /// has not started is listed. See [`panel_flags::AT_LEVEL`].
    min_level: u32,
}

impl RaceClass {
    fn covers(&self, race: u8, class: u8) -> bool {
        covers(self.race_mask, self.class_mask, race, class)
    }
}

/// `SkillLineCategory.dbc`: the skills panel's headings, and the only thing
/// that decides their order.
///
/// Eight rows and eleven fields: `0 id`, `1..10` the localised name, `10` a
/// sort index. The client sorts the headings by that last column alone,
/// ascending, and not by name or id. That puts Class Skills above Professions
/// above Secondary Skills above Weapon Skills whatever their ids are.
mod category_fields {
    pub const NAME: usize = 1;
    /// The only sort key for the headings.
    pub const SORT: usize = 10;
}

/// One heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: u32,
    pub name: String,
    /// The only thing the headings are ordered by. See [`category_fields`].
    pub sort: i32,
}

/// The skill tables, parsed.
#[derive(Debug, Default, Clone)]
pub struct Skills {
    /// Every `SkillLineAbility` row for a spell, in file order. A `Vec` rather
    /// than one row because the masks decide which row applies: `Shoot Bow`
    /// has several and only one of them is this character's.
    abilities: HashMap<u32, Vec<Ability>>,
    lines: HashMap<u32, SkillLine>,
    /// `SkillLineCategory.dbc`, by id: the panel's headings. Empty when the
    /// table is absent, which draws an empty list rather than a wrong one:
    /// [`SkillList::build`] drops a line whose category has no row, as the
    /// client does.
    categories: HashMap<u32, Category>,
    /// `SkillRaceClassInfo`, keyed by skill line: the tab gate, and `None` when
    /// the table is absent. See [`Skills::parse`].
    race_class: Option<HashMap<u32, Vec<RaceClass>>>,
}

impl Skills {
    /// Parse both tables, plus `SpellIcon.dbc` for the tab art and
    /// `SkillRaceClassInfo.dbc` for the tab gate.
    ///
    /// Either of the first two absent is `None`, rather than a half-built
    /// answer. A `Skills` with lines and no abilities would put every spell the
    /// character knows in General, and the result would look the same as a
    /// correctly read table for a character who has one page.
    ///
    /// The other two are optional and degrade in opposite directions. No icons
    /// gives a tab with no art, which is a visible gap rather than a wrong
    /// answer. No `SkillRaceClassInfo` gives no gate: every line that has a
    /// spell opens a tab, so the book has too many tabs rather than too few.
    /// The client's answer to a missing row is [`GENERAL`], but that applies to
    /// a missing row in a table it has. Applying it to a table that failed to
    /// parse would turn the whole book into one page without any sign of the
    /// failure, because one long General tab is also what a correctly read
    /// book looks like for a level-1 character.
    pub fn parse(
        ability_dbc: &[u8],
        line_dbc: &[u8],
        icon_dbc: &[u8],
        race_class_dbc: &[u8],
    ) -> Option<Skills> {
        let abilities = Dbc::parse(ability_dbc).ok()?;
        let lines = Dbc::parse(line_dbc).ok()?;
        let icons = Dbc::parse(icon_dbc).ok();

        let mut by_spell: HashMap<u32, Vec<Ability>> = HashMap::new();
        for record in 0..abilities.record_count {
            let field = |f: usize| abilities.u32_at(record, f).unwrap_or(0);
            let spell = field(ability_fields::SPELL_ID);
            if spell == 0 {
                continue;
            }
            by_spell.entry(spell).or_default().push(Ability {
                skill: field(ability_fields::SKILL_ID),
                race_mask: field(ability_fields::RACE_MASK),
                class_mask: field(ability_fields::CLASS_MASK),
            });
        }

        let icon_path = |id: u32| -> Option<String> {
            let icons = icons.as_ref()?;
            (0..icons.record_count)
                .find(|&r| icons.u32_at(r, 0) == Some(id))
                .and_then(|r| icons.string_at(r, ICON_PATH))
                .filter(|path| !path.is_empty())
        };

        let mut by_id = HashMap::new();
        for record in 0..lines.record_count {
            let Some(id) = lines.u32_at(record, 0) else {
                continue;
            };
            by_id.insert(
                id,
                SkillLine {
                    id,
                    name: lines
                        .string_at(record, line_fields::NAME)
                        .unwrap_or_default(),
                    icon: lines
                        .u32_at(record, line_fields::ICON_ID)
                        .and_then(icon_path),
                    category: lines
                        .u32_at(record, line_fields::CATEGORY_ID)
                        .unwrap_or_default(),
                    description: lines
                        .string_at(record, line_fields::DESCRIPTION)
                        .unwrap_or_default(),
                },
            );
        }
        let race_class = Dbc::parse(race_class_dbc).ok().map(|table| {
            let mut by_line: HashMap<u32, Vec<RaceClass>> = HashMap::new();
            for record in 0..table.record_count {
                let field = |f: usize| table.u32_at(record, f).unwrap_or(0);
                by_line
                    .entry(field(race_class_fields::SKILL_ID))
                    .or_default()
                    .push(RaceClass {
                        race_mask: field(race_class_fields::RACE_MASK),
                        class_mask: field(race_class_fields::CLASS_MASK),
                        flags: field(race_class_fields::FLAGS),
                        min_level: field(race_class_fields::MIN_LEVEL),
                    });
            }
            by_line
        });

        Some(Skills {
            abilities: by_spell,
            lines: by_id,
            categories: HashMap::new(),
            race_class,
        })
    }

    /// Attach `SkillLineCategory.dbc`, which only the skills panel needs.
    ///
    /// Separate from [`Skills::parse`] and optional, in the same way as
    /// [`crate::tables::faction::Factions::with_groups`]. Without it the
    /// spellbook is unaffected and the skills panel draws no rows at all,
    /// because the 1.12.1 client also drops a line whose category has no row.
    /// An empty skills panel with no `SkillLineCategory.dbc` matches the
    /// client.
    pub fn with_categories(mut self, raw: &[u8]) -> Skills {
        let Ok(table) = Dbc::parse(raw) else {
            return self;
        };
        for record in 0..table.record_count {
            let Some(id) = table.u32_at(record, 0) else {
                continue;
            };
            self.categories.insert(
                id,
                Category {
                    id,
                    name: table
                        .string_at(record, category_fields::NAME)
                        .unwrap_or_default(),
                    sort: table.u32_at(record, category_fields::SORT).unwrap_or(0) as i32,
                },
            );
        }
        self
    }

    /// Which page `spell` goes on for this character, or [`GENERAL`].
    ///
    /// See the module comment: the race and class masks decide which skill line
    /// the spell is on, `SkillRaceClassInfo` then decides whether that line is a
    /// tab at all, and a "no" to either question gives skill line 0 rather than
    /// an error.
    pub fn line_of(&self, spell: u32, race: u8, class: u8) -> u32 {
        let line = self.ability_line(spell, race, class);
        if line == GENERAL || self.has_tab(line, race, class) {
            line
        } else {
            GENERAL
        }
    }

    /// The same lookup without the tab gate: `SkillLineAbility` filtered by
    /// race and class and nothing else, or [`GENERAL`] for a spell no row
    /// covers.
    ///
    /// This is the first half of [`Skills::line_of`]. It exists because the
    /// trainer groups its rows by skill lines that the spellbook gate would
    /// refuse. The 1.12.1 client groups trainer services by the character's
    /// race and class through the same masked `SkillLineAbility` lookup the
    /// book uses, and does not consult `SkillRaceClassInfo`. Applying the gate
    /// here would send every service to line 0, which `crate::tables::trainer`
    /// drops, and the trainer window would be empty.
    pub fn ability_line(&self, spell: u32, race: u8, class: u8) -> u32 {
        self.abilities
            .get(&spell)
            .and_then(|rows| rows.iter().find(|row| row.covers(race, class)))
            .map_or(GENERAL, |row| row.skill)
    }

    /// Whether `line` opens a tab for this character, or sends its spells to
    /// [`GENERAL`]. See [`NO_SPELLBOOK_TAB`].
    ///
    /// A character's weapon skills, languages, professions, armour
    /// proficiencies and riding lines all fail this; the three class
    /// specialisations pass it.
    fn has_tab(&self, line: u32, race: u8, class: u8) -> bool {
        // No table means no gate. `parse` explains why.
        let Some(by_line) = self.race_class.as_ref() else {
            return true;
        };
        by_line
            .get(&line)
            .and_then(|rows| rows.iter().find(|row| row.covers(race, class)))
            .is_some_and(|row| row.flags & NO_SPELLBOOK_TAB == 0)
    }

    /// The `SkillRaceClassInfo` row this character matches, or `None`.
    ///
    /// The skills panel reads this row: its flags and its `minLevel` decide
    /// whether a line is listed, and a line with no row is never listed.
    /// `None` covers both "no row for this race and class" and "no table", and
    /// the caller drops the line either way, as the client does.
    fn race_class_row(&self, line: u32, race: u8, class: u8) -> Option<&RaceClass> {
        self.race_class
            .as_ref()?
            .get(&line)?
            .iter()
            .find(|row| row.covers(race, class))
    }

    /// The `minLevel` of this character's `SkillRaceClassInfo` row for `line`,
    /// or `None` for no row. The craft window's required level is the larger
    /// of this and the spell's `spellLevel` when the row exists, and the
    /// `spellLevel` alone when it does not.
    pub fn race_class_min_level(&self, line: u32, race: u8, class: u8) -> Option<u32> {
        self.race_class_row(line, race, class).map(|row| row.min_level)
    }

    /// Every `SkillLine.dbc` id, ascending — for `vale skills`, which builds
    /// a census over the whole table rather than over one character's block.
    pub fn line_ids(&self) -> Vec<u32> {
        let mut out: Vec<u32> = self.lines.keys().copied().collect();
        out.sort_unstable();
        out
    }

    /// The eight headings in the order the panel shows them, sorted by the
    /// sort column alone. See [`category_fields`].
    pub fn categories_in_order(&self) -> Vec<&Category> {
        let mut out: Vec<&Category> = self.categories.values().collect();
        out.sort_by_key(|category| category.sort);
        out
    }

    /// `(always, never, at-level)` over every `SkillRaceClassInfo` row — the
    /// three flags that decide whether an unused line is listed, counted so a
    /// changed reading of the bits is visible as a changed number.
    pub fn flag_census(&self) -> (usize, usize, usize) {
        let Some(by_line) = self.race_class.as_ref() else {
            return (0, 0, 0);
        };
        let mut counts = (0, 0, 0);
        for row in by_line.values().flatten() {
            counts.0 += usize::from(row.flags & panel_flags::ALWAYS != 0);
            counts.1 += usize::from(row.flags & panel_flags::NEVER != 0);
            counts.2 += usize::from(row.flags & panel_flags::AT_LEVEL != 0);
        }
        counts
    }

    /// A skill line's name and icon. `None` for [`GENERAL`], which has no row
    /// of its own; [`crate::tables::book`] supplies its name and icon.
    pub fn line(&self, id: u32) -> Option<&SkillLine> {
        self.lines.get(&id)
    }

    /// Every spell whose own row names this class, ascending: the class's
    /// abilities, without the professions and weapon skills everybody shares.
    ///
    /// The filter is `class_mask != 0`. Most rows in the table carry no class
    /// mask: Cooking, Blacksmithing and every weapon skill are open to
    /// everyone, and they stay out of a warrior's spellbook only because the
    /// character has not learned them. So this is not a rule the client
    /// applies; the client filters by the spells the server sent. It stands in
    /// for a character, so that `vale book` can print a spellbook with no
    /// session.
    ///
    /// Nothing in `crates/client` calls it. It exists to check the rule in
    /// [`crate::tables::book`].
    pub fn class_abilities(&self, race: u8, class: u8) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .abilities
            .iter()
            .filter(|(_, rows)| {
                rows.iter()
                    .any(|row| row.class_mask != 0 && row.covers(race, class))
            })
            .map(|(spell, _)| *spell)
            .collect();
        ids.sort_unstable();
        ids
    }

    /// How many lines the table carries, for the checks.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// How many spells have a line at all, for the checks.
    pub fn ability_count(&self) -> usize {
        self.abilities.len()
    }

    /// How many lines the gate has rows for, and how many of those may open a
    /// tab for some character. `None` when `SkillRaceClassInfo` is absent;
    /// `vale book` reports that case, because without the table the book has
    /// too many tabs and its counts are wrong.
    pub fn tab_counts(&self) -> Option<(usize, usize)> {
        let by_line = self.race_class.as_ref()?;
        let open = by_line
            .values()
            .filter(|rows| rows.iter().any(|r| r.flags & NO_SPELLBOOK_TAB == 0))
            .count();
        Some((by_line.len(), open))
    }
}

impl std::fmt::Display for SkillLine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.id)
    }
}

/// One line of the skills panel: a heading or a bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillRow {
    /// `SkillLine.dbc`'s id, or `0` for a heading, which is how the client
    /// tells the two apart.
    pub line: u32,
    /// The heading this row is under, or its own id if it is a heading.
    pub category: u32,
    /// What the panel prints: the line's name, or the category's for a heading.
    pub name: String,
    /// What the detail pane prints. Empty for a heading.
    pub description: String,
    pub rank: i32,
    pub max_rank: i32,
    /// The temporary bonus, drawn in green or red beside the rank.
    pub modifier: i32,
    /// True when the character has no value in this line (the raw value is
    /// 0). It distinguishes a line listed by [`panel_flags::AT_LEVEL`] from
    /// one the character has used.
    pub unstarted: bool,
    /// Whether the panel offers an Unlearn button. Two conditions must hold:
    /// the slot's step word is non-zero, and the `SkillRaceClassInfo` row
    /// carries bit 5.
    ///
    /// A non-zero step is what marks a profession in practice; a weapon
    /// skill's step is zero. Testing only the flag would put an Unlearn button
    /// on Defense.
    pub abandonable: bool,
    pub is_header: bool,
    pub is_collapsed: bool,
}

/// The skills panel's whole list, built as the 1.12.1 client builds it.
///
/// It has the same shape as [`crate::tables::reputation::Reputation`], for the
/// same reason: none of it crosses the wire. The server sends 128 `(id, value,
/// max, bonus)` slots in whatever order it wrote them. The client decides which
/// of them are listed, what they are called, what heading they go under and
/// the order of all of it:
///
/// ```text
/// build:            walk the 128 slots, apply the three flags, add the heading
/// recount:          the displayed count, and the two sorts
///                   (headings by category sort index, then rows)
/// GetSkillLineInfo: the twelve return values, in order
/// ```
#[derive(Debug, Clone, Default)]
pub struct SkillList {
    rows: Vec<SkillRow>,
    /// The headings in display order, and whether each is collapsed.
    headings: Vec<(u32, bool)>,
    /// How many rows `GetNumSkillLines` reports: the visible prefix, as in
    /// the reputation panel.
    displayed: usize,
    /// `GetSelectedSkill`, a one-based display index, and `0` for none.
    selected: usize,
}

impl SkillList {
    /// `GetNumSkillLines()`.
    pub fn len(&self) -> usize {
        self.displayed
    }

    pub fn is_empty(&self) -> bool {
        self.displayed == 0
    }

    /// `GetSkillLineInfo(index)`, one-based. `None` past the end.
    pub fn row(&self, index: usize) -> Option<&SkillRow> {
        self.rows.get(index.checked_sub(1)?)
    }

    /// Every row, including the ones under a collapsed heading. For
    /// `vale skills` and the tests, not for the panel.
    pub fn rows(&self) -> &[SkillRow] {
        &self.rows
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// `SetSelectedSkill(index)`. Not range-checked, because the interface
    /// passes 0 to mean "nothing", and `SkillFrame_OnLoad` does so.
    pub fn select(&mut self, index: usize) {
        self.selected = index;
    }

    /// `CollapseSkillHeader(index)` / `ExpandSkillHeader(index)`, one-based.
    ///
    /// The client treats an index of `-1` as every heading, and
    /// `SkillFrameCollapseAllButton` passes it. That is why this takes an
    /// `i32` where the reputation panel's equivalent takes a `usize`.
    pub fn set_collapsed(&mut self, index: i32, collapsed: bool) {
        if index < 0 {
            for heading in &mut self.headings {
                heading.1 = collapsed;
            }
        } else {
            let Some(row) = self.row(index as usize) else {
                return;
            };
            if !row.is_header {
                return;
            }
            let category = row.category;
            for heading in &mut self.headings {
                if heading.0 == category {
                    heading.1 = collapsed;
                }
            }
        }
        self.recount();
    }

    /// Whether every heading is open. `SkillFrame_UpdateSkills` chooses the
    /// collapse-all button's art from this, and works it out by walking the
    /// list itself.
    pub fn all_expanded(&self) -> bool {
        self.headings.iter().all(|(_, collapsed)| !collapsed)
    }

    /// Build the list.
    ///
    /// `have` is the character's own block. [`crate::tables::skills`] does not
    /// depend on `vale-protocol`, so the block arrives as [`SkillEntry`]
    /// values rather than as that crate's type.
    ///
    /// The walk is over the server's slots rather than over `SkillLine.dbc`,
    /// which is the order the client uses. A line is listed only if the
    /// character's block has a slot for it, and a slot with no value is listed
    /// only if one of the two flags below says so.
    pub fn build(
        &mut self,
        skills: &Skills,
        race: u8,
        class: u8,
        level: u32,
        have: &[SkillEntry],
    ) {
        self.rows.clear();
        let previously: Vec<(u32, bool)> = self.headings.clone();
        self.headings.clear();
        for entry in have {
            if entry.id == 0 {
                continue;
            }
            let Some(line) = skills.line(entry.id) else {
                continue;
            };
            let Some(info) = skills.race_class_row(entry.id, race, class) else {
                continue;
            };
            // A line whose category has no row is dropped, not defaulted.
            let Some(category) = skills.categories.get(&line.category) else {
                continue;
            };
            if info.flags & panel_flags::NEVER != 0 {
                continue;
            }
            // A line with no value is listed only if its row says "always", or
            // says "at level" and the character has reached that level.
            if info.flags & panel_flags::ALWAYS == 0 && entry.value == 0 {
                if info.flags & panel_flags::AT_LEVEL == 0 || level < info.min_level {
                    continue;
                }
            }
            let proficiency = info.flags & panel_flags::PROFICIENCY != 0;
            let (rank, max_rank) = if proficiency {
                // Both clamped to 1; the rank changes only when it is above 1.
                (entry.rank.min(1), 1)
            } else {
                (entry.rank, entry.max_rank)
            };
            self.rows.push(SkillRow {
                line: entry.id,
                category: category.id,
                name: line.name.clone(),
                description: line.description.clone(),
                rank,
                max_rank,
                modifier: entry.modifier,
                unstarted: entry.value == 0,
                abandonable: entry.step != 0 && info.flags & CAN_UNLEARN != 0,
                is_header: false,
                is_collapsed: false,
            });
            // The heading row is added immediately after the first line under
            // it, as the client adds it, so the rows must be sorted afterwards.
            if self.headings.iter().any(|(id, _)| *id == category.id) {
                continue;
            }
            let collapsed = previously
                .iter()
                .find(|(id, _)| *id == category.id)
                .is_some_and(|(_, collapsed)| *collapsed);
            self.headings.push((category.id, collapsed));
            self.rows.push(SkillRow {
                line: 0,
                category: category.id,
                name: category.name.clone(),
                description: String::new(),
                rank: 0,
                max_rank: 0,
                modifier: 0,
                unstarted: false,
                abandonable: false,
                is_header: true,
                is_collapsed: collapsed,
            });
        }
        self.recount_with(skills);
    }

    /// The sorts and the displayed count.
    fn recount_with(&mut self, skills: &Skills) {
        // Headings by their category's own sort column, ascending,
        // and by nothing else.
        self.headings.sort_by_key(|(id, _)| {
            skills.categories.get(id).map_or(i32::MAX, |c| c.sort)
        });
        self.recount();
    }

    /// The row order and the displayed count, over the headings' current
    /// order: used directly by the collapse verbs, which change which rows are
    /// hidden and not the order of the headings.
    ///
    /// The rows are sorted in full every time. Sorting only by whether a row
    /// is hidden moved a collapsed heading's rows to the end and, being a
    /// stable sort, left them there when the heading was expanded again, so
    /// they reappeared at the bottom of the list rather than under their
    /// heading.
    fn recount(&mut self) {
        let order: Vec<u32> = self.headings.iter().map(|(id, _)| *id).collect();
        let collapsed: Vec<bool> = self.headings.iter().map(|(_, c)| *c).collect();
        let slot = |category: u32| order.iter().position(|id| *id == category).unwrap_or(order.len());
        for row in &mut self.rows {
            row.is_collapsed = collapsed.get(slot(row.category)).copied().unwrap_or(false);
        }
        // A hidden row last, then by heading slot, then the heading itself
        // before its own lines, then by name: the same comparator shape the
        // reputation panel's rows have.
        self.rows.sort_by(|a, b| {
            let hidden = |row: &SkillRow| !row.is_header && row.is_collapsed;
            match (hidden(a), hidden(b)) {
                (false, true) => return std::cmp::Ordering::Less,
                (true, false) => return std::cmp::Ordering::Greater,
                _ => {}
            }
            slot(a.category)
                .cmp(&slot(b.category))
                .then(b.is_header.cmp(&a.is_header))
                .then_with(|| a.name.cmp(&b.name))
        });
        self.displayed = self
            .rows
            .iter()
            .filter(|row| row.is_header || !row.is_collapsed)
            .count();
    }
}

/// The `SkillRaceClassInfo` flag behind [`SkillRow::abandonable`]: bit 5,
/// `flags & 0x20`.
pub const CAN_UNLEARN: u32 = 0x20;

/// One slot of the character's own block, as [`SkillList::build`] takes it.
///
/// A plain struct rather than `vale_protocol`'s type because this crate does
/// not depend on that one, the same boundary every other table module keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SkillEntry {
    pub id: u32,
    /// The high half of the block's first dword. Only
    /// [`SkillRow::abandonable`] reads it.
    pub step: u16,
    /// The raw field value, which the three flags are tested against. It is
    /// not [`Self::rank`], which includes the permanent bonus.
    pub value: u16,
    pub rank: i32,
    pub max_rank: i32,
    pub modifier: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing::dbc;

    /// `\0Fire\0Frost\0`: the names are at offsets 1 and 6.
    const NAMES: &[u8] = b"\0Fire\0Frost\0";

    fn ability(id: u32, skill: u32, spell: u32, race: u32, class: u32) -> Vec<u32> {
        let mut row = vec![0; 15];
        row[0] = id;
        row[ability_fields::SKILL_ID] = skill;
        row[ability_fields::SPELL_ID] = spell;
        row[ability_fields::RACE_MASK] = race;
        row[ability_fields::CLASS_MASK] = class;
        row
    }

    fn line(id: u32, name_offset: u32, icon: u32) -> Vec<u32> {
        let mut row = vec![0; 22];
        row[0] = id;
        row[line_fields::CATEGORY_ID] = CATEGORY_CLASS;
        row[line_fields::NAME] = name_offset;
        row[line_fields::ICON_ID] = icon;
        row
    }

    fn race_class(id: u32, skill: u32, race: u32, class: u32, flags: u32) -> Vec<u32> {
        let mut row = vec![0; 8];
        row[0] = id;
        row[race_class_fields::SKILL_ID] = skill;
        row[race_class_fields::RACE_MASK] = race;
        row[race_class_fields::CLASS_MASK] = class;
        row[race_class_fields::FLAGS] = flags;
        row
    }

    const ABILITIES: [[u32; 5]; 3] = [
        // Fireball: mage only (class 8 -> bit 7), any race.
        [1, 8, 133, 0, 1 << 7],
        // Shoot Bow: the same spell against two classes, which is why the
        // masks are read.
        [2, 45, 2480, 0, 1 << 2],
        [3, 46, 2480, 0, 1 << 3],
    ];

    /// The three lines, with a `SkillRaceClassInfo` row for each: line 8 open
    /// (a class page) and lines 45 and 46 carrying [`NO_SPELLBOOK_TAB`] the way
    /// every weapon skill in 5875 does.
    fn skills() -> Skills {
        gated(&[
            race_class(1, 8, 0, 0, 0x410),
            race_class(2, 45, 0, 0, NO_SPELLBOOK_TAB),
            race_class(3, 46, 0, 0, NO_SPELLBOOK_TAB),
        ])
    }

    /// The same two tables with the given gate rows. An empty slice is a table
    /// with no row for any line, which is not the same as no table.
    fn gated(rows: &[Vec<u32>]) -> Skills {
        let abilities: Vec<Vec<u32>> = ABILITIES
            .iter()
            .map(|a| ability(a[0], a[1], a[2], a[3], a[4]))
            .collect();
        let lines = [line(8, 1, 10), line(45, 6, 11), line(46, 6, 0)];
        let icons = [vec![10u32, 1], vec![11, 6]];
        Skills::parse(
            &dbc(&abilities, 15, NAMES),
            &dbc(&lines, 22, NAMES),
            &dbc(&icons, 2, b"\0Interface\\Icons\\A\0"),
            &dbc(rows, 8, b"\0"),
        )
        .expect("both tables parse")
    }

    /// The two required tables and no gate: the documented degradation, where
    /// every line keeps its tab.
    fn ungated() -> Skills {
        let abilities: Vec<Vec<u32>> = ABILITIES
            .iter()
            .map(|a| ability(a[0], a[1], a[2], a[3], a[4]))
            .collect();
        let lines = [line(8, 1, 10), line(45, 6, 11), line(46, 6, 0)];
        Skills::parse(&dbc(&abilities, 15, NAMES), &dbc(&lines, 22, NAMES), &[], &[])
            .expect("both required tables parse")
    }

    #[test]
    fn a_spell_lands_on_the_line_its_own_class_row_names() {
        let skills = skills();
        // Class 8 is the mage; class 1 is the warrior, whose bit the row does
        // not carry.
        assert_eq!(skills.line_of(133, 1, 8), 8);
        assert_eq!(skills.line_of(133, 1, 1), GENERAL);
    }

    /// The masks decide the row. One spell, two rows, two classes: taking the
    /// first row would put a hunter's ability on the rogue's page.
    ///
    /// This uses the ungated tables, because the question here is which
    /// ability row wins, and 5875 gates both of those lines out of the book.
    #[test]
    fn one_spell_with_two_class_rows_resolves_per_class() {
        let skills = ungated();
        assert_eq!(skills.line_of(2480, 1, 3), 45);
        assert_eq!(skills.line_of(2480, 1, 4), 46);
    }

    /// A zero mask is "everybody". See [`covers`].
    #[test]
    fn a_zero_mask_matches_every_race() {
        let skills = skills();
        for race in 1..=8 {
            assert_eq!(skills.line_of(133, race, 8), 8, "race {race}");
        }
    }

    /// A spell with no `SkillLineAbility` row is General, which is a real
    /// answer rather than a missing one.
    #[test]
    fn a_spell_with_no_ability_row_is_general() {
        assert_eq!(skills().line_of(999_999, 1, 1), GENERAL);
        assert!(skills().line(GENERAL).is_none());
    }

    #[test]
    fn a_line_carries_its_name_and_its_resolved_icon() {
        let skills = skills();
        let fire = skills.line(8).expect("the line");
        assert_eq!(fire.name, "Fire");
        assert_eq!(fire.icon.as_deref(), Some("Interface\\Icons\\A"));
        // A line whose icon id is not in `SpellIcon.dbc` draws nothing rather
        // than a placeholder.
        assert_eq!(skills.line(46).expect("the line").icon, None);
    }

    /// Either required table absent is `None`. [`Skills::parse`] says why, and
    /// why the other two are optional.
    #[test]
    fn a_half_built_table_is_no_table() {
        assert!(Skills::parse(&[], &dbc(&[], 22, NAMES), &[], &[]).is_none());
        assert!(Skills::parse(&dbc(&[], 15, NAMES), &[], &[], &[]).is_none());
    }

    /// A weapon skill is not a page. The [`NO_SPELLBOOK_TAB`] bit is the
    /// difference between the 1.12 spellbook's four tabs and the nine this
    /// client drew without the gate: `Shoot Bow` falls to General while the
    /// class page keeps its own.
    #[test]
    fn a_line_the_gate_refuses_sends_its_spells_to_general() {
        let skills = skills();
        assert_eq!(skills.line_of(133, 1, 8), 8, "a class page opens");
        assert_eq!(skills.line_of(2480, 1, 3), GENERAL, "a weapon skill does not");
        // The gate tests the bit, not the whole word: a row carrying every
        // other flag and not this one still opens.
        let open = gated(&[race_class(1, 45, 0, 0, !NO_SPELLBOOK_TAB)]);
        assert_eq!(open.line_of(2480, 1, 3), 45);
    }

    /// A line with no `SkillRaceClassInfo` row of its own is General too, so
    /// the gate cannot be modelled as a filter on the flags alone.
    #[test]
    fn a_line_the_gate_has_never_heard_of_is_general() {
        let skills = gated(&[race_class(1, 8, 0, 0, 0x410)]);
        assert_eq!(skills.line_of(133, 1, 8), 8);
        assert_eq!(skills.line_of(2480, 1, 3), GENERAL);
    }

    /// The gate takes the character's race and class, which is why it is a
    /// second lookup rather than a column on the line. `Swords` is the shipped
    /// example: two rows for two sets of races, and 5875 gates both.
    #[test]
    fn the_gate_is_asked_per_character() {
        let skills = gated(&[
            race_class(1, 45, 1 << 0, 0, 0),
            race_class(2, 45, 1 << 1, 0, NO_SPELLBOOK_TAB),
        ]);
        assert_eq!(skills.line_of(2480, 1, 3), 45, "race 1's row opens");
        assert_eq!(skills.line_of(2480, 2, 3), GENERAL, "race 2's does not");
    }

    /// No table means no gate, not a book of one page. See [`Skills::parse`].
    #[test]
    fn an_absent_gate_leaves_every_line_a_tab() {
        let skills = ungated();
        assert_eq!(skills.line_of(2480, 1, 3), 45);
        assert_eq!(skills.tab_counts(), None);
    }

    /// The counts the check prints: how many lines the gate speaks about, and
    /// how many of those may open a tab for somebody.
    #[test]
    fn the_tab_counts_are_lines_and_open_lines() {
        assert_eq!(skills().tab_counts(), Some((3, 1)));
    }
}
