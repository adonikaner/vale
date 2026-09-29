//! `quest_template`: what a quest asks for, what it says and what it gives —
//! and the four relation tables that say who hands it out and who takes it.
//!
//! The fourth table in this crate with no client file behind it. The client
//! learns a quest by asking (`CMSG_QUEST_QUERY`) and keeps the answer in
//! `WDB\`, so the row in vmangos' database is the only copy there is.
//!
//! ## 131 columns, in `LoadQuests`' own order
//!
//! [`TEMPLATE_COLUMNS`] is the `SELECT` at `ObjectMgr.cpp:5318`, column for
//! column and in its order, for [`crate::item`]'s reason: it is the list to
//! compare against the source when vmangos adds a column. That order is the
//! `Quest` constructor's and not a reader's — `MaxLevel`, `RewXP` and
//! `RequiredCondition` are the last few it selects — so the form draws by
//! [`Group`] and never by position.
//!
//! ## The patch column is part of the key
//!
//! `quest_template`'s primary key is `(entry, patch)` and `LoadQuests` takes the
//! highest `patch` at or below the server's `WowPatch`, exactly as
//! `item_template` does. The reference database holds 4,725 rows for 4,433
//! quests: 287 quests have more than one version.
//!
//! Deleting the winning row of a quest with two versions does not remove the
//! quest: the server loads the older one instead. [`delete_statements`]
//! therefore names the entry alone and takes every version, and the rows of
//! [`DEPENDENTS`] with it.
//!
//! ## A quest is created, edited and removed, and all three are live on a reload
//!
//! `.reload quest_template` calls `ObjectMgr::LoadQuests`, whose first statement
//! is `m_QuestTemplatesMap.clear()` (`ObjectMgr.cpp:5313`), so a row that has
//! gone from the table is gone from the server. What made an item's removal
//! unsafe is absent here: `Item::GetProto` is dereferenced unchecked, and every
//! one of `Player.cpp`'s thirty-two `GetQuestTemplate` calls tests the pointer
//! before using it. A character holding a removed quest keeps the row in
//! `character_queststatus` and the server skips it.
//!
//! A reload has a cost for any quest change, not only for a removal: the map
//! holds each `Quest` by `unique_ptr`, so clearing it frees every one,
//! and two script bases keep a `Quest const*` across frames —
//! `m_pQuestForEscort` (`ScriptedEscortAI.h:119`) and `m_pQuestForFollow`
//! (`ScriptedFollowerAI.h:68`). An escort in progress while the table is
//! reloaded reads freed memory when it completes. A playtest server with no
//! escort running is unaffected.
//!
//! The relation tables are the same: `LoadQuestRelationsHelper` begins with
//! `map.clear()` (`ObjectMgr.cpp:8882`). It also drops, with one line in the
//! log, any relation whose quest is not in the template map — so a reload of
//! the relations has to follow the reload of the templates, never precede it.
//! [`RELOAD_ORDER`] is that order.
//!
//! ## A relation row is a key and two patch columns
//!
//! ```text
//! creature_questrelation      id, quest   who hands a quest out
//! creature_involvedrelation   id, quest   who takes it back
//! gameobject_questrelation    id, quest   …and the same for a game object
//! gameobject_involvedrelation id, quest
//! ```
//!
//! `(id, quest)` is the primary key of all four and `id` is a *template* entry,
//! not a spawn guid. The other two columns are `patch_min` and `patch_max`, and
//! the loader's filter is `WowPatch BETWEEN patch_min AND patch_max` — a
//! different column pair from the template's own `patch`, and a row outside the
//! band is a row the server never sees. A relation made here is written
//! `0..10`, which is the DDL's default and every patch.
//!
//! ## Three columns whose sign is part of the value
//!
//! ```text
//! ZoneOrSort           > 0 an AreaTable.dbc id, < 0 a QuestSort.dbc id negated
//! ReqCreatureOrGOId    > 0 a creature_template entry, < 0 a gameobject_template
//!                      entry negated
//! PrevQuestId          > 0 that quest must be rewarded, < 0 that quest must be
//!                      active; NextQuestId is read the same way
//! RewOrReqMoney        > 0 copper the quest pays, < 0 copper it costs to hand in
//! ```
//!
//! [`Kind::Either`] and [`Kind::SignedMoney`] are how the schema says so.

use crate::row::{Assignment, Key, Life};

pub use crate::schema::{
    mask_words, money_words, seconds_words, value_word, Bit, Column, Group, Kind, Row, RowValue,
    Value,
};

/// What a quest is.
pub const TEMPLATE: &str = "quest_template";

/// Which creature hands a quest out.
pub const CREATURE_GIVES: &str = "creature_questrelation";

/// …which takes it back,
pub const CREATURE_TAKES: &str = "creature_involvedrelation";

/// …and the same two for a game object.
pub const OBJECT_GIVES: &str = "gameobject_questrelation";
pub const OBJECT_TAKES: &str = "gameobject_involvedrelation";

/// The four relation tables.
pub const RELATIONS: [&str; 4] = [CREATURE_GIVES, CREATURE_TAKES, OBJECT_GIVES, OBJECT_TAKES];

/// Every table this module writes, for a caller that has to resolve a name read
/// out of a file back to one of these constants — see
/// [`crate::creature::TABLES`], where the reason is.
pub const TABLES: [&str; 5] = [
    TEMPLATE,
    CREATURE_GIVES,
    CREATURE_TAKES,
    OBJECT_GIVES,
    OBJECT_TAKES,
];

/// The order the tables are reloaded in: the templates first, because the
/// relation loader drops a row whose quest it cannot find — see the module
/// comment.
pub const RELOAD_ORDER: [&str; 5] = TABLES;

/// Every table that names a quest by id and is removed with it, as the
/// table and the column the id is in.
///
/// `locales_quest` is the translated text and `areatrigger_involvedrelation`
/// the exploration objectives. Three more tables name a quest and are left
/// alone, because a row of theirs is about something else that happens to
/// mention one: `game_event_quest`, `game_event_mail` and `script_escort_data`.
pub const DEPENDENTS: [(&str, &str); 6] = [
    (CREATURE_GIVES, "quest"),
    (CREATURE_TAKES, "quest"),
    (OBJECT_GIVES, "quest"),
    (OBJECT_TAKES, "quest"),
    ("areatrigger_involvedrelation", "quest"),
    ("locales_quest", "entry"),
];

