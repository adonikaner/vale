//! The read side of the Lua API: C functions that answer during the call.
//!
//! [`self::verbs`] is the write side and it works by recording: a registered
//! closure cannot hold `&mut World`, so `ToggleSheath()` pushes a value the
//! caller drains afterwards. A query cannot work that way.
//! `UnitHealth("target")` must return a value in the middle of a Lua
//! expression:
//!
//! ```lua
//! if ( UnitHealth("target") / UnitHealthMax("target") < 0.2 ) then
//! ```
//!
//! ## A scoped borrow, and a trait that erases the lifetimes
//!
//! `mlua::Lua::scope` creates functions that may capture non-`'static`
//! references and are destroyed when the scope ends. The sequence is:
//!
//! ```text
//! a system holds the world's state       Units, ActionBar, Cooldowns, Time
//!   -> wraps it as one &dyn Answers      Live, below
//!   -> LuaHost::run opens a scope        the functions exist only inside it
//!   -> the chunk runs and asks           UnitHealth("target") reads the live query
//!   -> the scope closes                  and the functions are gone again
//! ```
//!
//! The trait is needed for this to compile: `Live` borrows a `SystemParam` whose
//! two lifetimes would otherwise have to be threaded through every signature in
//! this directory. `&dyn Answers` erases them. It also makes the read side
//! unit-testable with no `World`, since a test can implement `Answers` in
//! twenty lines. Every test in this file does.
//!
//! No Lua may run outside such a scope. A global left pointing at a destroyed
//! scoped function raises "callback destructed" when called. [`super::host`]
//! therefore has one entry point, and it takes an `&dyn Answers`.
//!
//! ## Return values follow the 1.12 conventions: `1`/`nil`, not `true`/`false`
//!
//! A 1.12 API function answers a boolean as `1` or `nil`, not `true`/`false`:
//! the C side pushes a number or nothing. `if ( x )` behaves the same either
//! way, so every FrameXML use of these would work with Lua booleans, but an
//! addon that writes `if UnitAffectingCombat("player") == 1` would stop
//! working. The only thing checked here is that the shipped FrameXML never
//! distinguishes the two, so this file uses the game's convention.
//!
//! One exception is unresolved: the 1.12 API reference this file follows
//! answers `UnitExists` with a Lua boolean and its neighbours with `1`/`nil`, and this file does not know which
//! of the two the client returns for that function. It answers `1`/`nil`
//! throughout, because `== 1` is a comparison an addon can write and
//! `== true` is not.
//!
//! An absent value is `nil`, never a placeholder: `UnitName("party1")` with no
//! party is `nil` and not `""`, because FrameXML asks whether a unit is there
//! with `if ( UnitName(u) ) then`.
//!
//! ```text
//! mod.rs      the `Answers` trait (the sum of the subject traits the panels
//!             declare) and the reads that belong to no panel
//! verbs.rs    the write side: C functions that record an intent
//! stubs.rs    C functions that answer a constant, counted separately
//! cvars.rs    the client's settings (CVars), which every options panel in
//!             the game reads and writes
//! savedvars.rs the forty-four `uvar` rows: Lua globals persisted by
//!             `RegisterForSave` into a file of their own
//! events.rs   events the world sends the interface, and their registrations
//! update.rs   the interface's `OnUpdate` clock, which runs at 30 Hz, not per frame
//! mouse.rs    what the pointer is over, and the five handlers it fires
//! keyboard.rs key routing: a key is either a binding or a character, never both
//! sound.rs    the four sound verbs
//! portrait.rs the verb that draws a unit's portrait onto a texture
//! ```


use crate::interface::action::{ActionBar, Casting, Cooldowns};
use crate::interface::api::{self, UnitId, Units};
use crate::interface::spellbook::Spellbook;
use vale_assets::interface::strings::Strings;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

pub mod cvars;
pub mod events;
pub mod keyboard;
pub mod mouse;
pub mod portrait;
pub mod savedvars;
pub mod sound;
pub mod stubs;
pub mod update;
pub mod verbs;


/// Everything the interface may ask, as one name: the sum of the subject
/// traits listed below. No method is declared here.
///
/// Each subject declares its own trait beside its Lua registration, so a
/// feature's declaration, answer, registration and `READS` entry are in the
/// file the feature is named after. The traits together hold 132 methods.
/// `&dyn Answers` resolves every one of them, because a trait object carries
/// its supertraits' methods.
///
/// The blanket impl below makes adding a subject a one-line change: a new
/// subject trait goes in both lists, and any type that implements all of them
/// is an `Answers` automatically.
pub trait Answers:
    UnitAnswers
    + ActionAnswers
    + super::panels::container::ContainerAnswers
    + super::panels::quest::QuestAnswers
    + super::panels::gossip::GossipAnswers
    + super::panels::merchant::MerchantAnswers
    + super::panels::mail::MailAnswers
    + super::panels::trainer::TrainerAnswers
    + super::panels::tradeskill::TradeSkillAnswers
    + super::panels::craft::CraftAnswers
    + super::panels::taxi::TaxiAnswers
    + super::panels::loot::LootAnswers
    + super::panels::lootroll::LootRollAnswers
    + super::panels::worldmap::MapAnswers
    + super::panels::party::PartyAnswers
    + super::panels::raid::RaidAnswers
    + super::panels::pet::PetAnswers
    + super::panels::shapeshift::ShapeshiftAnswers
    + super::panels::stable::StableAnswers
    + super::panels::bank::BankAnswers
    + super::panels::pagetext::PageTextAnswers
    + super::panels::trade::TradeAnswers
    + super::panels::summon::SummonAnswers
    + super::panels::uioptions::OptionsAnswers
    + super::panels::inspect::InspectAnswers
    + super::panels::guild::GuildAnswers
    + super::panels::glue::GlueAnswers
    + super::panels::spellbook::SpellbookAnswers
    + super::panels::talent::TalentAnswers
    + super::panels::auras::AuraAnswers
{
}

/// Any type that implements every subject trait is an `Answers`.
impl<T> Answers for T where
    T: UnitAnswers
    + ActionAnswers
    + super::panels::container::ContainerAnswers
    + super::panels::quest::QuestAnswers
    + super::panels::gossip::GossipAnswers
    + super::panels::merchant::MerchantAnswers
    + super::panels::mail::MailAnswers
    + super::panels::trainer::TrainerAnswers
    + super::panels::tradeskill::TradeSkillAnswers
    + super::panels::craft::CraftAnswers
    + super::panels::taxi::TaxiAnswers
    + super::panels::loot::LootAnswers
    + super::panels::lootroll::LootRollAnswers
    + super::panels::worldmap::MapAnswers
    + super::panels::party::PartyAnswers
    + super::panels::raid::RaidAnswers
    + super::panels::pet::PetAnswers
    + super::panels::shapeshift::ShapeshiftAnswers
    + super::panels::stable::StableAnswers
    + super::panels::bank::BankAnswers
    + super::panels::pagetext::PageTextAnswers
    + super::panels::trade::TradeAnswers
    + super::panels::summon::SummonAnswers
    + super::panels::uioptions::OptionsAnswers
    + super::panels::inspect::InspectAnswers
    + super::panels::guild::GuildAnswers
    + super::panels::glue::GlueAnswers
    + super::panels::spellbook::SpellbookAnswers
    + super::panels::talent::TalentAnswers
    + super::panels::auras::AuraAnswers
{
}

/// What the interface may ask about a unit, plus the reads about the local
/// player being dead (at the end).
///
/// One method per API function, named after it. Four implementations: [`Live`],
/// which reads the world; `Stub` in this file's own tests; and `Login` and
/// `Ticking` in [`super::audit`], which the headless probes answer from.
/// Neither the trait nor its Lua registration knows which of the four it holds.
///
/// A token this client has no state for answers the absent value rather than
/// a guess. Every `&str` token here goes through
/// [`crate::interface::api::UnitId::parse`].
///
/// The death reads are here rather than in a `DeathAnswers` trait because
/// there is no `lua::death` module for them to live beside: the game's three
/// death boxes are `StaticPopup`s, so their C functions have no file of their
/// own to be registered from. See [`super::api::Answers`] for the subjects
/// that do have one.
pub trait UnitAnswers {
    // --- the clock ---

    /// `GetTime()`: seconds since the client started. The base is set in
    /// [`crate::interface::api::get_time`] and nowhere else.
    fn now(&self) -> f64;

    /// `GetGameTime()`: the world's hour and minute, not the machine's.
    ///
    /// The server states it in `SMSG_LOGIN_SETTIMESPEED` and the session thread
    /// advances it. This reads [`crate::render::sky::WorldClock`], the one place
    /// the hour is decided, so the minimap clock and the sky lighting always
    /// agree. A hand-set hour (`--hour`, the settings panel) is already included
    /// in that value.
    ///
    /// `GameTime.lua` compares the answer against the minute it last drew and
    /// only recomputes its texture coordinates when the two differ. A constant
    /// answer therefore looks like a stopped clock: the frame keeps the 128x64
    /// sheet's default coordinates and draws the day icon and the night icon
    /// side by side.
    fn game_time(&self) -> (u32, u32);

    /// `GetBindLocation()`: the name of the place the hearthstone returns the
    /// character to.
    ///
    /// `HOME_INN` ("your inn") before the bind point has arrived. The 1.12.1
    /// client uses the same fallback, not an empty string, whenever the stored
    /// area id is unset or out of `AreaTable`'s range. See [`home_name`].
    fn bind_location(&self) -> String;

    /// `CheckBinderDist()`: whether the innkeeper who asked is still close
    /// enough.
    ///
    /// `StaticPopupDialogs["CONFIRM_BINDER"]`'s `OnUpdate` hides the box when
    /// this answers false, which removes the question when the player walks
    /// away. True with nothing pending, so a frame between the event and the
    /// popup's first update cannot close it.
    fn binder_in_range(&self) -> bool;

    /// `CheckPetUntrainerDist()`: whether the pet trainer is still close
    /// enough.
    ///
    /// The `CONFIRM_PET_UNLEARN` box's `OnUpdate` hides it when this answers
    /// false, as the binder's does. See [`crate::interface::untrainer`].
    fn untrainer_in_range(&self) -> bool;

    // --- units ---

    fn unit_exists(&self, token: &str) -> bool;
    /// `UnitIsVisible(unit)`: the unit is in the object manager and placed in
    /// the world. A party member across the zone exists and is not visible.
    /// Unit-frame addons ask it before drawing a row. Defaults to
    /// [`Self::unit_exists`] for a test double with no positions.
    fn unit_is_visible(&self, token: &str) -> bool {
        self.unit_exists(token)
    }
    /// `CheckInteractDistance(unit, index)`: within the range of one of the
    /// four interactions: 1 inspect (28 yards), 2 trade (11.11), 3 duel (9.9),
    /// 4 follow (28). The four distances are the ones the 1.12 API references
    /// give for this call; they have not been confirmed against the client.
    /// Defaults to false for a test double with no positions.
    fn unit_in_range(&self, _token: &str, _index: u32) -> bool {
        false
    }
    /// `nil` for a unit that is not there, never `""`.
    fn unit_name(&self, token: &str) -> Option<String>;
    /// `-1` for a level we do not know. The game answers the same, and the
    /// interface draws it as a skull.
    fn unit_level(&self, token: &str) -> i32;
    /// `UnitSex(unit)`: 2 male, 3 female, 1 neuter, and 2 for a unit that is
    /// not there. The table and the fallback are in
    /// [`crate::interface::api::Units::sex`].
    fn unit_sex(&self, token: &str) -> u32;
    fn unit_health(&self, token: &str) -> u32;
    fn unit_health_max(&self, token: &str) -> u32;
    /// `UnitXP` and `UnitXPMax` together. See
    /// [`crate::interface::api::Units::experience`] for why the pair is one answer.
    fn unit_experience(&self, token: &str) -> (u32, u32);
    /// `UnitCharacterPoints(unit)`: unspent talent points, then unspent
    /// profession points. They are one answer for the same reason the XP pair
    /// is: one field pair, read in one call, by two different panels. See
    /// [`crate::interface::api::Units::character_points`].
    fn unit_character_points(&self, token: &str) -> (u32, u32);
    /// `GetXPExhaustion()`: the rested pool, `nil` when there is none.
    fn rested_experience(&self) -> Option<u32>;
    /// `UnitMana`, scaled as the interface shows it: a warrior's rage is
    /// 0..100, not the update field's 0..1000.
    fn unit_mana(&self, token: &str) -> u32;
    fn unit_mana_max(&self, token: &str) -> u32;
    fn unit_power_type(&self, token: &str) -> Option<u8>;
    /// `UnitIsConnected`: false only for a party member the roster says is
    /// offline.
    fn unit_is_connected(&self, token: &str) -> bool;
    fn unit_is_dead(&self, token: &str) -> bool;
    /// `UnitIsGhost`: released, not only dead. The two states show two
    /// different boxes. See [`crate::interface::death`].
    fn unit_is_ghost(&self, token: &str) -> bool;
    fn unit_affecting_combat(&self, token: &str) -> bool;

    // --- being dead ---
    //
    // Five reads, all about the local player, all answered from
    // [`crate::interface::death::Dying`]. They are here rather than in
    // [`self::stubs`] because each one decides what a box shows: the release
    // box's text, whether the Retrieve button counts down, and which of three
    // resurrect popups opens.

    /// `GetReleaseTimeRemaining()`: seconds, or `-1` for "no timer". `-1` is
    /// the answer inside an instance; the `DEATH` box tests for it and swaps
    /// its text for `DEATH_RELEASE_NOTIMER`.
    fn release_time_remaining(&self) -> i32;
    /// `GetCorpseRecoveryDelay()`: seconds until the corpse may be recovered.
    /// Both corpse popups use it as their `StartDelay`.
    fn corpse_recovery_delay(&self) -> i32;
    /// `ResurrectGetOfferer()`: the name of the unit offering, or `nil`.
    fn resurrect_offerer(&self) -> Option<String>;
    /// `ResurrectHasSickness()` / `ResurrectHasTimer()`: the two flags the
    /// offer carries. `UIParent_OnEvent` uses them to pick one of three
    /// popups.
    fn resurrect_has_sickness(&self) -> bool;
    fn resurrect_has_timer(&self) -> bool;
    /// `GetResSicknessDuration()`: the duration text for the sentence, such as
    /// "10 minutes", or `None` for a character who would get no sickness.
    /// `None` selects `XP_LOSS_NO_SICKNESS` over `XP_LOSS`. The client's rule:
    /// the race's `ResSicknessSpellID` (`ChrRaces` field 12), that spell's
    /// `SpellDuration` row at the character's level, `None` under one second,
    /// and `%s_MIN`/`%s_SEC` from the `GENERIC` family for the words.
    /// Defaults to `None` because a headless harness has no level.
    fn res_sickness_duration(&self) -> Option<String> {
        None
    }
    /// `CheckSpiritHealerDist()`: whether the spirit healer whose offer is
    /// pending is still in reach. The client compares the squared distance to
    /// the guid the confirm carried. The check that decides the outcome is the
    /// server's `INTERACTION_DISTANCE` on `CMSG_SPIRIT_HEALER_ACTIVATE`, so
    /// that is the distance used. `false` with no offer, which closes a stale
    /// box.
    fn spirit_healer_in_reach(&self) -> bool {
        false
    }
    /// Two tokens that both resolve to nothing are not the same unit. See
    /// [`crate::interface::api::Units::is_unit`].
    fn unit_is_unit(&self, a: &str, b: &str) -> bool;
    /// Every character-sheet stat as one answer. Eleven of the game's C
    /// functions read from it, and each is a method on
    /// [`vale_protocol::play::stats::UnitStats`]. `None` for a unit whose
    /// stat block the server never sent, which is every unit but the player.
    ///
    /// One trait method rather than eleven because the only part that needs
    /// the live world is fetching the block; the arithmetic on it is testable
    /// with no `Answers`.
    fn unit_stats(&self, token: &str) -> Option<vale_protocol::play::stats::UnitStats>;
    /// `UnitRace`: `(localised, fileName)`.
    fn unit_race(&self, token: &str) -> Option<(&'static str, &'static str)>;
    /// `UnitClass`: the same pair. Both FrameXML call sites `strupper` the
    /// second value.
    fn unit_class(&self, token: &str) -> Option<(&'static str, &'static str)>;
    /// `UnitCreatureType`: the `CreatureType.dbc` name, and nil for a player,
    /// as the game answers.
    fn unit_creature_type(&self, token: &str) -> Option<&'static str>;
    /// `UnitClassification`: `"elite"`, `"rareelite"`, `"worldboss"`, `"rare"`
    /// or `"normal"`. Lower case, because FrameXML compares against lower-case
    /// literals.
    fn unit_classification(&self, token: &str) -> &'static str;
    /// `UnitIsPVP`: flagged for PvP combat.
    fn unit_is_pvp(&self, token: &str) -> bool;
    /// `UnitFactionGroup`: `(internalName, localisedName)`, or `None` for a
    /// unit with no named faction group, which is every creature.
    ///
    /// The interface builds a file path from the first value
    /// (`Interface\GroupFrame\UI-Group-PVP-<group>`), so it is untranslated;
    /// the second is the display text. The parent-faction walk and its
    /// empty-name rule are in
    /// [`vale_assets::tables::faction::Factions::group_name`].
    fn unit_faction_group(&self, token: &str) -> Option<(String, String)>;
    /// Everything `GameTooltip:SetUnit` shows, as one answer, for the same
    /// reason as [`Answers::unit_stats`]: the tooltip describes one unit and
    /// its composition is testable with no `Answers`.
    fn unit_tooltip(&self, token: &str) -> Option<crate::interface::api::UnitTip>;

    // --- friend or foe ---
    //
    // Five of the game's API functions answered from one reading, because they
    // are five questions about one fact; answering them from different sources
    // could produce a combination the real client never shows. The rule is in
    // [`vale_assets::tables::faction`]. The note on [`Answers::unit_reaction`]
    // describes what the interface does when these are nil.

    /// The answer behind `UnitReaction(a, b)`: how the first unit stands
    /// towards the second on the client's eight-rank scale, or `None` when
    /// either token names nothing. On that nil, `TargetFrame_CheckFaction`
    /// falls through to a blue name background.
    ///
    /// The rank, not the three-way hostile/neutral/friendly fold, because
    /// `UnitReactionColor` has eight rows. The orange row at index 3 is
    /// Unfriendly, a rank only the character's own reputation produces.
    fn unit_rank(&self, a: &str, b: &str) -> Option<vale_assets::tables::faction::Rank>;
    /// `UnitCanAttack(a, b)`: the reaction and the target's own flags. See
    /// [`crate::interface::api::Units::can_attack`].
    fn unit_can_attack(&self, a: &str, b: &str) -> bool;
    /// `UnitCanAssist(a, b)`; see [`crate::interface::api::Units::can_assist`].
    /// The default, for the stand-ins with no faction table, is "friendly".
    fn unit_can_assist(&self, a: &str, b: &str) -> bool {
        use vale_assets::tables::faction::Reaction;
        self.unit_rank(a, b).map(Reaction::from) == Some(Reaction::Friendly)
    }
    /// `UnitPlayerControlled`: whether a player controls this unit.
    fn unit_player_controlled(&self, token: &str) -> bool;
}

/// `GetSpellTabInfo`'s four answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellTab {
    pub name: String,
    /// `Interface\Icons\…`, or empty for a line whose icon id is not in
    /// `SpellIcon.dbc`. Empty rather than absent because
    /// `skillLineTab:SetNormalTexture(texture)` accepts either, and a texture
    /// set to `nil` and one set to `""` both draw nothing.
    pub texture: String,
    pub offset: usize,
    pub count: usize,
}

/// The live world, as an [`Answers`].
///
/// Built fresh by each system that enters Lua and dropped when the call
/// returns. It holds only borrows, so nothing in it can go stale. That is the
/// reason for this design over a snapshot refreshed once a frame.
pub struct Live<'a, 'w, 's> {
    pub units: &'a Units<'w, 's>,
    pub bar: &'a ActionBar,
    pub cooldowns: &'a Cooldowns,
    pub book: &'a Spellbook,
    /// The pet action bar. The server sends its contents, whereas the
    /// player's own bar is kept by the client. See [`crate::interface::pet`].
    pub pet_bar: &'a crate::interface::pet::PetBar,
    /// The talent trees, which the panel reads twenty buttons at a time. See
    /// [`crate::interface::talents`] and [`super::panels::talent`].
    pub talents: &'a crate::interface::talents::Talents,
    pub casting: &'a Casting,
    /// The repeating ranged attack, for `IsAutoRepeatAction`. A spell id
    /// rather than a borrow, because the id is the whole state and copying a
    /// `u32` is cheaper than a reference to it. See
    /// [`crate::interface::action::AutoRepeat`].
    pub auto_repeat: Option<u32>,
    /// The spell cursor's state. See
    /// [`crate::interface::action::SpellTargeting`]. It sits beside `casting`
    /// because the two together answer "is a cast happening": one cast is
    /// running, the other is waiting for a target, and the interface asks
    /// about them separately.
    pub targeting: &'a crate::interface::action::SpellTargeting,
    /// The auras on the units the interface can ask about. See
    /// [`crate::interface::auras`].
    pub auras: &'a crate::interface::auras::Auras,
    /// Death state: the two timers and the pending resurrect offer. See
    /// [`crate::interface::death`].
    pub dying: &'a crate::interface::death::Dying,
    /// The open book or page text, if any. See [`crate::interface::pagetext`].
    pub page: &'a crate::interface::pagetext::OpenBook,
    /// The pending summon, if any. See [`crate::interface::summon`].
    pub summon: &'a crate::interface::summon::Summon,
    /// Whom the character is inspecting. See [`crate::interface::inspect`].
    pub inspect: &'a crate::interface::inspect::Inspect,
    /// `GlobalStrings.lua`, for the one spellbook tab whose name is a string
    /// key. See [`vale_assets::tables::book::GENERAL_NAME_KEY`]. `None` before
    /// the table has loaded, in which case the key itself is drawn.
    pub strings: Option<&'a Strings>,
    /// The contents of the corpse or object being looted, while a loot window
    /// is open. See [`crate::interface::loot`]. Empty for a session in which
    /// nothing has been looted.
    pub loot: &'a crate::interface::loot::LootWindow,
    /// In a group, the open loot rolls. See [`crate::interface::lootroll`].
    /// Empty for every session that never groups; the systems behind it check
    /// that first.
    pub rolls: &'a crate::interface::lootroll::LootRolls,
    /// The quest log and the open quest dialog. See
    /// [`crate::interface::quest`].
    pub quests: &'a crate::interface::quest::Quests,
    /// The gossip and merchant windows. See [`crate::interface::gossip`] and
    /// [`crate::interface::merchant`].
    pub gossip: &'a crate::interface::gossip::GossipWindow,
    pub merchant: &'a crate::interface::merchant::MerchantWindow,
    /// The mailbox, a window no NPC owns. See [`crate::interface::mail`].
    pub mail: &'a crate::interface::mail::Mailbox,
    /// The trainer window, whose panel is the game's load-on-demand addon. See
    /// [`crate::interface::trainer`].
    pub trainer: &'a crate::interface::trainer::TrainerWindow,
    /// The two profession windows, which belong to the character rather than
    /// an NPC. See [`crate::interface::tradeskill`].
    pub tradeskill: &'a crate::interface::tradeskill::TradeSkillWindow,
    pub craft: &'a crate::interface::tradeskill::CraftWindow,
    /// The flight map, whose content the client computes itself. See
    /// [`crate::interface::taxi`].
    pub taxi: &'a crate::interface::taxi::TaxiWindow,
    /// The stable window, reached through one gossip option. See
    /// [`crate::interface::stable`].
    pub stable: &'a crate::interface::stable::StableWindow,
    /// The bank window: its contents are inventory slots and the window itself
    /// is a banker guid. See [`crate::interface::bank`].
    pub bank: &'a crate::interface::bank::BankWindow,
    /// The trade window, opened with another player rather than an NPC. See
    /// [`crate::interface::trade`].
    pub trade: &'a crate::interface::trade::TradeWindow,
    /// [`crate::interface::api::get_time`]'s value for this frame.
    pub now: f64,
    /// The world's hour and minute, for `GetGameTime`. See
    /// [`UnitAnswers::game_time`]. Read from [`crate::render::sky::WorldClock`]
    /// rather than from the session, so a hand-set hour reaches the minimap
    /// clock as well as the sky.
    pub game_clock: (u32, u32),
    /// The hearthstone's bind location as text: `GetBindLocation()`'s answer,
    /// and the `$z` in the hearthstone's spell description. The 1.12.1 client
    /// resolves both the same way.
    ///
    /// Resolved once here rather than at each read, because it is an
    /// `AreaTable` lookup with a fallback and three different reads want the
    /// same string. Empty only with no archives open, since the fallback is
    /// `HOME_INN` ("your inn"), which the client also shows for an unknown
    /// bind point.
    pub home: String,
    /// The innkeeper waiting on an answer, for `CheckBinderDist`. See
    /// [`crate::interface::binder::HomeBind`].
    pub binder: &'a crate::interface::binder::HomeBind,
    /// The pet trainer, for `CheckPetUntrainerDist`. See
    /// [`crate::interface::untrainer::Untrainer`].
    pub untrainer: &'a crate::interface::untrainer::Untrainer,
    /// The stance bar, which the client builds from `Spell.dbc` with no packet
    /// behind it. See [`crate::interface::shapeshift`].
    pub shapeshift: &'a crate::interface::shapeshift::ShapeshiftBar,
    /// Whether an auto-attack is in progress, for `IsCurrentAction`. This is
    /// the one piece of state read from the session rather than a resource.
    pub attacking: bool,
    /// Where the character is and which map the world map panel is showing.
    /// See [`crate::interface::worldmap`].
    pub place: &'a crate::interface::worldmap::WorldMapState,
    /// The one landmark the server can place on that map. See
    /// [`crate::interface::worldmap::MapLandmarks`].
    pub landmarks: &'a crate::interface::worldmap::MapLandmarks,
    /// The party. See [`crate::interface::party`]. The one unit subject whose
    /// members may not be in the world at all.
    pub party: &'a crate::interface::party::Party,
    /// The item types the character may use. See
    /// [`crate::world::proficiency`].
    pub proficiency: &'a crate::world::proficiency::Proficiencies,
    /// What the character is carrying: the bags, the equipped slots, the
    /// money, and the item templates for all of it. See
    /// [`crate::interface::items`].
    pub inventory: &'a crate::interface::items::Inventory,
    /// The item on the cursor, which decides whether a left click on a bag
    /// slot picks up or puts down. See [`crate::interface::cursor`].
    pub cursor: &'a crate::interface::cursor::Cursor,
    /// The archive's DBC tables. They answer four kinds of question here: the
    /// two tables that turn a place into a name and a map rectangle, the spell
    /// table behind every tooltip, and `FactionTemplate.dbc` behind reaction.
    /// `None` before the archives are open; each read then answers its own
    /// empty value rather than a constant that looks real.
    pub tables: Option<std::sync::Arc<vale_assets::tables::dbc::DisplayTables>>,
    /// The local player's map id and position, used by the map reads.
    /// Resolved once by [`LuaWorld::live`] rather than per call.
    pub here: Option<(u32, f32, f32)>,
    /// The local player's facing, for the map arrow. See
    /// [`Answers::player_facing`].
    pub facing: f32,
    /// The pre-world screens: the handshake this client holds open, if any,
    /// and what the account box remembers.
    ///
    /// `None` at the login screen and `None` in the world, because the
    /// character list exists only between `CMSG_CHAR_ENUM` and
    /// `CMSG_PLAYER_LOGIN`. Every glue read answers its own empty value then.
    /// See [`super::panels::glue`].
    pub selection: Option<&'a crate::world::session::Handshake>,
    /// The client-side glue state: the remembered account name and whether
    /// the socket is still up. See [`crate::glue::glue::GlueState`].
    pub glue: &'a crate::glue::glue::GlueState,
    /// The object manager, for the one read that needs a server-owned name
    /// the ECS does not mirror. A spell's reagents are item entries and
    /// `Item.dbc` is not in the archives, so a name such as "Rune of
    /// Teleportation" lives in the templates `CMSG_ITEM_QUERY_SINGLE` fills.
    /// Only [`Live::reagent_name`] touches it, and only on tooltip hover, not
    /// every frame.
    pub world: Option<std::sync::Arc<std::sync::Mutex<vale_protocol::state::objects::ObjectManager>>>,
}

