//! `spell_template`: the server's copy of `Spell.dbc`, and how an edit reaches it.
//!
//! ## The two tables are the same spell and not the same row
//!
//! The client reads `DBFilesClient\Spell.dbc` — 22,360 records of 173 fields.
//! The server reads `spell_template` — 151 columns, one row per `(entry, build)`,
//! and `SpellMgr::LoadSpells` (`SpellMgr.cpp:3619`) takes, for each entry, the
//! row with the **highest `build` at or below 5875**. So an edit that is to be
//! live on both sides is a `Spell.dbc` field on one and a column of the winning
//! row on the other, and [`COLUMNS`] is the join between them.
//!
//! The mapping is transcribed from `SpellMgr::LoadSpell`
//! (`SpellMgr.cpp:3693..3851`), which reads the row `SELECT *` returned column
//! by column. It is not derivable from either side alone: the column order is
//! neither the DBC's field order nor anything else, because three columns in the
//! middle of it are the server's own.
//!
//! ## Three columns have no answer in the client's file, and they hold real data
//!
//! [`Source::Server`] is four columns, and the count is the whole point:
//!
//! ```text
//! effectBonusCoefficient1..3   the spell-power coefficient. NOT in 1.12's DBC.
//!                              Measured on the reference install: non-zero in
//!                              35,022 of 35,761 rows
//! minTargetLevel               not in 1.12's DBC either; non-zero in 4 rows
//! customFlags                  vmangos' own; non-zero in 2,746
//! script_name                  vmangos' own; set in 498
//! build                        the key this whole arrangement turns on
//! ```
//!
//! **So a writer that built a whole row out of `Spell.dbc` would erase the
//! coefficient of every spell it touched**, and the spell would go on working
//! while doing the wrong damage — which is the class of failure this project
//! calls rendering plausibly instead of failing. Nothing here ever writes a
//! column the client's file does not feed, except when the entry is new
//! ([`insert_new`]) and there is nothing to erase.
//!
//! ## An edit is the difference between two files, not a row
//!
//! [`changes`] takes the `Spell.dbc` the archives ship and the one this project
//! edited, and produces one assignment per column whose value moved. That is
//! narrower than "every column the DBC feeds" and the difference matters:
//! vmangos' rows are not a dump of the client's file, they carry the server's own
//! corrections, and a save that wrote every DBC-fed column would revert every one
//! of them for that spell. A column nobody edited is a column no statement names.
//!
//! ## What Save emits, and why it is three statements
//!
//! ```sql
//! -- 1. make sure the dev row exists, as a copy of the row the server is using
//! INSERT IGNORE INTO `spell_template` SELECT `entry`, 5875, `school`, … FROM …
//! -- 2. …or build one from the client's file, for an entry the table has never had
//! INSERT IGNORE INTO `spell_template` (`entry`, `build`, …) VALUES (…)
//! -- 3. and apply what was edited
//! UPDATE `spell_template` SET `effectBasePoints1` = 41 WHERE `entry` = 133 AND `build` = 5875
//! ```
//!
//! All three are idempotent and the pair of inserts is deliberate: whichever of
//! them applies, the other does nothing, so one sequence is right both for a
//! spell vmangos already knows and for one this editor invented. `INSERT IGNORE`
//! rather than a lookup because the key is `(entry, build)` and the server is
//! the only thing that can answer whether the row is there — asking costs a
//! round trip and a race.
//!
//! **A dev row at build 5875 is the override and there is no other mechanism.**
//! It wins at load against every lower build, it needs no schema change and no
//! flag, and `.reload spell_template` makes it live. 1,012 entries on the
//! reference install already carry one.

use vale_edit::dbc::DbcFile;
use std::collections::HashMap;

/// The table.
pub const TABLE: &str = "spell_template";

/// **The build a dev edit is written at**, which is this client's own.
///
/// `LoadSpells` takes the highest row at or below `SUPPORTED_CLIENT_BUILD`, and
/// 5875 is that number, so a row here beats every shipped one. Writing *above*
/// it would be invisible instead.
pub const BUILD: u32 = 5875;

/// **The largest spell id `spell_template` can hold.**
///
/// The column is `smallint(5) unsigned`, and `Spell.dbc`'s id is a `u32` — so
/// the client's file can name a spell the server's table physically cannot
/// store. The two do not agree and the table is the narrower.
///
/// **MySQL does not refuse the difference, it silently clamps it.** Measured:
/// writing entry 90,210 produced a row at **65,535** — a real row, for a
/// different spell, with the edited spell's values in it. The undo written
/// beside it said `WHERE entry = 90210` and matched nothing, so the one thing
/// that could have taken it back could not. (A server in strict mode errors
/// instead, which is the same problem reported rather than hidden.)
///
/// So an entry past this is refused here, before a statement exists. See
/// [`fits`].
pub const MAX_ENTRY: u32 = u16::MAX as u32;

/// Whether the server's table can hold this spell id at all — see
/// [`MAX_ENTRY`].
///
/// The client's own new-row minting is `max id + 1` over a table whose largest
/// shipped id is about 26,000, so this is headroom rather than a limit anybody
/// meets by accident. What meets it is an id typed by hand.
pub fn fits(entry: u32) -> bool {
    entry <= MAX_ENTRY
}