/// Every column of the world database that names a quest by entry, which
/// is what follows a quest whose entry changes — see
/// [`crate::row::move_statements`], and `crate::item::REFERENCES` for the item
/// half's own list.
///
/// [`DEPENDENTS`] and more: a removal takes only the rows that are *about* the
/// quest, and a move has to take every row that *mentions* it, or the mention
/// is left pointing at nothing. `PrevQuestId` and `NextQuestId` name a quest by
/// its entry or by the negative of it, so each is listed both ways
/// (`ObjectMgr.cpp:5936` reads `abs(NextQuestId)`). Read off the reference
/// install's `information_schema`.
///
/// Not reached: `character_queststatus` in the `characters` database, which
/// is another connection, so a character half way through a moved quest loses
/// it; and `quest_start_scripts`/`quest_end_scripts`, whose `id` is a script id
/// that a quest names in `StartScript`/`CompleteScript` and that only happens
/// to equal the entry.
pub const REFERENCES: [crate::row::Reference; 19] = {
    use crate::row::Reference as R;
    [
        R::new(CREATURE_GIVES, "quest"),
        R::new(CREATURE_TAKES, "quest"),
        R::new(OBJECT_GIVES, "quest"),
        R::new(OBJECT_TAKES, "quest"),
        R::new("areatrigger_involvedrelation", "quest"),
        R::new("locales_quest", "entry"),
        R::new("game_event_quest", "quest"),
        R::new("game_event_mail", "quest"),
        R::new("script_escort_data", "quest"),
        R::new("spell_area", "quest_start"),
        R::new("spell_area", "quest_end"),
        R::new("item_template", "start_quest"),
        R::new("player_factionchange_quests", "alliance_id"),
        R::new("player_factionchange_quests", "horde_id"),
        R::new(TEMPLATE, "NextQuestInChain"),
        R::new(TEMPLATE, "PrevQuestId"),
        R::negated(TEMPLATE, "PrevQuestId"),
        R::new(TEMPLATE, "NextQuestId"),
        R::negated(TEMPLATE, "NextQuestId"),
    ]
};

/// The references a move of a row of `table` takes along: a quest's, and
/// nothing for a relation, whose key is not an id anything names.
pub fn references(table: &str) -> &'static [crate::row::Reference] {
    match table == TEMPLATE {
        true => &REFERENCES,
        false => &[],
    }
}

/// The static name for a table read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// Whether a table is one of the four relations.
pub fn is_relation(table: &str) -> bool {
    RELATIONS.contains(&table)
}

// ---------------------------------------------------------------------------
// The enumerations, each from vmangos' own header
// ---------------------------------------------------------------------------

/// `QuestMethod`, from `QuestDef.h:177`. 2 is the ordinary quest; 0 completes
/// the moment it is accepted, which is how a quest that only hands something
/// over is written.
pub const METHODS: [Value; 3] = [
    Value { value: 0, name: "Auto-complete" },
    Value { value: 1, name: "Disabled" },
    Value { value: 2, name: "Normal" },
];

/// `QuestTypes`, from `QuestDef.h:133`: the ids of `QuestInfo.dbc`, which is
/// where the client reads the word in brackets after a quest's level.
pub const TYPES: [Value; 9] = [
    Value { value: 0, name: "None" },
    Value { value: 1, name: "Elite" },
    Value { value: 21, name: "Life" },
    Value { value: 41, name: "PvP" },
    Value { value: 62, name: "Raid" },
    Value { value: 81, name: "Dungeon" },
    Value { value: 82, name: "World Event" },
    Value { value: 83, name: "Legendary" },
    Value { value: 84, name: "Escort" },
];

/// `QuestFlags`, from `QuestDef.h:146`, with the header's own comments.
pub const FLAGS: [Bit; 9] = [
    Bit { bit: 0x001, name: "STAY_ALIVE", about: "not used currently" },
    Bit { bit: 0x002, name: "PARTY_ACCEPT", about: "every party member who can take it is asked to" },
    Bit { bit: 0x004, name: "EXPLORATION", about: "not used currently" },
    Bit { bit: 0x008, name: "SHARABLE", about: "can be shared: Player::CanShareQuest" },
    Bit { bit: 0x020, name: "EPIC", about: "not used currently" },
    Bit { bit: 0x040, name: "RAID", about: "not used currently" },
    Bit { bit: 0x100, name: "UNK2", about: "not used currently" },
    Bit { bit: 0x200, name: "HIDDEN_REWARDS", about: "rewards are sent only when it is handed in" },
    Bit { bit: 0x400, name: "AUTO_REWARDED", about: "rewarded on completion and never shown in the log" },
];

/// `QuestSpecialFlags`, from `QuestDef.h:163`. Only the first two may be set in
/// the database (`QUEST_SPECIAL_FLAG_DB_ALLOWED`); the rest are computed at
/// load and a row carrying one is reported by `LoadQuests` and masked.
pub const SPECIAL_FLAGS: [Bit; 2] = [
    Bit { bit: 0x1, name: "REPEATABLE", about: "can be taken again once rewarded" },
    Bit {
        bit: 0x2,
        name: "EXPLORATION_OR_EVENT",
        about: "completed by an area trigger, a SPELL_EFFECT_QUEST_COMPLETE or a script",
    },
];

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// A reference to an item, which most of the numbered columns are.
const ITEM: Kind = Kind::Ref(crate::item::TEMPLATE);

/// …to a spell, a faction and another quest.
const SPELL: Kind = Kind::Ref("Spell");
const FACTION: Kind = Kind::Ref("Faction");
const QUEST: Kind = Kind::Ref(TEMPLATE);

/// A creature, or a game object negated — see the module comment.
const CREATURE_OR_OBJECT: Kind =
    Kind::Either(crate::creature::TEMPLATE, "gameobject_template");

/// A quest whose sign says which state it must be in.
const QUEST_SIGNED: Kind = Kind::Either(TEMPLATE, TEMPLATE);

/// An emote the giver plays: a row of `Emotes.dbc`.
const EMOTE: Kind = Kind::Ref("Emotes");