/// Everything the interface may ask about, as one system parameter.
///
/// Seven systems in this directory enter Lua and each needs the same set of
/// resources. Without this bundle, a new read would need a new `Res<…>` on
/// each of the seven systems and a new field at each `Live { … }`. With it, a
/// read is one field here and one method on [`Answers`].
///
/// It also keeps two of those systems under `SystemParam`'s limit of sixteen
/// parameters.
#[derive(SystemParam)]
pub struct LuaWorld<'w, 's> {
    pub units: Units<'w, 's>,
    pub bar: Res<'w, ActionBar>,
    pub cooldowns: Res<'w, Cooldowns>,
    pub casting: Res<'w, Casting>,
    /// The repeating ranged attack, for `IsAutoRepeatAction`. See
    /// [`crate::interface::action::AutoRepeat`].
    pub auto_repeat: Res<'w, crate::interface::action::AutoRepeat>,
    pub targeting: Res<'w, crate::interface::action::SpellTargeting>,
    pub book: Res<'w, Spellbook>,
    pub pet_bar: Res<'w, crate::interface::pet::PetBar>,
    pub talents: Res<'w, crate::interface::talents::Talents>,
    pub auras: Res<'w, crate::interface::auras::Auras>,
    /// Death state. See [`crate::interface::death`].
    pub dying: Res<'w, crate::interface::death::Dying>,
    pub page: Res<'w, crate::interface::pagetext::OpenBook>,
    pub summon: Res<'w, crate::interface::summon::Summon>,
    pub inspect: Res<'w, crate::interface::inspect::Inspect>,
    pub session: Res<'w, crate::world::session::Session>,
    pub strings: Res<'w, crate::interface::messages::UiStrings>,
    pub time: Res<'w, Time>,
    /// Where the character is and which map the world map panel shows. See
    /// [`crate::interface::worldmap`].
    pub place: Res<'w, crate::interface::worldmap::WorldMapState>,
    /// The landmark a guard's directions place on the map.
    pub landmarks: Res<'w, crate::interface::worldmap::MapLandmarks>,
    /// The party. See [`crate::interface::party`].
    pub party: Res<'w, crate::interface::party::Party>,
    /// The item types the character may use, which is the only thing that
    /// colours an item slot red. See [`crate::world::proficiency`].
    pub proficiency: Res<'w, crate::world::proficiency::Proficiencies>,
    /// What the character is carrying. See [`crate::interface::items`].
    pub inventory: Res<'w, crate::interface::items::Inventory>,
    /// The item on the cursor. See [`crate::interface::cursor`].
    pub cursor: Res<'w, crate::interface::cursor::Cursor>,
    /// The contents of the corpse or object being looted. See
    /// [`crate::interface::loot`].
    pub loot: Res<'w, crate::interface::loot::LootWindow>,
    /// The open loot rolls. See [`crate::interface::lootroll`].
    pub rolls: Res<'w, crate::interface::lootroll::LootRolls>,
    /// The quest log and the open quest dialog. See
    /// [`crate::interface::quest`].
    pub quests: Res<'w, crate::interface::quest::Quests>,
    /// The two windows a right-click on an NPC opens. See
    /// [`crate::interface::gossip`] and [`crate::interface::merchant`].
    pub gossip: Res<'w, crate::interface::gossip::GossipWindow>,
    pub merchant: Res<'w, crate::interface::merchant::MerchantWindow>,
    /// The mailbox. See [`crate::interface::mail`].
    pub mail: Res<'w, crate::interface::mail::Mailbox>,
    /// The trainer window. See [`crate::interface::trainer`].
    pub trainer: Res<'w, crate::interface::trainer::TrainerWindow>,
    /// The two profession windows. See [`crate::interface::tradeskill`].
    pub tradeskill: Res<'w, crate::interface::tradeskill::TradeSkillWindow>,
    pub craft: Res<'w, crate::interface::tradeskill::CraftWindow>,
    /// The flight map. See [`crate::interface::taxi`].
    pub taxi: Res<'w, crate::interface::taxi::TaxiWindow>,
    /// The stable. See [`crate::interface::stable`].
    pub stable: Res<'w, crate::interface::stable::StableWindow>,
    /// The bank. See [`crate::interface::bank`].
    pub bank: Res<'w, crate::interface::bank::BankWindow>,
    pub trade: Res<'w, crate::interface::trade::TradeWindow>,
    /// The archives, for the two tables that turn a place into a name.
    pub assets: Res<'w, crate::assets::GameAssets>,
    /// The local player's position, for `GetPlayerMapPosition`, the one map
    /// read that needs a world position rather than an id.
    pub status: Res<'w, crate::world::session::WorldStatus>,
    /// The client-side glue state. See [`crate::glue::glue`].
    pub glue: Res<'w, crate::glue::glue::GlueState>,
    /// The hearthstone's bind point. See [`crate::interface::binder`].
    pub home: Res<'w, crate::interface::binder::HomeBind>,
    /// The pet trainer's pending question. See
    /// [`crate::interface::untrainer`].
    pub untrainer: Res<'w, crate::interface::untrainer::Untrainer>,
    /// The stance bar. See [`crate::interface::shapeshift`].
    pub shapeshift: Res<'w, crate::interface::shapeshift::ShapeshiftBar>,
    /// The world's hour, for `GetGameTime`. See
    /// [`crate::render::sky::WorldClock`]. Taken from the sky's clock rather
    /// than from the session so that the minimap clock and the sky lighting
    /// use the same value, hand-set hours included.
    pub clock: Res<'w, crate::render::sky::WorldClock>,
}

impl LuaWorld<'_, '_> {
    /// Inserts every resource this bundle needs into a test app.
    ///
    /// Five test harnesses in this directory each build a minimal `App` to run
    /// one system. Without this function, each field added to the bundle would
    /// need a new `init_resource` line in five files, and those files would
    /// fail to compile until it was added. The `Query` needs nothing: an empty
    /// world answers every token as absent, which is what those tests want.
    #[cfg(test)]
    pub(crate) fn init(app: &mut bevy::app::App) -> &mut bevy::app::App {
        app.init_resource::<Time>()
            .init_resource::<ActionBar>()
            .init_resource::<Cooldowns>()
            .init_resource::<Casting>()
            .init_resource::<crate::interface::action::AutoRepeat>()
            .init_resource::<crate::interface::action::SpellTargeting>()
            .init_resource::<Spellbook>()
            .init_resource::<crate::interface::pet::PetBar>()
            .init_resource::<crate::interface::talents::Talents>()
            .init_resource::<crate::interface::auras::Auras>()
            .init_resource::<crate::interface::death::Dying>()
            .init_resource::<crate::world::session::Session>()
            .init_resource::<crate::interface::messages::UiStrings>()
            .init_resource::<crate::interface::worldmap::WorldMapState>()
            .init_resource::<crate::interface::worldmap::MapLandmarks>()
            .init_resource::<crate::interface::party::Party>()
            .init_resource::<crate::interface::items::Inventory>()
            .init_resource::<crate::interface::cursor::Cursor>()
            .init_resource::<crate::interface::loot::LootWindow>()
            .init_resource::<crate::interface::pagetext::OpenBook>()
            .init_resource::<crate::interface::summon::Summon>()
            .init_resource::<crate::interface::inspect::Inspect>()
            .init_resource::<crate::interface::lootroll::LootRolls>()
            .init_resource::<crate::interface::quest::Quests>()
            .init_resource::<crate::interface::gossip::GossipWindow>()
            .init_resource::<crate::interface::gossip::NpcUnit>()
            .init_resource::<crate::interface::reputation::PlayerStanding>()
            .init_resource::<crate::interface::merchant::MerchantWindow>()
            .init_resource::<crate::interface::mail::Mailbox>()
            .init_resource::<crate::interface::trainer::TrainerWindow>()
            .init_resource::<crate::interface::tradeskill::TradeSkillWindow>()
            .init_resource::<crate::interface::tradeskill::CraftWindow>()
            .init_resource::<crate::interface::taxi::TaxiWindow>()
            .init_resource::<crate::interface::stable::StableWindow>()
            .init_resource::<crate::interface::bank::BankWindow>()
            .init_resource::<crate::interface::trade::TradeWindow>()
            .init_resource::<crate::world::session::WorldStatus>()
            .init_resource::<crate::glue::glue::GlueState>()
            .init_resource::<crate::render::sky::WorldClock>()
            .init_resource::<crate::interface::binder::HomeBind>()
            .init_resource::<crate::interface::untrainer::Untrainer>()
            .init_resource::<crate::interface::shapeshift::ShapeshiftBar>()
            // The item types the character may use, which the trade slots
            // read. See [`crate::world::proficiency`]. Inserted here for the
            // same reason as the resources below: the headless probes build
            // this app without the state plugin groups.
            .init_resource::<crate::world::proficiency::Proficiencies>()
            .insert_resource(crate::assets::GameAssets::new(String::new()))
            .init_resource::<crate::interface::target::Selection>()
            .init_resource::<crate::interface::target::Hovered>()
            // The hovered game object, which the world tooltip reads beside
            // `Hovered`. See `crate::interface::object::HoveredObject`. Both
            // are inserted here rather than left to `InterfacePlugins`
            // because the headless probes build this app without it.
            .init_resource::<crate::interface::object::HoveredObject>()
    }

    /// The borrow to lend Lua for the length of one call.
    ///
    /// Cheap enough to build per call, and correct to build once per frame: it
    /// holds only borrows, so nothing in it can go stale between two chunks.
    pub fn live(&self) -> Live<'_, '_, '_> {
        Live {
            units: &self.units,
            bar: &self.bar,
            cooldowns: &self.cooldowns,
            casting: &self.casting,
            auto_repeat: self.auto_repeat.spell,
            targeting: &self.targeting,
            page: &self.page,
            pet_bar: &self.pet_bar,
            talents: &self.talents,
            auras: &self.auras,
            dying: &self.dying,
            summon: &self.summon,
            inspect: &self.inspect,
            book: &self.book,
            strings: self.strings.get(),
            place: &self.place,
            landmarks: &self.landmarks,
            party: &self.party,
            proficiency: &self.proficiency,
            inventory: &self.inventory,
            cursor: &self.cursor,
            loot: &self.loot,
            rolls: &self.rolls,
            quests: &self.quests,
            gossip: &self.gossip,
            merchant: &self.merchant,
            mail: &self.mail,
            trainer: &self.trainer,
            tradeskill: &self.tradeskill,
            craft: &self.craft,
            taxi: &self.taxi,
            stable: &self.stable,
            bank: &self.bank,
            trade: &self.trade,
            tables: self.assets.display_tables().ok(),
            here: self.session.active.as_ref().filter(|_| self.status.in_world).map(
                |active| (active.map_id, self.status.position.x, self.status.position.y),
            ),
            facing: self.status.orientation,
            now: api::get_time(&self.time),
            game_clock: self.clock.hour_minute(),
            binder: &self.home,
            untrainer: &self.untrainer,
            shapeshift: &self.shapeshift,
            home: home_name(
                self.home.area(),
                self.assets.display_tables().ok().as_deref(),
                self.strings.get(),
            ),
            attacking: self
                .session
                .active
                .as_ref()
                .and_then(|active| active.live.attacking())
                .is_some(),
            world: self
                .session
                .active
                .as_ref()
                .map(|active| std::sync::Arc::clone(active.live.world())),
            selection: self.session.selection.as_ref(),
            glue: &self.glue,
        }
    }
}

/// The text `GetBindLocation()` returns, resolved from the area id the bind
/// point carries.
///
/// The 1.12.1 client uses `AreaTable`'s `AreaName` for the row itself, not for
/// the zone it belongs to, so an inn's sub-area gives "Lion's Pride Inn" and a
/// bind in open country gives the zone. `GetBindLocation` and the spell text's
/// `$z` token resolve it the same way, so one string answers both.
///
/// The fallback is `HOME_INN` ("your inn"), as in the client: both use it
/// when the area id is unset or out of the table's range, which is the case on
/// every frame before the login packets arrive.
fn home_name(
    area: Option<u32>,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    strings: Option<&Strings>,
) -> String {
    let named = area
        .zip(tables.and_then(|tables| tables.areas()))
        .and_then(|(area, areas)| areas.get(area))
        .map(|area| area.name.clone());
    named
        .filter(|name| !name.is_empty())
        .or_else(|| strings.and_then(|strings| strings.get("HOME_INN")).map(str::to_string))
        .unwrap_or_default()
}
impl Live<'_, '_, '_> {
    /// A token to a [`UnitId`], or `None` for one this client has no state for.
    pub(super) fn id(token: &str) -> Option<UnitId> {
        UnitId::parse(token)
    }

    /// The map tables, or `None` before the archives are open. One accessor,
    /// because six of the reads above need the same two steps.
    pub(super) fn world_map(&self) -> Option<&vale_assets::tables::worldmap::WorldMap> {
        self.tables.as_ref()?.world_map()
    }

    /// A party member's position as `(map, x, y)`. `None` for a member no
    /// stats packet has arrived for, and for one whose zone `AreaTable` cannot
    /// place on a map.
    ///
    /// The position is `SMSG_PARTY_MEMBER_STATS`' two `int16`s, the only
    /// position the server sends for a party member. The map is not in that
    /// packet: it is derived from the zone the same packet carries, which keeps
    /// a member in Kalimdor off an Eastern Kingdoms map.
    pub(super) fn party_position(&self, index: usize) -> Option<(u32, f32, f32)> {
        let stats = self.party.member(index)?.stats.as_ref()?;
        let (x, y) = stats.position?;
        let map = self
            .tables
            .as_ref()?
            .areas()?
            .get(u32::from(stats.zone?))
            .map(|area| area.map)?;
        Some((map, f32::from(x), f32::from(y)))
    }

    /// A reagent's name, or `None` with a query queued if the name is not yet
    /// known.
    ///
    /// The lookup and the queueing happen under one lock, which makes this
    /// safe to call from a hover: a miss queues the entry
    /// ([`ObjectManager::want_item`]) and never touches the socket, so the
    /// query goes out on the session's own interval with the equipment
    /// queries, and a hundred hovers cost one packet.
    ///
    /// The first hover on a cold cache therefore shows the tooltip without its
    /// reagent line and the next one shows it. The retail client behaves the
    /// same with an empty item cache.
    pub(super) fn reagent_name(&self, entry: u32) -> Option<String> {
        Some(self.session_template(entry)?.name)
    }

    /// A template from the session-wide cache, queued for query on a miss.
    /// Same contract as [`Self::reagent_name`]: the miss never touches the
    /// socket, so a hundred reads cost one packet on the session's query
    /// interval.
    ///
    /// This is the read for items the character does not carry, such as a
    /// vendor's stock or a corpse's loot. [`super::game::items::Inventory`]'s
    /// template map holds only the carried subset of this cache, so resolving
    /// those rows through it answers blank until the item is bought or looted.
    pub(super) fn session_template(&self, entry: u32) -> Option<vale_protocol::state::query::ItemInfo> {
        let world = self.world.as_ref()?;
        let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = world.items.get(&entry) {
            return Some(item.clone());
        }
        world.want_item(entry);
        None
    }

    /// The name of a quest objective's creature or game object, queued for
    /// query on a miss, with the same contract as [`Self::session_template`].
    ///
    /// The quest packets do not carry the name: `ReqCreatureOrGOId` is an id,
    /// and only `CMSG_CREATURE_QUERY` / `CMSG_GAMEOBJECT_QUERY` resolve it. A
    /// quest log opened in a city usually asks about a creature that is not in
    /// view, and the queries sent for objects in sight never reach it. That is
    /// why `ObjectManager::want_creature` exists.
    pub(super) fn objective_target_name(&self, target: vale_protocol::play::quest::Target) -> Option<String> {
        use vale_protocol::play::quest::Target;
        let world = self.world.as_ref()?;
        let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
        match target {
            Target::Creature(entry) => {
                if let Some(info) = world.creatures.get(&entry) {
                    return Some(info.name.clone());
                }
                world.want_creature(entry);
            }
            Target::GameObject(entry) => {
                if let Some(info) = world.gameobjects.get(&entry) {
                    return Some(info.name.clone());
                }
                world.want_gameobject(entry);
            }
        }
        None
    }

    /// One line of the quest log's objectives, in the client's wording.
    ///
    /// Each objective kind has one `GlobalStrings.lua` format key, the same
    /// keys `GetQuestLogLeaderBoard` uses in the client:
    ///
    /// ```text
    /// QUEST_MONSTERS_KILLED   "%s slain: %d/%d"    with "monster"
    /// QUEST_OBJECTS_FOUND     "%s: %d/%d"          with "object"
    /// QUEST_ITEMS_NEEDED      "%s: %d/%d"          with "item"
    /// QUEST_FACTION_NEEDED    "%s:  %s / %s"       with "reputation"
    /// (none)                  the wording verbatim with "event"
    /// ```
    ///
    /// The monster line is the only one that is not `%s: %d/%d`, and it is the
    /// most common kind: "Kobold Vermin slain: 3/8" rather than
    /// "Kobold Vermin: 3/8". Using one format for all kinds would be wrong on
    /// most quests in Elwynn.
    ///
    /// The formats are read from the archive's `GlobalStrings.lua` rather than
    /// written here, so a key the file does not carry shows as nothing, as in
    /// the client, instead of as text invented in this repository.
    pub(super) fn leader_board_line(&self, kind: &str, name: &str, have: u32, want: u32) -> String {
        let key = match kind {
            "monster" => "QUEST_MONSTERS_KILLED",
            "object" => "QUEST_OBJECTS_FOUND",
            _ => "QUEST_ITEMS_NEEDED",
        };
        let format = self.strings.and_then(|s| s.get(key));
        match format {
            // `%s` then `%d` then `%d`, in that order in all three formats, so
            // the substitution is positional and needs no printf.
            Some(format) => format
                .replacen("%s", name, 1)
                .replacen("%d", &have.to_string(), 1)
                .replacen("%d", &want.to_string(), 1),
            // With no `GlobalStrings.lua` (the headless case), the counter is
            // still shown.
            None => format!("{name}: {have}/{want}"),
        }
    }

    /// The level every spell tooltip is composed at. See
    /// [`crate::interface::api::TipContext`].
    ///
    /// The level is the player's, because an effect's value scales with the
    /// caster's level and the tooltip describes what the spell would do if the
    /// player cast it. `unit_level` answers -1 for a player who has not
    /// arrived yet, which is clamped to 1, the level every spell's base value
    /// is stated at.
    pub(super) fn tip_level(&self) -> u32 {
        Self::id("player")
            .map(|id| self.units.level(id))
            .unwrap_or(1)
            .max(1) as u32
    }

    /// Whether a token is `player`, the only unit with known bags and
    /// equipment.
    ///
    /// No packet this client reads carries another unit's inventory or
    /// durability (it does not handle inspect packets), so every equipped-slot
    /// read answers the absent value for any other token. Answering with the
    /// local player's gear instead would look correct and be wrong.
    pub(super) fn is_player(token: &str) -> bool {
        Self::id(token) == Some(UnitId::Player)
    }

    /// The item entry another player wears in inventory slot `id`: the entry
    /// in that slot's `PLAYER_VISIBLE_ITEM_n_0`, which every client in view
    /// receives. Slot 1 is equipment slot 0. `None` for an empty slot, for a
    /// unit that is not a player, and for the character itself, whose items
    /// are objects in its own inventory.
    pub(super) fn worn_by_other(&self, token: &str, id: u32) -> Option<u32> {
        let unit = self.units.get(Self::id(token)?)?;
        let world = self.world.as_ref()?;
        let world = world.lock().unwrap_or_else(|e| e.into_inner());
        let entries = world.get(unit.guid)?.equipment()?;
        let entry = *entries.get(usize::try_from(id).ok()?.checked_sub(1)?)?;
        (entry != 0).then_some(entry)
    }

    /// One slot's drawable state, from whatever it holds.
    ///
    /// The quality is -1 rather than 0 when the template has not arrived. The
    /// reason is given in [`super::panels::container`].
    pub(super) fn slot_contents(
        &self,
        item: &vale_protocol::play::items::ItemSlot,
        place: crate::interface::cursor::Place,
    ) -> super::panels::container::SlotContents {
        let template = self.inventory.template(item.entry);
        super::panels::container::SlotContents {
            texture: template
                .and_then(|t| self.tables.as_ref()?.item_icon(t.display_id)),
            count: item.count,
            quality: template.map_or(-1, |t| t.quality as i32),
            readable: template.is_some_and(vale_protocol::state::query::ItemInfo::is_readable),
            broken: item.broken(),
            // Locked by slot, not by item entry. Two stacks of the same entry
            // are two slots, and locking by entry would desaturate both.
            locked: self.cursor.locks(place),
        }
    }


    /// What the ammo slot draws: the loaded entry's icon and quality, and
    /// `GetItemCount` of it for the number. `None` with nothing loaded, which
    /// restores the slot's empty art. Never locked: the cursor holds bag
    /// slots, and loading ammo is not a move.
    pub(super) fn ammo_contents(&self) -> Option<super::panels::container::SlotContents> {
        let entry = self.inventory.ammo;
        if entry == 0 {
            return None;
        }
        let template = self.inventory.template(entry);
        Some(super::panels::container::SlotContents {
            texture: template.and_then(|t| self.tables.as_ref()?.item_icon(t.display_id)),
            count: self.inventory.ammo_count(),
            quality: template.map_or(-1, |t| t.quality as i32),
            readable: false,
            broken: false,
            locked: false,
        })
    }

    /// One reward or requirement line, named through the item cache.
    ///
    /// A quest packet carries `ItemPrototype::DisplayInfoID` for every item it
    /// names, so the icon needs no query, unlike a loot row's. The name and the
    /// quality still need the template.
    pub(super) fn quest_line(&self, entry: u32, count: u32, display_id: u32) -> super::panels::quest::RewardLine {
        // The session-wide cache, not the carried one: a reward is an item the
        // character does not own yet, as with a vendor's stock or a corpse's
        // loot. See [`Self::session_template`].
        let template = self.session_template(entry);
        let display = match display_id {
            0 => template.as_ref().map(|t| t.display_id).unwrap_or(0),
            id => id,
        };
        super::panels::quest::RewardLine {
            entry,
            name: template.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
            texture: self.tables.as_ref().and_then(|t| t.item_icon(display)),
            count,
            quality: template.as_ref().map_or(0, |t| t.quality),
            usable: true,
        }
    }

    /// A quest's reward spell as the panel reads it. See
    /// [`super::panels::quest::RewardSpell`] for why this is three values
    /// rather than an id.
    ///
    /// `0` means "no reward spell" in the packet, and this function exists
    /// mainly for that case: it must reach Lua as nil, not as a number.
    pub(super) fn reward_spell(&self, spell_id: u32) -> Option<super::panels::quest::RewardSpell> {
        let info = self.tables.as_ref()?.spellbook()?.info(spell_id)?;
        Some(super::panels::quest::RewardSpell {
            // The client returns nil for an icon it cannot resolve and still
            // returns the name; the panel then hides the block, because it
            // tests the first value. An empty icon becomes that nil.
            texture: Some(info.icon.clone()).filter(|icon| !icon.is_empty()),
            name: info.name.clone(),
            tradeskill: info.attributes & vale_assets::tables::spellbook::spell_attributes::TRADESPELL
                != 0,
        })
    }

    /// The unit the `$` variables in a quest's or an NPC's text refer to. See
    /// [`crate::interface::messages::substitute`]. Always the local player,
    /// because the text is addressed to whoever is reading it.
    pub(super) fn speaker(&self) -> crate::interface::messages::Speaker<'_> {
        let unit = self.units.get(UnitId::Player);
        crate::interface::messages::Speaker {
            name: unit.map(|u| u.name.as_str()).unwrap_or_default(),
            class: self.units.class(UnitId::Player).map(|(n, _)| n).unwrap_or_default(),
            race: self.units.race(UnitId::Player).map(|(n, _)| n).unwrap_or_default(),
        }
    }

    /// The template of whatever row the log panel has selected.
    pub(super) fn selected_template(&self) -> Option<&vale_protocol::play::quest::QuestTemplate> {
        let slot = self.quests.at(self.quests.selected())?;
        self.quests.template(slot.quest_id)
    }

    /// The item link for a carried slot. It needs the item name, so it needs
    /// the template.
    pub(super) fn slot_link(&self, item: &vale_protocol::play::items::ItemSlot) -> Option<String> {
        let template = self.inventory.template(item.entry)?;
        // The copy's permanent enchantment and random property go into the
        // link, and the suffix into its name, as the 1.12.1 client writes a
        // link to a carried item.
        let suffix = self
            .tables
            .as_deref()
            .and_then(|t| t.random_properties().suffix(item.random_property));
        let name = match (suffix, self.strings.and_then(|s| s.get("ITEM_SUFFIX_TEMPLATE"))) {
            (Some(suffix), Some(format)) => vale_assets::interface::strings::substitute_all(
                format,
                &[&template.name, suffix],
            ),
            _ => template.name.clone(),
        };
        Some(super::panels::container::item_link_with(
            template.entry,
            item.enchantments[0].id,
            item.random_property,
            template.quality,
            &name,
        ))
    }

    /// An item's cooldown is its `ON_USE` spell's cooldown, read through the
    /// same timers and the same arithmetic as an action button's.
    ///
    /// This client keeps no per-item timer. `SMSG_SPELL_COOLDOWN` names a
    /// spell, and the server sends it when a potion is used, so the cooldown
    /// is already recorded under that spell id by the time the bag frame asks.
    ///
    /// Answers the idle triple when there is no item, no template yet, no
    /// `ON_USE` block, or a spell the catalog does not carry.
    pub(super) fn item_cooldown(&self, entry: Option<u32>) -> (f64, f64, bool) {
        let resolved = || {
            let template = self.inventory.template(entry?)?;
            let index = template.on_use_spell()?;
            let spell = template
                .spells
                .get(usize::from(index))
                .map(|spell| spell.spell_id)
                .filter(|id| *id != 0)?;
            let info = self.tables.as_ref()?.spellbook()?.info(spell)?;
            Some(crate::interface::api::cooldown_of(self.cooldowns, &info, self.now))
        };
        resolved().unwrap_or(IDLE_COOLDOWN)
    }

    /// The context an item tooltip is composed against. It is the same as a
    /// spell's, since an item's "Use:" line is a spell's description.
    pub(super) fn item_context(&self) -> crate::interface::api::TipContext<'_> {
        // Only the item tooltip uses race and class. A spell tooltip has no
        // requirement lines, so the two other `TipContext`s below leave them
        // at zero and skip the lookup.
        let (race, class) = Self::id("player")
            .and_then(|id| self.units.race_class_ids(id))
            .unwrap_or((0, 0));
        crate::interface::api::TipContext {
            level: self.tip_level(),
            race,
            class,
            catalog: self.tables.as_ref().and_then(|t| t.spellbook()),
            // Replaced by `tip_from`, which names the pieces of an item's set
            // through the queueing lookup.
            item_names: &|_| None,
            home: Some(self.home.clone()),
        }
    }

    /// One of the local player's own update fields, 0 when absent. Takes the
    /// world lock.
    fn player_field(&self, index: u16) -> u32 {
        let Some(world) = self.world.as_ref() else {
            return 0;
        };
        let world = world.lock().unwrap_or_else(|e| e.into_inner());
        world.player().and_then(|player| player.field(index)).unwrap_or(0)
    }

    /// The item tooltip, from a template and the carried stack if there is
    /// one.
    ///
    /// The template is looked up in two caches, in this order.
    /// [`crate::interface::items::Inventory`]'s map holds only carried items
    /// (it is built from `carried.entries()`), so it misses every item the
    /// character does not own: a merchant's stock, a corpse's loot, a quest
    /// reward, a chat link. Those rows already get their name, icon and link
    /// through [`Self::session_template`], so the tooltip falls back to the
    /// same cache. Without the fallback, `item_plate` would hide the tooltip,
    /// which looks the same as hovering an empty slot (see [`self::stubs`] on
    /// silent empty answers).
    ///
    /// The carried map is checked first because it needs no lock: a bag hover
    /// is the most common item tooltip, and the session cache takes the
    /// world's mutex. A miss on both queues the query and answers `None`, so
    /// the first hover on a cold cache shows nothing and the next one shows the
    /// tooltip. The real client behaves the same with an empty
    /// `ItemCache.wdb`.
    pub(super) fn tip_from(
        &self,
        entry: u32,
        carried: Option<&vale_protocol::play::items::ItemSlot>,
    ) -> Option<crate::interface::api::ItemTip> {
        // Held across the call so the borrowed and the owned template can share
        // one code path; `session_template` answers by value.
        let queried;
        let template = match self.inventory.template(entry) {
            Some(template) => template,
            None => {
                queried = self.session_template(entry)?;
                &queried
            }
        };
        // An item tooltip names other items (the pieces of its set), so its
        // context names them through the carried templates first and then the
        // session cache, which queues a query on a miss.
        let names = |entry: u32| {
            self.inventory
                .template(entry)
                .map(|t| t.name.clone())
                .or_else(|| self.reagent_name(entry))
        };
        let context = crate::interface::api::TipContext {
            item_names: &names,
            ..self.item_context()
        };
        let player = Self::id("player");
        let skills = player.and_then(|id| self.units.skills(id));
        let skill_rank = |line: u32| -> Option<i32> {
            let skill = skills?.get(u16::try_from(line).ok()?)?;
            Some(skill.rank())
        };
        let knows_spell = |spell: u32| -> bool {
            let Some(world) = self.world.as_ref() else {
                return false;
            };
            let world = world.lock().unwrap_or_else(|e| e.into_inner());
            world.spellbook.known.contains(&spell)
        };
        let standing = |faction: u32| self.units.standing_rank(faction);
        let proficient = |class: u32, subclass: u32| {
            match (u8::try_from(class), u8::try_from(subclass)) {
                (Ok(class), Ok(subclass)) => self.proficiency.0.allows(class, subclass),
                _ => true,
            }
        };
        let player_name = |guid: u64| -> Option<String> {
            let world = self.world.as_ref()?;
            let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(info) = world.players.get(&guid) {
                return Some(info.name.clone());
            }
            world.want_social_guid(guid);
            None
        };
        // A set piece counts only while it is equipped and not broken.
        let equipped = self
            .inventory
            .carried
            .equipped
            .iter()
            .flatten()
            .filter(|item| !item.broken())
            .map(|item| item.entry)
            .collect();
        // `UnitFactionGroup`'s internal name picks the rank titles' faction:
        // `PVP_RANK_<rank>_0` is the Horde's and `_1` the Alliance's.
        let team = self
            .unit_faction_group("player")
            .and_then(|(group, _)| match group.as_str() {
                "Horde" => Some(0),
                "Alliance" => Some(1),
                _ => None,
            });
        let wearer = crate::interface::api::Wearer {
            equipped,
            skill_rank: &skill_rank,
            knows_spell: &knows_spell,
            standing: &standing,
            proficient: &proficient,
            player_name: &player_name,
            template: &|entry| {
                self.inventory
                    .template(entry)
                    .cloned()
                    .or_else(|| self.session_template(entry))
            },
            // `PLAYER_FIELD_BYTES` byte 3 is the highest rank reached.
            honor_rank: self.player_field(vale_protocol::state::fields::player::FIELD_BYTES) >> 24,
            medals: self.player_field(vale_protocol::state::fields::player::PVP_MEDALS),
            team,
            female: player.is_some_and(|id| self.units.sex(id) == 3),
            now: std::time::Instant::now(),
        };
        Some(crate::interface::api::item_tip(
            template,
            carried,
            self.tables.as_deref(),
            &context,
            &wearer,
        ))
    }
}