/// Where a column's value comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    /// The client's file has no answer for it. See the module comment: these
    /// are never written over.
    Server,
    /// One field of `Spell.dbc`.
    Field(usize),
    /// Two fields, low then high, as one 64-bit column — `spellFamilyFlags`,
    /// which the DBC splits and the table does not.
    Pair(usize, usize),
}

/// How a column's value is read and written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Unsigned,
    Signed,
    Float,
    /// `bigint unsigned`.
    Big,
    Text,
}

/// One column of `spell_template`.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    /// Its name, as vmangos spells it — typos included: `manCostPerLevel` and
    /// `modelNextSpell` are the table's own, and a statement that corrected
    /// them would not apply.
    pub name: &'static str,
    pub source: Source,
    pub kind: Kind,
}

impl Column {
    /// **Whether vmangos reads this column into a spell at all.**
    ///
    /// Thirteen of the 151 are `SELECT`ed and then not assigned — see
    /// [`IGNORED`]. An edit to one of them is a real edit to the client's own
    /// file and reaches nothing on the server, and a save that writes only
    /// those has nothing to reload.
    pub fn used_by_the_server(&self) -> bool {
        !IGNORED.contains(&self.name)
    }

    /// The value this column takes from a record of `Spell.dbc`, as a SQL
    /// literal — or `None` for a column the file does not feed.
    pub fn value(&self, dbc: &DbcFile, record: usize) -> Option<String> {
        match self.source {
            Source::Server => None,
            Source::Pair(low, high) => {
                let low = dbc.u32_at(record, low)? as u64;
                let high = dbc.u32_at(record, high)? as u64;
                Some(((high << 32) | low).to_string())
            }
            Source::Field(field) => match self.kind {
                Kind::Signed => Some(dbc.i32_at(record, field)?.to_string()),
                Kind::Float => Some(crate::sql::float(dbc.f32_at(record, field)?)),
                Kind::Text => Some(crate::sql::text(&dbc.string_at(record, field)?)),
                Kind::Unsigned | Kind::Big => Some(dbc.u32_at(record, field)?.to_string()),
            },
        }
    }
}

const fn c(name: &'static str, source: Source, kind: Kind) -> Column {
    Column { name, source, kind }
}

/// **The thirteen columns vmangos selects and then does not read.**
///
/// Each is a line of `LoadSpell` that is commented out, and the comment on most
/// of them is the words *not used* (`SpellMgr.cpp:3703, 3738, 3819, 3824`, the
/// block at `3826..3832`, and `3843, 3847..3849`). They are in the table
/// because the table is the DBC's shape; nothing on the server ever looks at
/// them.
///
/// It is worth naming rather than counting, because two of them are things a
/// person would expect to matter: **a spell's description and its tooltip do
/// not reach the server at all.** The client draws both out of `Spell.dbc`, so
/// editing them works — it simply works on one side only, and there is nothing
/// to reload.
///
/// Measured, and this is what pinned the list: over the 22,360 spells in both
/// the file and the reference database, the four `*Flags` columns agree with
/// the client's file for 10–17% of rows while every column vmangos reads agrees
/// for 99.1% or better. A column nothing reads is a column whose value nobody
/// kept true.
pub const IGNORED: [&str; 13] = [
    "castUI",
    "modelNextSpell",
    "spellVisual2",
    "nameFlags",
    "nameSubtextFlags",
    "description",
    "descriptionFlags",
    "auraDescription",
    "auraDescriptionFlags",
    "stanceBarOrder",
    "minFactionId",
    "minReputation",
    "requiredAuraVision",
];