/// `quest_template`'s 131 columns, in the order `ObjectMgr::LoadQuests`
/// selects them (`ObjectMgr.cpp:5318`).
///
/// `patch` is not among them: it is half the key, and [`template_key`] is where
/// it goes.
pub const TEMPLATE_COLUMNS: [Column; 131] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the quest id" },
    Column { name: "Method", kind: Kind::Choice(&METHODS), group: Group::Identity, about: "QuestMethod: 2 is an ordinary quest, 0 completes when it is accepted, 1 is switched off" },
    Column { name: "ZoneOrSort", kind: Kind::Either("AreaTable", "QuestSort"), group: Group::Identity, about: "the heading it goes under in the log: an AreaTable id, or a QuestSort id negated" },
    Column { name: "MinLevel", kind: Kind::Unsigned, group: Group::Requirements, about: "the level a character must be to be offered it" },
    Column { name: "QuestLevel", kind: Kind::Unsigned, group: Group::Identity, about: "the level it is coloured and rewarded at" },
    Column { name: "Type", kind: Kind::Choice(&TYPES), group: Group::Identity, about: "QuestInfo.dbc id: the word in brackets after the level" },
    Column { name: "RequiredClasses", kind: Kind::Flags(&crate::item::CLASS_MASK), group: Group::Requirements, about: "which classes are offered it; 0 is all of them" },
    Column { name: "RequiredRaces", kind: Kind::Flags(&crate::item::RACE_MASK), group: Group::Requirements, about: "which races are offered it; 0 is all of them" },
    Column { name: "RequiredSkill", kind: Kind::Ref("SkillLine"), group: Group::Requirements, about: "SkillLine.dbc id the character must have" },
    Column { name: "RequiredSkillValue", kind: Kind::Unsigned, group: Group::Requirements, about: "…and how much of it" },
    Column { name: "RepObjectiveFaction", kind: FACTION, group: Group::Objectives, about: "Faction.dbc id a standing must be reached with, as an objective" },
    Column { name: "RepObjectiveValue", kind: Kind::Signed, group: Group::Objectives, about: "…and the reputation that completes it" },
    Column { name: "RequiredMinRepFaction", kind: FACTION, group: Group::Requirements, about: "Faction.dbc id a standing is needed with before it is offered" },
    Column { name: "RequiredMinRepValue", kind: Kind::Signed, group: Group::Requirements, about: "…and the least reputation with it" },
    Column { name: "RequiredMaxRepFaction", kind: FACTION, group: Group::Requirements, about: "Faction.dbc id a standing must stay under" },
    Column { name: "RequiredMaxRepValue", kind: Kind::Signed, group: Group::Requirements, about: "…and the reputation it must be below" },
    Column { name: "SuggestedPlayers", kind: Kind::Unsigned, group: Group::Identity, about: "the party size the log suggests" },
    Column { name: "LimitTime", kind: Kind::Seconds, group: Group::Objectives, about: "seconds to finish it in; 0 is untimed" },
    Column { name: "QuestFlags", kind: Kind::Flags(&FLAGS), group: Group::Advanced, about: "QuestFlags: sharable, hidden rewards, auto-rewarded" },
    Column { name: "SpecialFlags", kind: Kind::Flags(&SPECIAL_FLAGS), group: Group::Advanced, about: "vmangos' own: repeatable, and completed by an event rather than an objective" },
    Column { name: "PrevQuestId", kind: QUEST_SIGNED, group: Group::Chain, about: "positive: that quest must be rewarded first. Negative: that quest must be active" },
    Column { name: "NextQuestId", kind: QUEST_SIGNED, group: Group::Chain, about: "the quest this one unlocks, read into that quest's own prerequisites at load" },
    Column { name: "ExclusiveGroup", kind: Kind::Signed, group: Group::Chain, about: "positive: only one quest of the group may be taken. Negative: all of the group must be done" },
    Column { name: "NextQuestInChain", kind: QUEST, group: Group::Chain, about: "the quest the giver offers as this one is handed in" },
    Column { name: "SrcItemId", kind: ITEM, group: Group::Objectives, about: "an item handed over when the quest is accepted" },
    Column { name: "SrcItemCount", kind: Kind::Unsigned, group: Group::Objectives, about: "…and how many" },
    Column { name: "SrcSpell", kind: SPELL, group: Group::Objectives, about: "a spell cast on the character when the quest is accepted" },
    Column { name: "Title", kind: Kind::Text, group: Group::Identity, about: "what the log calls it" },
    Column { name: "Details", kind: Kind::Paragraph, group: Group::Text, about: "what the giver says when offering it. $B is a line break, $N the character's name, $C the class, $R the race" },
    Column { name: "Objectives", kind: Kind::Paragraph, group: Group::Text, about: "the summary in the log, under the title" },
    Column { name: "OfferRewardText", kind: Kind::Paragraph, group: Group::Text, about: "what the taker says when it is complete and the reward is offered" },
    Column { name: "RequestItemsText", kind: Kind::Paragraph, group: Group::Text, about: "what the taker says while it is not complete" },
    Column { name: "EndText", kind: Kind::Text, group: Group::Text, about: "the objective line of a quest completed by an event, where there is no item or kill to count" },
    Column { name: "ObjectiveText1", kind: Kind::Text, group: Group::Objectives, about: "what the first kill, use or cast objective is called, in place of the creature's or object's own name" },
    Column { name: "ObjectiveText2", kind: Kind::Text, group: Group::Objectives, about: "…the second" },
    Column { name: "ObjectiveText3", kind: Kind::Text, group: Group::Objectives, about: "…the third" },
    Column { name: "ObjectiveText4", kind: Kind::Text, group: Group::Objectives, about: "…the fourth" },
    Column { name: "ReqItemId1", kind: ITEM, group: Group::Objectives, about: "an item to bring" },
    Column { name: "ReqItemId2", kind: ITEM, group: Group::Objectives, about: "a second" },
    Column { name: "ReqItemId3", kind: ITEM, group: Group::Objectives, about: "a third" },
    Column { name: "ReqItemId4", kind: ITEM, group: Group::Objectives, about: "a fourth" },
    Column { name: "ReqItemCount1", kind: Kind::Unsigned, group: Group::Objectives, about: "how many of the first item" },
    Column { name: "ReqItemCount2", kind: Kind::Unsigned, group: Group::Objectives, about: "…of the second" },
    Column { name: "ReqItemCount3", kind: Kind::Unsigned, group: Group::Objectives, about: "…of the third" },
    Column { name: "ReqItemCount4", kind: Kind::Unsigned, group: Group::Objectives, about: "…of the fourth" },
    Column { name: "ReqSourceId1", kind: ITEM, group: Group::Objectives, about: "an item the quest needs in the bags without asking for it: what a required item is made from" },
    Column { name: "ReqSourceId2", kind: ITEM, group: Group::Objectives, about: "a second" },
    Column { name: "ReqSourceId3", kind: ITEM, group: Group::Objectives, about: "a third" },
    Column { name: "ReqSourceId4", kind: ITEM, group: Group::Objectives, about: "a fourth" },
    Column { name: "ReqSourceCount1", kind: Kind::Unsigned, group: Group::Objectives, about: "how many of the first source item may be held" },
    Column { name: "ReqSourceCount2", kind: Kind::Unsigned, group: Group::Objectives, about: "…of the second" },
    Column { name: "ReqSourceCount3", kind: Kind::Unsigned, group: Group::Objectives, about: "…of the third" },
    Column { name: "ReqSourceCount4", kind: Kind::Unsigned, group: Group::Objectives, about: "…of the fourth" },
    Column { name: "ReqCreatureOrGOId1", kind: CREATURE_OR_OBJECT, group: Group::Objectives, about: "a creature to kill, or a game object to use when negative" },
    Column { name: "ReqCreatureOrGOId2", kind: CREATURE_OR_OBJECT, group: Group::Objectives, about: "a second" },
    Column { name: "ReqCreatureOrGOId3", kind: CREATURE_OR_OBJECT, group: Group::Objectives, about: "a third" },
    Column { name: "ReqCreatureOrGOId4", kind: CREATURE_OR_OBJECT, group: Group::Objectives, about: "a fourth" },
    Column { name: "ReqCreatureOrGOCount1", kind: Kind::Unsigned, group: Group::Objectives, about: "how many of the first" },
    Column { name: "ReqCreatureOrGOCount2", kind: Kind::Unsigned, group: Group::Objectives, about: "…of the second" },
    Column { name: "ReqCreatureOrGOCount3", kind: Kind::Unsigned, group: Group::Objectives, about: "…of the third" },
    Column { name: "ReqCreatureOrGOCount4", kind: Kind::Unsigned, group: Group::Objectives, about: "…of the fourth" },
    Column { name: "ReqSpellCast1", kind: SPELL, group: Group::Objectives, about: "a spell that must be cast on the first creature or object, in place of killing or using it" },
    Column { name: "ReqSpellCast2", kind: SPELL, group: Group::Objectives, about: "…on the second" },
    Column { name: "ReqSpellCast3", kind: SPELL, group: Group::Objectives, about: "…on the third" },
    Column { name: "ReqSpellCast4", kind: SPELL, group: Group::Objectives, about: "…on the fourth" },
    Column { name: "RewChoiceItemId1", kind: ITEM, group: Group::Rewards, about: "an item offered as a choice: one of up to six is taken" },
    Column { name: "RewChoiceItemId2", kind: ITEM, group: Group::Rewards, about: "a second choice" },
    Column { name: "RewChoiceItemId3", kind: ITEM, group: Group::Rewards, about: "a third" },
    Column { name: "RewChoiceItemId4", kind: ITEM, group: Group::Rewards, about: "a fourth" },
    Column { name: "RewChoiceItemId5", kind: ITEM, group: Group::Rewards, about: "a fifth" },
    Column { name: "RewChoiceItemId6", kind: ITEM, group: Group::Rewards, about: "a sixth" },
    Column { name: "RewChoiceItemCount1", kind: Kind::Unsigned, group: Group::Rewards, about: "how many of the first choice" },
    Column { name: "RewChoiceItemCount2", kind: Kind::Unsigned, group: Group::Rewards, about: "…of the second" },
    Column { name: "RewChoiceItemCount3", kind: Kind::Unsigned, group: Group::Rewards, about: "…of the third" },
    Column { name: "RewChoiceItemCount4", kind: Kind::Unsigned, group: Group::Rewards, about: "…of the fourth" },
    Column { name: "RewChoiceItemCount5", kind: Kind::Unsigned, group: Group::Rewards, about: "…of the fifth" },
    Column { name: "RewChoiceItemCount6", kind: Kind::Unsigned, group: Group::Rewards, about: "…of the sixth" },
    Column { name: "RewItemId1", kind: ITEM, group: Group::Rewards, about: "an item always given" },
    Column { name: "RewItemId2", kind: ITEM, group: Group::Rewards, about: "a second" },
    Column { name: "RewItemId3", kind: ITEM, group: Group::Rewards, about: "a third" },
    Column { name: "RewItemId4", kind: ITEM, group: Group::Rewards, about: "a fourth" },
    Column { name: "RewItemCount1", kind: Kind::Unsigned, group: Group::Rewards, about: "how many of the first" },
    Column { name: "RewItemCount2", kind: Kind::Unsigned, group: Group::Rewards, about: "…of the second" },
    Column { name: "RewItemCount3", kind: Kind::Unsigned, group: Group::Rewards, about: "…of the third" },
    Column { name: "RewItemCount4", kind: Kind::Unsigned, group: Group::Rewards, about: "…of the fourth" },
    Column { name: "RewRepFaction1", kind: FACTION, group: Group::Rewards, about: "Faction.dbc id reputation is given with" },
    Column { name: "RewRepFaction2", kind: FACTION, group: Group::Rewards, about: "a second" },
    Column { name: "RewRepFaction3", kind: FACTION, group: Group::Rewards, about: "a third" },
    Column { name: "RewRepFaction4", kind: FACTION, group: Group::Rewards, about: "a fourth" },
    Column { name: "RewRepFaction5", kind: FACTION, group: Group::Rewards, about: "a fifth" },
    Column { name: "RewRepValue1", kind: Kind::Signed, group: Group::Rewards, about: "how much reputation with the first; negative takes it away" },
    Column { name: "RewRepValue2", kind: Kind::Signed, group: Group::Rewards, about: "…with the second" },
    Column { name: "RewRepValue3", kind: Kind::Signed, group: Group::Rewards, about: "…with the third" },
    Column { name: "RewRepValue4", kind: Kind::Signed, group: Group::Rewards, about: "…with the fourth" },
    Column { name: "RewRepValue5", kind: Kind::Signed, group: Group::Rewards, about: "…with the fifth" },
    Column { name: "RewOrReqMoney", kind: Kind::SignedMoney, group: Group::Rewards, about: "positive: copper the quest pays. Negative: copper it costs to hand in" },
    Column { name: "RewMoneyMaxLevel", kind: Kind::Money, group: Group::Rewards, about: "copper paid in place of experience at the level cap" },
    Column { name: "RewSpell", kind: SPELL, group: Group::Rewards, about: "a spell shown as a reward; cast on the character when RewSpellCast is 0" },
    Column { name: "RewSpellCast", kind: SPELL, group: Group::Rewards, about: "the spell actually cast when it is handed in, where that differs from the one shown" },
    Column { name: "RewMailTemplateId", kind: Kind::Unsigned, group: Group::Rewards, about: "MailTemplate.dbc id of a letter sent after it is handed in" },
    Column { name: "RewMailDelaySecs", kind: Kind::Seconds, group: Group::Rewards, about: "…and how long after" },
    Column { name: "PointMapId", kind: Kind::Ref("Map"), group: Group::Advanced, about: "Map.dbc id of a point of interest shown when the quest is accepted" },
    Column { name: "PointX", kind: Kind::Float, group: Group::Advanced, about: "…its x" },
    Column { name: "PointY", kind: Kind::Float, group: Group::Advanced, about: "…its y" },
    Column { name: "PointOpt", kind: Kind::Unsigned, group: Group::Advanced, about: "…and its icon" },
    Column { name: "DetailsEmote1", kind: EMOTE, group: Group::Emotes, about: "Emotes.dbc id the giver plays while Details is on screen" },
    Column { name: "DetailsEmote2", kind: EMOTE, group: Group::Emotes, about: "a second" },
    Column { name: "DetailsEmote3", kind: EMOTE, group: Group::Emotes, about: "a third" },
    Column { name: "DetailsEmote4", kind: EMOTE, group: Group::Emotes, about: "a fourth" },
    Column { name: "DetailsEmoteDelay1", kind: Kind::Millis, group: Group::Emotes, about: "milliseconds before the first" },
    Column { name: "DetailsEmoteDelay2", kind: Kind::Millis, group: Group::Emotes, about: "…the second" },
    Column { name: "DetailsEmoteDelay3", kind: Kind::Millis, group: Group::Emotes, about: "…the third" },
    Column { name: "DetailsEmoteDelay4", kind: Kind::Millis, group: Group::Emotes, about: "…the fourth" },
    Column { name: "IncompleteEmote", kind: EMOTE, group: Group::Emotes, about: "Emotes.dbc id the taker plays while it is not complete" },
    Column { name: "CompleteEmote", kind: EMOTE, group: Group::Emotes, about: "…and when it is" },
    Column { name: "OfferRewardEmote1", kind: EMOTE, group: Group::Emotes, about: "Emotes.dbc id the taker plays while the reward is offered" },
    Column { name: "OfferRewardEmote2", kind: EMOTE, group: Group::Emotes, about: "a second" },
    Column { name: "OfferRewardEmote3", kind: EMOTE, group: Group::Emotes, about: "a third" },
    Column { name: "OfferRewardEmote4", kind: EMOTE, group: Group::Emotes, about: "a fourth" },
    Column { name: "OfferRewardEmoteDelay1", kind: Kind::Millis, group: Group::Emotes, about: "milliseconds before the first" },
    Column { name: "OfferRewardEmoteDelay2", kind: Kind::Millis, group: Group::Emotes, about: "…the second" },
    Column { name: "OfferRewardEmoteDelay3", kind: Kind::Millis, group: Group::Emotes, about: "…the third" },
    Column { name: "OfferRewardEmoteDelay4", kind: Kind::Millis, group: Group::Emotes, about: "…the fourth" },
    Column { name: "StartScript", kind: Kind::Unsigned, group: Group::Advanced, about: "quest_start_scripts id run when it is accepted" },
    Column { name: "CompleteScript", kind: Kind::Unsigned, group: Group::Advanced, about: "quest_end_scripts id run when it is handed in" },
    Column { name: "MaxLevel", kind: Kind::Unsigned, group: Group::Requirements, about: "the level past which it is no longer offered; 0 is no limit" },
    Column { name: "RewMailMoney", kind: Kind::Money, group: Group::Rewards, about: "copper enclosed in the reward letter" },
    Column { name: "RewXP", kind: Kind::Unsigned, group: Group::Rewards, about: "experience given at QuestLevel; the server scales it by the character's level" },
    Column { name: "RequiredCondition", kind: Kind::Unsigned, group: Group::Requirements, about: "conditions id that must hold before it is offered" },
    Column { name: "BreadcrumbForQuestId", kind: QUEST, group: Group::Chain, about: "the quest this one only leads to: it stops being offered once that one is taken" },
    Column { name: "RewRepSpilloverMask", kind: Kind::Unsigned, group: Group::Rewards, about: "which of the five reputation rewards do not spill over to sister factions, a bit each" },
];