impl UnitAnswers for Live<'_, '_, '_> {
    fn now(&self) -> f64 {
        self.now
    }

    fn game_time(&self) -> (u32, u32) {
        self.game_clock
    }

    fn bind_location(&self) -> String {
        self.home.clone()
    }

    fn binder_in_range(&self) -> bool {
        crate::interface::binder::binder_in_range(self.binder, self.units)
    }

    fn untrainer_in_range(&self) -> bool {
        crate::interface::untrainer::untrainer_in_range(self.untrainer, self.units)
    }

    fn unit_exists(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.exists(id))
    }

    fn unit_is_visible(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.placed(id).is_some())
    }

    fn unit_in_range(&self, token: &str, index: u32) -> bool {
        let range = match index {
            1 | 4 => 28.0,
            2 => 11.11,
            3 => 9.9,
            _ => return false,
        };
        let Some((_, here)) = self.units.placed(crate::interface::api::UnitId::Player) else {
            return false;
        };
        let Some((_, there)) = Self::id(token).and_then(|id| self.units.placed(id)) else {
            return false;
        };
        here.translation.distance(there.translation) <= range
    }

    fn unit_name(&self, token: &str) -> Option<String> {
        self.units.name(Self::id(token)?).map(str::to_string)
    }

    fn unit_level(&self, token: &str) -> i32 {
        // `level_shown`, not `level`: a unit ten or more levels above a
        // hostile player, and any worldboss, reports `-1`, which
        // `TargetFrame_CheckLevel`'s `targetLevel > 0` test draws as the
        // skull. The rule is on
        // [`crate::interface::api::Units::level_shown`].
        Self::id(token).map_or(-1, |id| self.units.level_shown(self.tables.as_deref(), id))
    }

    fn unit_sex(&self, token: &str) -> u32 {
        // 2 rather than 0 for an unparseable token, as the client answers.
        // See `Units::sex`.
        Self::id(token).map_or(2, |id| self.units.sex(id))
    }

    fn unit_health(&self, token: &str) -> u32 {
        Self::id(token).map_or(0, |id| self.units.health(id))
    }

    fn unit_health_max(&self, token: &str) -> u32 {
        Self::id(token).map_or(0, |id| self.units.health_max(id))
    }

    fn unit_experience(&self, token: &str) -> (u32, u32) {
        Self::id(token).map_or((0, 0), |id| self.units.experience(id))
    }

    fn unit_character_points(&self, token: &str) -> (u32, u32) {
        Self::id(token)
            .and_then(|id| self.units.character_points(id))
            .unwrap_or((0, 0))
    }

    fn rested_experience(&self) -> Option<u32> {
        self.units.rested_experience(Self::id("player")?)
    }

    fn unit_mana(&self, token: &str) -> u32 {
        Self::id(token).map_or(0, |id| self.units.mana(id))
    }

    fn unit_mana_max(&self, token: &str) -> u32 {
        Self::id(token).map_or(0, |id| self.units.mana_max(id))
    }

    fn unit_power_type(&self, token: &str) -> Option<u8> {
        self.units.power_type(Self::id(token)?)
    }

    fn unit_is_connected(&self, token: &str) -> bool {
        Self::id(token).is_none_or(|id| self.units.is_connected(id))
    }

    fn unit_is_dead(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.is_dead(id))
    }

    fn unit_is_ghost(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.is_ghost(id))
    }

    fn release_time_remaining(&self) -> i32 {
        self.dying.release_remaining(self.now as f32)
    }

    fn corpse_recovery_delay(&self) -> i32 {
        self.dying.recovery_delay(self.now as f32)
    }

    fn resurrect_offerer(&self) -> Option<String> {
        self.dying.offer.as_ref().map(|offer| offer.name.clone())
    }

    fn resurrect_has_sickness(&self) -> bool {
        self.dying.offer.as_ref().is_some_and(|offer| offer.sickness)
    }

    fn resurrect_has_timer(&self) -> bool {
        self.dying.offer.as_ref().is_some_and(|offer| offer.timer)
    }

    fn res_sickness_duration(&self) -> Option<String> {
        let me = self.units.get(UnitId::Player)?;
        let (race, _) = me.race_class?;
        let level = me.level?;
        let tables = self.tables.as_deref()?;
        let spell = tables.res_sickness_spell(race)?;
        let ms = tables.spellbook()?.duration_at(spell, level)?;
        // Under a second means no sickness.
        if ms < 1000 {
            return None;
        }
        // The `"GENERIC"` family: `%s_MIN` for a whole number of
        // minutes, `%s_SEC` under one, each with its `_P1` plural. The
        // sickness duration is always whole minutes (60000 ms per level).
        let strings = self.strings?;
        let (count, key) = if ms >= 60_000 {
            let minutes = ms / 60_000;
            (minutes, if minutes == 1 { "GENERIC_MIN" } else { "GENERIC_MIN_P1" })
        } else {
            let seconds = ms / 1000;
            (seconds, if seconds == 1 { "GENERIC_SEC" } else { "GENERIC_SEC_P1" })
        };
        Some(strings.get(key)?.replacen("%d", &count.to_string(), 1))
    }

    fn spirit_healer_in_reach(&self) -> bool {
        let Some(healer) = self.dying.healer else {
            return false;
        };
        self.units
            .reach_to_guid(UnitId::Player, healer)
            .is_some_and(|reach| reach <= vale_protocol::play::gossip::INTERACTION_DISTANCE)
    }

    fn unit_affecting_combat(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.affecting_combat(id))
    }

    fn unit_is_unit(&self, a: &str, b: &str) -> bool {
        match (Self::id(a), Self::id(b)) {
            (Some(a), Some(b)) => self.units.is_unit(a, b),
            _ => false,
        }
    }

    fn unit_stats(&self, token: &str) -> Option<vale_protocol::play::stats::UnitStats> {
        self.units.stats(Self::id(token)?).copied()
    }

    fn unit_race(&self, token: &str) -> Option<(&'static str, &'static str)> {
        self.units.race(Self::id(token)?)
    }

    fn unit_class(&self, token: &str) -> Option<(&'static str, &'static str)> {
        self.units.class(Self::id(token)?)
    }

    fn unit_creature_type(&self, token: &str) -> Option<&'static str> {
        Self::id(token).and_then(|id| self.units.creature_type(id))
    }

    fn unit_classification(&self, token: &str) -> &'static str {
        Self::id(token)
            .and_then(|id| self.units.get(id))
            .map_or("normal", |unit| classification_word(unit.classification))
    }

    fn unit_is_pvp(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.is_pvp(id))
    }

    fn unit_faction_group(&self, token: &str) -> Option<(String, String)> {
        let template = self.units.get(Self::id(token)?)?.faction?;
        let (internal, localised) = self.tables.as_ref()?.faction_group_name(template)?;
        Some((internal.to_string(), localised.to_string()))
    }

    fn unit_tooltip(&self, token: &str) -> Option<crate::interface::api::UnitTip> {
        let id = Self::id(token)?;
        let mut tip = self.units.unit_tip(self.tables.as_deref(), id)?;
        // The zone line is resolved and filtered here, because this is the
        // only place that holds `AreaTable` and knows the character's current
        // zone. A member in the same zone gets no line. See
        // [`crate::interface::api::UnitTip::zone`].
        if let Some(zone) = self.units.party_zone(id) {
            if zone != self.place.zone {
                tip.zone = self
                    .tables
                    .as_ref()
                    .and_then(|tables| tables.areas())
                    .map_or_else(String::new, |areas| areas.zone_name(zone));
            }
        }
        Some(tip)
    }

    fn unit_rank(&self, a: &str, b: &str) -> Option<vale_assets::tables::faction::Rank> {
        // No tables, no answer. Before the archives are open every unit would
        // otherwise read Neutral, which is `Factions::template_rank`'s answer
        // for a missing table. A reaction that looks real but is not is the
        // kind of answer [`self::stubs`]' first line warns about.
        let tables = self.tables.as_ref()?;
        self.units.rank(tables, Self::id(a)?, Self::id(b)?)
    }

    fn unit_can_attack(&self, a: &str, b: &str) -> bool {
        let Some(tables) = self.tables.as_ref() else {
            return false;
        };
        match (Self::id(a), Self::id(b)) {
            (Some(a), Some(b)) => self.units.can_attack(tables, a, b),
            _ => false,
        }
    }

    fn unit_can_assist(&self, a: &str, b: &str) -> bool {
        let Some(tables) = self.tables.as_ref() else {
            return false;
        };
        match (Self::id(a), Self::id(b)) {
            (Some(a), Some(b)) => self.units.can_assist(tables, a, b),
            _ => false,
        }
    }

    fn unit_player_controlled(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.player_controlled(id))
    }
}

/// One aura, flattened for the interface. The remaining time is resolved
/// against this frame's `now`, so a caller never needs to know which time base
/// it was recorded in.
pub(super) fn aura_info(aura: &crate::interface::auras::Aura, now: f64) -> super::panels::auras::AuraInfo {
    super::panels::auras::AuraInfo {
        spell: aura.spell,
        icon: aura.icon.clone(),
        name: aura.name.clone(),
        description: aura.description.clone(),
        applications: aura.applications,
        dispel_type: aura.dispel_type.clone(),
        time_left: aura.time_left(now),
        until_cancelled: aura.expires_at.is_none(),
    }
}

/// Reads that FrameXML stores as function values, not only calls.
///
/// Every other read in this file is a `scope.create_function`: it lives for the
/// length of one call into Lua, which is what lets it borrow the world. A stored
/// reference to one is invalid once the scope closes, and calling it raises
/// "a destructed callback or destructed userdata method was called".
///
/// `StaticPopup.lua` stores three of them:
///
/// ```lua
/// StaticPopupDialogs["RECOVER_CORPSE"] = { StartDelay = GetCorpseRecoveryDelay, … }
/// ```
///
/// The function value is captured into a table at load and called later from
/// `StaticPopup_Show`. The five death reads are therefore persistent closures
/// over this cell, and [`install`] refreshes the cell at the start of every
/// call. With scoped functions, `--audit --events` showed `CORPSE_IN_RANGE`
/// failing on `StartDelay()` at `StaticPopup.lua:1685`.
///
/// As a consequence, these five answer with the values as of the start of the
/// call rather than during it. Both timers are per-frame values and the offer
/// cannot change within a chunk, so the answers are the same. Anything that
/// could change during a chunk must stay a scoped read.
pub(super) type Held = std::rc::Rc<std::cell::RefCell<HeldReads>>;

/// The values [`Held`] holds. A flat struct, because the five reads share
/// nothing except this mechanism.
#[derive(Default)]
pub(super) struct HeldReads {
    pub release: i32,
    pub recovery: i32,
    pub offerer: Option<String>,
    pub sickness: bool,
    pub timer: bool,
}

/// A registered [`Held`] for a test that stands up its own `mlua::Lua`.
///
/// `LuaHost::new` does this once; five test harnesses in this directory build a
/// bare state and would each have to repeat it.
#[cfg(test)]
pub(super) fn held_for_test(lua: &mlua::Lua) -> (Held, self::verbs::Queue) {
    let held: Held = std::rc::Rc::new(std::cell::RefCell::new(HeldReads::default()));
    install_held(lua, &held).expect("the held reads register");
    // Also returns the verb queue [`install`] needs for the six drag verbs.
    // Created here rather than inside `install`, because it must outlive the
    // scope.
    (held, std::rc::Rc::new(std::cell::RefCell::new(Vec::new())))
}

/// Registers the five held reads once, against a cell [`install`] keeps
/// current.
pub(super) fn install_held(lua: &mlua::Lua, held: &Held) -> mlua::Result<()> {
    let globals = lua.globals();
    macro_rules! held {
        ($name:literal, |$read:ident| $body:expr) => {{
            let cell = std::rc::Rc::clone(held);
            let f = lua.create_function(move |_, ()| {
                let $read = cell.borrow();
                Ok($body)
            })?;
            globals.set($name, f)?;
        }};
    }
    held!("GetReleaseTimeRemaining", |r| r.release);
    held!("GetCorpseRecoveryDelay", |r| r.recovery);
    held!("ResurrectGetOfferer", |r| r.offerer.clone());
    held!("ResurrectHasSickness", |r| one_or_nil(r.sickness));
    held!("ResurrectHasTimer", |r| one_or_nil(r.timer));
    Ok(())
}

/// Every read registered below, for the check that counts unimplemented API
/// functions. See [`self::verbs::REGISTERED`], the same list for the write
/// side.
pub const READS: [&str; 67] = [
    "ActionHasRange",
    "CheckBinderDist",
    "CheckInteractDistance",
    "CheckPetUntrainerDist",
    "CheckSpiritHealerDist",
    "ClearTarget",
    "GetActionBarToggles",
    "GetActionCooldown",
    "GetActionCount",
    "GetActionText",
    "GetActionTexture",
    "GetBindLocation",
    "GetBonusBarOffset",
    "GetCorpseRecoveryDelay",
    "GetGameTime",
    "GetQuestGreenRange",
    "GetReleaseTimeRemaining",
    "GetResSicknessDuration",
    "GetTime",
    "GetXPExhaustion",
    "HasAction",
    "IsActionInRange",
    "IsAttackAction",
    "IsAutoRepeatAction",
    "IsConsumableAction",
    "IsCurrentAction",
    "IsEquippedAction",
    "IsUsableAction",
    "ResurrectGetOfferer",
    "ResurrectHasSickness",
    "ResurrectHasTimer",
    "SpellCanTargetUnit",
    "SpellIsTargeting",
    "SpellStopCasting",
    "SpellStopTargeting",
    "UnitAffectingCombat",
    "UnitCanAssist",
    "UnitCanAttack",
    "UnitCanCooperate",
    "UnitCharacterPoints",
    "UnitClassification",
    "UnitCreatureType",
    "UnitExists",
    "UnitFactionGroup",
    "UnitHealth",
    "UnitHealthMax",
    "UnitIsConnected",
    "UnitIsDead",
    "UnitIsDeadOrGhost",
    "UnitIsEnemy",
    "UnitIsFriend",
    "UnitIsGhost",
    "UnitIsPVP",
    "UnitIsPlayer",
    "UnitIsUnit",
    "UnitIsVisible",
    "UnitLevel",
    "UnitMana",
    "UnitManaMax",
    "UnitName",
    "UnitPVPName",
    "UnitPlayerControlled",
    "UnitPowerType",
    "UnitReaction",
    "UnitSex",
    "UnitXP",
    "UnitXPMax",
];

/// `(start, duration, enable)` for something that is not on a cooldown.
///
/// `enable` is true here, as in the game. The third value says whether the
/// button's cooldown sweep may run at all (a passive or a disabled action
/// answers 0); a ready item is enabled with nothing to sweep.
/// `CooldownFrame_SetTimer` draws nothing when the duration is zero,
/// whatever `enable` says.
pub(super) const IDLE_COOLDOWN: (f64, f64, bool) = (0.0, 0.0, true);

/// `UnitClassification`'s five values, from the creature template's rank.
///
/// Lower case and these exact spellings, because `TargetFrame.lua` compares
/// against literals: `"worldboss"`, `"rareelite"`, `"elite"` and `"rare"` choose
/// between the three target-frame borders and anything else gets the plain one.
/// FrameXML uses the answer for nothing else. A constant `"normal"` would draw
/// the plain border around every elite.
pub fn classification_word(rank: u32) -> &'static str {
    match rank {
        1 => "elite",
        2 => "rareelite",
        3 => "worldboss",
        4 => "rare",
        _ => "normal",
    }
}

/// The game's boolean return value: `1` or `nil`, never `true`/`false`. The
/// module comment gives the reason.
///
/// Defined once and also used by [`super::widgets::frames`], because it states
/// a convention of the game's C API; a second copy could diverge.
pub(super) fn one_or_nil(yes: bool) -> mlua::Value {
    if yes {
        mlua::Value::Integer(1)
    } else {
        mlua::Value::Nil
    }
}