/// **Every column of `spell_template`, in the order `SELECT *` returns them.**
///
/// The order is load-bearing twice over: `LoadSpell` reads by position, and
/// [`copy_forward`] names them all in a `SELECT` whose result has to line up
/// with the table it is inserted into.
pub const COLUMNS: [Column; 151] = [
    c("entry", Source::Field(0), Kind::Unsigned),
    c("build", Source::Server, Kind::Unsigned),
    c("school", Source::Field(1), Kind::Unsigned),
    c("category", Source::Field(2), Kind::Unsigned),
    c("castUI", Source::Field(3), Kind::Unsigned),
    c("dispel", Source::Field(4), Kind::Unsigned),
    c("mechanic", Source::Field(5), Kind::Unsigned),
    c("attributes", Source::Field(6), Kind::Unsigned),
    c("attributesEx", Source::Field(7), Kind::Unsigned),
    c("attributesEx2", Source::Field(8), Kind::Unsigned),
    c("attributesEx3", Source::Field(9), Kind::Unsigned),
    c("attributesEx4", Source::Field(10), Kind::Unsigned),
    c("stances", Source::Field(11), Kind::Unsigned),
    c("stancesNot", Source::Field(12), Kind::Unsigned),
    c("targets", Source::Field(13), Kind::Unsigned),
    c("targetCreatureType", Source::Field(14), Kind::Unsigned),
    c("requiresSpellFocus", Source::Field(15), Kind::Unsigned),
    c("casterAuraState", Source::Field(16), Kind::Unsigned),
    c("targetAuraState", Source::Field(17), Kind::Unsigned),
    c("castingTimeIndex", Source::Field(18), Kind::Unsigned),
    c("recoveryTime", Source::Field(19), Kind::Unsigned),
    c("categoryRecoveryTime", Source::Field(20), Kind::Unsigned),
    c("interruptFlags", Source::Field(21), Kind::Unsigned),
    c("auraInterruptFlags", Source::Field(22), Kind::Unsigned),
    c("channelInterruptFlags", Source::Field(23), Kind::Unsigned),
    c("procFlags", Source::Field(24), Kind::Unsigned),
    c("procChance", Source::Field(25), Kind::Unsigned),
    c("procCharges", Source::Field(26), Kind::Unsigned),
    c("maxLevel", Source::Field(27), Kind::Unsigned),
    c("baseLevel", Source::Field(28), Kind::Unsigned),
    c("spellLevel", Source::Field(29), Kind::Unsigned),
    c("durationIndex", Source::Field(30), Kind::Unsigned),
    c("powerType", Source::Field(31), Kind::Unsigned),
    c("manaCost", Source::Field(32), Kind::Unsigned),
    c("manCostPerLevel", Source::Field(33), Kind::Unsigned),
    c("manaPerSecond", Source::Field(34), Kind::Unsigned),
    c("manaPerSecondPerLevel", Source::Field(35), Kind::Unsigned),
    c("rangeIndex", Source::Field(36), Kind::Unsigned),
    c("speed", Source::Field(37), Kind::Float),
    c("modelNextSpell", Source::Field(38), Kind::Unsigned),
    c("stackAmount", Source::Field(39), Kind::Unsigned),
    c("totem1", Source::Field(40), Kind::Unsigned),
    c("totem2", Source::Field(41), Kind::Unsigned),
    c("reagent1", Source::Field(42), Kind::Signed),
    c("reagent2", Source::Field(43), Kind::Signed),
    c("reagent3", Source::Field(44), Kind::Signed),
    c("reagent4", Source::Field(45), Kind::Signed),
    c("reagent5", Source::Field(46), Kind::Signed),
    c("reagent6", Source::Field(47), Kind::Signed),
    c("reagent7", Source::Field(48), Kind::Signed),
    c("reagent8", Source::Field(49), Kind::Signed),
    c("reagentCount1", Source::Field(50), Kind::Unsigned),
    c("reagentCount2", Source::Field(51), Kind::Unsigned),
    c("reagentCount3", Source::Field(52), Kind::Unsigned),
    c("reagentCount4", Source::Field(53), Kind::Unsigned),
    c("reagentCount5", Source::Field(54), Kind::Unsigned),
    c("reagentCount6", Source::Field(55), Kind::Unsigned),
    c("reagentCount7", Source::Field(56), Kind::Unsigned),
    c("reagentCount8", Source::Field(57), Kind::Unsigned),
    c("equippedItemClass", Source::Field(58), Kind::Signed),
    c("equippedItemSubClassMask", Source::Field(59), Kind::Signed),
    c("equippedItemInventoryTypeMask", Source::Field(60), Kind::Signed),
    c("effect1", Source::Field(61), Kind::Unsigned),
    c("effect2", Source::Field(62), Kind::Unsigned),
    c("effect3", Source::Field(63), Kind::Unsigned),
    c("effectDieSides1", Source::Field(64), Kind::Signed),
    c("effectDieSides2", Source::Field(65), Kind::Signed),
    c("effectDieSides3", Source::Field(66), Kind::Signed),
    c("effectBaseDice1", Source::Field(67), Kind::Unsigned),
    c("effectBaseDice2", Source::Field(68), Kind::Unsigned),
    c("effectBaseDice3", Source::Field(69), Kind::Unsigned),
    c("effectDicePerLevel1", Source::Field(70), Kind::Float),
    c("effectDicePerLevel2", Source::Field(71), Kind::Float),
    c("effectDicePerLevel3", Source::Field(72), Kind::Float),
    c("effectRealPointsPerLevel1", Source::Field(73), Kind::Float),
    c("effectRealPointsPerLevel2", Source::Field(74), Kind::Float),
    c("effectRealPointsPerLevel3", Source::Field(75), Kind::Float),
    c("effectBasePoints1", Source::Field(76), Kind::Signed),
    c("effectBasePoints2", Source::Field(77), Kind::Signed),
    c("effectBasePoints3", Source::Field(78), Kind::Signed),
    c("effectBonusCoefficient1", Source::Server, Kind::Float),
    c("effectBonusCoefficient2", Source::Server, Kind::Float),
    c("effectBonusCoefficient3", Source::Server, Kind::Float),
    c("effectMechanic1", Source::Field(79), Kind::Unsigned),
    c("effectMechanic2", Source::Field(80), Kind::Unsigned),
    c("effectMechanic3", Source::Field(81), Kind::Unsigned),
    c("effectImplicitTargetA1", Source::Field(82), Kind::Unsigned),
    c("effectImplicitTargetA2", Source::Field(83), Kind::Unsigned),
    c("effectImplicitTargetA3", Source::Field(84), Kind::Unsigned),
    c("effectImplicitTargetB1", Source::Field(85), Kind::Unsigned),
    c("effectImplicitTargetB2", Source::Field(86), Kind::Unsigned),
    c("effectImplicitTargetB3", Source::Field(87), Kind::Unsigned),
    c("effectRadiusIndex1", Source::Field(88), Kind::Unsigned),
    c("effectRadiusIndex2", Source::Field(89), Kind::Unsigned),
    c("effectRadiusIndex3", Source::Field(90), Kind::Unsigned),
    c("effectApplyAuraName1", Source::Field(91), Kind::Unsigned),
    c("effectApplyAuraName2", Source::Field(92), Kind::Unsigned),
    c("effectApplyAuraName3", Source::Field(93), Kind::Unsigned),
    c("effectAmplitude1", Source::Field(94), Kind::Unsigned),
    c("effectAmplitude2", Source::Field(95), Kind::Unsigned),
    c("effectAmplitude3", Source::Field(96), Kind::Unsigned),
    c("effectMultipleValue1", Source::Field(97), Kind::Float),
    c("effectMultipleValue2", Source::Field(98), Kind::Float),
    c("effectMultipleValue3", Source::Field(99), Kind::Float),
    c("effectChainTarget1", Source::Field(100), Kind::Unsigned),
    c("effectChainTarget2", Source::Field(101), Kind::Unsigned),
    c("effectChainTarget3", Source::Field(102), Kind::Unsigned),
    c("effectItemType1", Source::Field(103), Kind::Big),
    c("effectItemType2", Source::Field(104), Kind::Big),
    c("effectItemType3", Source::Field(105), Kind::Big),
    c("effectMiscValue1", Source::Field(106), Kind::Signed),
    c("effectMiscValue2", Source::Field(107), Kind::Signed),
    c("effectMiscValue3", Source::Field(108), Kind::Signed),
    c("effectTriggerSpell1", Source::Field(109), Kind::Unsigned),
    c("effectTriggerSpell2", Source::Field(110), Kind::Unsigned),
    c("effectTriggerSpell3", Source::Field(111), Kind::Unsigned),
    c("effectPointsPerComboPoint1", Source::Field(112), Kind::Float),
    c("effectPointsPerComboPoint2", Source::Field(113), Kind::Float),
    c("effectPointsPerComboPoint3", Source::Field(114), Kind::Float),
    c("spellVisual1", Source::Field(115), Kind::Unsigned),
    c("spellVisual2", Source::Field(116), Kind::Unsigned),
    c("spellIconId", Source::Field(117), Kind::Unsigned),
    c("activeIconId", Source::Field(118), Kind::Unsigned),
    c("spellPriority", Source::Field(119), Kind::Unsigned),
    c("name", Source::Field(120), Kind::Text),
    c("nameFlags", Source::Field(128), Kind::Unsigned),
    c("nameSubtext", Source::Field(129), Kind::Text),
    c("nameSubtextFlags", Source::Field(137), Kind::Unsigned),
    c("description", Source::Field(138), Kind::Text),
    c("descriptionFlags", Source::Field(146), Kind::Unsigned),
    c("auraDescription", Source::Field(147), Kind::Text),
    c("auraDescriptionFlags", Source::Field(155), Kind::Unsigned),
    c("manaCostPercentage", Source::Field(156), Kind::Unsigned),
    c("startRecoveryCategory", Source::Field(157), Kind::Unsigned),
    c("startRecoveryTime", Source::Field(158), Kind::Unsigned),
    c("minTargetLevel", Source::Server, Kind::Unsigned),
    c("maxTargetLevel", Source::Field(159), Kind::Unsigned),
    c("spellFamilyName", Source::Field(160), Kind::Unsigned),
    c("spellFamilyFlags", Source::Pair(161, 162), Kind::Big),
    c("maxAffectedTargets", Source::Field(163), Kind::Unsigned),
    c("dmgClass", Source::Field(164), Kind::Unsigned),
    c("preventionType", Source::Field(165), Kind::Unsigned),
    c("stanceBarOrder", Source::Field(166), Kind::Signed),
    c("dmgMultiplier1", Source::Field(167), Kind::Float),
    c("dmgMultiplier2", Source::Field(168), Kind::Float),
    c("dmgMultiplier3", Source::Field(169), Kind::Float),
    c("minFactionId", Source::Field(170), Kind::Unsigned),
    c("minReputation", Source::Field(171), Kind::Unsigned),
    c("requiredAuraVision", Source::Field(172), Kind::Unsigned),
    c("customFlags", Source::Server, Kind::Unsigned),
    c("script_name", Source::Server, Kind::Text),
];