/// A relation table's two columns, the key's two aside — see the module
/// comment.
pub const RELATION_COLUMNS: [Column; 4] = [
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the creature_template or gameobject_template entry" },
    Column { name: "quest", kind: Kind::Key, group: Group::Identity, about: "the quest_template entry" },
    Column { name: "patch_min", kind: Kind::Unsigned, group: Group::Identity, about: "the first content patch the row is loaded at" },
    Column { name: "patch_max", kind: Kind::Unsigned, group: Group::Identity, about: "…and the last" },
];

/// The columns of a table, or an empty slice for a name this module does not
/// know.
pub fn columns_of(table: &str) -> &'static [Column] {
    match table {
        TEMPLATE => &TEMPLATE_COLUMNS,
        table if is_relation(table) => &RELATION_COLUMNS,
        _ => &[],
    }
}

/// One column, by name.
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    columns_of(table).iter().find(|column| column.name == name)
}

/// The four numbered groups an objective is spread across, as the column
/// stems a form walks to draw one objective per line: the creature or object,
/// its count, the spell cast on it and what the log calls it.
pub const KILL_STEMS: [&str; 4] =
    ["ReqCreatureOrGOId", "ReqCreatureOrGOCount", "ReqSpellCast", "ObjectiveText"];

/// The key an edit to a quest row is written under.
pub fn template_key(entry: u32, patch: u32) -> Key {
    Key::two(("entry", entry as u64), ("patch", patch as u64))
}