/// How a game C function reads a boolean argument. This is not Lua's own
/// truthiness rule.
///
/// FrameXML writes boolean arguments in five different ways and expects all
/// five to work:
///
/// ```lua
/// button:SetChecked(1);        button:SetChecked(0);        -- ActionButton.lua
/// this:SetChecked("true");     this:SetChecked("false");    -- SpellBookFrame.lua
/// skillLineTab:SetChecked(nil);
/// ```
///
/// Under Lua's own rule `0` and `"false"` are both true, so a host that used
/// `lua_toboolean` would draw every action button and every spellbook spell
/// with its `<CheckedTexture>` on: the `CheckButtonHilight` texture, blended
/// additively, over every icon. The 1.12.1 client reads `0` and `"false"` as
/// false, and FrameXML is written for that behaviour.
///
/// The client's rule, by Lua type, with a string decided by its first
/// character:
///
/// ```text
/// nil / none        false
/// boolean           itself
/// number            (int)n != 0      — truncated, so 0.5 is false
/// string            first character:  '0' false; '1'..'9' true;
///                                     f/F/n/N false;  t/T/y/Y true;
///                   anything else falls to four case-insensitive compares:
///                     "off", "disabled" false;  "on", "enabled" true;
///                     no match -> the caller's own default
/// anything else     the caller's own default
/// ```
///
/// `default` is the value the caller supplies for the cases the rule does not
/// decide. Every widget setter in the client uses true, which is why
/// `SetChecked({})` checks a button.
///
/// One known deviation: the client keeps only the low byte of the truncated
/// number, so `SetChecked(256)` is false there and true here. Nothing in
/// FrameXML or in any known addon passes a boolean as 256.
pub(super) fn to_boolean(value: Option<&mlua::Value>, default: bool) -> bool {
    match value {
        None | Some(mlua::Value::Nil) => false,
        Some(mlua::Value::Boolean(b)) => *b,
        Some(mlua::Value::Integer(n)) => *n != 0,
        // Truncation towards zero, so 0.5 is false.
        Some(mlua::Value::Number(n)) => (*n as i64) != 0,
        Some(mlua::Value::String(s)) => {
            let text = s.to_string_lossy();
            match text.as_bytes().first() {
                Some(b'0') => false,
                Some(b'1'..=b'9') => true,
                Some(b'f' | b'F' | b'n' | b'N') => false,
                Some(b't' | b'T' | b'y' | b'Y') => true,
                _ => {
                    if text.eq_ignore_ascii_case("off") || text.eq_ignore_ascii_case("disabled") {
                        false
                    } else if text.eq_ignore_ascii_case("on")
                        || text.eq_ignore_ascii_case("enabled")
                    {
                        true
                    } else {
                        default
                    }
                }
            }
        }
        Some(_) => default,
    }
}

/// Registers the reads into a scope, for the length of one call into Lua.
///
/// Called by [`super::host::LuaHost::run`] and nowhere else, so no chunk can
/// run without them. See the module comment.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
    held: &Held,
    // The verb queue, used only by the six drag verbs. Every other write in
    // this client is registered once by [`self::verbs::register`] and never
    // needs the world. `PutItemInBag` both answers and records, so it must be
    // a scoped function with access to the queue. See
    // [`super::panels::container`].
    queue: &'env self::verbs::Queue,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    // A unit token argument. Optional because addons do call `UnitHealth()`
    // with no argument, and the game answers it rather than raising an error,
    // so a missing token is an absent unit.
    macro_rules! unit {
        ($name:expr, |$token:ident| $body:expr) => {{
            let f = scope.create_function(move |_, token: Option<String>| {
                let $token: &str = token.as_deref().unwrap_or("");
                Ok($body)
            })?;
            globals.set($name, f)?;
        }};
    }
    // An action-slot argument, one-based like every action id in the game.
    macro_rules! slot {
        ($name:expr, |$slot:ident| $body:expr) => {{
            let f = scope.create_function(move |_, slot: Option<u8>| {
                // Slot 0 does not exist, so an absent argument becomes 0 and
                // every accessor answers "nothing there" rather than reading
                // the first button.
                let $slot: u8 = slot.unwrap_or(0);
                Ok($body)
            })?;
            globals.set($name, f)?;
        }};
    }

    globals.set(
        "GetTime",
        scope.create_function(move |_, ()| Ok(answers.now()))?,
    )?;

    // `GetGameTime()`: the world's hour and minute. This is a different clock
    // from `GetTime`'s: `GetTime` counts seconds since this process started,
    // and `GetGameTime` is the time the sky is lit for. See
    // [`UnitAnswers::game_time`] for why a constant here stops
    // `GameTimeFrame` rather than only reporting a wrong time.
    globals.set(
        "GetGameTime",
        scope.create_function(move |_, ()| Ok(answers.game_time()))?,
    )?;

    // The two hearthstone reads, which have no panel of their own:
    // `GetBindLocation` is read by three `StaticPopupDialogs` entries and
    // `CheckBinderDist` by one `OnUpdate`. See [`crate::interface::binder`].
    globals.set(
        "GetBindLocation",
        scope.create_function(move |_, ()| Ok(answers.bind_location()))?,
    )?;
    globals.set(
        "CheckBinderDist",
        scope.create_function(move |_, ()| Ok(answers.binder_in_range()))?,
    )?;
    globals.set(
        "CheckPetUntrainerDist",
        scope.create_function(move |_, ()| Ok(answers.untrainer_in_range()))?,
    )?;

    // `GetXPExhaustion()`: the rested pool, and `nil` when there is none.
    // `ExhaustionTick_Update` tests for nil (`if ( not exhaustionThreshold )`),
    // so a zero here would draw the blue tick at the left edge of the bar
    // instead of hiding it. Always the player's: the game takes no unit
    // argument.
    globals.set(
        "GetXPExhaustion",
        scope.create_function(move |_, ()| Ok(answers.rested_experience()))?,
    )?;

    // `GetBonusBarOffset()`: which of the four bonus bars is shown. No
    // arguments and no unit: it is always about the player, whose form
    // `SpellShapeshiftForm.dbc` maps to a bar. Zero is the normal paged bar,
    // and `BonusActionBar_OnEvent` hides the frame on it.
    globals.set(
        "GetBonusBarOffset",
        scope.create_function(move |_, ()| Ok(answers.bonus_bar_offset()))?,
    )?;

    // `GetActionBarToggles()`: the four extra action bars. It is the only read
    // in this file that answers four values.
    //
    // `UIParent.lua`'s `PLAYER_ENTERING_WORLD` arm is the only caller:
    //
    // ```lua
    // SHOW_MULTI_ACTIONBAR_1, ..._2, ..._3, ..._4 = GetActionBarToggles();
    // MultiActionBar_Update();
    // ```
    //
    // If the read answers nothing, the four frames exist and lay out correctly
    // but never appear. Each value is `1` or `nil` rather than a boolean,
    // because `MultiBar1_IsVisible` hands the same value back to a checkbox's
    // `func` and `OptionsFrame` tests it against `1`.
    //
    // The write takes a fifth toggle, "Always Show ActionBars", which is not
    // returned here: the 1.12.1 client's read answers four toggles, so the
    // fifth is never read back from the wire. See
    // [`vale_protocol::play::spells::multi_bar`].
    globals.set(
        "GetActionBarToggles",
        scope.create_function(move |_, ()| {
            use vale_protocol::play::spells::multi_bar;
            let mask = answers.action_bar_toggles();
            let on = |bit: u8| one_or_nil(mask & bit != 0);
            Ok((
                on(multi_bar::BOTTOM_LEFT),
                on(multi_bar::BOTTOM_RIGHT),
                on(multi_bar::RIGHT),
                on(multi_bar::LEFT),
            ))
        })?,
    )?;

    // `GetQuestGreenRange()`: the level band below the player's in which a
    // target is drawn green. `TargetFrame_CheckLevel` colours the level number
    // through `GetDifficultyColor`, which calls this last. When the global was
    // missing, the call raised and `TargetFrame_Update` stopped at its second
    // line. Nothing after that line ran: the name plate kept its untinted art
    // (no friendly or hostile colour), the classification, the dead check and
    // `TargetPortrait:SetAlpha` were skipped, and `TargetDebuffButton_Update`
    // never hid the twenty-one aura buttons the markup ships visible, so the
    // frame showed blank buff slots. The path only runs when `UnitCanAttack`
    // is true, so a friendly target drew the frame correctly and a hostile one
    // broke it again.
    //
    // It is a read rather than a stub because it answers the world: the player's
    // own level indexes the table. See [`api::quest_green_range`] for the
    // twenty values and why the server's arithmetic is not a substitute.
    globals.set(
        "GetQuestGreenRange",
        scope.create_function(move |_, ()| {
            Ok(api::quest_green_range(answers.unit_level("player")))
        })?,
    )?;

    unit!("UnitExists", |t| one_or_nil(answers.unit_exists(t)));
    unit!("UnitIsVisible", |t| one_or_nil(answers.unit_is_visible(t)));
    globals.set(
        "CheckInteractDistance",
        scope.create_function(move |_, (token, index): (Option<String>, Option<u32>)| {
            Ok(one_or_nil(answers.unit_in_range(token.as_deref().unwrap_or(""), index.unwrap_or(0))))
        })?,
    )?;
    unit!("UnitName", |t| answers.unit_name(t));
    // `UnitPVPName`: the name with the PvP rank title in front of it, as in
    // "Sergeant Bram". This client models no honour rank (the `PLAYER_FIELD_*`
    // rank fields are unread, and the rank titles are `PVP_RANK_*` in
    // `GlobalStrings.lua`), so it answers the bare name. The 1.12.1 client
    // answers the same for a character below rank 1, which is most
    // characters. This is a stated deviation, not an alias: once the rank is
    // read, this read must add the prefix.
    //
    // It is here rather than in [`self::stubs`] because it answers the world.
    // `CharacterFrame_OnShow`'s third line is `CharacterNameText:SetText(
    // UnitPVPName("player"))`. With the name nil, that function raised there
    // and the character sheet's title bar kept the placeholder "Name" from its
    // `<FontString text="Name">`.
    unit!("UnitPVPName", |t| answers.unit_name(t));
    unit!("UnitLevel", |t| answers.unit_level(t));
    // `UnitSex` is the one read `ReputationFrame_Update` makes before its loop.
    // A nil here raised on line 44, every `Hide()` after it was skipped, and
    // the panel drew fifteen empty bars.
    unit!("UnitSex", |t| answers.unit_sex(t));
    unit!("UnitHealth", |t| answers.unit_health(t));
    unit!("UnitHealthMax", |t| answers.unit_health_max(t));
    unit!("UnitMana", |t| answers.unit_mana(t));
    // The XP bar's two numbers. The maximum decides whether the bar is drawn:
    // `MainMenuExpBar_Update` feeds both to `SetMinMaxValues`, and
    // `TextStatusBar_UpdateTextString` hides a bar whose maximum is zero. While
    // these were stubs answering zero, the XP bar was not drawn at all.
    unit!("UnitXP", |t| answers.unit_experience(t).0);
    unit!("UnitXPMax", |t| answers.unit_experience(t).1);
    unit!("UnitManaMax", |t| answers.unit_mana_max(t));
    // `UnitCharacterPoints`: the two unspent-point pools. The talent panel and
    // the skills panel both read it. Both are numbers rather than nil for a unit
    // that is not there: `TalentFrame_UpdateTalentPoints` puts the first
    // straight into a `FontString` and `SkillFrame_UpdateSkills` compares the
    // second, and neither guards. See [`super::panels::talent`].
    globals.set(
        "UnitCharacterPoints",
        scope.create_function(move |_, token: Option<String>| {
            Ok(answers.unit_character_points(token.as_deref().unwrap_or("")))
        })?,
    )?;
    // `UnitPowerType` answers mana (`0`) for a unit that is not there, because
    // the interface code requires it. `PetFrame`, `TargetofTargetFrame` and
    // the four `PartyMemberFrame`s all call `UnitFrame_UpdateManaType` from
    // their `OnLoad`, at a login where none of those units exists, and the
    // next line is `ManaBarColor[UnitPowerType(unit)].r`. A nil answer would
    // raise in all six frames on every login; the 1.12.1 client's does not.
    // `0` is `ManaBarColor`'s first row.
    unit!("UnitPowerType", |t| answers.unit_power_type(t).unwrap_or(0));
    // Only a party member can be disconnected. Every unit the world knows
    // about is connected, and the 1.12.1 client answers the same. The party
    // roster's status byte is the only source that says otherwise, for a
    // member in another zone. The read answered a constant `1` before party
    // support existed, and `UnitFrameManaBar_Update` greys the whole bar on a
    // nil, so the answer is unchanged for every token but `party<n>`.
    unit!("UnitIsConnected", |t| one_or_nil(
        answers.unit_is_connected(t)
    ));
    unit!("UnitIsDead", |t| one_or_nil(answers.unit_is_dead(t)));
    // `UnitIsDead`, `UnitIsGhost` and `UnitIsDeadOrGhost` give three separate
    // answers. During a corpse run the player is a ghost and not dead
    // (`PLAYER_FLAGS_GHOST`, health 1), and this client models that, so
    // `UnitIsDeadOrGhost` is the union of the other two. Both
    // `FriendsFrameAddFriendButton` and `PetitionFrameRenameButton` open with
    // it. The interface uses a dead-or-ghost check to refuse an action to a
    // corpse; a check on `UnitIsDead` alone would pass a ghost, which has 1
    // hit point.
    unit!("UnitIsGhost", |t| one_or_nil(answers.unit_is_ghost(t)));
    unit!("UnitIsDeadOrGhost", |t| one_or_nil(
        answers.unit_is_dead(t) || answers.unit_is_ghost(t)
    ));
    unit!("UnitAffectingCombat", |t| one_or_nil(
        answers.unit_affecting_combat(t)
    ));

    // The death reads (the release and corpse-recovery clocks, and the
    // resurrection offer) are written into [`Held`] rather than registered
    // here. See that type, which says why these five
    // cannot be scoped functions like every other read in this file.
    *held.borrow_mut() = HeldReads {
        release: answers.release_time_remaining(),
        recovery: answers.corpse_recovery_delay(),
        offerer: answers.resurrect_offerer(),
        sickness: answers.resurrect_has_sickness(),
        timer: answers.resurrect_has_timer(),
    };
    // The spirit healer's two reads are scoped functions, because both are
    // called during a chunk rather than stored by one: `UIParent_OnEvent` reads the
    // duration on `CONFIRM_XP_LOSS`, and the `XP_LOSS` box's `OnUpdate` asks
    // the distance every tick. See [`Answers::res_sickness_duration`].
    globals.set(
        "GetResSicknessDuration",
        scope.create_function(move |_, ()| Ok(answers.res_sickness_duration()))?,
    )?;
    globals.set(
        "CheckSpiritHealerDist",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.spirit_healer_in_reach())))?,
    )?;

    // Two tokens rather than one, so it does not fit the macro above.
    globals.set(
        "UnitIsUnit",
        scope.create_function(move |_, (a, b): (Option<String>, Option<String>)| {
            let token = |t: &Option<String>| t.as_deref().unwrap_or("").to_string();
            Ok(one_or_nil(answers.unit_is_unit(&token(&a), &token(&b))))
        })?,
    )?;

    // Friend or foe: five names answered from one reaction rank. All of them
    // take two tokens, so they use the `pair!` macro, shaped like `UnitIsUnit`
    // above, rather than `unit!`.
    //
    // These were stubs answering nil. Besides the missing attackable border on
    // the target frame, `TargetDebuffButton_Update` branches on
    // `UnitIsFriend("player", "target")` to decide where the aura rows go, so
    // with it nil every target took the hostile layout and a friendly target's
    // buffs were drawn 46 units (two rows) below the frame. This is an example
    // of this directory's first rule: a constant answer cannot be told apart
    // from a working one.
    macro_rules! pair {
        ($name:literal, |$a:ident, $b:ident| $body:expr) => {{
            let f = scope.create_function(move |_, (a, b): (Option<String>, Option<String>)| {
                let $a: &str = a.as_deref().unwrap_or("");
                let $b: &str = b.as_deref().unwrap_or("");
                Ok($body)
            })?;
            globals.set($name, f)?;
        }};
    }

    // `UnitReaction`: the rank, used as an index rather than a measurement.
    // `TargetFrame_CheckFaction` uses it only as `UnitReactionColor[reaction]`,
    // an eight-row table that is red at 1..2, orange at 3, yellow at 4 and
    // green at 5..8. The value is the reaction rank plus one, with no other
    // change. It is `nil` for an absent unit, which that function handles in a
    // separate branch.
    use vale_assets::tables::faction::Reaction;
    pair!("UnitReaction", |a, b| answers
        .unit_rank(a, b)
        .map(vale_assets::tables::faction::Rank::lua));
    pair!("UnitIsFriend", |a, b| one_or_nil(
        answers.unit_rank(a, b).map(Reaction::from) == Some(Reaction::Friendly)
    ));
    pair!("UnitIsEnemy", |a, b| one_or_nil(
        answers.unit_rank(a, b).map(Reaction::from) == Some(Reaction::Hostile)
    ));
    pair!("UnitCanAttack", |a, b| one_or_nil(
        answers.unit_can_attack(a, b)
    ));
    // `UnitCanAssist` follows the 1.12.1 client's rule, which for a player
    // asking about a friendly creature is the creature's PvP flag; see
    // `Factions::can_assist`. `UnitCanCooperate` answers "friendly", a stated
    // deviation: in the 1.12.1 client it means the two may party, trade and
    // duel, which requires the other unit to be a player of a faction group
    // that can group with ours. Every caller in the interface asks about a
    // player target (`UnitPopup`'s trade and invite entries, `FriendsFrame`'s
    // add-friend), and for a player the two answers agree.
    pair!("UnitCanAssist", |a, b| one_or_nil(answers.unit_can_assist(a, b)));
    pair!("UnitCanCooperate", |a, b| one_or_nil(
        answers.unit_rank(a, b).map(Reaction::from) == Some(Reaction::Friendly)
    ));
    unit!("UnitPlayerControlled", |t| one_or_nil(
        answers.unit_player_controlled(t)
    ));
    // `UnitIsPlayer` answers the same as `UnitPlayerControlled` here. In the
    // 1.12.1 client they differ for a pet, which is player-controlled but not
    // a player. This client has no pet; [`Units::player_controlled`] records
    // that deviation. When pets are added, both reads must change together.
    unit!("UnitIsPlayer", |t| one_or_nil(
        answers.unit_player_controlled(t)
    ));
    // The three reads the name plate and the target frame's border are built
    // from. They were in [`self::stubs`]: `UnitClassification` answered the constant
    // `"normal"`, which drew the ordinary border round every elite, and the
    // other two answered nil.
    unit!("UnitIsPVP", |t| one_or_nil(answers.unit_is_pvp(t)));
    // `UnitFactionGroup` returns two values; the interface relies on the
    // first. `PartyMemberFrame_UpdatePvPStatus` concatenates it into a texture
    // path, so it must be the English word whatever the client's locale. That
    // is why `FactionGroup.dbc` carries `internalName` beside the eight
    // localised names.
    // Two returns, so this one cannot go through `unit!`, which answers a
    // single value.
    globals.set(
        "UnitFactionGroup",
        scope.create_function(move |_, token: Option<String>| {
            Ok(match answers.unit_faction_group(token.as_deref().unwrap_or("")) {
                Some((internal, localised)) => (Some(internal), Some(localised)),
                None => (None, None),
            })
        })?,
    )?;
    unit!("UnitCreatureType", |t| answers.unit_creature_type(t));
    unit!("UnitClassification", |t| answers.unit_classification(t));

    // The spell cursor's two reads, `SpellIsTargeting` and
    // `SpellCanTargetUnit`. `UnitFrame_OnEnter` is
    // `if SpellIsTargeting() then SetCursor(SpellCanTargetUnit(this.unit) and
    // "CAST_CURSOR" or "CAST_ERROR_CURSOR") end`: the interface chooses the
    // pointer over a unit frame. This client's world pick
    // (`interface::target::spell_cursor_validity`) answers the same question
    // for the 3D scene. `SetCursor` is still a stub, so only the world pick
    // changes the pointer on screen; the frames' answers are correct but
    // unused.
    let f = scope.create_function(|_, ()| Ok(one_or_nil(answers.spell_is_targeting())))?;
    globals.set("SpellIsTargeting", f)?;

    // --- `ToggleGameMenu`'s three reads that also act ---
    //
    // Escape is bound to `TOGGLEGAMEMENU` in the game's shipped defaults, and
    // that binding's body is a seven-branch `elseif` chain:
    // `StaticPopup_EscapePressed()`, `OptionsFrame`, `GameMenuFrame`,
    // `CloseMenus()`, `SpellStopCasting()`, `SpellStopTargeting()`,
    // `CloseAllWindows()`, `ClearTarget()`. The menu opens only if none of
    // them did anything.
    //
    // These three are here and not in [`self::verbs`] because the chain uses
    // their return value: `elseif ( SpellStopCasting() )` means "a cast was
    // cancelled, so do not open the menu". A verb answering nil would open the
    // game menu on every Escape, including mid-cast. They have the same shape
    // as `PutItemInBag`: a read off the world, plus a write pushed on the
    // queue verbs write to.
    //
    // Before the key-bindings panel, two other files read
    // `just_pressed(KeyCode::Escape)` for all three actions. Escape then did
    // all three at once, and none of them could be rebound. See
    // `interface::action::stop_casting` and `interface::target`'s
    // `ClearTarget` arm.
    {
        let queue = std::rc::Rc::clone(queue);
        let f = scope.create_function(move |_, ()| {
            let casting = answers.spell_is_casting();
            if casting {
                queue.borrow_mut().push(crate::input::bindings::Binding::SpellStopCasting);
            }
            Ok(one_or_nil(casting))
        })?;
        globals.set("SpellStopCasting", f)?;
    }
    {
        let queue = std::rc::Rc::clone(queue);
        let f = scope.create_function(move |_, ()| {
            let targeting = answers.spell_is_targeting();
            if targeting {
                queue
                    .borrow_mut()
                    .push(crate::input::bindings::Binding::SpellStopTargeting);
            }
            Ok(one_or_nil(targeting))
        })?;
        // Registered here rather than in `verbs.rs`. As a verb it answered
        // nothing, which made no difference to its only caller outside this
        // chain, `SpellButton_OnClick`'s `else` branch, because that caller
        // discards the answer. The chain uses it.
        globals.set("SpellStopTargeting", f)?;
    }
    {
        let queue = std::rc::Rc::clone(queue);
        let f = scope.create_function(move |_, ()| {
            // The answer is `UnitExists("target")`, which is the question the
            // chain's branch asks. It needs no state of its own.
            let had = answers.unit_exists("target");
            if had {
                queue.borrow_mut().push(crate::input::bindings::Binding::ClearTarget);
            }
            Ok(one_or_nil(had))
        })?;
        globals.set("ClearTarget", f)?;
    }
    unit!("SpellCanTargetUnit", |t| one_or_nil(
        answers.spell_can_target_unit(t)
    ));

    slot!("HasAction", |s| one_or_nil(answers.has_action(s)));
    slot!("GetActionText", |s| answers.action_text(s));
    slot!("GetActionTexture", |s| answers.action_texture(s));
    slot!("IsAttackAction", |s| one_or_nil(answers.is_attack_action(s)));
    slot!("IsCurrentAction", |s| one_or_nil(
        answers.is_current_action(s)
    ));
    slot!("IsAutoRepeatAction", |s| one_or_nil(
        answers.is_auto_repeat_action(s)
    ));
    // The range pair. `IsActionInRange` is the one read in this file where
    // `nil` and `0` are different answers. `ActionButton_OnUpdate` tests
    // `IsActionInRange(button) == 0` for the red hotkey and `== 1` for the
    // range dot, so an unanswerable question (no target, or a unit the
    // renderer has not placed) must return nil. `one_or_nil(false)` would also
    // return nil, but it would turn a real "out of range" into nil too. The
    // answer is therefore `Option<bool>` all the way down rather than a bool.
    slot!("ActionHasRange", |s| one_or_nil(answers.action_has_range(s)));
    slot!("IsActionInRange", |s| answers
        .is_action_in_range(s)
        .map(u32::from));
    // The item slot's three reads: the count under the icon, and the green
    // border round it. `GetActionCount` answers a number for every slot and
    // `ActionButton_UpdateCount` writes it only under a button the first of
    // these says yes to, so the two belong together.
    slot!("IsConsumableAction", |s| one_or_nil(
        answers.is_consumable_action(s)
    ));
    slot!("IsEquippedAction", |s| one_or_nil(
        answers.is_equipped_action(s)
    ));
    slot!("GetActionCount", |s| answers.action_count(s));
    // The two that return more than one value. `mlua` turns a tuple into a
    // multi-return, which is what `local start, duration, enable = …` needs.
    slot!("GetActionCooldown", |s| {
        let (start, duration, enable) = answers.action_cooldown(s);
        (start, duration, one_or_nil(enable))
    });
    slot!("IsUsableAction", |s| {
        let (usable, not_enough_mana) = answers.action_usable(s);
        (one_or_nil(usable), one_or_nil(not_enough_mana))
    });

    // Each panel's reads live in that panel's own module, one subject per
    // module, as elsewhere in this directory. The spellbook has seven; the
    // character sheet (`paperdoll`) has fourteen.
    super::panels::spellbook::install(lua, scope, answers)?;
    super::panels::auras::install(lua, scope, answers)?;
    super::panels::paperdoll::install(lua, scope, answers)?;
    super::panels::container::install(lua, scope, answers, queue)?;
    super::panels::loot::install(lua, scope, answers)?;
    super::panels::lootroll::install(lua, scope, answers)?;
    super::panels::quest::install(lua, scope, answers)?;
    super::panels::gossip::install(lua, scope, answers)?;
    super::panels::merchant::install(lua, scope, answers, queue)?;
    super::panels::mail::install(lua, scope, answers)?;
    super::panels::trainer::install(lua, scope, answers)?;
    // The stable master's seven reads. One of them aims a `<PlayerModel>` and
    // is scoped for that reason; see [`super::panels::stable`].
    super::panels::stable::install(lua, scope, answers)?;
    // The bank's three reads. Without them the bank's thirty buttons would
    // read the paper doll's; see [`super::panels::bank`].
    super::panels::bank::install(lua, scope, answers)?;
    // The six reads a sign, a plaque or a book makes. This is the one window
    // in the directory whose subject is an object; see
    // [`super::panels::pagetext`].
    super::panels::pagetext::install(lua, scope, answers)?;
    // The summon popup's three reads. See [`super::panels::summon`].
    super::panels::summon::install(lua, scope, answers)?;
    // The options panel's two reads that are not CVars. See
    // [`super::panels::uioptions`].
    super::panels::uioptions::install(lua, scope, answers)?;
    // The inspect window's four reads. See [`super::panels::inspect`].
    super::panels::inspect::install(lua, scope, answers)?;
    // `GetGuildInfo`, which reads a unit's guild fields. See
    // [`super::panels::guild`].
    super::panels::guild::install(lua, scope, answers)?;
    super::panels::trade::install(lua, scope, answers)?;
    // The two profession windows. Their create buttons push onto the same
    // queue the bags' drag uses; see [`super::panels::tradeskill`].
    super::panels::tradeskill::install(lua, scope, answers, queue)?;
    super::panels::craft::install(lua, scope, answers, queue)?;
    super::panels::talent::install(lua, scope, answers)?;
    super::panels::taxi::install(lua, scope, answers)?;
    super::panels::worldmap::install(lua, scope, answers)?;
    // The party reads. The party is the one unit subject whose members may
    // not be in the world at all; see [`super::panels::party`].
    super::panels::party::install(lua, scope, answers)?;
    // The raid's eight reads, which ask different questions of the same
    // roster; see [`super::panels::raid`].
    super::panels::raid::install(lua, scope, answers)?;
    // Group members' pets. Their tokens are derived from another unit's
    // fields rather than held; see [`super::panels::pet`].
    super::panels::pet::install(lua, scope, answers)?;
    super::panels::shapeshift::install(lua, scope, answers)?;
    // The two screens shown before there is a world. Their reads are about a
    // session that has not started; see [`super::panels::glue`].
    super::panels::glue::install(lua, scope, answers)?;

    // The two tooltip populations. They answer onto a widget rather than
    // into an expression, but the value must still be the world's at the
    // moment of the call.
    super::widgets::tooltip::install_scoped(lua, scope, answers)?;

    Ok(())
}