/// The column that holds the key, for the two statements that name it.
const ENTRY: &str = "entry";
const BUILD_COLUMN: &str = "build";

/// One column of one row, set to one value.
///
/// [`crate::row::Assignment`] itself, re-exported here because this was the
/// first table with a writer and every reader of the spell half names it
/// through this module. The type is shared because an assignment is the same
/// thing whichever table it lands in — see [`crate::row`], which is what the
/// tables with no client file behind them are written through.
pub use crate::row::Assignment;

/// **What changed between the file the game shipped and the file this project
/// edited**, for one spell.
///
/// `record` is the row *in each file*, which is the same row only while no row
/// has been added or removed above it — so both are looked up by id rather than
/// taken as given. A spell the shipped file does not have at all is a new spell
/// and every column it feeds is a change; see [`insert_new`], which is what a
/// new spell wants instead.
///
/// Returns nothing for a spell whose row is identical, which is the ordinary
/// case for every spell but the one being worked on: a project's `Spell.dbc` is
/// the whole file, so a save that did not ask this question would write 22,360
/// rows.
pub fn changes(shipped: &DbcFile, edited: &DbcFile, entry: u32) -> Vec<Assignment> {
    let Some(new_row) = edited.row_of(entry) else {
        return Vec::new();
    };
    let old_row = shipped.row_of(entry);
    let mut out = Vec::new();
    for column in COLUMNS.iter() {
        let Some(after) = column.value(edited, new_row) else {
            continue;
        };
        // **The key is never assigned.** It is what the statement selects on,
        // and `entry` differing would mean this was asked about two spells.
        if column.name == ENTRY || column.name == BUILD_COLUMN {
            continue;
        }
        let before = old_row.and_then(|row| column.value(shipped, row));
        if before.as_deref() != Some(after.as_str()) {
            out.push(Assignment { column: column.name, value: after });
        }
    }
    out
}