/// …and the key of one relation: who, and which quest.
pub fn relation_key(id: u32, quest: u32) -> Key {
    Key::two(("id", id as u64), ("quest", quest as u64))
}

/// Where a quest this project creates gets its entry from.
///
/// `quest_template.entry` is `mediumint unsigned`, so the ceiling is
/// [`MAX_ENTRY`]. The reference database's highest is 9,665 and upstream
/// numbers its own additions below 100,000; the base is [`crate::item`]'s two
/// million, for one number to remember, and [`MAX_ENTRY_QUERY`] is read beside
/// it because a reserved range means nothing if it is already in use.
pub const RESERVED_ENTRY_BASE: u32 = 2_000_000;

/// The highest entry the column can hold.
pub const MAX_ENTRY: u32 = 0x00FF_FFFF;

/// Every column of a new quest, as the values a row created here starts
/// from.
///
/// `Method` 2, which is the ordinary quest and the DDL's own default; a row
/// of zeros is `Method` 0, which completes the moment it is accepted. Levels
/// 1. The text columns are empty strings rather than the `NULL` the DDL
/// defaults them to, because a form draws an empty box for one and the word
/// `NULL` for the other. Everything else is zero, which for every remaining
/// column means none: no item, no kill, no reward, no prerequisite.
pub fn new_quest(title: &str) -> Vec<Assignment> {
    TEMPLATE_COLUMNS
        .iter()
        .filter(|column| column.editable())
        .map(|column| Assignment {
            column: column.name,
            value: match (column.name, column.kind.is_text()) {
                ("Title", _) => crate::sql::text(title),
                ("Method", _) => "2".to_string(),
                ("MinLevel" | "QuestLevel", _) => "1".to_string(),
                (_, true) => crate::sql::text(""),
                _ => "0".to_string(),
            },
        })
        .collect()
}