/// The stub world every test in this directory answers from.
///
/// `pub(crate)` rather than `pub(super)`: [`crate::settings::cvars`]'s own test
/// drives a real [`super::host::LuaHost`] to check that a `SetCVar` reaches the
/// resource, and a chunk cannot run without an `Answers`. `#[cfg(test)]`, so
/// this widens nothing a shipped build can see.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// One unit in a [`Stub`] world.
    pub struct StubUnit {
        pub token: String,
        pub name: String,
        /// The zone line on a party member's plate. Empty for every other
        /// unit. See [`crate::interface::api::UnitTip::zone`].
        pub zone: String,
        pub level: i32,
        pub health: u32,
        pub health_max: u32,
        pub dead: bool,
        /// `UnitIsGhost`: released, which is not the same as [`Self::dead`].
        pub ghost: bool,
        pub in_combat: bool,
        /// The identity `UnitIsUnit` compares. Two tokens naming one creature
        /// share it, which is how `TARGETSELF`'s branch is tested.
        pub guid: u64,
        /// `UnitPlayerControlled`: a player controls it rather than the server.
        pub player: bool,
        /// The three fields the unit plate's second and fourth lines are built from: a
        /// `CreatureType.dbc` row, a classification (0 normal … 4 rare), and the
        /// PvP flag. Zero and false is a plain, unflagged creature.
        pub creature_type: u32,
        pub classification: u32,
        pub pvp: bool,
        /// `<Innkeeper>`.
        pub sub_name: String,
        /// The honor rank and the owner the unit tooltip's first and third
        /// lines are built from. The default is a unit with neither.
        pub title: crate::interface::api::UnitTitle,
    }

    /// One slot of a [`Stub`]'s bar.
    pub struct StubAction {
        pub slot: u8,
        pub text: String,
        pub texture: String,
        pub cooldown: (f64, f64, bool),
        pub usable: bool,
        pub not_enough_mana: bool,
        pub attack: bool,
        pub current: bool,
        /// Whether this slot is the ranged attack currently repeating: the
        /// state `ActionButton_UpdateState` reads beside [`Self::current`].
        pub auto_repeat: bool,
        /// The item half of a slot: how many are carried, whether the stack
        /// count is drawn, and whether it is being worn. All three are `0`/false
        /// for a spell, which is what every slot a test builds through
        /// [`Stub::action`] is.
        pub count: u32,
        pub consumable: bool,
        pub equipped: bool,
        /// The range pair. It is two fields because the interface reads three
        /// states from them: no range at all, in range, and out of range.
        /// `in_range` is `None` for a slot that has a range and nothing to
        /// measure it against (no target). A single bool cannot express that,
        /// so the trait method returns an `Option`.
        pub has_range: bool,
        pub in_range: Option<bool>,
    }

    /// One row of a [`Stub`]'s spellbook.
    pub struct StubSpell {
        pub name: String,
        pub rank: String,
        pub passive: bool,
    }

    /// A stub world. [`Answers`] being a trait is what makes it possible; see
    /// the module comment. Everything a test needs to set is a public field.
    #[derive(Default)]
    pub struct Stub {
        pub now: f64,
        pub units: Vec<StubUnit>,
        pub actions: Vec<StubAction>,
        /// The book, flat, in the order [`vale_assets::tables::book`] would
        /// sort it. A test states the order it wants rather than building one.
        pub spells: Vec<StubSpell>,
        /// `(name, offset, count)` per tab, which is what `GetSpellTabInfo`
        /// answers. Not derived from `spells`: the panel's arithmetic depends
        /// on these two agreeing, so a test that checks the agreement must be
        /// able to state both.
        pub tabs: Vec<(String, usize, usize)>,
        /// `(token, aura)` pairs in the order they were added, which is the
        /// order the player's own bar keeps. See [`crate::interface::auras`].
        pub auras: Vec<(String, crate::lua::panels::auras::AuraInfo)>,
        /// The character sheet's block, for whichever unit answers `"player"`.
        /// [`Stub::stats`] builds it from real update fields, so the tests over
        /// `lua::paperdoll` exercise the same decode the client runs.
        pub stats: Option<vale_protocol::play::stats::UnitStats>,
        /// What every item answers for its cooldown. See
        /// [`Stub::container_item_cooldown`]. `(0, 0, false)` by `Default`,
        /// which is the "nothing there" triple with the swirl disabled.
        pub item_cooldown: (f64, f64, bool),
        /// Which of the four extra action bars are on, as the `multi_bar`
        /// mask the wire carries. `0` by `Default`, which is a fresh account
        /// with all four bars off. See [`super::ActionAnswers::action_bar_toggles`].
        pub bar_toggles: u8,
        /// The rows of an open loot window, one-based when read, with the
        /// coins first if there are any. See [`super::loot`]. Empty by
        /// `Default`: nothing has been looted, and `GetNumLootItems` answers 0.
        pub loot: Vec<crate::lua::panels::loot::LootRow>,
        /// The one group roll a test may have open, under id 0. Id 0 is a real
        /// id, since the roll counter starts there. `None` by `Default`, so
        /// every id answers nothing.
        pub roll: Option<crate::lua::panels::lootroll::RollItem>,
        /// Parallel to [`Stub::auras`]. See [`Stub::buff`].
        helpful: Vec<bool>,
        /// How every unit in this world stands towards every other. One value
        /// rather than a matrix, because the directory only asks about
        /// `("player", "target")` in one order or the other; a test that
        /// wants a different standing builds a second world.
        standing: vale_assets::tables::faction::Reaction,
        /// What `UpdateMapHighlight` answers. `None` is the ordinary case: the
        /// pointer over open water, and every point on a zone map.
        highlight: Option<crate::lua::panels::worldmap::Highlight>,
        /// The explored overlays on the map. Empty is the ordinary case: a
        /// client with no world, or a zone nobody has walked.
        overlays: Vec<crate::lua::panels::worldmap::OverlayArt>,
        /// The party: `(name, is the leader)` per member, in `party1..N`
        /// order. Empty is a solo character, which is what most of this
        /// directory's tests use.
        party: Vec<(String, bool)>,
        /// The raid: `(name, subgroup byte)` for the members the server
        /// names, without the local player. [`crate::interface::raid`]
        /// appends the local player as the last slot, and this stub's answer
        /// does the same. Empty means a party, not a raid.
        raid: Vec<(String, u8)>,
        /// Death state, as five plain fields: the two clocks and the three
        /// parts of a resurrection offer. Flat rather than an
        /// `Option<ResurrectOffer>` because every test that touches them sets
        /// one value.
        release_remaining: i32,
        recovery_delay: i32,
        offerer: Option<String>,
        offer_sickness: bool,
        offer_timer: bool,
        /// The bags: `(bag id, [(entry, count)])`, one-based when read.
        /// An entry of 0 is an empty slot, which is how a bag with a gap in
        /// the middle is written.
        containers: Vec<(i32, Vec<(u32, u32)>)>,
        /// `(inventory slot id, entry, count)`: the worn slots, in the
        /// interface's numbering.
        worn: Vec<(u32, u32, u32)>,
        /// Which item occupies each bag slot, for `GetBagName`.
        bag_entries: Vec<(i32, u32)>,
        /// Item templates, kept separate from the slots deliberately. See
        /// [`Stub::bags`]. A slot whose entry is not here is a stack whose
        /// `CMSG_ITEM_QUERY_SINGLE` has not come back, which is the state every
        /// bag is in for the first second of a login.
        templates: Vec<StubTemplate>,
        money: u32,
    }

    /// One item template a [`Stub`] knows about.
    #[derive(Clone)]
    pub struct StubTemplate {
        pub entry: u32,
        pub name: String,
        pub icon: String,
        pub quality: u32,
        pub inventory_type: u32,
        /// The two fields the requirement lines are drawn from. The stub
        /// carries them so tests can check the colour those lines are drawn in.
        pub required_level: u32,
        pub level_met: bool,
    }

    impl Stub {
        /// A character carrying items: a backpack with twenty linen in slot
        /// one and an unresolved stack in slot two, a worn sword, and
        /// templates for two of the three.
        ///
        /// The third has no template on purpose. That makes the `-1` quality
        /// and the missing icon checkable, and it is the state every bag is in
        /// for the first second of a login. See
        /// [`crate::interface::items`], where the two-phase fill is written up.
        pub fn bags(mut self) -> Stub {
            self.containers = vec![(0, vec![(2589, 20), (858, 5)]), (1, vec![])];
            self.worn = vec![(16, 19019, 1)];
            self.templates = vec![
                StubTemplate {
                    entry: 2589,
                    name: "Linen Cloth".into(),
                    icon: "Interface\\Icons\\INV_Misc_Cloth".into(),
                    quality: 1,
                    inventory_type: 0,
                    required_level: 0,
                    level_met: true,
                },
                StubTemplate {
                    entry: 19019,
                    name: "Thunderfury".into(),
                    icon: "Interface\\Icons\\INV_Sword_39".into(),
                    quality: 5,
                    inventory_type: 21,
                    required_level: 0,
                    level_met: true,
                },
                // A potion whose required level the character does not meet.
                // The requirement lines were added for this case: the level is
                // stated and not met, so the plate must show it in red.
                StubTemplate {
                    entry: 20004,
                    name: "Major Troll's Blood Potion".into(),
                    icon: "Interface\\Icons\\INV_Potion_67".into(),
                    quality: 1,
                    inventory_type: 0,
                    required_level: 45,
                    level_met: false,
                },
            ];
            self.containers[0].1.push((20004, 1));
            self
        }

        fn container(&self, bag: i32) -> Option<&[(u32, u32)]> {
            self.containers
                .iter()
                .find(|(id, _)| *id == bag)
                .map(|(_, slots)| slots.as_slice())
        }

        /// One container slot, one-based, `None` for an empty one.
        fn stub_slot(&self, bag: i32, slot: usize) -> Option<(u32, u32)> {
            let held = *self.container(bag)?.get(slot.checked_sub(1)?)?;
            (held.0 != 0).then_some(held)
        }

        /// One worn slot.
        fn stub_worn(&self, id: u32) -> Option<(u32, u32)> {
            self.worn
                .iter()
                .find(|(slot, _, _)| *slot == id)
                .map(|(_, entry, count)| (*entry, *count))
        }

        fn template(&self, entry: u32) -> Option<&StubTemplate> {
            self.templates.iter().find(|t| t.entry == entry)
        }

        /// A slot's drawable state, on the same rule the live world uses: a
        /// missing template is `-1` and no icon.
        fn contents(&self, (entry, count): (u32, u32)) -> crate::lua::panels::container::SlotContents {
            let template = self.template(entry);
            crate::lua::panels::container::SlotContents {
                texture: template.map(|t| t.icon.clone()),
                count,
                quality: template.map_or(-1, |t| t.quality as i32),
                readable: false,
                broken: false,
                // Nothing is on the stub's cursor. Only a real drag locks a
                // slot, and `cursor_has_item` below answers false.
                locked: false,
            }
        }

        /// A unit at a token, with the fields most tests care about; the rest
        /// take the ordinary values a live level-60 unit has.
        pub fn unit(mut self, token: &str, name: &str, guid: u64) -> Stub {
            self.units.push(StubUnit {
                token: token.to_string(),
                name: name.to_string(),
                level: 60,
                health: 100,
                health_max: 100,
                dead: false,
                ghost: false,
                in_combat: false,
                guid,
                player: false,
                // A plain unflagged humanoid, the ordinary case, which the
                // existing tests assume.
                creature_type: 7,
                classification: 0,
                pvp: false,
                sub_name: String::new(),
                title: Default::default(),
                // Empty for every unit except a group member in another zone.
                zone: String::new(),
            });
            self
        }

        /// Sets the rank and owner of the unit added last. Whether the unit
        /// is a player is taken from the unit and not from `title`.
        pub fn titled(mut self, title: crate::interface::api::UnitTitle) -> Stub {
            if let Some(unit) = self.units.last_mut() {
                unit.title = title;
            }
            self
        }

        /// Sets the zone of the unit added last, for the plate line that only
        /// a party member gets.
        pub fn in_zone(mut self, zone: &str) -> Stub {
            if let Some(unit) = self.units.last_mut() {
                unit.zone = zone.to_string();
            }
            self
        }

        /// Sets the fields the unit plate's lines are built from, on the unit
        /// added last, for tests about the plate rather than the unit.
        pub fn described(
            mut self,
            sub_name: &str,
            creature_type: u32,
            classification: u32,
            pvp: bool,
        ) -> Stub {
            if let Some(unit) = self.units.last_mut() {
                unit.sub_name = sub_name.to_string();
                unit.creature_type = creature_type;
                unit.classification = classification;
                unit.pvp = pvp;
            }
            self
        }

        /// Makes the unit added last a player. A player's plate composes
        /// "Human Warrior" from a race and a class.
        pub fn player_controlled(mut self) -> Stub {
            if let Some(unit) = self.units.last_mut() {
                unit.player = true;
                unit.creature_type = 0;
            }
            self
        }

        /// Sets how this world's units stand towards each other.
        /// `UnitIsFriend`, `UnitIsEnemy`, `UnitReaction` and `UnitCanAttack`
        /// are all answered from this one value.
        pub fn standing(mut self, standing: vale_assets::tables::faction::Reaction) -> Stub {
            self.standing = standing;
            self
        }

        /// Sets what the pointer is over on the world map: the name and the
        /// art, which `WorldMapButton_OnUpdate` reads in one call.
        pub fn highlight(
            mut self,
            name: &str,
            directory: &str,
            size: (f32, f32),
            offset: (f32, f32),
        ) -> Stub {
            self.highlight = Some(crate::lua::panels::worldmap::Highlight {
                name: name.to_string(),
                art: vale_assets::tables::worldmap::MapHighlight {
                    target: vale_assets::tables::worldmap::HighlightTarget::Zone {
                        continent: 0,
                        zone: 0,
                    },
                    directory: directory.to_string(),
                    tex_percentage: (1.0, 85.0 / 128.0),
                    size,
                    offset,
                }
                .into(),
            });
            self
        }

        /// Sets the explored overlays on the map, for the two reads
        /// `WorldMapFrame_Update` iterates them with.
        pub fn overlays(mut self, overlays: Vec<crate::lua::panels::worldmap::OverlayArt>) -> Stub {
            self.overlays = overlays;
            self
        }

        /// Sets the party the reads answer from, one entry per `party<n>`.
        pub fn party(mut self, members: &[(&str, bool)], count: usize) -> Stub {
            let _ = count;
            self.party = members
                .iter()
                .map(|(name, leader)| ((*name).to_string(), *leader))
                .collect();
            self
        }

        /// Sets a raid: the server's list plus the local player. The members
        /// named here are `raid1..N` and the local player is `raid<N+1>`; the
        /// wire does not include the local player in the list. See
        /// [`crate::interface::raid`]. `own_flags` is the local player's
        /// subgroup byte, the only place the local player's column is stated.
        pub fn raid(mut self, members: &[(&str, u8)], own_flags: u8) -> Stub {
            self.raid = members
                .iter()
                .map(|(name, flags)| ((*name).to_string(), *flags))
                .collect();
            self.raid.push(("Alden".to_string(), own_flags));
            self
        }

        /// Sets a highlight with a name and no art. This is a zone map's
        /// answer, the second of `UpdateMapHighlight`'s two shapes, and it
        /// hides `WorldMapHighlight`.
        pub fn named_highlight(mut self, name: &str) -> Stub {
            self.highlight = Some(crate::lua::panels::worldmap::Highlight {
                name: name.to_string(),
                art: None,
            });
            self
        }

        /// Mark the unit added last as player-controlled.
        pub fn played(mut self) -> Stub {
            if let Some(unit) = self.units.last_mut() {
                unit.player = true;
            }
            self
        }

        /// Appends a tab and its spells. The offsets follow from the order of
        /// the calls, as a real book's do.
        pub fn tab(mut self, name: &str, spells: &[(&str, &str)]) -> Stub {
            self.tabs
                .push((name.to_string(), self.spells.len(), spells.len()));
            self.spells
                .extend(spells.iter().map(|(name, rank)| StubSpell {
                    name: (*name).to_string(),
                    rank: (*rank).to_string(),
                    passive: false,
                }));
            self
        }

        pub fn action(mut self, slot: u8, text: &str) -> Stub {
            self.actions.push(StubAction {
                slot,
                text: text.to_string(),
                texture: "Interface\\Icons\\Test".to_string(),
                cooldown: (0.0, 0.0, true),
                usable: true,
                not_enough_mana: false,
                attack: false,
                current: false,
                auto_repeat: false,
                count: 0,
                consumable: false,
                equipped: false,
                // No range and nothing to measure: the untinted button with no
                // range dot. A spell like Battle Shout draws this, and tests
                // that are not about range use it.
                has_range: false,
                in_range: None,
            });
            self
        }

        /// The same slot with a range on it. `Some(false)` in `in_range` is the
        /// red hotkey; `None` is a range the client cannot measure right now,
        /// which the interface draws differently. See
        /// [`crate::interface::api::is_action_in_range`].
        pub fn ranged_action(mut self, slot: u8, text: &str, in_range: Option<bool>) -> Stub {
            self = self.action(slot, text);
            if let Some(action) = self.actions.last_mut() {
                action.has_range = true;
                action.in_range = in_range;
            }
            self
        }

        /// A level-60 character sheet's update fields, decoded the way the
        /// client decodes a real one: 60 Strength with a +10/-4
        /// pair on it, 1000 armour, 50 fire resistance with a +10, 200 attack
        /// power, a 2.9-second weapon, 300 sword skill and 300 defense with a
        /// +5. Answered for `"player"` only.
        pub fn stats(mut self) -> Stub {
            use vale_protocol::state::fields::{player, unit};
            let slot = |n: u16| player::SKILL_INFO_1_1 + n * 3;
            let sword = vale_protocol::state::objects::HeldItem {
                display_id: 1,
                class: 2,
                subclass: 7,
                ..Default::default()
            };
            self.stats = vale_protocol::play::stats::UnitStats::from_fields(
                &[
                    (unit::STAT0, 60),
                    (player::POSSTAT0, 10),
                    (player::NEGSTAT0, (-4i32) as u32),
                    (unit::RESISTANCES, 1000),
                    (unit::RESISTANCES + 2, 50),
                    (player::RESISTANCEBUFFMODSPOSITIVE + 2, 10),
                    (unit::ATTACK_POWER, 200),
                    (unit::BASEATTACKTIME, 2900),
                    (unit::BASEATTACKTIME + 1, 1800),
                    (slot(0), 43),
                    (slot(0) + 1, 300 | (300 << 16)),
                    (slot(1), 95),
                    (slot(1) + 1, 295 | (300 << 16)),
                    (slot(1) + 2, 5 << 16),
                ],
                &[sword, Default::default(), Default::default()],
            );
            self
        }

        /// An aura on a token: a spell, a name, which half, and whether it has
        /// a clock. The icon follows the name, as every other stub's does.
        pub fn buff(
            mut self,
            token: &str,
            spell: u32,
            name: &str,
            helpful: bool,
            until_cancelled: bool,
        ) -> Stub {
            self.auras.push((
                token.to_ascii_lowercase(),
                crate::lua::panels::auras::AuraInfo {
                    spell,
                    icon: format!("Interface\\Icons\\{name}"),
                    name: name.to_string(),
                    description: format!("{name}, at work."),
                    applications: 1,
                    dispel_type: if helpful { String::new() } else { "Curse".to_string() },
                    time_left: if until_cancelled { 0.0 } else { 30.0 },
                    until_cancelled,
                },
            ));
            self.helpful.push(helpful);
            self
        }

        /// Which half each of [`Stub::auras`] is in, parallel to it.
        fn half(&self, index: usize) -> bool {
            self.helpful.get(index).copied().unwrap_or(true)
        }

        fn find(&self, token: &str) -> Option<&StubUnit> {
            let token = token.to_ascii_lowercase();
            self.units.iter().find(|unit| unit.token == token)
        }

        fn slot(&self, slot: u8) -> Option<&StubAction> {
            self.actions.iter().find(|action| action.slot == slot)
        }
    }

    impl crate::lua::panels::container::ContainerAnswers for Stub {

        // The bags, from the three flat lists [`Stub::bags`] fills. The stub
        // keeps slots and templates apart as the client does, so a test can
        // put a stack in a bag whose template has not arrived, which is the
        // state every bag is in for the first second of a login.
        fn container_num_slots(&self, bag: i32) -> usize {
            self.container(bag).map_or(0, <[_]>::len)
        }
        fn container_item(
            &self,
            bag: i32,
            slot: usize,
        ) -> Option<crate::lua::panels::container::SlotContents> {
            Some(self.contents(self.stub_slot(bag, slot)?))
        }
        fn container_item_link(&self, bag: i32, slot: usize) -> Option<String> {
            let (entry, _) = self.stub_slot(bag, slot)?;
            let template = self.template(entry)?;
            Some(crate::lua::panels::container::item_link(
                entry,
                template.quality,
                &template.name,
            ))
        }
        /// Returns the `item_cooldown` field unchanged, whatever the slot
        /// holds. A test about the cooldown swirl states the triple it wants.
        /// The lookup from slot to item to spell to cooldown record belongs to
        /// the live world and is tested there.
        fn container_item_cooldown(&self, _bag: i32, _slot: usize) -> (f64, f64, bool) {
            self.item_cooldown
        }
        fn inventory_item_cooldown(&self, _token: &str, _id: u32) -> (f64, f64, bool) {
            self.item_cooldown
        }
        fn bag_name(&self, bag: i32) -> Option<String> {
            Some(self.template(self.bag_entries.iter().find(|(b, _)| *b == bag)?.1)?.name.clone())
        }
        fn inventory_item(
            &self,
            token: &str,
            id: u32,
        ) -> Option<crate::lua::panels::container::SlotContents> {
            if token != "player" {
                return None;
            }
            Some(self.contents(self.stub_worn(id)?))
        }
        fn inventory_item_link(&self, token: &str, id: u32) -> Option<String> {
            if token != "player" {
                return None;
            }
            let (entry, _) = self.stub_worn(id)?;
            let template = self.template(entry)?;
            Some(crate::lua::panels::container::item_link(
                entry,
                template.quality,
                &template.name,
            ))
        }
        fn inventory_slot_info(&self, name: &str) -> Option<(u32, String, bool)> {
            // The two rows the tests name, in the shape the real table gives
            // them. See [`vale_assets::tables::inventory`].
            match name {
                "HeadSlot" => Some((1, "art-head".into(), false)),
                "RangedSlot" => Some((
                    vale_assets::tables::inventory::RANGED_SLOT,
                    "art-ranged".into(),
                    true,
                )),
                _ => None,
            }
        }
        fn item_info(&self, entry: u32) -> Option<crate::lua::panels::container::ItemDetails> {
            let template = self.template(entry)?;
            Some(crate::lua::panels::container::ItemDetails {
                link: crate::lua::panels::container::item_link(entry, template.quality, &template.name),
                name: template.name.clone(),
                quality: template.quality,
                required_level: 0,
                class_name: String::new(),
                subclass_name: String::new(),
                stack_count: 1,
                equip_location: vale_assets::tables::inventory::inventory_type_key(
                    template.inventory_type,
                )
                .to_string(),
                texture: Some(template.icon.clone()),
            })
        }
        fn item_count(&self, entry: u32) -> u32 {
            let carried: u32 = self
                .containers
                .iter()
                .flat_map(|(_, slots)| slots.iter())
                .filter(|(held, _)| *held == entry)
                .map(|(_, count)| *count)
                .sum();
            let worn: u32 = self
                .worn
                .iter()
                .filter(|(_, held, _)| *held == entry)
                .map(|(_, _, count)| *count)
                .sum();
            carried + worn
        }
        fn money(&self) -> u32 {
            self.money
        }
        fn cursor_has_item(&self) -> bool {
            false
        }
        fn cursor_has_spell(&self) -> bool {
            false
        }
        fn bag_item_tip(&self, bag: i32, slot: usize) -> Option<crate::interface::api::ItemTip> {
            let (entry, _) = self.stub_slot(bag, slot)?;
            self.item_tip(entry)
        }
        fn inventory_item_tip(&self, token: &str, id: u32) -> Option<crate::interface::api::ItemTip> {
            if token != "player" {
                return None;
            }
            self.item_tip(self.stub_worn(id)?.0)
        }
        fn item_tip(&self, entry: u32) -> Option<crate::interface::api::ItemTip> {
            let template = self.template(entry)?;
            Some(crate::interface::api::ItemTip {
                name: template.name.clone(),
                quality: template.quality,
                inventory_type: template.inventory_type,
                required_level: template.required_level,
                level_met: template.level_met,
                ..Default::default()
            })
        }
    
        fn coin_icon(&self, _copper: u32) -> Option<String> {
            None
        }
}

    impl crate::lua::panels::quest::QuestAnswers for Stub {

        // Empty by default: a stub world has no quest conversation and no
        // quest log unless the test states one.
        fn quest_greeting_text(&self) -> String {
            String::new()
        }
        fn quest_offers(&self, _active: bool) -> usize {
            0
        }
        fn quest_offer_title(&self, _active: bool, _index: Option<usize>) -> (String, u32) {
            (String::new(), 0)
        }
        fn quest_page_text(&self, _page: crate::lua::panels::quest::Page) -> String {
            String::new()
        }
        fn quest_items(
            &self,
            _which: crate::lua::panels::quest::Which,
        ) -> Vec<crate::lua::panels::quest::RewardLine> {
            Vec::new()
        }
        fn quest_money(&self, _required: bool) -> u32 {
            0
        }
        fn quest_reward_spell(&self) -> Option<crate::lua::panels::quest::RewardSpell> {
            None
        }
        fn quest_log_reward_spell(&self) -> Option<crate::lua::panels::quest::RewardSpell> {
            None
        }
        fn quest_completable(&self) -> bool {
            false
        }
        fn quest_log_rows(&self) -> usize {
            0
        }
        fn quest_log_quest_rows(&self) -> usize {
            0
        }
        fn select_log_row(&self, _row: usize) {}
        fn quest_set_collapsed(&self, _row: usize, _collapsed: bool) {}
        fn quest_set_abandon(&self) {}
        fn quest_abandon_name(&self) -> Option<String> {
            None
        }
        fn quest_abandon_items(&self) -> Option<String> {
            None
        }
        fn quest_log_selection(&self) -> usize {
            0
        }
        fn quest_log_text(&self) -> (String, String) {
            (String::new(), String::new())
        }
        fn quest_log_objectives(&self, _row: usize) -> Vec<crate::lua::panels::quest::ObjectiveLine> {
            Vec::new()
        }
        fn quest_log_items(&self, _choices: bool) -> Vec<crate::lua::panels::quest::RewardLine> {
            Vec::new()
        }
        fn quest_reward_spell_tip(&self, _from_log: bool) -> Option<crate::interface::api::SpellTip> {
            None
        }
        fn quest_log_money(&self, _required: bool) -> u32 {
            0
        }
        fn quest_log_failed(&self) -> bool {
            false
        }
        fn quest_log_time_left(&self) -> Option<u32> {
            None
        }
        fn quest_log_row(&self, _row: usize) -> Option<crate::lua::panels::quest::LogRow> {
            None
        }
    
        // No quest is watched, which is the state of a log nobody has
        // right-clicked.
        fn quest_watch_count(&self) -> usize {
            0
        }
        fn quest_is_watched(&self, _row: usize) -> bool {
            false
        }
        fn quest_watch_row(&self, _watch: usize) -> Option<usize> {
            None
        }
        fn quest_add_watch(&self, _row: usize) {}
        fn quest_remove_watch(&self, _row: usize) {}
}

    impl crate::lua::panels::gossip::GossipAnswers for Stub {


        // --- NPC gossip: a small sample greeting, menu and quest list ---
        fn gossip_text(&self) -> String {
            "Probe greeting.".to_string()
        }
        fn gossip_options(&self) -> Vec<(String, &'static str)> {
            vec![
                ("Let me browse your goods.".to_string(), "vendor"),
                ("Train me.".to_string(), "trainer"),
            ]
        }
        fn gossip_quests(&self, active: bool) -> Vec<(String, u32)> {
            match active {
                false => vec![("Probe Quest 1".to_string(), 5)],
                true => vec![("Probe Quest 2".to_string(), 5)],
            }
        }
    }

    impl crate::lua::panels::merchant::MerchantAnswers for Stub {
        fn merchant_rows(&self) -> usize {
            2
        }
        fn merchant_item(
            &self,
            row: usize,
        ) -> Option<crate::lua::panels::merchant::MerchantLine> {
            (1..=2).contains(&row).then(|| crate::lua::panels::merchant::MerchantLine {
                name: format!("Probe Ware {row}"),
                texture: Some(crate::lua::panels::loot::UNKNOWN_ICON.to_string()),
                price: 15,
                quantity: 1,
                available: if row == 1 { -1 } else { 3 },
                usable: true,
            })
        }
        fn merchant_item_link(&self, row: usize) -> Option<String> {
            let line = self.merchant_item(row)?;
            Some(crate::lua::panels::container::item_link(2589, 1, &line.name))
        }
        fn merchant_max_stack(&self, _row: usize) -> u32 {
            5
        }
        /// Two rows, in the order they were sold. That lets a test tell the
        /// last row from the first, which is what the front tab's
        /// `GetBuybackItemInfo(GetNumBuybackItems())` depends on.
        fn buyback_rows(&self) -> usize {
            2
        }
        fn buyback_item(
            &self,
            row: usize,
        ) -> Option<crate::lua::panels::merchant::MerchantLine> {
            (1..=2).contains(&row).then(|| crate::lua::panels::merchant::MerchantLine {
                name: format!("Probe Sold {row}"),
                texture: Some(crate::lua::panels::loot::UNKNOWN_ICON.to_string()),
                price: 40 * row as u32,
                quantity: row as u32,
                available: 0,
                usable: true,
            })
        }
        fn buyback_entry(&self, row: usize) -> Option<u32> {
            (1..=2).contains(&row).then(|| 2589 + row as u32)
        }
        fn repairs(&self) -> crate::interface::merchant::Repairs {
            crate::interface::merchant::Repairs {
                can_repair: true,
                cost: 1234,
                priced: true,
                mode: false,
            }
        }
    }

    impl crate::lua::panels::taxi::TaxiAnswers for Stub {
        // A small sample flight map: the current node, and one node one hop
        // away. `lua::taxi`'s tests read these numbers back.
        fn taxi_nodes(&self) -> usize {
            2
        }
        fn taxi_node(&self, row: usize) -> Option<crate::lua::panels::taxi::TaxiNodeLine> {
            match row {
                1 => Some(crate::lua::panels::taxi::TaxiNodeLine {
                    name: "Stormwind".to_string(),
                    kind: "CURRENT",
                    at: [0.25, 0.75],
                    cost: 0,
                    hops: 0,
                }),
                2 => Some(crate::lua::panels::taxi::TaxiNodeLine {
                    name: "Sentinel Hill".to_string(),
                    kind: "REACHABLE",
                    at: [0.5, 0.125],
                    cost: 110,
                    hops: 1,
                }),
                _ => None,
            }
        }
        fn taxi_hop(&self, row: usize, hop: usize) -> Option<([f32; 2], [f32; 2])> {
            (row == 2 && hop == 1).then_some(([0.25, 0.75], [0.5, 0.125]))
        }
        fn taxi_map_art(&self) -> Option<String> {
            Some(vale_assets::tables::taxi::map_art(0))
        }
        fn unit_on_taxi(&self, _token: &str) -> bool {
            false
        }
    }

    impl crate::lua::panels::tradeskill::TradeSkillAnswers for Stub {
        // A sample profession, so the panel probes exercise the window: one
        // header, one recipe under it, one reagent half-met. The shapes match
        // what a real Blacksmithing line answers.
        fn trade_line(&self) -> Option<(String, u32, u32)> {
            Some(("Stubsmithing".to_string(), 150, 300))
        }
        fn trade_rows(&self) -> usize {
            2
        }
        fn trade_row(&self, index: usize) -> Option<crate::lua::panels::tradeskill::TradeRow> {
            match index {
                1 => Some(crate::lua::panels::tradeskill::TradeRow {
                    kind: "header".to_string(),
                    name: "Weapons".to_string(),
                    num_available: 0,
                    expanded: true,
                    header: true,
                }),
                2 => Some(crate::lua::panels::tradeskill::TradeRow {
                    kind: "optimal".to_string(),
                    name: "Stub Dagger".to_string(),
                    num_available: 2,
                    expanded: true,
                    header: false,
                }),
                _ => None,
            }
        }
        fn trade_first(&self) -> usize {
            2
        }
        fn trade_selection(&self) -> usize {
            2
        }
        fn trade_select(&self, _index: usize) {}
        fn trade_set_expanded(&self, _index: usize, _expanded: bool) {}
        fn trade_icon(&self, _index: usize) -> Option<String> {
            Some(r"Interface\Icons\Ability_Kick".to_string())
        }
        fn trade_cooldown(&self, _index: usize) -> Option<f64> {
            None
        }
        fn trade_num_made(&self, _index: usize) -> (i32, i32) {
            (1, 1)
        }
        fn trade_num_reagents(&self, index: usize) -> usize {
            usize::from(index == 2)
        }
        fn trade_reagent(
            &self,
            index: usize,
            reagent: usize,
        ) -> Option<(Option<String>, Option<String>, u32, u32)> {
            (index == 2 && reagent == 1).then(|| {
                (
                    Some("Stub Ore".to_string()),
                    Some(r"Interface\Icons\Ability_Kick".to_string()),
                    2,
                    4,
                )
            })
        }
        fn trade_reagent_link(&self, _index: usize, _reagent: usize) -> Option<String> {
            None
        }
        fn trade_item_link(&self, _index: usize) -> Option<String> {
            None
        }
        fn trade_tools(&self, _index: usize) -> Vec<(String, bool)> {
            vec![("Stub Anvil".to_string(), false)]
        }
        fn trade_repeat_count(&self) -> u32 {
            1
        }
        fn trade_subclasses(&self) -> Vec<String> {
            vec!["Weapons".to_string()]
        }
        fn trade_subclass_filter(&self, _index: usize) -> bool {
            true
        }
        fn trade_set_subclass_filter(&self, _index: usize, _on: bool, _exclusive: bool) {}
        fn trade_recipe(&self, index: usize) -> Option<(u32, u32)> {
            (index == 2).then_some((2661, 2))
        }
        fn trade_close(&self) {}
        fn trade_tip_item(&self, _index: usize, _reagent: Option<usize>) -> Option<u32> {
            None
        }
    }

    impl crate::lua::panels::craft::CraftAnswers for Stub {
        // A sample Enchanting window: one recipe and no header, which is the
        // flat shape the craft list has.
        fn craft_name(&self) -> Option<String> {
            Some("Enchanting".to_string())
        }
        fn craft_button_token(&self) -> String {
            "ENSCRIBE".to_string()
        }
        fn craft_display_line(&self) -> Option<(String, u32, u32)> {
            Some(("Enchanting".to_string(), 80, 150))
        }
        fn craft_rows(&self) -> usize {
            1
        }
        fn craft_row(&self, index: usize) -> Option<crate::lua::panels::craft::CraftLine> {
            (index == 1).then(|| crate::lua::panels::craft::CraftLine {
                kind: "medium".to_string(),
                name: "Stub Bracer".to_string(),
                sub_text: String::new(),
                num_available: 1,
                expanded: None,
                header: false,
                train_points: 0,
                required_level: 0,
            })
        }
        fn craft_selection(&self) -> usize {
            1
        }
        fn craft_select(&self, _index: usize) {}
        fn craft_set_expanded(&self, _index: usize, _expanded: bool) {}
        fn craft_icon(&self, _index: usize) -> Option<String> {
            Some(r"Interface\Icons\Ability_Kick".to_string())
        }
        fn craft_description(&self, _index: usize) -> Option<String> {
            Some("Imbues the stub with health.".to_string())
        }
        fn craft_num_reagents(&self, _index: usize) -> usize {
            1
        }
        fn craft_reagent(
            &self,
            index: usize,
            reagent: usize,
        ) -> Option<(Option<String>, Option<String>, u32, u32)> {
            (index == 1 && reagent == 1).then(|| {
                (
                    Some("Stub Dust".to_string()),
                    Some(r"Interface\Icons\Ability_Kick".to_string()),
                    1,
                    3,
                )
            })
        }
        fn craft_reagent_link(&self, _index: usize, _reagent: usize) -> Option<String> {
            None
        }
        fn craft_focus(&self, _index: usize) -> Vec<(String, bool)> {
            vec![("Stub Rod".to_string(), true)]
        }
        fn craft_recipe(&self, index: usize) -> Option<u32> {
            (index == 1).then_some(7418)
        }
        fn craft_close(&self) {}
        fn craft_tip_item(&self, _index: usize, _reagent: usize) -> Option<u32> {
            None
        }
        fn craft_spell_tip(&self, _index: usize) -> Option<crate::interface::api::SpellTip> {
            None
        }
    }

    impl crate::lua::panels::mail::MailAnswers for Stub {
        // An empty mailbox, as a character who has never been sent anything
        // has. The panel's tests check the zero-versus-nil answers against it.
        fn mail_count(&self) -> usize {
            0
        }
        fn mail_row(&self, _row: usize) -> Option<crate::lua::panels::mail::InboxRow> {
            None
        }
        fn mail_text(&self, _row: usize) -> crate::lua::panels::mail::InboxText {
            crate::lua::panels::mail::InboxText::default()
        }
        fn mail_open(&self, _row: usize) {}
        fn mail_item(&self, _row: usize) -> Option<crate::lua::panels::mail::MailItemLine> {
            None
        }
        fn mail_can_delete(&self, _row: usize) -> bool {
            false
        }
        fn mail_has_new(&self) -> bool {
            false
        }
        fn mail_stationery(&self) -> Vec<crate::lua::panels::mail::StationeryLine> {
            Vec::new()
        }
        fn mail_selected_stationery(&self) -> Option<String> {
            None
        }
        fn mail_select_stationery(&self, _id: u32) {}
        fn mail_send_item(&self) -> Option<crate::lua::panels::mail::MailItemLine> {
            None
        }
        fn mail_send_price(&self) -> u32 {
            vale_protocol::play::mail::POSTAGE
        }
        fn mail_send_money(&self) -> u32 {
            0
        }
        fn mail_send_cod(&self) -> u32 {
            0
        }
        fn mail_set_money(&self, _copper: u32) -> bool {
            false
        }
        fn mail_set_cod(&self, _copper: u32) {}
            fn mail_item_entry(&self, _row: usize) -> Option<u32> {
            None
        }
        fn mail_send_entry(&self) -> Option<u32> {
            None
        }
}

    /// A shut trade window, which is every default. The summon, inspect, bank
    /// and page-text impls below also take the defaults.
    ///
    /// The `StableAnswers` impl after them is a small sample stable: one slot
    /// bought, a pet in the current stall and one in the bought stall, so
    /// `PetStable_Update` runs its occupied, bought-but-empty and unbought
    /// branches in one pass.
    impl crate::lua::panels::trade::TradeAnswers for Stub {}
    impl crate::lua::panels::summon::SummonAnswers for Stub {}
    impl crate::lua::panels::uioptions::OptionsAnswers for Stub {}
    impl crate::lua::panels::inspect::InspectAnswers for Stub {}
    impl crate::lua::panels::guild::GuildAnswers for Stub {}
    impl crate::lua::panels::bank::BankAnswers for Stub {}
    impl crate::lua::panels::pagetext::PageTextAnswers for Stub {}

    impl crate::lua::panels::stable::StableAnswers for Stub {
        fn stable_slots(&self) -> u32 {
            1
        }
        fn stable_pets(&self) -> u32 {
            2
        }
        fn selected_stable_pet(&self) -> i32 {
            0
        }
        fn stable_pet_info(&self, panel_slot: u8) -> Option<crate::lua::panels::stable::StableLine> {
            let line = |name: &str, level: u32, family: &str| {
                crate::lua::panels::stable::StableLine {
                    icon: crate::lua::panels::loot::UNKNOWN_ICON.to_string(),
                    name: name.to_string(),
                    level,
                    family: family.to_string(),
                    loyalty: "(Loyalty Level 4) Dependable".to_string(),
                }
            };
            match panel_slot {
                0 => Some(line("Bruiser", 32, "Wolf")),
                1 => Some(line("Snarl", 28, "Cat")),
                _ => None,
            }
        }
        fn stable_pet_food_types(&self, panel_slot: u8) -> Vec<String> {
            match self.stable_pet_info(panel_slot) {
                Some(_) => vec!["Meat".to_string()],
                None => Vec::new(),
            }
        }
        fn next_stable_slot_cost(&self) -> u32 {
            500
        }
        fn stable_paperdoll_unit(&self) -> Option<String> {
            Some("pet".to_string())
        }
    }

    impl crate::lua::panels::trainer::TrainerAnswers for Stub {

        // A small sample trainer: a header and an available (green) spell
        // under it.
        fn trainer_rows(&self) -> usize {
            2
        }
        fn trainer_line(&self, row: usize) -> Option<crate::lua::panels::trainer::TrainerLine> {
            match row {
                1 => Some(crate::lua::panels::trainer::TrainerLine {
                    kind: "header".to_string(),
                    name: "Fire".to_string(),
                    expanded: true,
                    skill_line: "Fire".to_string(),
                    ..Default::default()
                }),
                2 => Some(crate::lua::panels::trainer::TrainerLine {
                    kind: "available".to_string(),
                    name: "Fireball".to_string(),
                    sub_text: "Rank 2".to_string(),
                    expanded: true,
                    icon: Some(crate::lua::panels::loot::UNKNOWN_ICON.to_string()),
                    description: "Hurls a fiery ball.".to_string(),
                    cost: (1000, 0, 0),
                    level_req: 6,
                    skill_line: "Fire".to_string(),
                    learn_spell: true,
                    ..Default::default()
                }),
                _ => None,
            }
        }
        fn trainer_greeting(&self) -> String {
            "I can teach you the ways of fire.".to_string()
        }
        fn trainer_selection(&self) -> usize {
            2
        }
        fn trainer_select(&self, _row: usize) {}
        fn trainer_tooltip(&self, _row: usize) -> Option<crate::interface::api::SpellTip> {
            None
        }
        fn trainer_is_tradeskill(&self) -> bool {
            false
        }
        fn trainer_is_talent(&self) -> bool {
            false
        }
        fn trainer_type_filter(&self, word: &str) -> bool {
            word != "used"
        }
        fn trainer_line_filter(&self, _group: usize) -> bool {
            true
        }
    }

    impl crate::lua::panels::loot::LootAnswers for Stub {

        fn loot_rows(&self) -> usize {
            self.loot.len()
        }
        fn loot_slot(&self, row: usize) -> Option<crate::lua::panels::loot::LootRow> {
            self.loot.get(row.checked_sub(1)?).cloned()
        }
        fn loot_slot_link(&self, row: usize) -> Option<String> {
            let row = self.loot_slot(row).filter(|slot| !slot.is_coin)?;
            Some(crate::lua::panels::container::item_link(2589, row.quality, &row.name))
        }
        fn is_fishing_loot(&self) -> bool {
            false
        }
    }

    impl crate::lua::panels::lootroll::LootRollAnswers for Stub {
        fn loot_roll_item(&self, id: u32) -> Option<crate::lua::panels::lootroll::RollItem> {
            (id == 0).then(|| self.roll.clone()).flatten()
        }
        fn loot_roll_link(&self, id: u32) -> Option<String> {
            let roll = self.loot_roll_item(id)?;
            Some(crate::lua::panels::container::item_link(2589, roll.quality, &roll.name))
        }
        fn loot_roll_time_left(&self, id: u32) -> Option<f64> {
            self.loot_roll_item(id).map(|_| 42_000.0)
        }
    }

    impl crate::lua::panels::worldmap::MapAnswers for Stub {

        // The map for a client with no world. Each method gives the answer a
        // character screen gives. These tests run in that state, and the
        // trait's "nothing" answers are written for it.
        fn current_map_view(&self) -> vale_assets::tables::worldmap::MapView {
            vale_assets::tables::worldmap::MapView::default()
        }
        fn map_directory(&self) -> Option<String> {
            None
        }
        fn map_landmarks(&self) -> Vec<vale_assets::tables::areapoi::Landmark> {
            Vec::new()
        }
        fn corpse_map_position(&self) -> (f32, f32) {
            (0.0, 0.0)
        }
        fn map_continents(&self) -> Vec<String> {
            Vec::new()
        }
        fn map_zones(&self, _: usize) -> Vec<String> {
            Vec::new()
        }
        fn player_map_position(&self, _: &str) -> (f32, f32) {
            (0.0, 0.0)
        }
        fn player_facing(&self) -> f32 {
            0.0
        }
        fn map_highlight(&self, _: f32, _: f32) -> Option<crate::lua::panels::worldmap::Highlight> {
            self.highlight.clone()
        }
        fn map_overlays(&self) -> Vec<crate::lua::panels::worldmap::OverlayArt> {
            self.overlays.clone()
        }
        fn zone_text(&self) -> String {
            String::new()
        }
        fn sub_zone_text(&self) -> String {
            String::new()
        }
    }

    /// No pet. This is the ordinary case, and every non-pet test in this file
    /// assumes it. The pet reads have their own doubles in
    /// [`crate::lua::audit`].
    impl crate::lua::panels::pet::PetAnswers for Stub {
        fn has_pet_ui(&self) -> (bool, bool) {
            (false, false)
        }
        fn pet_can_be_abandoned(&self) -> bool {
            false
        }
        fn pet_can_be_renamed(&self) -> bool {
            false
        }
        fn pet_has_action_bar(&self) -> bool {
            false
        }
        fn pet_action_info(&self, _slot: usize) -> Option<crate::interface::pet::PetSlot> {
            None
        }
        fn pet_action_cooldown(&self, _slot: usize) -> (f64, f64, u32) {
            (0.0, 0.0, 0)
        }
        fn pet_action_tooltip(&self, _slot: usize) -> Option<crate::interface::api::SpellTip> {
            None
        }
        fn pet_actions_usable(&self) -> bool {
            false
        }
        fn is_pet_attack_active(&self, _slot: usize) -> bool {
            false
        }
        fn pet_happiness(&self) -> Option<(u32, f32, f32)> {
            None
        }
        fn pet_loyalty(&self) -> Option<String> {
            None
        }
        fn pet_experience(&self) -> (u32, u32) {
            (0, 0)
        }
        fn pet_training_points(&self) -> (u32, u32) {
            (0, 0)
        }
        fn pet_icon(&self) -> Option<String> {
            None
        }
        fn pet_food_types(&self) -> Vec<String> {
            Vec::new()
        }
        fn creature_family(&self, _unit: crate::interface::api::UnitId) -> Option<String> {
            None
        }
        fn has_pet_spells(&self) -> Option<(u32, &'static str)> {
            None
        }
    }

    /// No forms, which is true of every class but four and is the case the
    /// bare interpreter's tests cover. The stance bar has its own test double
    /// in [`crate::lua::audit`].
    impl crate::lua::panels::shapeshift::ShapeshiftAnswers for Stub {
        fn shapeshift_form_count(&self) -> usize {
            0
        }
        fn shapeshift_form_info(
            &self,
            _index: usize,
        ) -> Option<crate::lua::panels::shapeshift::ShapeshiftInfo> {
            None
        }
        fn shapeshift_form_cooldown(&self, _index: usize) -> (f64, f64, u32) {
            (0.0, 0.0, 1)
        }
    }

    impl crate::lua::panels::raid::RaidAnswers for Stub {
        fn raid_count(&self) -> usize {
            self.raid.len()
        }
        fn raid_roster_info(
            &self,
            index: usize,
        ) -> Option<crate::lua::panels::raid::RaidRow> {
            let (name, flags) = self.raid.get(index.checked_sub(1)?)?;
            Some(crate::lua::panels::raid::RaidRow {
                name: name.clone(),
                // The last row, the local player, is the leader. If `raid1`
                // led, the stub would never exercise the branch the raid panel
                // takes most of the time.
                rank: vale_protocol::play::group::rank_of(
                    index as u64,
                    self.raid.len() as u64,
                    flags & 0x80 != 0,
                ),
                subgroup: usize::from(flags & 0x7f) + 1,
                level: 60,
                class: Some(("Warrior".to_string(), "WARRIOR".to_string())),
                zone: "Elwynn Forest".to_string(),
                online: true,
                dead: false,
            })
        }
        fn raid_roster_selection(&self) -> usize {
            0
        }
        fn is_raid_leader(&self) -> bool {
            false
        }
        fn is_raid_officer(&self) -> bool {
            false
        }
        fn unit_in_raid(&self, token: &str, _or_pet: bool) -> bool {
            !self.raid.is_empty() && crate::interface::api::UnitId::parse(token).is_some()
        }
    }

    impl crate::lua::panels::party::PartyAnswers for Stub {
        fn can_show_reset_instances(&self) -> bool {
            false
        }
        fn party_count(&self) -> usize {
            self.party.len()
        }
        fn party_member_exists(&self, index: usize) -> bool {
            index >= 1 && index <= self.party.len()
        }
        fn party_leader_index(&self) -> usize {
            self.party
                .iter()
                .position(|(_, leader)| *leader)
                .map_or(0, |index| index + 1)
        }
        fn unit_is_party_leader(&self, token: &str) -> bool {
            token
                .strip_prefix("party")
                .and_then(|n| n.parse::<usize>().ok())
                .and_then(|n| self.party.get(n.checked_sub(1)?))
                .is_some_and(|(_, leader)| *leader)
        }
        fn loot_method(&self) -> (String, Option<usize>) {
            ("freeforall".to_string(), None)
        }
        fn loot_threshold(&self) -> u32 {
            2
        }
    }

    impl crate::lua::panels::glue::GlueAnswers for Stub {
        // No handshake data, which is what the client holds at a login screen
        // and in the world, the two states this stub stands in for.
        fn character_count(&self) -> usize {
            0
        }
        fn character_row(&self, _: usize) -> Option<crate::lua::panels::glue::CharacterRow> {
            None
        }
        fn realm(&self) -> (Option<String>, bool, bool) {
            (None, false, false)
        }
        fn connected(&self) -> bool {
            false
        }
        fn saved_account_name(&self) -> String {
            String::new()
        }
    }

    /// No talent tree, because the stub world has no class. `GetNumTalentTabs()`
    /// answering 0 is also what a character below the talent level reads.
    ///
    /// The populated case is [`crate::lua::audit`]'s `Login`, which drives the
    /// panel.
    impl crate::lua::panels::talent::TalentAnswers for Stub {
        fn num_talent_tabs(&self) -> usize {
            0
        }
        fn talent_tab_info(&self, _: usize) -> Option<crate::lua::panels::talent::TalentTab> {
            None
        }
        fn num_talents(&self, _: usize) -> usize {
            0
        }
        fn talent_info(&self, _: usize, _: usize) -> Option<crate::lua::panels::talent::TalentInfo> {
            None
        }
        fn talent_prereqs(&self, _: usize, _: usize) -> Vec<crate::lua::panels::talent::TalentPrereq> {
            Vec::new()
        }
        fn talent_tooltip(&self, _: usize, _: usize) -> Option<crate::interface::api::SpellTip> {
            None
        }
    }

    impl crate::lua::panels::spellbook::SpellbookAnswers for Stub {

        fn num_spell_tabs(&self) -> usize {
            self.tabs.len()
        }
        fn spell_tab_info(&self, index: usize) -> Option<SpellTab> {
            let (name, offset, count) = self.tabs.get(index.checked_sub(1)?)?;
            Some(SpellTab {
                name: name.clone(),
                texture: format!("Interface\\Icons\\{name}"),
                offset: *offset,
                count: *count,
            })
        }
        fn spell_name(&self, index: usize) -> Option<(String, String)> {
            let spell = self.spells.get(index.checked_sub(1)?)?;
            Some((spell.name.clone(), spell.rank.clone()))
        }
        fn spell_texture(&self, index: usize) -> Option<String> {
            let spell = self.spells.get(index.checked_sub(1)?)?;
            Some(format!("Interface\\Icons\\{}", spell.name))
        }
        fn spell_cooldown(&self, _: usize) -> (f64, f64, bool) {
            (0.0, 0.0, true)
        }
        fn spell_passive(&self, index: usize) -> bool {
            index
                .checked_sub(1)
                .and_then(|i| self.spells.get(i))
                .is_some_and(|spell| spell.passive)
        }
        fn spell_is_current_cast(&self, _: usize) -> bool {
            false
        }
        fn spell_tooltip(&self, index: usize) -> Option<crate::interface::api::SpellTip> {
            let (name, rank) = self.spell_name(index)?;
            Some(crate::interface::api::SpellTip {
                name,
                rank,
                ..Default::default()
            })
        }
    }

    impl crate::lua::panels::auras::AuraAnswers for Stub {
        fn player_buff(&self, index: usize, filter: &str) -> i32 {
            // The stub's filter only distinguishes helpful from harmful,
            // which is all any shipped button passes.
            let want = !filter.eq_ignore_ascii_case("HARMFUL");
            self.auras
                .iter()
                .enumerate()
                .filter(|(i, (token, _))| token == "player" && self.half(*i) == want)
                .nth(index)
                .map_or(-1, |(handle, _)| handle as i32)
        }
        fn player_buff_at(&self, handle: i32) -> Option<crate::lua::panels::auras::AuraInfo> {
            let handle = usize::try_from(handle).ok()?;
            let (token, aura) = self.auras.get(handle)?;
            (token == "player").then(|| aura.clone())
        }
        fn unit_aura(
            &self,
            token: &str,
            index: usize,
            helpful: bool,
        ) -> Option<crate::lua::panels::auras::AuraInfo> {
            let token = token.to_ascii_lowercase();
            // Only the tokens the live client keeps aura lists for, so the
            // "a unit with no list answers nothing" test has a unit to fail on.
            if !matches!(token.as_str(), "player" | "target" | "targettarget") {
                return None;
            }
            self.auras
                .iter()
                .enumerate()
                .filter(|(i, (held, _))| *held == token && self.half(*i) == helpful)
                .nth(index.checked_sub(1)?)
                .map(|(_, (_, aura))| aura.clone())
        }
    }

    impl super::ActionAnswers for Stub {
        fn has_action(&self, slot: u8) -> bool {
            self.slot(slot).is_some()
        }
        fn bonus_bar_offset(&self) -> u8 {
            0
        }
        fn action_bar_toggles(&self) -> u8 {
            self.bar_toggles
        }
        /// A fixed, fully-populated tip for any filled slot, so the tooltip
        /// tests can assert on every line the tooltip rule composes.
        fn action_tooltip(&self, slot: u8) -> Option<crate::interface::api::SpellTip> {
            self.slot(slot).map(|action| crate::interface::api::SpellTip {
                talent_rank: None,
                name: action.text.clone(),
                rank: "Rank 1".to_string(),
                power_type: 0,
                power_cost: 30,
                range_yards: 30.0,
                cast_time_ms: 3500,
                cooldown_ms: 0,
                description: "Hurls a fiery ball.".to_string(),
                reagents: vec![("Rune of Teleportation".to_string(), 2)],
            })
        }
        /// No stub slot holds an item, so a bar plate here is always the
        /// spell's. The tooltip tests below rely on that.
        fn action_item_tooltip(&self, _: u8) -> Option<crate::interface::api::ItemTip> {
            None
        }
        fn action_text(&self, slot: u8) -> Option<String> {
            self.slot(slot).map(|action| action.text.clone())
        }
        fn action_texture(&self, slot: u8) -> Option<String> {
            self.slot(slot).map(|action| action.texture.clone())
        }
        fn action_cooldown(&self, slot: u8) -> (f64, f64, bool) {
            self.slot(slot).map_or((0.0, 0.0, true), |action| action.cooldown)
        }
        fn action_usable(&self, slot: u8) -> (bool, bool) {
            self.slot(slot)
                .map_or((false, false), |a| (a.usable, a.not_enough_mana))
        }
        fn is_attack_action(&self, slot: u8) -> bool {
            self.slot(slot).is_some_and(|action| action.attack)
        }
        fn is_current_action(&self, slot: u8) -> bool {
            self.slot(slot).is_some_and(|action| action.current)
        }
        fn is_auto_repeat_action(&self, slot: u8) -> bool {
            self.slot(slot).is_some_and(|action| action.auto_repeat)
        }
        fn action_has_range(&self, slot: u8) -> bool {
            self.slot(slot).is_some_and(|action| action.has_range)
        }
        fn is_action_in_range(&self, slot: u8) -> Option<bool> {
            self.slot(slot)?.in_range
        }
        fn is_consumable_action(&self, slot: u8) -> bool {
            self.slot(slot).is_some_and(|action| action.consumable)
        }
        fn is_equipped_action(&self, slot: u8) -> bool {
            self.slot(slot).is_some_and(|action| action.equipped)
        }
        fn action_count(&self, slot: u8) -> u32 {
            self.slot(slot).map_or(0, |action| action.count)
        }
        /// The stub has no spell cursor: the mode lives in a resource this
        /// harness does not build, and the two reads are checked where it does
        /// (`interface::action`).
        fn spell_is_targeting(&self) -> bool {
            false
        }

        fn spell_is_casting(&self) -> bool {
            false
        }
        fn spell_can_target_unit(&self, _token: &str) -> bool {
            false
        }
    }

    impl super::UnitAnswers for Stub {
        fn now(&self) -> f64 {
            self.now
        }
        /// Noon, which is [`crate::render::sky::WorldClock`]'s own default
        /// before a session has stated the hour.
        fn game_time(&self) -> (u32, u32) {
            (12, 0)
        }
        /// The 1.12.1 client's text for a character with no bind point, which
        /// is the state of a harness with no session.
        fn bind_location(&self) -> String {
            "your inn".to_string()
        }
        fn binder_in_range(&self) -> bool {
            true
        }
        fn untrainer_in_range(&self) -> bool {
            true
        }
        fn unit_exists(&self, token: &str) -> bool {
            self.find(token).is_some()
        }
        fn unit_name(&self, token: &str) -> Option<String> {
            self.find(token).map(|unit| unit.name.clone())
        }
        fn unit_level(&self, token: &str) -> i32 {
            self.find(token).map_or(-1, |unit| unit.level)
        }
        fn unit_sex(&self, _token: &str) -> u32 {
            2
        }
        fn unit_experience(&self, _token: &str) -> (u32, u32) {
            (0, 0)
        }
        fn unit_character_points(&self, _token: &str) -> (u32, u32) {
            (0, 0)
        }
        fn rested_experience(&self) -> Option<u32> {
            None
        }
        fn unit_health(&self, token: &str) -> u32 {
            self.find(token).map_or(0, |unit| unit.health)
        }
        fn unit_health_max(&self, token: &str) -> u32 {
            self.find(token).map_or(0, |unit| unit.health_max)
        }
        fn unit_mana(&self, _token: &str) -> u32 {
            0
        }
        fn unit_mana_max(&self, _token: &str) -> u32 {
            0
        }
        fn unit_power_type(&self, _token: &str) -> Option<u8> {
            None
        }
        fn unit_is_dead(&self, token: &str) -> bool {
            self.find(token).is_some_and(|unit| unit.dead)
        }
        fn unit_is_ghost(&self, token: &str) -> bool {
            self.find(token).is_some_and(|unit| unit.ghost)
        }
        fn release_time_remaining(&self) -> i32 {
            self.release_remaining
        }
        fn corpse_recovery_delay(&self) -> i32 {
            self.recovery_delay
        }
        fn resurrect_offerer(&self) -> Option<String> {
            self.offerer.clone()
        }
        fn resurrect_has_sickness(&self) -> bool {
            self.offer_sickness
        }
        fn resurrect_has_timer(&self) -> bool {
            self.offer_timer
        }
        fn unit_affecting_combat(&self, token: &str) -> bool {
            self.find(token).is_some_and(|unit| unit.in_combat)
        }
        fn unit_is_unit(&self, a: &str, b: &str) -> bool {
            match (self.find(a), self.find(b)) {
                (Some(a), Some(b)) => a.guid == b.guid,
                _ => false,
            }
        }
        /// Only the player has stats. See [`Stub::stats`].
        fn unit_stats(&self, token: &str) -> Option<vale_protocol::play::stats::UnitStats> {
            (token.eq_ignore_ascii_case("player")).then_some(self.stats).flatten()
        }
        fn unit_race(&self, token: &str) -> Option<(&'static str, &'static str)> {
            (token.eq_ignore_ascii_case("player") && self.stats.is_some())
                .then_some(("Night Elf", "NightElf"))
        }
        fn unit_class(&self, token: &str) -> Option<(&'static str, &'static str)> {
            (token.eq_ignore_ascii_case("player") && self.stats.is_some())
                .then_some(("Warrior", "Warrior"))
        }
        fn unit_creature_type(&self, token: &str) -> Option<&'static str> {
            let name = vale_protocol::state::query::creature_type_name(
                self.find(token).map_or(0, |unit| unit.creature_type),
            );
            (!name.is_empty()).then_some(name)
        }
        fn unit_classification(&self, token: &str) -> &'static str {
            super::classification_word(self.find(token).map_or(0, |unit| unit.classification))
        }
        /// No faction group: the tests here are about creatures, and the
        /// faction-group cases the interface asks about are tested in
        /// [`crate::lua::audit`].
        fn unit_faction_group(&self, _token: &str) -> Option<(String, String)> {
            None
        }
        fn unit_is_pvp(&self, token: &str) -> bool {
            self.find(token).is_some_and(|unit| unit.pvp)
        }
        fn unit_tooltip(&self, token: &str) -> Option<crate::interface::api::UnitTip> {
            let unit = self.find(token)?;
            Some(crate::interface::api::UnitTip {
                name: unit.name.clone(),
                title: crate::interface::api::UnitTitle {
                    player: unit.player,
                    ..unit.title.clone()
                },
                sub_name: unit.sub_name.clone(),
                level: unit.level,
                race: unit.player.then_some("Human"),
                class: unit.player.then_some("Warrior"),
                creature_type: self.unit_creature_type(token),
                classification: vale_protocol::state::query::classification_key(unit.classification),
                player_controlled: unit.player,
                pvp: unit.pvp,
                dead: unit.dead,
                health: Some((unit.health, unit.health_max)),
                zone: unit.zone.clone(),
            })
        }
        fn unit_is_connected(&self, _: &str) -> bool {
            true
        }
        /// The value set with [`Stub::standing`], or `None` for a token with
        /// no unit in this world. Every caller in the directory has a separate
        /// branch for `None`.
        fn unit_rank(&self, a: &str, b: &str) -> Option<vale_assets::tables::faction::Rank> {
            use vale_assets::tables::faction::{Rank, Reaction};
            self.find(a)?;
            self.find(b)?;
            Some(match self.standing {
                Reaction::Hostile => Rank::Hostile,
                Reaction::Neutral => Rank::Neutral,
                Reaction::Friendly => Rank::Friendly,
            })
        }
        fn unit_can_attack(&self, a: &str, b: &str) -> bool {
            self.unit_rank(a, b)
                .map(vale_assets::tables::faction::Reaction::from)
                .is_some_and(vale_assets::tables::faction::Reaction::is_attackable)
        }
        fn unit_player_controlled(&self, token: &str) -> bool {
            self.find(token).is_some_and(|unit| unit.player)
        }
    }

    /// Runs a chunk with the reads installed and returns its `return` value
    /// as a string, which lets a test assert on what Lua saw.
    pub fn eval(answers: &dyn Answers, chunk: &str) -> String {
        let lua = mlua::Lua::new();
        // The five held reads are registered once on the state, as
        // `LuaHost::new` does. See [`Held`].
        let (held, queue) = held_for_test(&lua);
        lua.scope(|scope| {
            install(&lua, scope, answers, &held, &queue)?;
            let value: mlua::Value = lua.load(chunk).eval()?;
            Ok(format!("{value:?}"))
        })
        .expect("the chunk runs")
    }

    /// A read is answered during the call. This is the property this module
    /// provides: the value comes back into a Lua expression rather than onto a
    /// queue drained later.
    #[test]
    fn a_read_answers_inside_the_expression() {
        let world = Stub::default().unit("target", "Kobold Vermin", 7);
        assert_eq!(
            eval(&world, r#"return UnitName("target") .. " at " .. UnitHealth("target")"#),
            r#"String("Kobold Vermin at 100")"#
        );
    }

    /// A boolean is `1` or `nil`, not `true`/`false`. This is the game's shape,
    /// and addons compare against it. See the module comment.
    #[test]
    fn a_boolean_comes_back_as_the_games_one_or_nil() {
        let world = Stub::default().unit("player", "Alden", 1);
        assert_eq!(eval(&world, r#"return UnitExists("player")"#), "Integer(1)");
        assert_eq!(eval(&world, r#"return UnitExists("target")"#), "Nil");
        // This comparison only holds if the value is a number.
        assert_eq!(
            eval(&world, r#"return UnitExists("player") == 1"#),
            "Boolean(true)"
        );
    }

    /// `GetActionBarToggles()` answers four values in the interface's order.
    /// This test checks the order.
    ///
    /// `UIParent.lua` assigns them in sequence,
    /// `SHOW_MULTI_ACTIONBAR_1, ..._2, ..._3, ..._4 = GetActionBarToggles()`,
    /// and `MultiActionBar_Update` uses the four for the bottom-left, the
    /// bottom-right, the right and the left column in that order. A swapped
    /// pair raises no error: it shows the right number of bars in the wrong
    /// places, which would only be visible after a relog.
    #[test]
    fn the_four_extra_bars_come_back_in_the_interfaces_own_order() {
        use vale_protocol::play::spells::multi_bar;
        let one = |mask: u8| Stub { bar_toggles: mask, ..Default::default() };
        let read = "local a, b, c, d = GetActionBarToggles(); \
                    return tostring(a)..tostring(b)..tostring(c)..tostring(d)";
        // A fresh character: four bars off, four nils, on which
        // `MultiActionBar_Update`'s `else` branch hides each bar.
        assert_eq!(eval(&one(0), read), r#"String("nilnilnilnil")"#);
        // One bar at a time, each on its own return value.
        assert_eq!(eval(&one(multi_bar::BOTTOM_LEFT), read), r#"String("1nilnilnil")"#);
        assert_eq!(eval(&one(multi_bar::BOTTOM_RIGHT), read), r#"String("nil1nilnil")"#);
        assert_eq!(eval(&one(multi_bar::RIGHT), read), r#"String("nilnil1nil")"#);
        assert_eq!(eval(&one(multi_bar::LEFT), read), r#"String("nilnilnil1")"#);
        assert_eq!(eval(&one(multi_bar::ALL), read), r#"String("1111")"#);
    }

    /// Each of the four extra-bar values is `1` rather than `true`, for the
    /// reason the boolean test above gives: `MultiBar1_IsVisible` hands its
    /// answer to an options checkbox, and `OptionsFrame` tests those against
    /// `1`.
    #[test]
    fn an_extra_bar_that_is_on_is_the_number_one() {
        let world = Stub {
            bar_toggles: vale_protocol::play::spells::multi_bar::BOTTOM_LEFT,
            ..Default::default()
        };
        assert_eq!(
            eval(&world, "local a = GetActionBarToggles(); return a == 1"),
            "Boolean(true)"
        );
    }

    /// An absent unit is `nil`, never `""`. FrameXML asks whether a unit is
    /// there with `if ( UnitName(u) )`, and an empty string is true in Lua.
    #[test]
    fn an_absent_unit_is_nil_rather_than_empty() {
        let world = Stub::default();
        assert_eq!(eval(&world, r#"return UnitName("target")"#), "Nil");
        assert_eq!(
            eval(&world, r#"if ( UnitName("target") ) then return 1 else return 0 end"#),
            "Integer(0)"
        );
        // A token this client has no state for answers the same as an empty
        // one, rather than raising an error.
        assert_eq!(eval(&world, r#"return UnitName("party3")"#), "Nil");
        // A call with no argument, which addons make, also answers nil.
        assert_eq!(eval(&world, "return UnitName()"), "Nil");
    }

    /// The branch in `TargetDebuffButton_Update` that lays out the aura rows.
    /// This one `UnitIsFriend` call decides whether a target's buffs sit
    /// against the frame or two rows under its debuffs. When it answered nil,
    /// every friendly target took the hostile layout.
    #[test]
    fn a_friendly_target_answers_the_branch_the_aura_rows_are_laid_out_on() {
        use vale_assets::tables::faction::Reaction;
        let branch = r#"
            if ( UnitIsFriend("player", "target") ) then return "buffs first"
            else return "debuffs first" end
        "#;
        let friendly = Stub::default()
            .unit("player", "Alden", 1)
            .unit("target", "Miravell", 2)
            .standing(Reaction::Friendly);
        assert_eq!(eval(&friendly, branch), r#"String("buffs first")"#);

        let hostile = Stub::default()
            .unit("player", "Alden", 1)
            .unit("target", "Kobold Vermin", 7)
            .standing(Reaction::Hostile);
        assert_eq!(eval(&hostile, branch), r#"String("debuffs first")"#);
        // `TargetFrame_OnShow` has a three-way branch whose first case asks
        // `UnitIsEnemy`. A neutral critter is neither enemy nor friend.
        assert_eq!(
            eval(&hostile, r#"return UnitIsEnemy("target", "player")"#),
            "Integer(1)"
        );
        let neutral = Stub::default()
            .unit("player", "Alden", 1)
            .unit("target", "Rabbit", 9)
            .standing(Reaction::Neutral);
        assert_eq!(
            eval(&neutral, r#"return UnitIsEnemy("target", "player")"#),
            "Nil"
        );
        assert_eq!(
            eval(&neutral, r#"return UnitIsFriend("player", "target")"#),
            "Nil"
        );
        // A neutral unit is still attackable, as
        // `vale_assets::tables::faction::Reaction::is_attackable` states. A
        // client that required hostility could not attack a rabbit.
        assert_eq!(
            eval(&neutral, r#"return UnitCanAttack("player", "target")"#),
            "Integer(1)"
        );
    }

    /// `UnitReaction` is an index into `UnitReactionColor`. The three rows
    /// this client can produce are the game's three colours: red at 2, yellow
    /// at 4, green at 5. A unit that is not there answers `nil`, which
    /// `TargetFrame_CheckFaction` handles in a separate branch. The constant
    /// `4` this read used to answer could not produce `nil`.
    #[test]
    fn the_reaction_is_the_row_of_the_colour_it_means() {
        use vale_assets::tables::faction::Reaction;
        let world = |standing| {
            Stub::default()
                .unit("player", "Alden", 1)
                .unit("target", "Somebody", 2)
                .standing(standing)
        };
        let read = r#"return UnitReaction("target", "player")"#;
        assert_eq!(eval(&world(Reaction::Hostile), read), "Integer(2)");
        assert_eq!(eval(&world(Reaction::Neutral), read), "Integer(4)");
        assert_eq!(eval(&world(Reaction::Friendly), read), "Integer(5)");
        assert_eq!(eval(&Stub::default(), read), "Nil");
    }

    /// `UnitPlayerControlled` is the first branch of `TargetFrame_CheckFaction`.
    /// It gives a player target's name plate the PvP colours instead of the
    /// reaction colours.
    #[test]
    fn a_player_target_is_player_controlled_and_a_creature_is_not() {
        let world = Stub::default()
            .unit("player", "Alden", 1)
            .played()
            .unit("target", "Kobold Vermin", 7);
        assert_eq!(
            eval(&world, r#"return UnitPlayerControlled("player")"#),
            "Integer(1)"
        );
        assert_eq!(
            eval(&world, r#"return UnitPlayerControlled("target")"#),
            "Nil"
        );
    }

    /// The `UnitIsUnit` read in the `TARGETSELF` binding's body: two absent
    /// tokens are not the same unit.
    #[test]
    fn two_absent_tokens_are_not_the_same_unit() {
        let world = Stub::default().unit("player", "Alden", 1);
        assert_eq!(
            eval(&world, r#"return UnitIsUnit("player", "target")"#),
            "Nil"
        );
        assert_eq!(
            eval(&world, r#"return UnitIsUnit("player", "player")"#),
            "Integer(1)"
        );
    }

    /// The cooldown comes back as the game's three values, which
    /// `local start, duration, enable = GetActionCooldown(n)` needs. A single
    /// value would leave `duration` nil without an error, and every cooldown
    /// swirl would have zero length.
    #[test]
    fn the_cooldown_is_three_return_values() {
        let mut world = Stub::default().action(1, "Rend");
        world.actions[0].cooldown = (100.0, 6.0, true);
        assert_eq!(
            eval(
                &world,
                "local s, d, e = GetActionCooldown(1); return s .. \"/\" .. d .. \"/\" .. e"
            ),
            r#"String("100/6/1")"#
        );
        // The same for the usable pair. Its second value is what makes a
        // button draw blue (not enough mana) rather than grey.
        let mut unaffordable = Stub::default().action(2, "Battle Shout");
        unaffordable.actions[0].usable = false;
        unaffordable.actions[0].not_enough_mana = true;
        assert_eq!(
            eval(
                &unaffordable,
                "local u, m = IsUsableAction(2); if ( not u and m ) then return 1 end return 0"
            ),
            "Integer(1)"
        );
    }

    /// `IsActionInRange` has three answers, and the interface tells them apart
    /// with `== 1` and `== 0`. A bool would produce a wrong but plausible
    /// picture: `ActionButton_OnUpdate` reads `nil` as "show nothing" and `0`
    /// as "colour the hotkey red", and `one_or_nil(false)` turns both into nil.
    ///
    /// The test asserts on the raw values rather than on truthiness, because
    /// Lua's `if` treats `nil` and `false` alike and would hide the difference.
    #[test]
    fn the_range_read_answers_one_zero_and_nil() {
        let inside = Stub::default().ranged_action(1, "Fireball", Some(true));
        assert_eq!(eval(&inside, "return IsActionInRange(1)"), "Integer(1)");
        assert_eq!(eval(&inside, "return ActionHasRange(1)"), "Integer(1)");

        let outside = Stub::default().ranged_action(1, "Fireball", Some(false));
        assert_eq!(eval(&outside, "return IsActionInRange(1)"), "Integer(0)");
        // The `Option<bool>` signature exists for this: an explicit 0 is not
        // the same value as nil, and the interface compares against both.
        assert_eq!(
            eval(
                &outside,
                "if ( IsActionInRange(1) == 0 ) then return 1 end return 0"
            ),
            "Integer(1)"
        );

        // A range the client cannot measure (nothing targeted) and a slot with
        // no range both answer nil, and neither draws a dot.
        let unmeasurable = Stub::default().ranged_action(1, "Fireball", None);
        assert_eq!(eval(&unmeasurable, "return IsActionInRange(1)"), "Nil");
        assert_eq!(eval(&unmeasurable, "return ActionHasRange(1)"), "Integer(1)");

        let rangeless = Stub::default().action(1, "Battle Shout");
        assert_eq!(eval(&rangeless, "return IsActionInRange(1)"), "Nil");
        assert_eq!(eval(&rangeless, "return ActionHasRange(1)"), "Nil");
    }

    /// `GetTime` is the time base every timer uses. It is not a wall clock and
    /// has no second origin. See [`crate::interface::api::get_time`].
    #[test]
    fn get_time_is_the_clock_the_bars_scrub_against() {
        let world = Stub {
            now: 1234.5,
            ..Stub::default()
        };
        assert_eq!(eval(&world, "return GetTime()"), "Number(1234.5)");
    }

    /// `GetGameTime` is the world's clock and returns two numbers, which
    /// `GameTime.lua` unpacks into `hour, minute`.
    ///
    /// The test asserts two values rather than one. A single return reads in
    /// Lua as the hour with a `nil` minute, and the frame's arithmetic
    /// (`hour * 60 + minute`) then raises. The load probe would report that
    /// error and the panel probe would not.
    #[test]
    fn the_game_clock_answers_an_hour_and_a_minute() {
        let world = Stub::default();
        assert_eq!(
            eval(&world, "local h, m = GetGameTime(); return h * 60 + m"),
            "Integer(720)"
        );
    }

    /// `GetQuestGreenRange` is about the player, whoever is targeted. It takes
    /// no argument, and the table is indexed by the player's level only.
    ///
    /// The interface calls it inside `GetDifficultyColor(targetLevel)`, which
    /// can make it look like a read about the target. It is not: the target's
    /// level is the argument to `GetDifficultyColor`, and this read is the
    /// width of the green band below the player's level.
    #[test]
    fn the_green_range_reads_the_players_level_and_not_the_targets() {
        let mut world = Stub::default().unit("player", "Alden", 1);
        world.units[0].level = 60;
        let mut world = world.unit("target", "Roc", 2);
        world.units[1].level = 42;
        assert_eq!(eval(&world, "return GetQuestGreenRange()"), "Integer(7)");

        // Lowering the player's level narrows the band while the target is
        // unchanged, which shows the read uses the player's level.
        world.units[0].level = 39;
        assert_eq!(eval(&world, "return GetQuestGreenRange()"), "Integer(5)");
    }

    /// All of `GetDifficultyColor`'s branches, using the interface's own
    /// function body. The failure this test guards against was not in the
    /// arithmetic: the last call in the chain, `GetQuestGreenRange`, was not
    /// defined.
    ///
    /// `QuestLogFrame.lua`'s five branches are copied here rather than
    /// loaded, deliberately: a test that loads 175 files to reach four
    /// comparisons is an acceptance run, and `--audit --events` is that run.
    /// The test checks that a target 18 levels below the player reaches the
    /// grey branch without raising. When `GetQuestGreenRange` was missing,
    /// this path raised on the nil global and stopped `TargetFrame_Update`.
    #[test]
    fn a_target_far_below_the_player_reaches_the_grey_branch_rather_than_raising() {
        let mut world = Stub::default().unit("player", "Alden", 1);
        world.units[0].level = 60;
        let cascade = r#"
            local level = 42
            local levelDiff = level - UnitLevel("player")
            if ( levelDiff >= 5 ) then return "impossible"
            elseif ( levelDiff >= 3 ) then return "verydifficult"
            elseif ( levelDiff >= -2 ) then return "difficult"
            elseif ( -levelDiff <= GetQuestGreenRange() ) then return "standard"
            else return "trivial" end
        "#;
        assert_eq!(eval(&world, cascade), "String(\"trivial\")");

        // The boundary the table decides: 60 - 7 = 53 is still green, 52 is
        // grey. No other branch can move that boundary.
        let at = |level: i32| cascade.replace("local level = 42", &format!("local level = {level}"));
        assert_eq!(eval(&world, &at(53)), "String(\"standard\")");
        assert_eq!(eval(&world, &at(52)), "String(\"trivial\")");
    }

    /// A slot outside the bar answers "nothing there" rather than reading the
    /// first button. Neither `0` nor a missing argument is a slot.
    #[test]
    fn a_slot_outside_the_bar_is_empty() {
        let world = Stub::default().action(1, "Rend");
        assert_eq!(eval(&world, "return HasAction(1)"), "Integer(1)");
        assert_eq!(eval(&world, "return HasAction(0)"), "Nil");
        assert_eq!(eval(&world, "return HasAction(13)"), "Nil");
        assert_eq!(eval(&world, "return HasAction()"), "Nil");
    }

    /// The 1.12.1 client's boolean coercion, case by case. See
    /// [`to_boolean`], where the rule is written out.
    ///
    /// The important cases are the two where Lua differs: `0` and `"false"`
    /// are false here and true under `lua_toboolean`, and the interface code
    /// passes both. Treating them as true drew a checked border on every
    /// action button and every spell in the book.
    #[test]
    fn the_boolean_coercion_is_the_clients_and_not_luas() {
        let t = |v: mlua::Value| to_boolean(Some(&v), true);
        let s = |text: &str| {
            let lua = mlua::Lua::new();
            to_boolean(
                Some(&mlua::Value::String(lua.create_string(text).expect("a string"))),
                true,
            )
        };

        assert!(!to_boolean(None, true), "an absent argument is false");
        assert!(!t(mlua::Value::Nil));
        assert!(t(mlua::Value::Boolean(true)));
        assert!(!t(mlua::Value::Boolean(false)));

        // Truncation towards zero, so 0.5 is false and -1.5 is true.
        assert!(!t(mlua::Value::Integer(0)));
        assert!(t(mlua::Value::Integer(1)));
        assert!(t(mlua::Value::Integer(-1)));
        assert!(!t(mlua::Value::Number(0.5)));
        assert!(t(mlua::Value::Number(-1.5)));

        // The first character decides, case-insensitively.
        for yes in ["1", "9", "true", "TRUE", "yes", "Y", "t"] {
            assert!(s(yes), "{yes}");
        }
        for no in ["0", "false", "FALSE", "no", "N", "f", "nil"] {
            assert!(!s(no), "{no}");
        }
        // The four strings whose first character does not decide, and which
        // the case-insensitive compares catch.
        assert!(!s("off"));
        assert!(!s("disabled"));
        assert!(s("on"));
        assert!(s("enabled"));

        // Anything else, including the empty string, takes the caller's
        // default, which every widget setter in the client sets to true.
        assert!(s(""));
        assert!(s("wombat"));
        let lua = mlua::Lua::new();
        let empty = mlua::Value::String(lua.create_string("").expect("a string"));
        assert!(!to_boolean(Some(&empty), false), "the default is the caller's");
        assert!(t(mlua::Value::Table(lua.create_table().expect("a table"))));
    }

    /// [`READS`] and what [`install`] registers are the same set, checked in
    /// both directions, and the list is sorted.
    ///
    /// The check used to run in one direction only. `UnitName` was registered
    /// but missing from the list, so `vale bindings` reported 18 reads where
    /// there were 19. The error made the client look less complete than it
    /// was, so no other check caught it.
    ///
    /// The reverse direction is a diff of the globals table across the install
    /// rather than a second hand-written list, because a second hand-written
    /// list can fall out of date in the same way as the first.
    #[test]
    fn the_list_and_the_registration_are_the_same_set() {
        for name in READS {
            assert_eq!(
                eval(&Stub::default(), &format!("return type({name})")),
                r#"String("function")"#,
                "{name} is claimed in READS and is not registered"
            );
        }
        let mut sorted = READS;
        sorted.sort_unstable();
        assert_eq!(sorted, READS, "READS is kept sorted");

        let world = Stub::default();
        let lua = mlua::Lua::new();
        let before = global_names(&lua);
        // Registered after the snapshot deliberately. These five are
        // persistent rather than scoped ([`Held`]), but they are still reads
        // in `READS`, so they must appear in the difference the check below
        // compares against the list.
        let (held, queue) = held_for_test(&lua);
        let installed = lua
            .scope(|scope| {
                install(&lua, scope, &world as &dyn Answers, &held, &queue)?;
                Ok(global_names(&lua))
            })
            .expect("the scope runs");
        let mut added: Vec<String> = installed.difference(&before).cloned().collect();
        added.sort();
        // Every panel's list as well as this file's, because `install` also
        // registers each panel module's reads into the same scope. The check
        // must cover everything the scope holds, not only what this file
        // added.
        let mut claimed: Vec<String> = READS
            .iter()
            .chain(crate::lua::panels::spellbook::READS.iter())
            .chain(crate::lua::panels::auras::READS.iter())
            .chain(crate::lua::panels::paperdoll::READS.iter())
            .chain(crate::lua::panels::container::READS.iter())
            .chain(crate::lua::panels::worldmap::READS.iter())
            .chain(crate::lua::panels::party::READS.iter())
            .chain(crate::lua::panels::raid::READS.iter())
            .chain(crate::lua::panels::pet::READS.iter())
            .chain(crate::lua::panels::shapeshift::READS.iter())
            .chain(crate::lua::panels::stable::READS.iter())
            .chain(crate::lua::panels::bank::READS.iter())
            .chain(crate::lua::panels::pagetext::READS.iter())
            .chain(crate::lua::panels::trade::READS.iter())
            .chain(crate::lua::panels::summon::READS.iter())
            .chain(crate::lua::panels::uioptions::READS.iter())
            .chain(crate::lua::panels::inspect::READS.iter())
            .chain(crate::lua::panels::guild::READS.iter())
            .chain(crate::lua::panels::loot::READS.iter())
            .chain(crate::lua::panels::lootroll::READS.iter())
            .chain(crate::lua::panels::quest::READS.iter())
            .chain(crate::lua::panels::gossip::READS.iter())
            .chain(crate::lua::panels::merchant::READS.iter())
            .chain(crate::lua::panels::mail::READS.iter())
            .chain(crate::lua::panels::trainer::READS.iter())
            .chain(crate::lua::panels::tradeskill::READS.iter())
            .chain(crate::lua::panels::craft::READS.iter())
            .chain(crate::lua::panels::talent::READS.iter())
            .chain(crate::lua::panels::taxi::READS.iter())
            .chain(crate::lua::panels::glue::READS.iter())
            .map(|name| (*name).to_string())
            .collect();
        claimed.sort();
        assert_eq!(
            added, claimed,
            "the scope registers a read no list claims, or the reverse"
        );
    }

    /// Every key in the globals table, as a set.
    fn global_names(lua: &mlua::Lua) -> std::collections::BTreeSet<String> {
        lua.globals()
            .pairs::<String, mlua::Value>()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .collect()
    }
}


/// What the interface may ask about the action bar, and about a spell waiting
/// to be aimed.
///
/// Separate from [`UnitAnswers`] because the two traits have different
/// subjects: one is about a creature in the world, the other about twelve
/// slots and a cursor. Both stay in this module because their `Live` bodies
/// read `crate::interface::action`, there is no `lua::action` module, and the
/// bar's registration is here.
pub trait ActionAnswers {

    // --- the action bar ---

    fn has_action(&self, slot: u8) -> bool;
    /// `GetBonusBarOffset`: 0 for the ordinary bar, 1..4 for a form's bar.
    /// See [`crate::interface::action::ActionBar::bonus_bar`].
    fn bonus_bar_offset(&self) -> u8;
    /// `GetActionBarToggles`: which of the four extra bars are switched on,
    /// as a mask of [`vale_protocol::play::spells::multi_bar`] bits.
    ///
    /// A mask rather than the four values the Lua call returns, because the
    /// four come from one byte the server keeps. Splitting them here would put
    /// the bit order in three implementations instead of one. See
    /// [`crate::interface::action::ActionBar::toggles`].
    fn action_bar_toggles(&self) -> u8;
    /// `GameTooltip:SetAction`: the data the spell tooltip is composed from,
    /// or `None` for a slot with no tooltip. See
    /// [`crate::interface::api::action_tooltip`].
    fn action_tooltip(&self, slot: u8) -> Option<api::SpellTip>;
    /// The same call for a slot holding an item, which uses a different
    /// plate. `None` for a spell slot, so the two cannot both answer.
    fn action_item_tooltip(&self, slot: u8) -> Option<api::ItemTip>;
    fn action_text(&self, slot: u8) -> Option<String>;
    fn action_texture(&self, slot: u8) -> Option<String>;
    /// `(start, duration, enable)`, in [`Answers::now`]'s base.
    fn action_cooldown(&self, slot: u8) -> (f64, f64, bool);
    /// `(usable, not_enough_mana)`: the game's pair. The second value is what
    /// makes a button draw blue rather than grey.
    fn action_usable(&self, slot: u8) -> (bool, bool);
    fn is_attack_action(&self, slot: u8) -> bool;
    fn is_current_action(&self, slot: u8) -> bool;
    /// `IsAutoRepeatAction`: whether this slot is the ranged attack currently
    /// repeating. Placed beside [`Self::is_current_action`] because
    /// `ActionButton.lua` asks the two together; see
    /// [`crate::interface::api::is_auto_repeat_action`].
    fn is_auto_repeat_action(&self, slot: u8) -> bool;
    /// `ActionHasRange`: whether this button has a range at all. See
    /// [`crate::interface::api::action_has_range`].
    fn action_has_range(&self, slot: u8) -> bool;
    /// `IsActionInRange`: three answers, `Some(true)`, `Some(false)` and
    /// `None`, which the interface reads as `1`, `0` and `nil` and treats as
    /// three different states. See
    /// [`crate::interface::api::is_action_in_range`].
    fn is_action_in_range(&self, slot: u8) -> Option<bool>;
    /// `IsConsumableAction`: whether `ActionButton_UpdateCount` writes a stack
    /// count under this button. The rule depends on the item, not the bar;
    /// see [`vale_protocol::state::query::ItemInfo::is_consumable`].
    fn is_consumable_action(&self, slot: u8) -> bool;
    /// `IsEquippedAction`: whether the item in this slot is being worn. It
    /// draws the green border round the button.
    fn is_equipped_action(&self, slot: u8) -> bool;
    /// `GetActionCount`: how many of the slot's item the character carries,
    /// across every container and the worn slots. `0` for a spell or a macro,
    /// as in the 1.12.1 client; [`Self::is_consumable_action`] keeps that
    /// zero from being drawn.
    fn action_count(&self, slot: u8) -> u32;

    // --- Spell targeting cursor ---
    //
    // The interface's side of 1.12.1's targeting mode is these two reads plus
    // two writes. `UnitFrame_OnEnter` asks both to choose between
    // `SetCursor("CAST_CURSOR")` and `SetCursor("CAST_ERROR_CURSOR")`, and
    // `TargetFrame_OnClick` asks the first to decide whether a click casts or
    // retargets. See [`crate::interface::action::SpellTargeting`].

    /// `SpellIsTargeting()`: whether a cast is waiting for a target.
    fn spell_is_targeting(&self) -> bool;
    /// Whether there is a cast to stop: the read behind `SpellStopCasting()`.
    /// That call answers whether it cancelled anything, and the answer decides
    /// whether `ToggleGameMenu`'s chain stops at that branch or goes on to
    /// open the menu.
    ///
    /// True in all three states a cancel can end: a cast in progress, a
    /// channel, and a cast request the server has not answered yet. The third
    /// is needed because the bar waits for `SMSG_SPELL_START`, so for one
    /// round trip a player pressing Escape intends to cancel while nothing is
    /// "casting" yet. See [`crate::interface::action::Casting`].
    fn spell_is_casting(&self) -> bool;
    /// `SpellCanTargetUnit(unit)`: whether the waiting cast would accept this
    /// unit.
    ///
    /// False when nothing is waiting. Every caller in the interface code
    /// already checks `if SpellIsTargeting()` first.
    fn spell_can_target_unit(&self, token: &str) -> bool;
}

impl Live<'_, '_, '_> {
    /// The spell that holds an item slot's cooldown.
    ///
    /// A bar slot holding an item carries only the entry (`SMSG_ACTION_BUTTONS`
    /// carries nothing else), so the swirl's timer is found through a chain:
    /// entry -> the item's cached template -> its on-use spell -> `Spell.dbc`.
    /// The timer is on that spell's category, which every healing potion in
    /// the game shares, so a lookup by entry would find nothing even if this
    /// client kept per-item timers.
    ///
    /// `None` in three states, and each correctly draws no swirl: the slot is
    /// not an item; the template has not arrived yet (every item's template
    /// takes a server round trip, see [`crate::interface::items`]); or the
    /// item has no on-use spell, such as a garment placed on the bar.
    fn item_action_spell(&self, slot: u8) -> Option<vale_assets::tables::spellbook::SpellInfo> {
        let entry = api::action_item(self.bar, slot)?;
        let template = self.inventory.template(entry)?;
        let index = template.on_use_spell()?;
        let id = template.spells.get(usize::from(index))?.spell_id;
        self.tables.as_ref()?.spellbook()?.info(id)
    }
}

impl ActionAnswers for Live<'_, '_, '_> {

    fn has_action(&self, slot: u8) -> bool {
        api::has_action(self.bar, slot)
    }

    fn bonus_bar_offset(&self) -> u8 {
        self.bar.bonus_bar
    }

    fn action_bar_toggles(&self) -> u8 {
        self.bar.toggles
    }

    fn action_tooltip(&self, slot: u8) -> Option<api::SpellTip> {
        let names = |entry| self.reagent_name(entry);
        api::action_tooltip(
            self.bar,
            slot,
            &api::TipContext {
                level: self.tip_level(),
                // A spell plate has no requirement lines, so race and class
                // are unused. [`Self::item_context`] fills them for items.
                race: 0,
                class: 0,
                catalog: self.tables.as_ref().and_then(|t| t.spellbook()),
                item_names: &names,
                home: Some(self.home.clone()),
            },
        )
    }

    /// The item plate for a bar slot, composed from the template alone with
    /// no stack. A bar slot names an entry, not a bag square: the same entry
    /// may sit in three bags, so there is no single stack to take a count or
    /// a durability from.
    fn action_item_tooltip(&self, slot: u8) -> Option<api::ItemTip> {
        self.tip_from(api::action_item(self.bar, slot)?, None)
    }

    fn action_text(&self, slot: u8) -> Option<String> {
        api::get_action_text(self.bar, slot)
    }

    /// A spell's icon or an item's. This branch decides whether an item on
    /// the bar, such as a mount, is drawn at all. An item slot has no
    /// `SpellInfo`, so the spell path answers `None` and `ActionButton_Update`
    /// hides the icon. Without the item branch, an item button showed as an
    /// empty slot.
    fn action_texture(&self, slot: u8) -> Option<String> {
        if let Some(entry) = api::action_item(self.bar, slot) {
            let template = self.inventory.template(entry)?;
            return self.tables.as_ref()?.item_icon(template.display_id);
        }
        api::get_action_texture(self.bar, slot).map(str::to_string)
    }

    /// The same split for the cooldown swirl. An item's cooldown is its
    /// `ON_USE` spell's, read through the same clocks a bag square uses, so
    /// the bar and the bag cannot show different timers.
    fn action_cooldown(&self, slot: u8) -> (f64, f64, bool) {
        if let Some(entry) = api::action_item(self.bar, slot) {
            return self.item_cooldown(Some(entry));
        }
        api::get_action_cooldown(
            self.bar,
            self.cooldowns,
            slot,
            self.now,
            self.item_action_spell(slot).as_ref(),
        )
    }

    fn action_usable(&self, slot: u8) -> (bool, bool) {
        api::is_usable_action(
            self.bar,
            self.units,
            self.inventory,
            slot,
            api::action_item(self.bar, slot).map(|entry| self.inventory.carried.count_of(entry)),
        )
    }

    fn is_attack_action(&self, slot: u8) -> bool {
        api::is_attack_action(self.bar, slot)
    }

    fn is_current_action(&self, slot: u8) -> bool {
        api::is_current_action(
            self.bar,
            slot,
            self.attacking,
            self.casting.next_swing,
            // The cast in flight, from `Casting`. `started` is set when the
            // cast is sent and cleared when it lands, is interrupted or is
            // refused, which is when the border should go out.
            // [`crate::lua::panels::spellbook`] asks the same pair for a book
            // button.
            self.casting.started.map(|_| self.casting.spell_id),
            // The spell waiting on the targeting cursor. The 1.12.1 client
            // also lights the border for it; see [`api::is_current_action`].
            self.targeting.spell(),
        )
    }

    fn is_auto_repeat_action(&self, slot: u8) -> bool {
        api::is_auto_repeat_action(self.bar, self.auto_repeat, slot)
    }

    fn action_has_range(&self, slot: u8) -> bool {
        api::action_has_range(self.bar, slot, self.item_action_spell(slot).as_ref())
    }

    fn is_action_in_range(&self, slot: u8) -> Option<bool> {
        api::is_action_in_range(
            self.bar,
            self.units,
            slot,
            self.item_action_spell(slot).as_ref(),
        )
    }

    fn is_consumable_action(&self, slot: u8) -> bool {
        api::action_item(self.bar, slot)
            .and_then(|entry| self.inventory.template(entry))
            .is_some_and(vale_protocol::state::query::ItemInfo::is_consumable)
    }

    fn is_equipped_action(&self, slot: u8) -> bool {
        api::action_item(self.bar, slot)
            .is_some_and(|entry| self.inventory.carried.is_equipped(entry))
    }

    fn action_count(&self, slot: u8) -> u32 {
        api::action_item(self.bar, slot)
            .map(|entry| self.inventory.carried.count_of(entry))
            .unwrap_or(0)
    }

    fn spell_is_targeting(&self) -> bool {
        self.targeting.is_targeting()
    }

    fn spell_is_casting(&self) -> bool {
        self.casting.started.is_some()
            || self.casting.pending.is_some()
            || self.casting.channelling.is_some()
    }

    fn spell_can_target_unit(&self, token: &str) -> bool {
        // Answered through the same [`resolve_aim`] the click will run, rather
        // than from a cached answer. See
        // [`crate::interface::target::spell_cursor_validity`], which asks the
        // same question about the world pick, for the same reason. Only
        // `Unit(_)` is a yes: a spell that would bind its target implicitly
        // cannot take this frame's unit as its target.
        let answer = || {
            let tables = self.tables.as_ref()?;
            let info = tables.spellbook()?.info(self.targeting.spell()?)?;
            let me = self.units.get(crate::interface::api::UnitId::Player)?;
            let unit = self.units.get(Self::id(token)?)?;
            let who = vale_assets::tables::spellbook::Candidate {
                guid: unit.guid,
                is_self: unit.guid == me.guid,
                reaction: tables.reaction(me.faction, unit.faction),
                unit_flags: unit.unit_flags,
                dead: unit.dead,
            };
            Some(matches!(
                vale_assets::tables::spellbook::resolve_aim(&info, Some(who), None, false, None),
                vale_assets::tables::spellbook::CastAim::Unit(_)
            ))
        };
        answer().unwrap_or(false)
    }
}