/// **Whether any of these assignments is one vmangos will read.**
///
/// The question a save has to ask before it sends `.reload spell_template`:
/// an edit to a description or a tooltip is thirteen columns' worth of nothing
/// as far as the server is concerned, and a reload reported as having made the
/// edit live would be a green light for a change that reached one side. See
/// [`IGNORED`].
pub fn reaches_the_server(changes: &[Assignment]) -> bool {
    changes.iter().any(|change| !IGNORED.contains(&change.column))
}

/// **Copy whatever row the server is using for this spell to a dev row**, if it
/// has no dev row already.
///
/// This is what keeps [`Source::Server`]'s four columns: the row is copied by
/// the server, inside the server, so the coefficient and the script name come
/// across without this end ever having to know them. The only column replaced
/// is `build`, which is why all 151 are named rather than `SELECT *`.
///
/// The sub-select is `LoadSpells`' own (`SpellMgr.cpp:3619`), so what is copied
/// is exactly the row that was being used a moment ago.
pub fn copy_forward(entry: u32) -> String {
    let selected: Vec<String> = COLUMNS
        .iter()
        .map(|column| match column.name {
            BUILD_COLUMN => BUILD.to_string(),
            name => crate::sql::name(name),
        })
        .collect();
    format!(
        "INSERT IGNORE INTO {table} SELECT {selected} FROM {table} t1 \
         WHERE t1.{entry_col} = {entry} \
         AND t1.{build_col} = (SELECT MAX(t2.{build_col}) FROM {table} t2 \
         WHERE t2.{entry_col} = {entry} AND t2.{build_col} <= {build});",
        table = crate::sql::name(TABLE),
        selected = selected.join(", "),
        entry_col = crate::sql::name(ENTRY),
        build_col = crate::sql::name(BUILD_COLUMN),
        build = BUILD,
    )
}

/// …and build a dev row out of the client's file, for a spell the table has
/// never heard of.
///
/// The four columns the file cannot answer are left to the table's own
/// defaults, which is the honest answer for a spell that did not exist a moment
/// ago: there is no coefficient to preserve and no script to keep.
///
/// `INSERT IGNORE`, so this does nothing when [`copy_forward`] has already made
/// the row — the two are emitted together and exactly one of them ever applies.
pub fn insert_new(dbc: &DbcFile, entry: u32) -> Option<String> {
    let record = dbc.row_of(entry)?;
    let mut names = Vec::new();
    let mut values = Vec::new();
    for column in COLUMNS.iter() {
        let value = match column.name {
            BUILD_COLUMN => BUILD.to_string(),
            _ => match column.value(dbc, record) {
                Some(value) => value,
                None => continue,
            },
        };
        names.push(crate::sql::name(column.name));
        values.push(value);
    }
    Some(format!(
        "INSERT IGNORE INTO {table} ({names}) VALUES ({values});",
        table = crate::sql::name(TABLE),
        names = names.join(", "),
        values = values.join(", "),
    ))
}

/// …and apply what was edited to the dev row.
///
/// `None` for an empty change list rather than a statement that sets nothing:
/// a `.reload` is sent for a table that was written, and an `UPDATE` with no
/// assignments is not a write.
pub fn update(entry: u32, changes: &[Assignment]) -> Option<String> {
    if changes.is_empty() {
        return None;
    }
    let sets: Vec<String> = changes.iter().map(Assignment::sql).collect();
    Some(format!(
        "UPDATE {table} SET {sets} WHERE {entry_col} = {entry} AND {build_col} = {build};",
        table = crate::sql::name(TABLE),
        sets = sets.join(", "),
        entry_col = crate::sql::name(ENTRY),
        build_col = crate::sql::name(BUILD_COLUMN),
        build = BUILD,
    ))
}