/// …and a new relation's two: every content patch.
pub fn new_relation() -> Vec<Assignment> {
    vec![
        Assignment { column: "patch_min", value: "0".to_string() },
        Assignment { column: "patch_max", value: "10".to_string() },
    ]
}

/// Everything a removed quest takes with it, as statements.
///
/// Every version of the template — see the module comment, where the reason
/// the patch is left out of the `WHERE` is — and then each of [`DEPENDENTS`].
pub fn delete_statements(entry: u32) -> Vec<String> {
    let mut out = vec![format!(
        "DELETE FROM {} WHERE `entry` = {entry};",
        crate::sql::name(TEMPLATE)
    )];
    for (table, column) in DEPENDENTS {
        out.push(format!(
            "DELETE FROM {} WHERE {} = {entry};",
            crate::sql::name(table),
            crate::sql::name(column)
        ));
    }
    out
}

/// The statements a save emits for one row this project claims.
///
/// One `UPDATE` for a row it edits, a `DELETE`/`INSERT` pair for one it creates
/// — so applying twice means the same as applying once — and for a removal the
/// template's [`delete_statements`] or the relation's one `DELETE`.
pub fn statements(table: &str, key: &Key, life: Life, changes: &[Assignment]) -> Vec<String> {
    match life {
        Life::Update => crate::row::update(table, key, changes).into_iter().collect(),
        Life::Insert => match crate::row::insert(table, key, changes) {
            Some(statement) => vec![crate::row::delete(table, key), statement],
            None => Vec::new(),
        },
        Life::Delete if table == TEMPLATE => match key.first() {
            Some(entry) => delete_statements(entry as u32),
            None => Vec::new(),
        },
        Life::Delete => vec![crate::row::delete(table, key)],
    }
}