/// **The statement that puts one spell back to what the server has right now.**
///
/// Taken *before* an apply, against the dev row as it stands, and there are
/// exactly two cases:
///
/// * **No dev row yet.** The edit is about to create one, so undoing it is
///   removing it — and the server then falls back to the highest shipped build
///   on its own, which is the whole reason this arrangement is reversible at
///   all. `DELETE`.
/// * **A dev row already there.** 1,012 entries ship one on the reference
///   install, and deleting one of those would destroy a row this project never
///   owned. So the undo sets the columns about to change back to what they
///   hold, and touches nothing else. `UPDATE`.
///
/// `now` is that row as `column -> value`, straight out of the database, with
/// `None` for a SQL `NULL`. A column the row does not carry at all is skipped:
/// it cannot be restored to a value nobody read, and guessing zero would be a
/// write dressed as an undo.
pub fn undo(entry: u32, changes: &[Assignment], now: Option<&HashMap<String, Option<String>>>) -> Option<String> {
    if changes.is_empty() || !fits(entry) {
        return None;
    }
    let Some(now) = now else {
        return Some(format!(
            "DELETE FROM {table} WHERE {entry_col} = {entry} AND {build_col} = {build};",
            table = crate::sql::name(TABLE),
            entry_col = crate::sql::name(ENTRY),
            build_col = crate::sql::name(BUILD_COLUMN),
            build = BUILD,
        ));
    };
    let mut sets = Vec::new();
    for change in changes {
        let Some(value) = now.get(change.column) else {
            continue;
        };
        let kind = COLUMNS
            .iter()
            .find(|column| column.name == change.column)
            .map(|column| column.kind);
        let literal = match value {
            None => "NULL".to_string(),
            Some(text) => match kind {
                Some(Kind::Text) => crate::sql::text(text),
                _ => text.clone(),
            },
        };
        sets.push(format!("{} = {literal}", crate::sql::name(change.column)));
    }
    if sets.is_empty() {
        return None;
    }
    Some(format!(
        "UPDATE {table} SET {sets} WHERE {entry_col} = {entry} AND {build_col} = {build};",
        table = crate::sql::name(TABLE),
        sets = sets.join(", "),
        entry_col = crate::sql::name(ENTRY),
        build_col = crate::sql::name(BUILD_COLUMN),
        build = BUILD,
    ))
}

/// The dev row of one spell, for [`undo`] to read before an apply.
pub fn dev_row_query(entry: u32) -> String {
    format!(
        "SELECT * FROM {table} WHERE {entry_col} = {entry} AND {build_col} = {build};",
        table = crate::sql::name(TABLE),
        entry_col = crate::sql::name(ENTRY),
        build_col = crate::sql::name(BUILD_COLUMN),
        build = BUILD,
    )
}

/// **The whole of what a save emits for one edited spell**, in the order it has
/// to be applied.
///
/// Empty when nothing about the spell changed, which is what keeps a save from
/// writing a statement per spell in the file.
pub fn statements(shipped: &DbcFile, edited: &DbcFile, entry: u32) -> Vec<String> {
    // **Refused here, where the statement would be made.** See [`MAX_ENTRY`]:
    // past it the database clamps rather than refusing, so the row lands on
    // another spell and the undo cannot find it.
    if !fits(entry) {
        return Vec::new();
    }
    let changes = changes(shipped, edited, entry);
    let Some(update) = update(entry, &changes) else {
        return Vec::new();
    };
    let mut out = vec![copy_forward(entry)];
    out.extend(insert_new(edited, entry));
    out.push(update);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Spell.dbc` of one record and 173 fields, all zero, with the id set.
    ///
    /// Built rather than read: every property below is about the *mapping*, and
    /// a real table would make each test depend on a spell's actual values.
    fn one_spell(entry: u32) -> DbcFile {
        const FIELDS: usize = 173;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"WDBC");
        bytes.extend_from_slice(&1u32.to_le_bytes()); // records
        bytes.extend_from_slice(&(FIELDS as u32).to_le_bytes());
        bytes.extend_from_slice(&((FIELDS * 4) as u32).to_le_bytes()); // record size
        bytes.extend_from_slice(&1u32.to_le_bytes()); // string block
        bytes.extend_from_slice(&entry.to_le_bytes());
        bytes.extend(std::iter::repeat_n(0u8, (FIELDS - 1) * 4));
        bytes.push(0); // the empty string every text field points at
        DbcFile::parse(&bytes).expect("a table this file builds itself")
    }

    #[test]
    fn the_table_is_the_hundred_and_fifty_one_columns_vmangos_selects() {
        assert_eq!(COLUMNS.len(), 151);
        assert_eq!(COLUMNS[0].name, "entry");
        assert_eq!(COLUMNS[1].name, "build");
        assert_eq!(COLUMNS[150].name, "script_name");
    }

    /// **No two columns read the same field**, which is the mistake the shape
    /// of this table invites: 151 entries transcribed by position, with three
    /// of the server's own in the middle shifting everything after them by
    /// three.
    #[test]
    fn no_dbc_field_feeds_two_columns() {
        let mut seen = std::collections::HashMap::new();
        for column in COLUMNS.iter() {
            let fields: Vec<usize> = match column.source {
                Source::Server => vec![],
                Source::Field(field) => vec![field],
                Source::Pair(low, high) => vec![low, high],
            };
            for field in fields {
                if let Some(other) = seen.insert(field, column.name) {
                    panic!("field {field} feeds both {other} and {}", column.name);
                }
            }
        }
    }

    /// …and every field it names is a field 1.12's `Spell.dbc` has.
    #[test]
    fn every_field_named_is_inside_the_file() {
        for column in COLUMNS.iter() {
            let highest = match column.source {
                Source::Server => continue,
                Source::Field(field) => field,
                Source::Pair(low, high) => low.max(high),
            };
            assert!(highest < 173, "{} reads field {highest}", column.name);
        }
    }

    /// The four the client's file cannot answer, named rather than counted, so
    /// that adding a fifth is a change to this test and not a silent one.
    #[test]
    fn the_server_owns_exactly_these_columns() {
        let server: Vec<&str> = COLUMNS
            .iter()
            .filter(|column| column.source == Source::Server)
            .map(|column| column.name)
            .collect();
        assert_eq!(
            server,
            [
                "build",
                "effectBonusCoefficient1",
                "effectBonusCoefficient2",
                "effectBonusCoefficient3",
                "minTargetLevel",
                "customFlags",
                "script_name",
            ]
        );
    }

    /// An unedited file produces no statements at all. This is the case that
    /// runs on every save of every project, so it is the one that has to be
    /// nothing rather than 22,360 rows.
    #[test]
    fn an_unchanged_spell_emits_nothing() {
        let shipped = one_spell(133);
        let edited = one_spell(133);
        assert!(changes(&shipped, &edited, 133).is_empty());
        assert!(statements(&shipped, &edited, 133).is_empty());
    }

    #[test]
    fn one_edited_field_is_one_assignment() {
        let shipped = one_spell(133);
        let mut edited = one_spell(133);
        // Field 76 is `EffectBasePoints 1`, which is column `effectBasePoints1`.
        edited.set_i32(0, 76, 41);
        let changes = changes(&shipped, &edited, 133);
        assert_eq!(
            changes,
            [Assignment { column: "effectBasePoints1", value: "41".into() }]
        );
        let statements = statements(&shipped, &edited, 133);
        assert_eq!(statements.len(), 3);
        assert!(statements[2].contains("SET `effectBasePoints1` = 41"));
        assert!(statements[2].contains("WHERE `entry` = 133 AND `build` = 5875"));
    }

    /// **A signed field is written signed.** `effectBasePoints1` is `int(4)`
    /// and a debuff's base points are negative; read as unsigned it would be
    /// 4,294,967,255 and the column would refuse or wrap.
    #[test]
    fn a_negative_base_point_stays_negative() {
        let shipped = one_spell(89);
        let mut edited = one_spell(89);
        edited.set_i32(0, 76, -46);
        assert_eq!(changes(&shipped, &edited, 89)[0].value, "-46");
    }

    /// The two halves of `spellFamilyFlags` become one 64-bit value, high word
    /// above low — the one column in the table the DBC splits.
    #[test]
    fn the_family_flags_are_rejoined_high_above_low() {
        let shipped = one_spell(133);
        let mut edited = one_spell(133);
        edited.set_u32(0, 161, 0x0000_0001);
        edited.set_u32(0, 162, 0x0000_0002);
        let changes = changes(&shipped, &edited, 133);
        assert_eq!(
            changes,
            [Assignment { column: "spellFamilyFlags", value: (0x2_0000_0001u64).to_string() }]
        );
    }

    /// Nothing ever assigns a column the client's file does not feed, however
    /// the file changed. This is the coefficient rule, as a test.
    #[test]
    fn no_statement_ever_writes_a_column_the_server_owns() {
        let shipped = one_spell(133);
        let mut edited = one_spell(133);
        for field in 0..173 {
            edited.set_u32(0, field, 7);
        }
        edited.set_u32(0, 0, 133);
        let changed = changes(&shipped, &edited, 133);
        assert!(!changed.is_empty());
        for assignment in &changed {
            let column = COLUMNS
                .iter()
                .find(|column| column.name == assignment.column)
                .expect("an assignment names a column of the table");
            assert_ne!(column.source, Source::Server, "{}", column.name);
        }
    }

    /// `copy_forward` names all 151 columns, because the `SELECT` has to line
    /// up with the table positionally — and replaces exactly one of them.
    #[test]
    fn the_copy_replaces_the_build_and_nothing_else() {
        let sql = copy_forward(133);
        for column in COLUMNS.iter() {
            if column.name == "build" {
                continue;
            }
            assert!(sql.contains(&format!("`{}`", column.name)), "{}", column.name);
        }
        assert!(sql.starts_with("INSERT IGNORE INTO `spell_template` SELECT `entry`, 5875, "));
        // …from the row the server is using now, which is `LoadSpells`' own
        // sub-select.
        assert!(sql.contains("MAX(t2.`build`)"));
        assert!(sql.contains("t2.`build` <= 5875"));
    }

    /// **A spell with no dev row is undone by removing the one about to be
    /// made**, and the server then falls back to the highest shipped build on
    /// its own. That is the whole reason this arrangement is reversible.
    #[test]
    fn undoing_a_row_this_project_creates_is_removing_it() {
        let changes = [Assignment { column: "effectBasePoints1", value: "41".into() }];
        let sql = undo(2136, &changes, None).expect("something to undo");
        assert_eq!(
            sql,
            "DELETE FROM `spell_template` WHERE `entry` = 2136 AND `build` = 5875;"
        );
    }

    /// …and a spell that **already had** one is undone by putting the columns
    /// back. 1,012 entries ship a 5875 row on the reference install, and
    /// deleting one of those would destroy a row this project never owned.
    #[test]
    fn undoing_a_row_that_was_already_there_restores_the_columns() {
        let mut now = HashMap::new();
        now.insert("effectBasePoints1".to_string(), Some("13".to_string()));
        now.insert("recoveryTime".to_string(), Some("0".to_string()));
        let changes = [Assignment { column: "effectBasePoints1", value: "41".into() }];
        let sql = undo(133, &changes, Some(&now)).expect("something to undo");
        assert_eq!(
            sql,
            "UPDATE `spell_template` SET `effectBasePoints1` = 13 \
             WHERE `entry` = 133 AND `build` = 5875;"
        );
        // …and only the columns about to change. A column the row also holds
        // but nothing is editing is not named.
        assert!(!sql.contains("recoveryTime"));
    }

    /// A text column is quoted on the way back, and a NULL stays NULL.
    #[test]
    fn a_restored_value_is_written_as_its_column_takes_it() {
        let mut now = HashMap::new();
        now.insert("name".to_string(), Some("Fireball's".to_string()));
        now.insert("maxLevel".to_string(), None);
        let changes = [
            Assignment { column: "name", value: "'x'".into() },
            Assignment { column: "maxLevel", value: "9".into() },
        ];
        let sql = undo(133, &changes, Some(&now)).expect("something to undo");
        assert!(sql.contains("`name` = 'Fireball\\'s'"), "{sql}");
        assert!(sql.contains("`maxLevel` = NULL"), "{sql}");
    }

    /// **A column the row does not carry is skipped rather than zeroed.** It
    /// cannot be restored to a value nobody read, and writing a default would
    /// be an edit wearing an undo's name.
    #[test]
    fn a_column_the_row_does_not_carry_is_not_guessed_at() {
        let now = HashMap::new();
        let changes = [Assignment { column: "effectBasePoints1", value: "41".into() }];
        assert!(undo(133, &changes, Some(&now)).is_none());
    }

    #[test]
    fn nothing_changed_is_nothing_to_undo() {
        assert!(undo(133, &[], None).is_none());
        assert!(undo(133, &[], Some(&HashMap::new())).is_none());
    }

    /// **A spell id the server's table cannot hold produces no statement.**
    ///
    /// The one that was found the expensive way: `spell_template.entry` is a
    /// `smallint unsigned` and MySQL clamps rather than refusing, so writing
    /// 90,210 made a row at 65,535 — a real row for a different spell, with an
    /// undo beside it naming 90,210 and matching nothing.
    #[test]
    fn a_spell_the_table_cannot_hold_is_refused_rather_than_clamped() {
        assert!(fits(MAX_ENTRY));
        assert!(!fits(MAX_ENTRY + 1));
        assert!(!fits(90210));

        let shipped = one_spell(133);
        let mut edited = one_spell(90210);
        edited.set_i32(0, 76, 41);
        // The change is real — the file says so — and the statement is not
        // made, because there is nowhere for it to land.
        assert!(!changes(&shipped, &edited, 90210).is_empty());
        assert!(statements(&shipped, &edited, 90210).is_empty());
        // …and no undo is written for it either, which is what stopped the
        // last one being recoverable.
        let changes = changes(&shipped, &edited, 90210);
        assert!(undo(90210, &changes, None).is_none());
    }

    #[test]
    fn the_dev_row_is_asked_for_by_both_halves_of_its_key() {
        let sql = dev_row_query(133);
        assert!(sql.contains("`entry` = 133"));
        assert!(sql.contains("`build` = 5875"));
    }

    /// Every name in [`IGNORED`] is a column of the table, which is what keeps
    /// the two lists tied: a column renamed on one side and not the other
    /// silently stops being ignored.
    #[test]
    fn every_ignored_column_is_a_column() {
        for name in IGNORED {
            assert!(
                COLUMNS.iter().any(|column| column.name == name),
                "{name} is not a column of {TABLE}"
            );
        }
    }

    /// An edit only to columns nothing reads does not reach the server, and
    /// says so. A description is the case a person meets.
    #[test]
    fn an_edited_description_reaches_nothing_on_the_server() {
        let shipped = one_spell(133);
        let mut edited = one_spell(133);
        // Field 138 is `Description`, which is column `description`.
        edited.set_string(0, 138, "a new description");
        let changed = changes(&shipped, &edited, 133);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].column, "description");
        assert!(!reaches_the_server(&changed));
        // …and one real field beside it does.
        edited.set_u32(0, 19, 42);
        assert!(reaches_the_server(&changes(&shipped, &edited, 133)));
    }

    /// A new spell is inserted whole, minus the columns the file cannot answer.
    #[test]
    fn a_new_spell_is_inserted_without_the_columns_the_file_cannot_answer() {
        let dbc = one_spell(90000);
        let sql = insert_new(&dbc, 90000).expect("the record is there");
        assert!(sql.contains("`build`"));
        assert!(sql.contains("`effect1`"));
        for absent in ["effectBonusCoefficient1", "minTargetLevel", "customFlags", "script_name"] {
            assert!(!sql.contains(absent), "{absent}");
        }
    }

    #[test]
    fn a_spell_the_edited_file_does_not_have_is_no_statements() {
        let shipped = one_spell(133);
        let edited = one_spell(133);
        assert!(statements(&shipped, &edited, 999).is_empty());
        assert!(insert_new(&edited, 999).is_none());
    }

    /// A spell the *shipped* file does not have is new, and every column it
    /// feeds counts as changed.
    #[test]
    fn a_spell_only_the_edited_file_has_is_changed_throughout() {
        let shipped = one_spell(133);
        let edited = one_spell(90000);
        let changed = changes(&shipped, &edited, 90000);
        let fed = COLUMNS
            .iter()
            .filter(|column| column.source != Source::Server && column.name != "entry")
            .count();
        assert_eq!(changed.len(), fed);
    }
}