/// Whether a table can hold a row that is created or removed. Every table
/// here can, and every one is live on a reload — see the module comment.
pub fn can_live(table: &str, _life: Life) -> bool {
    table_named(table).is_some()
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

/// The columns a list row and a heading need, and the three that say where a
/// quest sits in a chain — so that *what does this quest unlock* is a walk of
/// the list in memory rather than a query per quest opened.
const BRIEF: &str = "t.`entry`, t.`patch`, t.`Title`, t.`Method`, t.`ZoneOrSort`, t.`MinLevel`, \
     t.`QuestLevel`, t.`Type`, t.`SpecialFlags`, t.`PrevQuestId`, t.`NextQuestId`, \
     t.`NextQuestInChain`";

/// The join that picks the row the server would load — [`crate::item`]'s
/// derived table, one table along, and for its measured reason.
fn winning_join(wow_patch: u32) -> String {
    format!(
        "FROM `quest_template` t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM `quest_template` \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch`"
    )
}

/// Every quest the server would load, briefly — what the list reads once.
pub fn all_quests_query(wow_patch: u32) -> String {
    format!("SELECT {BRIEF} {} ORDER BY t.`entry`", winning_join(wow_patch))
}

/// The whole row the server would load for one entry, with its `patch`.
pub fn winning_template_query(entry: u32, wow_patch: u32) -> String {
    format!(
        "SELECT * FROM `quest_template` t1 WHERE `entry` = {entry} AND `patch` = \
         (SELECT MAX(t2.`patch`) FROM `quest_template` t2 \
          WHERE t2.`entry` = t1.`entry` AND t2.`patch` <= {wow_patch})"
    )
}

/// The row a key names — what an undo is taken from.
pub fn row_query(table: &str, key: &Key) -> String {
    format!(
        "SELECT * FROM {} WHERE {} LIMIT 1",
        crate::sql::name(table),
        key.where_clause()
    )
}

/// Whether a row is already there, which an apply asks before it creates one.
pub fn exists_query(table: &str, key: &Key) -> String {
    format!(
        "SELECT 1 FROM {} WHERE {} LIMIT 1",
        crate::sql::name(table),
        key.where_clause()
    )
}

/// The highest entry the table holds, at any patch.
pub const MAX_ENTRY_QUERY: &str = "SELECT MAX(`entry`) AS `entry` FROM `quest_template`";

/// Every row of one relation table the server would load, which is what the
/// tool reads once: about four thousand rows for the creature tables and a few
/// hundred for the game objects'.
pub fn relations_query(table: &str, wow_patch: u32) -> String {
    format!(
        "SELECT `id`, `quest` FROM {} WHERE {wow_patch} BETWEEN `patch_min` AND `patch_max` \
         ORDER BY `quest`, `id`",
        crate::sql::name(table)
    )
}

/// The name of every creature or game object that gives or takes a quest,
/// for a relation row to be drawn as a name rather than as an entry.
///
/// One query per kind over the ids the relation tables hold, rather than a
/// lookup per row drawn. `creature_template` is keyed by patch and the winning
/// row is picked as everywhere else; `gameobject_template` is as well.
pub fn relation_names_query(creatures: bool, wow_patch: u32) -> String {
    let (template, gives, takes) = match creatures {
        true => (crate::creature::TEMPLATE, CREATURE_GIVES, CREATURE_TAKES),
        false => ("gameobject_template", OBJECT_GIVES, OBJECT_TAKES),
    };
    format!(
        "SELECT t.`entry`, t.`name` FROM {template} t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM {template} \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch` \
         WHERE t.`entry` IN (SELECT `id` FROM {gives} UNION SELECT `id` FROM {takes})",
        template = crate::sql::name(template),
        gives = crate::sql::name(gives),
        takes = crate::sql::name(takes),
    )
}

/// Creatures or game objects whose name contains a term, for the picker a
/// relation is added through. An all-digit term also matches the entry.
pub fn holder_search_query(creatures: bool, term: &str, wow_patch: u32, limit: usize) -> String {
    let template = match creatures {
        true => crate::creature::TEMPLATE,
        false => "gameobject_template",
    };
    let like = crate::sql::text(&format!("%{}%", term.trim()));
    let by_entry = match term.trim().parse::<u32>() {
        Ok(entry) => format!(" OR t.`entry` = {entry}"),
        Err(_) => String::new(),
    };
    format!(
        "SELECT t.`entry`, t.`name` FROM {template} t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM {template} \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch` \
         WHERE t.`name` LIKE {like}{by_entry} ORDER BY t.`name` LIMIT {limit}",
        template = crate::sql::name(template),
    )
}

/// The names of a stated set of creatures or game objects, for the ids a quest's
/// objectives name. `None` for an empty set, which is a query with no `IN` list.
pub fn holder_names_query(creatures: bool, entries: &[u32], wow_patch: u32) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let template = match creatures {
        true => crate::creature::TEMPLATE,
        false => "gameobject_template",
    };
    let list: Vec<String> = entries.iter().map(u32::to_string).collect();
    Some(format!(
        "SELECT t.`entry`, t.`name` FROM {template} t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM {template} \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch` \
         WHERE t.`entry` IN ({})",
        list.join(", "),
        template = crate::sql::name(template),
    ))
}

/// The name, display id and quality of a stated set of items, for the ids a
/// quest's objectives and rewards name.
pub fn item_names_query(entries: &[u32], wow_patch: u32) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let list: Vec<String> = entries.iter().map(u32::to_string).collect();
    Some(format!(
        "SELECT t.`entry`, t.`name`, t.`quality`, t.`display_id` FROM `item_template` t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM `item_template` \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch` \
         WHERE t.`entry` IN ({})",
        list.join(", ")
    ))
}

/// …and items whose name contains a term, for the picker an item column is
/// chosen through. Besides the name, each row carries what the item list
/// draws: the quality, the display id its icon comes from, the class,
/// subclass and slot it is described by, and its item and required levels.
pub fn item_search_query(term: &str, wow_patch: u32, limit: usize) -> String {
    let like = crate::sql::text(&format!("%{}%", term.trim()));
    let by_entry = match term.trim().parse::<u32>() {
        Ok(entry) => format!(" OR t.`entry` = {entry}"),
        Err(_) => String::new(),
    };
    format!(
        "SELECT t.`entry`, t.`name`, t.`quality`, t.`display_id`, t.`class`, t.`subclass`, \
         t.`inventory_type`, t.`item_level`, t.`required_level` FROM `item_template` t \
         JOIN (SELECT `entry`, MAX(`patch`) AS `patch` FROM `item_template` \
               WHERE `patch` <= {wow_patch} GROUP BY `entry`) w \
           ON w.`entry` = t.`entry` AND w.`patch` = t.`patch` \
         WHERE t.`name` LIKE {like}{by_entry} ORDER BY t.`name` LIMIT {limit}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The schema is `LoadQuests`' `SELECT`, column for column. The list is
    /// the query at `ObjectMgr.cpp:5318` with the backticks taken off.
    #[test]
    fn every_column_is_the_servers_own_in_the_servers_own_order() {
        const SELECTED: &str = "entry, Method, ZoneOrSort, MinLevel, QuestLevel, Type, \
            RequiredClasses, RequiredRaces, RequiredSkill, RequiredSkillValue, \
            RepObjectiveFaction, RepObjectiveValue, RequiredMinRepFaction, RequiredMinRepValue, \
            RequiredMaxRepFaction, RequiredMaxRepValue, SuggestedPlayers, LimitTime, \
            QuestFlags, SpecialFlags, PrevQuestId, NextQuestId, ExclusiveGroup, NextQuestInChain, \
            SrcItemId, SrcItemCount, SrcSpell, \
            Title, Details, Objectives, OfferRewardText, RequestItemsText, EndText, \
            ObjectiveText1, ObjectiveText2, ObjectiveText3, ObjectiveText4, \
            ReqItemId1, ReqItemId2, ReqItemId3, ReqItemId4, \
            ReqItemCount1, ReqItemCount2, ReqItemCount3, ReqItemCount4, \
            ReqSourceId1, ReqSourceId2, ReqSourceId3, ReqSourceId4, \
            ReqSourceCount1, ReqSourceCount2, ReqSourceCount3, ReqSourceCount4, \
            ReqCreatureOrGOId1, ReqCreatureOrGOId2, ReqCreatureOrGOId3, ReqCreatureOrGOId4, \
            ReqCreatureOrGOCount1, ReqCreatureOrGOCount2, ReqCreatureOrGOCount3, \
            ReqCreatureOrGOCount4, \
            ReqSpellCast1, ReqSpellCast2, ReqSpellCast3, ReqSpellCast4, \
            RewChoiceItemId1, RewChoiceItemId2, RewChoiceItemId3, RewChoiceItemId4, \
            RewChoiceItemId5, RewChoiceItemId6, \
            RewChoiceItemCount1, RewChoiceItemCount2, RewChoiceItemCount3, RewChoiceItemCount4, \
            RewChoiceItemCount5, RewChoiceItemCount6, \
            RewItemId1, RewItemId2, RewItemId3, RewItemId4, \
            RewItemCount1, RewItemCount2, RewItemCount3, RewItemCount4, \
            RewRepFaction1, RewRepFaction2, RewRepFaction3, RewRepFaction4, RewRepFaction5, \
            RewRepValue1, RewRepValue2, RewRepValue3, RewRepValue4, RewRepValue5, \
            RewOrReqMoney, RewMoneyMaxLevel, RewSpell, RewSpellCast, RewMailTemplateId, \
            RewMailDelaySecs, PointMapId, PointX, PointY, PointOpt, \
            DetailsEmote1, DetailsEmote2, DetailsEmote3, DetailsEmote4, \
            DetailsEmoteDelay1, DetailsEmoteDelay2, DetailsEmoteDelay3, DetailsEmoteDelay4, \
            IncompleteEmote, CompleteEmote, \
            OfferRewardEmote1, OfferRewardEmote2, OfferRewardEmote3, OfferRewardEmote4, \
            OfferRewardEmoteDelay1, OfferRewardEmoteDelay2, OfferRewardEmoteDelay3, \
            OfferRewardEmoteDelay4, \
            StartScript, CompleteScript, MaxLevel, RewMailMoney, RewXP, RequiredCondition, \
            BreadcrumbForQuestId, RewRepSpilloverMask";
        let theirs: Vec<&str> = SELECTED.split(',').map(str::trim).collect();
        let ours: Vec<&str> = TEMPLATE_COLUMNS.iter().map(|column| column.name).collect();
        assert_eq!(theirs.len(), 131);
        assert_eq!(ours, theirs);
    }

    /// `entry` is the only key column, and `patch` is the other half of the key
    /// rather than a column an `INSERT` would name twice.
    #[test]
    fn the_key_is_the_entry_and_the_patch_is_not_a_column() {
        let keys: Vec<&str> = TEMPLATE_COLUMNS
            .iter()
            .filter(|column| !column.editable())
            .map(|column| column.name)
            .collect();
        assert_eq!(keys, vec!["entry"]);
        assert!(column(TEMPLATE, "patch").is_none());
        assert_eq!(template_key(783, 0).where_clause(), "`entry` = 783 AND `patch` = 0");
    }

    /// A new quest names every editable column once, as an ordinary quest of
    /// level 1 with empty text rather than `NULL`.
    #[test]
    fn a_new_quest_names_every_editable_column() {
        let made = new_quest("A Test");
        assert_eq!(made.len(), TEMPLATE_COLUMNS.len() - 1);
        let of = |name: &str| {
            made.iter()
                .find(|change| change.column == name)
                .map(|change| change.value.as_str())
                .unwrap_or("")
        };
        assert_eq!(of("Title"), "'A Test'");
        assert_eq!(of("Method"), "2");
        assert_eq!(of("QuestLevel"), "1");
        assert_eq!(of("Details"), "''");
        assert_eq!(of("RewXP"), "0");
    }

    /// A removal names the entry alone, so every content-patch version goes
    /// and the server cannot fall back to an older one, and takes the six
    /// dependent tables with it.
    #[test]
    fn a_removed_quest_takes_every_version_and_its_dependents() {
        let sql = statements(TEMPLATE, &template_key(783, 5), Life::Delete, &[]);
        assert_eq!(sql.len(), 1 + DEPENDENTS.len());
        assert_eq!(sql[0], "DELETE FROM `quest_template` WHERE `entry` = 783;");
        assert!(sql.contains(&"DELETE FROM `creature_questrelation` WHERE `quest` = 783;".to_string()));
        assert!(sql.contains(&"DELETE FROM `locales_quest` WHERE `entry` = 783;".to_string()));
    }

    /// A relation is created as a `DELETE` and an `INSERT` naming both key
    /// columns and the patch band, and removed as one `DELETE`.
    #[test]
    fn a_relation_is_a_key_and_a_patch_band() {
        let key = relation_key(197, 783);
        let made = statements(CREATURE_GIVES, &key, Life::Insert, &new_relation());
        assert_eq!(
            made,
            vec![
                "DELETE FROM `creature_questrelation` WHERE `id` = 197 AND `quest` = 783;",
                "INSERT INTO `creature_questrelation` (`id`, `quest`, `patch_min`, `patch_max`) \
                 VALUES (197, 783, 0, 10);",
            ]
        );
        assert_eq!(
            statements(CREATURE_TAKES, &key, Life::Delete, &[]),
            vec!["DELETE FROM `creature_involvedrelation` WHERE `id` = 197 AND `quest` = 783;"]
        );
    }

    /// The templates are reloaded before anything that is checked against them.
    #[test]
    fn the_templates_are_reloaded_first() {
        assert_eq!(RELOAD_ORDER[0], TEMPLATE);
        for table in RELATIONS {
            assert!(RELOAD_ORDER.contains(&table));
            assert!(is_relation(table));
        }
        assert!(!is_relation(TEMPLATE));
    }

    /// Every table is this module's and no other's, which is what the editor's
    /// store uses to decide whose row a line is.
    #[test]
    fn no_table_is_shared_with_another_subject() {
        for table in TABLES {
            assert!(crate::creature::table_named(table).is_none(), "{table}");
            assert!(crate::item::table_named(table).is_none(), "{table}");
        }
    }

    /// The signed columns say so, because a form that drew `-22` as an unsigned
    /// id would offer area 4,294,967,274.
    #[test]
    fn the_signed_columns_are_marked() {
        let kind = |name: &str| column(TEMPLATE, name).map(|column| column.kind);
        assert_eq!(kind("ZoneOrSort"), Some(Kind::Either("AreaTable", "QuestSort")));
        assert_eq!(kind("RewOrReqMoney"), Some(Kind::SignedMoney));
        assert!(matches!(kind("ReqCreatureOrGOId3"), Some(Kind::Either(_, "gameobject_template"))));
        assert!(matches!(kind("PrevQuestId"), Some(Kind::Either(_, _))));
    }
}
