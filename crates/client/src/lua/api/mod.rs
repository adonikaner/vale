//! **The reads**, answered *during* the call — which is the half a widget tree
//! cannot be written without.
//!
//! [`self::verbs`] is the write side and it works by recording: a registered
//! closure cannot hold `&mut World`, so `ToggleSheath()` pushes a value the
//! caller drains afterwards. That shape is fine for a verb and useless for a
//! query. `UnitHealth("target")` has to answer *now*, in the middle of a Lua
//! expression, and there is nowhere to defer it to:
//!
//! ```lua
//! if ( UnitHealth("target") / UnitHealthMax("target") < 0.2 ) then
//! ```
//!
//! ## How: a scoped borrow, and a trait to erase the lifetimes
//!
//! `mlua::Lua::scope` creates functions that may capture **non-`'static`**
//! references and are destroyed when the scope ends. So the shape is:
//!
//! ```text
//! a system holds the world's state       Units, ActionBar, Cooldowns, Time
//!   -> wraps it as one &dyn Answers      Live, below
//!   -> LuaHost::run opens a scope        the functions exist only inside it
//!   -> the chunk runs and asks           UnitHealth("target") reads the live query
//!   -> the scope closes                  and the functions are gone again
//! ```
//!
//! The trait is what makes that compile: `Live` borrows a `SystemParam` whose two
//! lifetimes would otherwise have to be threaded through every signature in this
//! directory. `&dyn Answers` erases them, and it buys a second thing that turns
//! out to matter more — **the read side is unit-testable with no `World`**, since
//! a test can implement `Answers` in twenty lines. Every test in this file does.
//!
//! **Nothing may run Lua outside such a scope.** Not a rule of taste: a global
//! left pointing at a destroyed scoped function raises "callback destructed" when
//! called, which is a confusing error in place of a working one. [`super::host`]
//! therefore has exactly one entry point and it takes an `&dyn Answers`.
//!
//! ## The return shapes are the game's, and `true` is not one of them
//!
//! **A 1.12 API function answers a boolean as `1` or `nil`.** Not `true`/`false`
//! — the C side pushes a number or nothing at all. `if ( x )` behaves the same
//! either way, which is exactly why this is worth being deliberate about: every
//! FrameXML use of these would work with Lua booleans, and the addon that writes
//! `if UnitAffectingCombat("player") == 1` would silently stop. What is
//! measured *here* is only that the shipped FrameXML never distinguishes the
//! two, which is why the safe choice is the game's own.
//!
//! One inconsistency is worth recording rather than smoothing over: that
//! reference answers `UnitExists` with a Lua boolean and its neighbours with
//! `1`/`nil`, and this file does not know which of the two the client does for
//! that particular function. It answers `1`/`nil` throughout, on the argument
//! that `== 1` is the comparison an addon can write and `== true` is not.
//!
//! Absent things are `nil`, never a placeholder: `UnitName("party1")` with no
//! party is `nil` and not `""`, because `if ( UnitName(u) ) then` is how FrameXML
//! asks whether a unit is there.
//!
//! ```text
//! mod.rs      the `Answers` trait — the sum of the twelve subject traits the
//!             panels declare — and the reads that belong to no panel
//! verbs.rs    …and the other direction: a C function that *records* an intent
//! stubs.rs    …and the ones that answer a constant, counted apart for it
//! cvars.rs    the client's own settings — which is what every options panel
//!             in the game is written against, and nothing else
//! savedvars.rs …except for its forty-four `uvar` rows, which are Lua globals
//!             persisted by `RegisterForSave` into a file of their own
//! events.rs   what the world tells the interface, and who registered for it
//! update.rs   …and the interface's own clock, which is 30 Hz and not the frame
//! mouse.rs    what the pointer is on, and the five handlers it fires
//! keyboard.rs …and whether a key is a binding or a character, never both
//! sound.rs    the four verbs that make a noise
//! portrait.rs …and the one that puts a unit's face on a texture
//! ```


use crate::game::combat::action::{ActionBar, Casting, Cooldowns};
use crate::game::api::{self, UnitId, Units};
use crate::game::combat::spellbook::Spellbook;
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


/// **Everything the interface may ask, as one name** — the sum of the twelve
/// subject traits, not a place any of them live.
///
/// It used to be a single trait with **132 methods** covering fifteen unrelated
/// subjects, in a 4,454-line file. A container read was declared here,
/// answered here, registered next door in `container.rs` and named in a
/// `READS` array there too, so four of a feature's five pieces were somewhere
/// other than the file the feature is named after.
///
/// Now each subject owns its own trait beside its own registration, and this is
/// what joins them. Nothing that consumes the API changed: `&dyn Answers` still
/// resolves every one of the 132 methods, because a trait object carries its
/// supertraits' methods.
///
/// **The blanket impl is what makes it free to add one.** A new subject trait
/// goes in the list below and any type answering all sixteen is an `Answers`
/// automatically — no fourth place to remember, which is the whole complaint
/// this split was fixing.
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
    + super::panels::glue::GlueAnswers
    + super::panels::spellbook::SpellbookAnswers
    + super::panels::talent::TalentAnswers
    + super::panels::auras::AuraAnswers
{
}

/// Anything that answers all sixteen answers the interface.
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
    + super::panels::glue::GlueAnswers
    + super::panels::spellbook::SpellbookAnswers
    + super::panels::talent::TalentAnswers
    + super::panels::auras::AuraAnswers
{
}

/// **What the interface may ask about a creature** — and, at the end, about
/// being one that has died.
///
/// One method per API function, named after it. Four implementations: [`Live`],
/// which reads the world; `Stub` in this file's own tests; and `Login` and
/// `Ticking` in [`super::audit`], which are what the headless probes answer
/// from. The point of the trait is that neither it nor its Lua registration
/// knows which of the four it is holding.
///
/// A token this client has no state for answers the *absent* value rather than
/// a guess, all the way down: see [`crate::game::api::UnitId::parse`], which is
/// what every `&str` token here goes through.
///
/// The death reads sit at the bottom rather than in a `DeathAnswers` of their
/// own because there is no `lua::death` for them to live beside — the game's
/// three death boxes are `StaticPopup`s, so their C functions have no file of
/// their own to be registered from. See [`super::api::Answers`] for the twelve
/// that do.
pub trait UnitAnswers {
    // --- the clock ---

    /// `GetTime()` — seconds since the client started. See
    /// [`crate::game::api::get_time`], which is the one place the base is set.
    fn now(&self) -> f64;

    /// `GetGameTime()` — the **world's** hour and minute, not the machine's.
    ///
    /// The server states it in `SMSG_LOGIN_SETTIMESPEED` and the session thread
    /// advances it; this reads [`crate::render::sky::WorldClock`], which is the
    /// one place the hour is decided, so the clock on the minimap and the light
    /// on the ground can never disagree. A hand-set hour (`--hour`, the settings
    /// panel) is in that number already.
    ///
    /// `GameTime.lua` compares the answer against the minute it last drew and
    /// only re-cuts its texture when the two differ, so a **constant** here is
    /// indistinguishable from a stopped clock: the frame keeps the whole
    /// 128x64 sheet's default coordinates and draws the day icon and the night
    /// icon side by side. That was this function for as long as it was a stub.
    fn game_time(&self) -> (u32, u32);

    /// `GetBindLocation()` — **where the hearthstone returns the character
    /// to**, as a name.
    ///
    /// `HOME_INN` ("your inn") before the bind point has arrived, which is the
    /// reference's own fallback rather than an empty string: it falls back to
    /// it whenever the stored area id is unset or out of `AreaTable`'s range.
    /// See [`home_name`].
    fn bind_location(&self) -> String;

    /// `CheckBinderDist()` — **is the innkeeper who asked still close
    /// enough?**
    ///
    /// `StaticPopupDialogs["CONFIRM_BINDER"]`'s `OnUpdate` hides the box when
    /// this answers false, which is what takes the question away when the
    /// player walks off mid-question. True with nothing pending, so a frame
    /// between the event and the popup's first update cannot close it.
    fn binder_in_range(&self) -> bool;

    /// `CheckPetUntrainerDist()` — **is the pet trainer still close enough?**
    ///
    /// The `CONFIRM_PET_UNLEARN` box's `OnUpdate` hides it when this answers
    /// false, exactly as the binder's does. See
    /// [`crate::game::npc::untrainer`].
    fn untrainer_in_range(&self) -> bool;

    // --- units ---

    fn unit_exists(&self, token: &str) -> bool;
    /// `UnitIsVisible(unit)` — the unit is in the object manager *and* placed:
    /// a party member across the zone exists and is not visible. Every
    /// unit-frame addon asks it before drawing a row. Defaults to
    /// [`Self::unit_exists`] for a double with no positions.
    fn unit_is_visible(&self, token: &str) -> bool {
        self.unit_exists(token)
    }
    /// `CheckInteractDistance(unit, index)` — within the range of one of the
    /// four interactions: 1 inspect (28 yards), 2 trade (11.11), 3 duel (9.9),
    /// 4 follow (28). The four distances are the ones every 1.12 reference
    /// states for this call; they have not been confirmed against the client.
    /// Defaults to false for a double with no positions.
    fn unit_in_range(&self, _token: &str, _index: u32) -> bool {
        false
    }
    /// `nil` for a unit that is not there, never `""`.
    fn unit_name(&self, token: &str) -> Option<String>;
    /// `-1` for a level we do not know, which is the game's own answer and what
    /// the interface draws as a skull.
    fn unit_level(&self, token: &str) -> i32;
    /// `UnitSex(unit)` — 2 male, 3 female, 1 neuter, and **2 for a unit that is
    /// not there**. See [`crate::game::api::Units::sex`], where the table and
    /// the fallback are.
    fn unit_sex(&self, token: &str) -> u32;
    fn unit_health(&self, token: &str) -> u32;
    fn unit_health_max(&self, token: &str) -> u32;
    /// `UnitXP` and `UnitXPMax`, together — see
    /// [`crate::game::api::Units::experience`] for why the pair is one answer.
    fn unit_experience(&self, token: &str) -> (u32, u32);
    /// `UnitCharacterPoints(unit)` — **unspent talent points, then unspent
    /// profession points**, together for the same reason the XP pair is: they
    /// are one field pair, read in one call, by two different panels. See
    /// [`crate::game::api::Units::character_points`].
    fn unit_character_points(&self, token: &str) -> (u32, u32);
    /// `GetXPExhaustion()` — the rested pool, `nil` when there is none.
    fn rested_experience(&self) -> Option<u32>;
    /// `UnitMana` — **as the interface shows it**, so a warrior's rage is 0..100
    /// and not the field's 0..1000.
    fn unit_mana(&self, token: &str) -> u32;
    fn unit_mana_max(&self, token: &str) -> u32;
    fn unit_power_type(&self, token: &str) -> Option<u8>;
    /// `UnitIsConnected` — false only for a party member the roster says is
    /// offline.
    fn unit_is_connected(&self, token: &str) -> bool;
    fn unit_is_dead(&self, token: &str) -> bool;
    /// `UnitIsGhost` — **released, not merely dead**, which are two different
    /// states with two different boxes on screen. See [`crate::game::character::death`].
    fn unit_is_ghost(&self, token: &str) -> bool;
    fn unit_affecting_combat(&self, token: &str) -> bool;

    // --- being dead ---
    //
    // Five reads, all about the local player, all answering off
    // [`crate::game::character::death::Dying`]. They are here rather than in
    // [`self::stubs`] because each one **decides what a box says**: the
    // release box's own sentence, whether the Retrieve button counts down, and
    // which of three resurrect popups opens.

    /// `GetReleaseTimeRemaining()` — seconds, or **`-1` for "no timer"**, which
    /// is the answer inside an instance and the one the `DEATH` box tests for
    /// before swapping its text for `DEATH_RELEASE_NOTIMER`.
    fn release_time_remaining(&self) -> i32;
    /// `GetCorpseRecoveryDelay()` — seconds until the body may be taken back.
    /// Both corpse popups use it as their `StartDelay`.
    fn corpse_recovery_delay(&self) -> i32;
    /// `ResurrectGetOfferer()` — who is offering, or `nil`.
    fn resurrect_offerer(&self) -> Option<String>;
    /// `ResurrectHasSickness()` / `ResurrectHasTimer()` — the two flags the
    /// offer carries, which is what `UIParent_OnEvent` picks between three
    /// popups on.
    fn resurrect_has_sickness(&self) -> bool;
    fn resurrect_has_timer(&self) -> bool;
    /// `GetResSicknessDuration()` — **the sentence's own duration word**, "10
    /// minutes", or `None` for a character who would get no sickness at all,
    /// which is what picks `XP_LOSS_NO_SICKNESS` over `XP_LOSS`. The rule is
    /// the client's: the race's `ResSicknessSpellID` (`ChrRaces` field 12),
    /// that spell's `SpellDuration` row at the character's level, `None` under
    /// one second, and `%s_MIN`/`%s_SEC` off the `GENERIC` family for the
    /// words. Defaulted to `None` because a headless harness has no level.
    fn res_sickness_duration(&self) -> Option<String> {
        None
    }
    /// `CheckSpiritHealerDist()` — is the healer whose offer is on the table
    /// still in reach? The client compares the squared distance to the guid
    /// the confirm carried; the gate that matters is the server's
    /// `INTERACTION_DISTANCE` on `CMSG_SPIRIT_HEALER_ACTIVATE`, so that is the
    /// number. `false` with no offer, which is what closes a stale box.
    fn spirit_healer_in_reach(&self) -> bool {
        false
    }
    /// Two tokens that both resolve to nothing are **not** the same unit — see
    /// [`crate::game::api::Units::is_unit`].
    fn unit_is_unit(&self, a: &str, b: &str) -> bool;
    /// **The character sheet's whole population, as one answer** — eleven of
    /// the game's C functions read off it, and every one of them is a method
    /// on [`vale_protocol::play::stats::UnitStats`]. `None` for a unit whose
    /// block never crossed the wire, which is everyone but us.
    ///
    /// One trait method rather than eleven because the split that matters is
    /// *live world / not live world*, and the arithmetic on the far side of
    /// this is testable with no `Answers` at all.
    fn unit_stats(&self, token: &str) -> Option<vale_protocol::play::stats::UnitStats>;
    /// `UnitRace` — `(localised, fileName)`.
    fn unit_race(&self, token: &str) -> Option<(&'static str, &'static str)>;
    /// `UnitClass` — the same pair, and both call sites `strupper` the second.
    fn unit_class(&self, token: &str) -> Option<(&'static str, &'static str)>;
    /// `UnitCreatureType` — the `CreatureType.dbc` word, and **nil for a
    /// player**, which is the game's own answer.
    fn unit_creature_type(&self, token: &str) -> Option<&'static str>;
    /// `UnitClassification` — `"elite"`, `"rareelite"`, `"worldboss"`, `"rare"`
    /// or `"normal"`. Lower case: the directory compares against literals in
    /// that case.
    fn unit_classification(&self, token: &str) -> &'static str;
    /// `UnitIsPVP` — flagged for open combat.
    fn unit_is_pvp(&self, token: &str) -> bool;
    /// **`UnitFactionGroup`** — `(internalName, localisedName)`, or `None` for a
    /// unit with no side in words, which is every creature in the world.
    ///
    /// The first return is what the interface builds a *path* out of
    /// (`Interface\GroupFrame\UI-Group-PVP-<group>`), so it is the untranslated
    /// one; the second is what it would print. See
    /// [`vale_assets::tables::faction::Factions::group_name`], where the walk and
    /// its empty-name rule are.
    fn unit_faction_group(&self, token: &str) -> Option<(String, String)>;
    /// **`GameTooltip:SetUnit`'s whole population, as one answer** — the same
    /// argument [`Answers::unit_stats`] makes one line up: the plate is about
    /// one unit and its composition is testable with no `Answers` at all.
    fn unit_tooltip(&self, token: &str) -> Option<crate::game::api::UnitTip>;

    // --- friend or foe ---
    //
    // **Five of the game's names off one reading**, because they are five
    // questions about one fact and answering some of them would put the
    // interface in a state the real client never reaches. See
    // [`vale_assets::tables::faction`], which owns the rule, and the note on
    // [`Answers::unit_reaction`] for what these cost when they are nil.

    /// `UnitReaction(a, b)`'s underlying answer: how the first stands towards
    /// the second **on the client's own eight-rank scale**, or `None` when
    /// either token names nothing — which is the nil
    /// `TargetFrame_CheckFaction` falls through to a blue name plate on.
    ///
    /// The rank rather than the three-way fold, because `UnitReactionColor` has
    /// eight rows and the orange one at index 3 is Unfriendly — a rank only the
    /// character's own reputation ever produces, and one this client could not
    /// reach at all until it read that reputation.
    fn unit_rank(&self, a: &str, b: &str) -> Option<vale_assets::tables::faction::Rank>;
    /// `UnitCanAttack(a, b)` — the reaction **and** the victim's own flags; see
    /// [`crate::game::api::Units::can_attack`].
    fn unit_can_attack(&self, a: &str, b: &str) -> bool;
    /// `UnitPlayerControlled` — is a person driving this unit?
    fn unit_player_controlled(&self, token: &str) -> bool;
}

/// `GetSpellTabInfo`'s four answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpellTab {
    pub name: String,
    /// `Interface\Icons\…`, or empty for a line whose icon id is not in
    /// `SpellIcon.dbc` — empty rather than absent because
    /// `skillLineTab:SetNormalTexture(texture)` takes it either way and a
    /// texture set to `nil` and one set to `""` draw the same nothing.
    pub texture: String,
    pub offset: usize,
    pub count: usize,
}

/// The live world, as an [`Answers`].
///
/// Built fresh by each system that enters Lua and thrown away when the call
/// returns; it holds only borrows, so there is nothing here that can go stale —
/// which is the whole argument for this shape over a snapshot refreshed once a
/// frame.
pub struct Live<'a, 'w, 's> {
    pub units: &'a Units<'w, 's>,
    pub bar: &'a ActionBar,
    pub cooldowns: &'a Cooldowns,
    pub book: &'a Spellbook,
    /// **…and the *other* bar**, which the server states outright where the
    /// player's own is the client's — see [`crate::game::combat::pet`].
    pub pet_bar: &'a crate::game::combat::pet::PetBar,
    /// **The talent trees**, which the panel reads twenty buttons at a time —
    /// see [`crate::game::character::talents`] and [`super::panels::talent`].
    pub talents: &'a crate::game::character::talents::Talents,
    pub casting: &'a Casting,
    /// **Which ranged attack is repeating**, for `IsAutoRepeatAction` — a spell
    /// id rather than a borrow, because that is the whole of the state and
    /// copying a `u32` is cheaper than the reference to it. See
    /// [`crate::game::combat::action::AutoRepeat`].
    pub auto_repeat: Option<u32>,
    /// The spell cursor's own state — see
    /// [`crate::game::combat::action::SpellTargeting`]. Beside `casting` because they
    /// are the two halves of "is a cast happening": one is running and the other
    /// is waiting to be aimed, and the interface asks about them separately.
    pub targeting: &'a crate::game::combat::action::SpellTargeting,
    /// What is on the units the interface can ask about — see
    /// [`crate::game::combat::auras`].
    pub auras: &'a crate::game::combat::auras::Auras,
    /// Being dead: the two clocks and the offer on the table — see
    /// [`crate::game::character::death`].
    pub dying: &'a crate::game::character::death::Dying,
    /// …and what is being read, if anything — see [`crate::game::npc::pagetext`].
    pub page: &'a crate::game::npc::pagetext::OpenBook,
    /// …and the summon on the table, if any — see
    /// [`crate::game::session::summon`].
    pub summon: &'a crate::game::session::summon::Summon,
    /// `GlobalStrings.lua`, for the one tab whose name is a key rather than a
    /// word — see [`vale_assets::tables::book::GENERAL_NAME_KEY`]. `None` before the
    /// table has loaded, which draws the key.
    pub strings: Option<&'a Strings>,
    /// **What is on the body**, while a loot window is open — see
    /// [`crate::game::npc::loot`]. `None` for the whole of a session in which
    /// nothing has been right-clicked.
    pub loot: &'a crate::game::npc::loot::LootWindow,
    /// …and, in a group, the rolls open on what is on it — see
    /// [`crate::game::npc::lootroll`]. Empty for every session that never
    /// groups, which is what the systems behind it check first.
    pub rolls: &'a crate::game::npc::lootroll::LootRolls,
    /// …and the quest log and the page in front of the character — see
    /// [`crate::game::npc::quest`].
    pub quests: &'a crate::game::npc::quest::Quests,
    /// …and the two NPC windows — see [`crate::game::npc::gossip`] and
    /// [`crate::game::npc::merchant`].
    pub gossip: &'a crate::game::npc::gossip::GossipWindow,
    pub merchant: &'a crate::game::npc::merchant::MerchantWindow,
    /// …and the box on the corner, which is a window no NPC owns — see
    /// [`crate::game::npc::mail`].
    pub mail: &'a crate::game::npc::mail::Mailbox,
    /// …and the third, whose panel is the game's own load-on-demand addon — see
    /// [`crate::game::npc::trainer`].
    pub trainer: &'a crate::game::npc::trainer::TrainerWindow,
    /// …and the two profession windows, which are the character's own rather
    /// than an NPC's — see [`crate::game::character::tradeskill`].
    pub tradeskill: &'a crate::game::character::tradeskill::TradeSkillWindow,
    pub craft: &'a crate::game::character::tradeskill::CraftWindow,
    /// …and the fourth, whose whole content the client works out for itself —
    /// see [`crate::game::npc::taxi`].
    pub taxi: &'a crate::game::npc::taxi::TaxiWindow,
    /// …and the fifth, one gossip option deeper than the rest — see
    /// [`crate::game::npc::stable`].
    pub stable: &'a crate::game::npc::stable::StableWindow,
    /// …and the sixth, whose contents are the inventory's and whose window is
    /// a guid — see [`crate::game::npc::bank`].
    pub bank: &'a crate::game::npc::bank::BankWindow,
    /// …and the trade window, which is another player's rather than an
    /// NPC's — see [`crate::game::session::trade`].
    pub trade: &'a crate::game::session::trade::TradeWindow,
    /// [`crate::game::api::get_time`]'s value for this frame.
    pub now: f64,
    /// The **world's** hour and minute, for `GetGameTime` — see
    /// [`UnitAnswers::game_time`]. Read off [`crate::render::sky::WorldClock`]
    /// rather than off the session, so a hand-set hour reaches the minimap's
    /// clock as well as the sky.
    pub game_clock: (u32, u32),
    /// **Where the hearthstone points, in words** — `GetBindLocation()`'s
    /// answer, and the `$z` in the stone's own sentence, which the reference
    /// resolves the same way in both places.
    ///
    /// Resolved once here rather than at each read: it is an `AreaTable` lookup
    /// and a fallback, and three different reads want the same string. Empty
    /// only with no archives open, since the fallback is `HOME_INN` — "your
    /// inn", the reference's own answer for a bind point it does not have.
    pub home: String,
    /// …and the innkeeper waiting on an answer, for `CheckBinderDist` — see
    /// [`crate::game::npc::binder::HomeBind`].
    pub binder: &'a crate::game::npc::binder::HomeBind,
    /// …and the pet trainer, for `CheckPetUntrainerDist` — see
    /// [`crate::game::npc::untrainer::Untrainer`].
    pub untrainer: &'a crate::game::npc::untrainer::Untrainer,
    /// …and the stance bar, which is a client read of `Spell.dbc` with no
    /// packet behind it — see [`crate::game::combat::shapeshift`].
    pub shapeshift: &'a crate::game::combat::shapeshift::ShapeshiftBar,
    /// Whether a swing is in progress, for `IsCurrentAction` — the one piece of
    /// state that lives on the session rather than in a resource.
    pub attacking: bool,
    /// Where the character is and which parchment is showing — see
    /// [`crate::game::place::worldmap`].
    pub place: &'a crate::game::place::worldmap::WorldMapState,
    /// …and the one mark the server can put on that parchment — see
    /// [`crate::game::place::worldmap::MapLandmarks`].
    pub landmarks: &'a crate::game::place::worldmap::MapLandmarks,
    /// …and who is with them — see [`crate::game::session::party`]. The one unit subject
    /// whose members may not be in the world at all.
    pub party: &'a crate::game::session::party::Party,
    /// …and what the character may hold — see
    /// [`crate::game::character::proficiency`].
    pub proficiency: &'a crate::game::character::proficiency::Proficiencies,
    /// **What the character is carrying** — the bags, the worn slots, the
    /// money, and the item templates for all of it. See
    /// [`crate::game::character::items`].
    pub inventory: &'a crate::game::character::items::Inventory,
    /// …and what is on the **pointer**, which is what makes a left click on a
    /// bag square a pick-up or a put-down. See [`crate::game::combat::cursor`].
    pub cursor: &'a crate::game::combat::cursor::Cursor,
    /// …and **the archives' own tables**, which by now answer four different
    /// questions here: the two that turn a place into words and rectangles, the
    /// spell catalogue behind every tooltip, and `FactionTemplate.dbc` behind
    /// friend-or-foe. `None` before the archives are open, which answers each of
    /// those with its own "nothing" rather than with a plausible constant.
    pub tables: Option<std::sync::Arc<vale_assets::tables::dbc::DisplayTables>>,
    /// The map id and position the map reads are against — the local player's,
    /// resolved once by [`LuaWorld::live`] rather than per call.
    pub here: Option<(u32, f32, f32)>,
    /// …and which way they are looking, for the arrow that says so. See
    /// [`Answers::player_facing`].
    pub facing: f32,
    /// **The screens before the world**: the handshake this client is holding
    /// open, if it is holding one, and what the account box remembers.
    ///
    /// `None` at the login screen and `None` in the world — the character list
    /// exists only between `CMSG_CHAR_ENUM` and `CMSG_PLAYER_LOGIN` — and every
    /// glue read answers its own "nothing" for that. See [`super::panels::glue`].
    pub selection: Option<&'a crate::world::session::Handshake>,
    /// …and the client-side half beside it: the remembered account name and
    /// whether the socket is still up. See [`crate::game::session::glue::GlueState`].
    pub glue: &'a crate::game::session::glue::GlueState,
    /// **The object manager, for the one read that needs a name the server owns
    /// and the ECS does not mirror**: a spell's reagents are item *entries* and
    /// `Item.dbc` is not in the archives, so "Rune of Teleportation" lives in
    /// the templates `CMSG_ITEM_QUERY_SINGLE` fills. See
    /// [`Live::reagent_name`], which is the only thing that touches it — and
    /// which is a hover, not a frame.
    pub world: Option<std::sync::Arc<std::sync::Mutex<vale_protocol::state::objects::ObjectManager>>>,
}

/// **Everything the interface may ask about, as one system parameter.**
///
/// Seven systems in this directory enter Lua and every one of them needs the
/// same set — so before this existed, adding a *read* meant a new `Res<…>` on
/// each of them and a new field at each `Live { … }`. That is the hub shape
/// worth avoiding, and it had already been paid twice; the
/// spellbook would have been the third. Now a read is one field here and one
/// method on [`Answers`].
///
/// It is also what keeps two of those systems under `SystemParam`'s sixteen.
#[derive(SystemParam)]
pub struct LuaWorld<'w, 's> {
    pub units: Units<'w, 's>,
    pub bar: Res<'w, ActionBar>,
    pub cooldowns: Res<'w, Cooldowns>,
    pub casting: Res<'w, Casting>,
    /// Which ranged attack is repeating, for `IsAutoRepeatAction` — see
    /// [`crate::game::combat::action::AutoRepeat`].
    pub auto_repeat: Res<'w, crate::game::combat::action::AutoRepeat>,
    pub targeting: Res<'w, crate::game::combat::action::SpellTargeting>,
    pub book: Res<'w, Spellbook>,
    pub pet_bar: Res<'w, crate::game::combat::pet::PetBar>,
    pub talents: Res<'w, crate::game::character::talents::Talents>,
    pub auras: Res<'w, crate::game::combat::auras::Auras>,
    /// Being dead — see [`crate::game::character::death`].
    pub dying: Res<'w, crate::game::character::death::Dying>,
    pub page: Res<'w, crate::game::npc::pagetext::OpenBook>,
    pub summon: Res<'w, crate::game::session::summon::Summon>,
    pub session: Res<'w, crate::world::session::Session>,
    pub strings: Res<'w, crate::game::messages::UiStrings>,
    pub time: Res<'w, Time>,
    /// Where the character is and which parchment the map panel is on — see
    /// [`crate::game::place::worldmap`].
    pub place: Res<'w, crate::game::place::worldmap::WorldMapState>,
    /// …and the flag a guard's directions put on it.
    pub landmarks: Res<'w, crate::game::place::worldmap::MapLandmarks>,
    /// …and who is with them — see [`crate::game::session::party`].
    pub party: Res<'w, crate::game::session::party::Party>,
    /// …and what they may *hold*, which is the only thing that can colour a
    /// square red — see [`crate::game::character::proficiency`].
    pub proficiency: Res<'w, crate::game::character::proficiency::Proficiencies>,
    /// …and what they are carrying — see [`crate::game::character::items`].
    pub inventory: Res<'w, crate::game::character::items::Inventory>,
    /// …and what is on the pointer — see [`crate::game::combat::cursor`].
    pub cursor: Res<'w, crate::game::combat::cursor::Cursor>,
    /// …and what is on the *body* — see [`crate::game::npc::loot`].
    pub loot: Res<'w, crate::game::npc::loot::LootWindow>,
    /// …and the rolls open on it — see [`crate::game::npc::lootroll`].
    pub rolls: Res<'w, crate::game::npc::lootroll::LootRolls>,
    /// …and the log and the conversation — see [`crate::game::npc::quest`].
    pub quests: Res<'w, crate::game::npc::quest::Quests>,
    /// …and the two windows a right-click on an NPC opens — see
    /// [`crate::game::npc::gossip`] and [`crate::game::npc::merchant`].
    pub gossip: Res<'w, crate::game::npc::gossip::GossipWindow>,
    pub merchant: Res<'w, crate::game::npc::merchant::MerchantWindow>,
    /// …and the mailbox — see [`crate::game::npc::mail`].
    pub mail: Res<'w, crate::game::npc::mail::Mailbox>,
    /// …and the trainer — see [`crate::game::npc::trainer`].
    pub trainer: Res<'w, crate::game::npc::trainer::TrainerWindow>,
    /// …and the two profession windows — see
    /// [`crate::game::character::tradeskill`].
    pub tradeskill: Res<'w, crate::game::character::tradeskill::TradeSkillWindow>,
    pub craft: Res<'w, crate::game::character::tradeskill::CraftWindow>,
    /// …and the flight map — see [`crate::game::npc::taxi`].
    pub taxi: Res<'w, crate::game::npc::taxi::TaxiWindow>,
    /// …and the stable — see [`crate::game::npc::stable`].
    pub stable: Res<'w, crate::game::npc::stable::StableWindow>,
    /// …and the bank — see [`crate::game::npc::bank`].
    pub bank: Res<'w, crate::game::npc::bank::BankWindow>,
    pub trade: Res<'w, crate::game::session::trade::TradeWindow>,
    /// …and the archives, for the two tables that turn a place into words.
    pub assets: Res<'w, crate::assets::GameAssets>,
    /// The local player's position, for `GetPlayerMapPosition` — the one map
    /// read that wants a world position rather than an id.
    pub status: Res<'w, crate::world::session::WorldStatus>,
    /// The glue's own client-side half — see [`crate::game::session::glue`].
    pub glue: Res<'w, crate::game::session::glue::GlueState>,
    /// **Where the hearthstone points** — see [`crate::game::npc::binder`].
    pub home: Res<'w, crate::game::npc::binder::HomeBind>,
    /// …and the pet trainer's pending question — see
    /// [`crate::game::npc::untrainer`].
    pub untrainer: Res<'w, crate::game::npc::untrainer::Untrainer>,
    /// …and the stance bar — see [`crate::game::combat::shapeshift`].
    pub shapeshift: Res<'w, crate::game::combat::shapeshift::ShapeshiftBar>,
    /// **The world's hour**, for `GetGameTime` — see
    /// [`crate::render::sky::WorldClock`]. Taken from the sky's clock rather
    /// than from the session so that the minimap's clock and the light on the
    /// ground are the same number, hand-set hours included.
    pub clock: Res<'w, crate::render::sky::WorldClock>,
}

impl LuaWorld<'_, '_> {
    /// **Every resource this bundle needs, in a test app.**
    ///
    /// Beside the bundle for the same reason the bundle exists: five test
    /// harnesses in this directory each stand up a minimal `App` to run one
    /// system, and without this every field added here is five more
    /// `init_resource` lines that fail to compile in five files. The `Query`
    /// needs nothing — an empty world answers every token as absent, which is
    /// what those tests want.
    #[cfg(test)]
    pub(crate) fn init(app: &mut bevy::app::App) -> &mut bevy::app::App {
        app.init_resource::<Time>()
            .init_resource::<ActionBar>()
            .init_resource::<Cooldowns>()
            .init_resource::<Casting>()
            .init_resource::<crate::game::combat::action::AutoRepeat>()
            .init_resource::<crate::game::combat::action::SpellTargeting>()
            .init_resource::<Spellbook>()
            .init_resource::<crate::game::combat::pet::PetBar>()
            .init_resource::<crate::game::character::talents::Talents>()
            .init_resource::<crate::game::combat::auras::Auras>()
            .init_resource::<crate::game::character::death::Dying>()
            .init_resource::<crate::world::session::Session>()
            .init_resource::<crate::game::messages::UiStrings>()
            .init_resource::<crate::game::place::worldmap::WorldMapState>()
            .init_resource::<crate::game::place::worldmap::MapLandmarks>()
            .init_resource::<crate::game::session::party::Party>()
            .init_resource::<crate::game::character::items::Inventory>()
            .init_resource::<crate::game::combat::cursor::Cursor>()
            .init_resource::<crate::game::npc::loot::LootWindow>()
            .init_resource::<crate::game::npc::pagetext::OpenBook>()
            .init_resource::<crate::game::session::summon::Summon>()
            .init_resource::<crate::game::npc::lootroll::LootRolls>()
            .init_resource::<crate::game::npc::quest::Quests>()
            .init_resource::<crate::game::npc::gossip::GossipWindow>()
            .init_resource::<crate::game::npc::gossip::NpcUnit>()
            .init_resource::<crate::game::character::reputation::PlayerStanding>()
            .init_resource::<crate::game::npc::merchant::MerchantWindow>()
            .init_resource::<crate::game::npc::mail::Mailbox>()
            .init_resource::<crate::game::npc::trainer::TrainerWindow>()
            .init_resource::<crate::game::character::tradeskill::TradeSkillWindow>()
            .init_resource::<crate::game::character::tradeskill::CraftWindow>()
            .init_resource::<crate::game::npc::taxi::TaxiWindow>()
            .init_resource::<crate::game::npc::stable::StableWindow>()
            .init_resource::<crate::game::npc::bank::BankWindow>()
            .init_resource::<crate::game::session::trade::TradeWindow>()
            .init_resource::<crate::world::session::WorldStatus>()
            .init_resource::<crate::game::session::glue::GlueState>()
            .init_resource::<crate::render::sky::WorldClock>()
            .init_resource::<crate::game::npc::binder::HomeBind>()
            .init_resource::<crate::game::npc::untrainer::Untrainer>()
            .init_resource::<crate::game::combat::shapeshift::ShapeshiftBar>()
            // …and what the character may hold, which the trade squares
            // read — see [`crate::game::character::proficiency`]. Named
            // here for the same reason the two below are: the headless
            // probes build this app without `GamePlugins`.
            .init_resource::<crate::game::character::proficiency::Proficiencies>()
            .insert_resource(crate::assets::GameAssets::new(String::new()))
            .init_resource::<crate::game::combat::target::Selection>()
            .init_resource::<crate::game::combat::target::Hovered>()
            // …and its twin, which the world tooltip reads beside it — see
            // `crate::game::npc::object::HoveredObject`. Both are named here
            // rather than left to `GamePlugins` because the headless probes
            // build this app without it.
            .init_resource::<crate::game::npc::object::HoveredObject>()
    }

    /// The borrow to lend Lua for the length of one call.
    ///
    /// Cheap enough to take per call and correct to take once per frame: it
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

/// **What `GetBindLocation()` says**, resolved from the area id the bind point
/// carries.
///
/// The reference reads `AreaTable`'s own `AreaName` for the row — not the zone
/// it belongs to — so an inn's sub-area gives "Lion's Pride Inn" and a bind in
/// open country gives the zone. `GetBindLocation` and the spell text's `$z`
/// arm resolve it the same way, which is why one string answers both.
///
/// **The fallback is `HOME_INN`** — "your inn" — which is the reference's own:
/// both functions jump to it when the area id is unset or out of the table's
/// range, which for a client is every frame before the login burst lands.
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

    /// The map tables, or `None` before the archives are open — one door, since
    /// six of the reads above want the same two-step.
    pub(super) fn world_map(&self) -> Option<&vale_assets::tables::worldmap::WorldMap> {
        self.tables.as_ref()?.world_map()
    }

    /// **Where a party member is standing**, as `(map, x, y)` — `None` for a
    /// member no stats packet has arrived for, and for one whose zone
    /// `AreaTable` cannot place on a map.
    ///
    /// The position is `SMSG_PARTY_MEMBER_STATS`' two `int16`s, which is the
    /// only thing on the wire that carries one. The **map** is not in that
    /// packet at all: it is derived from the zone the same packet carries, which
    /// is what keeps a member in Kalimdor off an Eastern Kingdoms parchment.
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

    /// **What a reagent is called**, and — if nothing knows yet — a request that
    /// something find out.
    ///
    /// Both halves under one lock, which is what makes this safe to call from a
    /// hover: a miss *queues* the entry ([`ObjectManager::want_item`]) and never
    /// touches the socket, so the query goes out on the session's own interval
    /// with the equipment queries and a hundred hovers cost one packet.
    ///
    /// The first hover on a cold cache therefore shows the plate without its
    /// reagent line and the next one shows it, which is exactly what the retail
    /// client does with an empty item cache.
    pub(super) fn reagent_name(&self, entry: u32) -> Option<String> {
        Some(self.session_template(entry)?.name)
    }

    /// **A template out of the session-wide cache**, queued for query on a
    /// miss — same contract as [`Self::reagent_name`]: the miss never touches
    /// the socket, so a hundred reads cost one packet on the session's own
    /// query interval.
    ///
    /// This is the read for a population the character does **not** carry —
    /// a vendor's shelf, a corpse's rows. [`super::game::items::Inventory`]'s
    /// template map is deliberately the *carried* subset of this cache, so
    /// resolving those rows through it answers blank until the item is bought
    /// or looted, which was precisely the reported bug.
    pub(super) fn session_template(&self, entry: u32) -> Option<vale_protocol::state::query::ItemInfo> {
        let world = self.world.as_ref()?;
        let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(item) = world.items.get(&entry) {
            return Some(item.clone());
        }
        world.want_item(entry);
        None
    }

    /// **What an objective is aimed at, in words** — queued for query on a
    /// miss, the same contract [`Self::session_template`] has.
    ///
    /// The name is not on the wire anywhere near the quest: `ReqCreatureOrGOId`
    /// is an id and `CMSG_CREATURE_QUERY` / `CMSG_GAMEOBJECT_QUERY` are the only
    /// things that answer it. A quest log opened in a city is almost always
    /// asking about a creature that is nowhere in view, which is why
    /// `ObjectManager::want_creature` exists at all — the on-sight walk cannot
    /// reach one.
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

    /// **One line of the quest log's objectives, in the client's own words.**
    ///
    /// The sentence is not composed here in any sense that matters: it is a
    /// `GlobalStrings.lua` key per objective kind, and the keys are the ones
    /// `GetQuestLogLeaderBoard` itself pushes, and they are the whole of the
    /// rule:
    ///
    /// ```text
    /// QUEST_MONSTERS_KILLED   "%s slain: %d/%d"    with "monster"
    /// QUEST_OBJECTS_FOUND     "%s: %d/%d"          with "object"
    /// QUEST_ITEMS_NEEDED      "%s: %d/%d"          with "item"
    /// QUEST_FACTION_NEEDED    "%s:  %s / %s"       with "reputation"
    /// (none)                  the wording verbatim with "event"
    /// ```
    ///
    /// **The monster line is the one that is not `%s: %d/%d`**, and it is the
    /// commonest kind in the game: "Kobold Vermin slain: 3/8" rather than
    /// "Kobold Vermin: 3/8". A client that used one shape for all of them reads
    /// as plausible and is wrong on most quests in Elwynn, which is exactly the
    /// class of fault this project keeps a rule about.
    ///
    /// Read out of the archive's own `GlobalStrings.lua` rather than written
    /// here, so a key the file does not carry shows as nothing — the client's
    /// own behaviour — instead of as a sentence invented in this repo.
    pub(super) fn leader_board_line(&self, kind: &str, name: &str, have: u32, want: u32) -> String {
        let key = match kind {
            "monster" => "QUEST_MONSTERS_KILLED",
            "object" => "QUEST_OBJECTS_FOUND",
            _ => "QUEST_ITEMS_NEEDED",
        };
        let format = self.strings.and_then(|s| s.get(key));
        match format {
            // `%s` then `%d` then `%d`, in that order in all three — so the
            // substitution is positional and does not need a printf.
            Some(format) => format
                .replacen("%s", name, 1)
                .replacen("%d", &have.to_string(), 1)
                .replacen("%d", &want.to_string(), 1),
            // No `GlobalStrings.lua` at all is the headless case; the counter
            // is still the useful half.
            None => format!("{name}: {have}/{want}"),
        }
    }

    /// The context every spell plate is composed against — see
    /// [`crate::game::api::TipContext`].
    ///
    /// The level is the *player's*, because that is what an effect's value
    /// scales on and the tooltip is always answering "what would this do if I
    /// cast it". `unit_level` answers -1 for a player who has not arrived yet,
    /// which floors to 1 — the level every spell's own base is stated at.
    pub(super) fn tip_level(&self) -> u32 {
        Self::id("player")
            .map(|id| self.units.level(id))
            .unwrap_or(1)
            .max(1) as u32
    }

    /// **Only `player` has bags**, and this is where that is said once.
    ///
    /// No packet carries another unit's inventory or another unit's durability
    /// — an inspect is a family this client does not read — so every worn-slot
    /// read answers the absent value for any other token rather than the local
    /// player's own gear, which would be a plausible wrong answer of exactly
    /// the kind this project keeps paying for.
    pub(super) fn is_player(token: &str) -> bool {
        Self::id(token) == Some(UnitId::Player)
    }

    /// One slot's drawable state, from whatever it holds.
    ///
    /// The quality is **-1** rather than 0 when the template has not arrived —
    /// see [`super::panels::container`], where the reason is.
    pub(super) fn slot_contents(
        &self,
        item: &vale_protocol::play::items::ItemSlot,
        place: crate::game::combat::cursor::Place,
    ) -> super::panels::container::SlotContents {
        let template = self.inventory.template(item.entry);
        super::panels::container::SlotContents {
            texture: template
                .and_then(|t| self.tables.as_ref()?.item_icon(t.display_id)),
            count: item.count,
            quality: template.map_or(-1, |t| t.quality as i32),
            readable: template.is_some_and(vale_protocol::state::query::ItemInfo::is_readable),
            broken: item.broken(),
            // **The place rather than the item.** Two stacks of the same entry
            // are two squares, and locking by entry would desaturate both.
            locked: self.cursor.locks(place),
        }
    }


    /// One reward or requirement line, named through the item cache.
    ///
    /// **The display id comes free and the name does not.** A quest packet
    /// carries `ItemPrototype::DisplayInfoID` for every item it names, so the
    /// icon needs no round trip - unlike a loot row's. The name and the quality
    /// still need the template.
    /// **What the ammo slot draws** — the loaded entry's icon and quality,
    /// and `GetItemCount` of it for the number. `None` with nothing loaded,
    /// which is what puts the slot's own art back. Never locked: the cursor
    /// holds bag squares, and loading is not a move.
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

    pub(super) fn quest_line(&self, entry: u32, count: u32, display_id: u32) -> super::panels::quest::RewardLine {
        // **The session-wide cache and not the carried one** — a reward is by
        // construction something the character does not own yet, which is the
        // same argument a vendor's shelf and a corpse's rows already make. See
        // [`Self::session_template`].
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

    /// **What a reward spell is to a panel** — see [`super::panels::quest::RewardSpell`],
    /// which carries the reason this is three answers rather than an id.
    ///
    /// `0` is "no reward spell" on the wire and is the case the whole thing
    /// exists for: it must reach Lua as **nil**, not as a number.
    pub(super) fn reward_spell(&self, spell_id: u32) -> Option<super::panels::quest::RewardSpell> {
        let info = self.tables.as_ref()?.spellbook()?.info(spell_id)?;
        Some(super::panels::quest::RewardSpell {
            // The client pushes nil for an icon row it cannot resolve and still
            // pushes the name; the panel then hides the block, since it gates on
            // the first answer. Empty is that nil.
            texture: Some(info.icon.clone()).filter(|icon| !icon.is_empty()),
            name: info.name.clone(),
            tradeskill: info.attributes & vale_assets::tables::spellbook::spell_attributes::TRADESPELL
                != 0,
        })
    }

    /// **Who the `$` variables in a quest's or an NPC's text are about** — see
    /// [`crate::game::messages::substitute`]. Always the local player: the
    /// server writes the column for whoever is reading it.
    pub(super) fn speaker(&self) -> crate::game::messages::Speaker<'_> {
        let unit = self.units.get(UnitId::Player);
        crate::game::messages::Speaker {
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

    /// …and its link, which needs the name and so needs the template.
    pub(super) fn slot_link(&self, item: &vale_protocol::play::items::ItemSlot) -> Option<String> {
        let template = self.inventory.template(item.entry)?;
        Some(super::panels::container::item_link(
            template.entry,
            template.quality,
            &template.name,
        ))
    }

    /// **An item's cooldown is its `ON_USE` spell's**, read through the same
    /// clocks and the same arithmetic as an action button's.
    ///
    /// There is no per-*item* timer anywhere in this client and there should not
    /// be: `SMSG_SPELL_COOLDOWN` names a spell, and it is what the server sends
    /// when a potion is drunk, so the record is already there under that id by
    /// the time the bag frame asks. What was missing was only the *read* — this
    /// answered `0, 0, 0` and every swirl in the bags stayed empty.
    ///
    /// The idle triple for everything with nothing to say: no item, no template
    /// yet, no `ON_USE` block, or a spell the catalog does not carry.
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
            Some(crate::game::api::cooldown_of(self.cooldowns, &info, self.now))
        };
        resolved().unwrap_or(IDLE_COOLDOWN)
    }

    /// The context an item plate is composed against — the same one a spell's
    /// is, since an item's "Use:" line is a spell's own sentence.
    pub(super) fn item_context(&self) -> crate::game::api::TipContext<'_> {
        // **The race and class are the item plate's alone.** A spell plate has
        // no requirement lines, which is why the two other `TipContext`s below
        // leave them at zero rather than paying for the lookup.
        let (race, class) = Self::id("player")
            .and_then(|id| self.units.race_class_ids(id))
            .unwrap_or((0, 0));
        crate::game::api::TipContext {
            level: self.tip_level(),
            race,
            class,
            catalog: self.tables.as_ref().and_then(|t| t.spellbook()),
            // **Not `reagent_name`**: an item plate never names another item, so
            // there is nothing here to queue a query for. Passing the queueing
            // closure would put a `want_item` behind every hover of every bag
            // slot for a lookup that never happens.
            item_names: &|_| None,
            home: Some(self.home.clone()),
        }
    }

    /// …and the plate itself, from a template and the stack that is in hand.
    ///
    /// **Two caches, in that order, and the second one is the whole of why a
    /// vendor's shelf had no tooltip.** [`crate::game::character::items::Inventory`]'s map
    /// is the *carried* subset — built from `carried.entries()` and nothing else
    /// — so an entry the character does not own misses it by construction:
    /// every item on a merchant's shelf, in a corpse, on a quest page and in a
    /// chat link. Each of those populations already draws its *row* through
    /// [`Self::session_template`] (the name, the icon, the link); only the plate
    /// was still asking the narrow map, so hovering one composed nothing and
    /// `item_plate` hid the tooltip — indistinguishable from hovering an empty
    /// square, which is [`self::stubs`]' own standing lesson arriving a third
    /// time.
    ///
    /// The carried map stays *first* because it is the one that costs no lock:
    /// a bag hover is the commonest plate in the game, and the session cache
    /// takes the world's mutex to answer. A miss on both queues the query and
    /// answers `None`, so the first hover on a cold cache shows nothing and the
    /// next one shows the plate — the real client's own behaviour with an empty
    /// `ItemCache.wdb`.
    pub(super) fn tip_from(
        &self,
        entry: u32,
        carried: Option<&vale_protocol::play::items::ItemSlot>,
    ) -> Option<crate::game::api::ItemTip> {
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
        Some(crate::game::api::item_tip(
            template,
            carried,
            self.tables.as_deref(),
            &self.item_context(),
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
        crate::game::npc::binder::binder_in_range(self.binder, self.units)
    }

    fn untrainer_in_range(&self) -> bool {
        crate::game::npc::untrainer::untrainer_in_range(self.untrainer, self.units)
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
        let Some((_, here)) = self.units.placed(crate::game::api::UnitId::Player) else {
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
        // **`level_shown`, not `level`** — a unit ten or more levels above a
        // hostile player, and any worldboss, report `-1`, which is what
        // `TargetFrame_CheckLevel`'s `targetLevel > 0` draws as the skull. The
        // rule is on
        // [`crate::game::api::Units::level_shown`].
        Self::id(token).map_or(-1, |id| self.units.level_shown(self.tables.as_deref(), id))
    }

    fn unit_sex(&self, token: &str) -> u32 {
        // **2 rather than 0 for an unparseable token**, which is the client's
        // own answer — see `Units::sex`.
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
        // Under a second is no sickness at all.
        if ms < 1000 {
            return None;
        }
        // The `"GENERIC"` family: `%s_MIN` for a whole number of
        // minutes, `%s_SEC` under one, each with its `_P1` plural. The
        // sickness is whole minutes by construction (60000 a level).
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

    fn unit_tooltip(&self, token: &str) -> Option<crate::game::api::UnitTip> {
        let id = Self::id(token)?;
        let mut tip = self.units.unit_tip(self.tables.as_deref(), id)?;
        // **The zone line, resolved and filtered here** — this is the only side
        // holding `AreaTable` and the only side that knows what zone the
        // character is standing in. A member in the same zone gets no line; see
        // [`crate::game::api::UnitTip::zone`].
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
        // **No tables, no opinion.** Before the archives are open every unit
        // would otherwise read Neutral — `Factions::template_rank`'s own answer
        // for a table it does not have — and a plausible reaction is exactly the
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

    fn unit_player_controlled(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.player_controlled(id))
    }
}

/// One held aura, flattened for the interface — the clock resolved against this
/// frame's `now`, so a caller never has to know which base it was recorded in.
pub(super) fn aura_info(aura: &crate::game::combat::auras::Aura, now: f64) -> super::panels::auras::AuraInfo {
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

/// **The reads the directory does not merely call — it *stores*.**
///
/// Every other read in this file is a `scope.create_function`: it lives for the
/// length of one call into Lua, which is what lets it borrow the world. A stored
/// reference to one is dead the moment the scope closes, and calling it raises
/// "a destructed callback or destructed userdata method was called".
///
/// `StaticPopup.lua` does exactly that, three times:
///
/// ```lua
/// StaticPopupDialogs["RECOVER_CORPSE"] = { StartDelay = GetCorpseRecoveryDelay, … }
/// ```
///
/// — the *function value*, captured into a table at load and called much later
/// from `StaticPopup_Show`. So the five death reads are ordinary persistent
/// closures over this cell instead, and [`install`] refreshes it at the top of
/// every call. `--audit --events` is what found it: `CORPSE_IN_RANGE` died on
/// `StartDelay()` at `StaticPopup.lua:1685` with every other check passing.
///
/// The cost of the indirection is that these five answer with the values as of
/// the *start* of the call rather than during it. Both clocks are per-frame
/// numbers and the offer cannot change mid-chunk, so the two are the same
/// answer; anything that could change under a chunk must stay a scoped read.
pub(super) type Held = std::rc::Rc<std::cell::RefCell<HeldReads>>;

/// What [`Held`] holds. Flat, because the five have nothing to do with each
/// other except the mechanism.
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
    // …and the verb queue [`install`] needs for the six drag verbs. Returned
    // rather than made inside `install`, because it has to outlive the scope.
    (held, std::rc::Rc::new(std::cell::RefCell::new(Vec::new())))
}

/// Register the five, once, against a cell [`install`] keeps current.
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

/// Every read registered below, for the check that counts the gap — see
/// [`self::verbs::REGISTERED`], which is the same list for the write side.
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

/// **`(start, duration, enable)` for something that is not on a cooldown.**
///
/// `enable` is *true* here, which reads backwards and is the game's own: the
/// third value says whether the button's swirl is allowed to run at all — a
/// passive or a disabled action answers 0 — and a ready item is enabled with
/// nothing to sweep. See `CooldownFrame_SetTimer`, which draws nothing when the
/// duration is zero whatever `enable` says.
pub(super) const IDLE_COOLDOWN: (f64, f64, bool) = (0.0, 0.0, true);

/// `UnitClassification`'s five words, from the creature template's rank.
///
/// **Lower case and these exact spellings**, because `TargetFrame.lua` compares
/// against literals: `"worldboss"`, `"rareelite"`, `"elite"` and `"rare"` choose
/// between the three target-frame borders and anything else takes the plain one.
/// That is the whole of what the directory does with the answer, and it is why a
/// stub answering the constant `"normal"` drew the ordinary border round every
/// elite in the game.
pub fn classification_word(rank: u32) -> &'static str {
    match rank {
        1 => "elite",
        2 => "rareelite",
        3 => "worldboss",
        4 => "rare",
        _ => "normal",
    }
}

/// The game's own boolean: `1` or `nil`, never `true`/`false`. See the module
/// comment on why this is not pedantry.
///
/// One home for it, used by [`super::widgets::frames`] too — it is a *fact about the
/// game's C API* rather than a helper, and a second copy is a second place to get
/// it wrong.
pub(super) fn one_or_nil(yes: bool) -> mlua::Value {
    if yes {
        mlua::Value::Integer(1)
    } else {
        mlua::Value::Nil
    }
}

/// **…and the same fact read from the other side: how a C function reads a
/// boolean *argument*.** It is emphatically not Lua's own truthiness.
///
/// This matters because the directory writes booleans in five different ways and
/// expects all five to work:
///
/// ```lua
/// button:SetChecked(1);        button:SetChecked(0);        -- ActionButton.lua
/// this:SetChecked("true");     this:SetChecked("false");    -- SpellBookFrame.lua
/// skillLineTab:SetChecked(nil);
/// ```
///
/// Under Lua's own rule `0` and `"false"` are both **true**, so a host that used
/// `lua_toboolean` draws every action button and every spell in the book with its
/// `<CheckedTexture>` on — the `CheckButtonHilight` sheet, additively, over every
/// icon in the game, for ever. That is exactly what this client did, and the
/// comment on `SetChecked` used to explain it away as a bug in the file that
/// happened not to matter. It is not a bug in the file: the client's own
/// coercion reads `0` and `"false"` as false, and the directory is written
/// against it.
///
/// The rule is one `switch` on `lua_type`, with the string case decided by its
/// first character:
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
/// `default` is the value the call site pushes for the cases it has no opinion
/// on; every widget setter in the client passes **true**, which is why
/// `SetChecked({})` checks a button.
///
/// One stated deviation, and it is in a corner nothing reaches: the client
/// keeps only the low *byte* of the truncated number, so `SetChecked(256)` is
/// false there and true here. Nothing in the directory or in
/// any addon passes a boolean as 256.
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

/// Register the reads into a scope, for the length of one call into Lua.
///
/// Called by [`super::host::LuaHost::run`] and nowhere else, so there is no
/// window in which a chunk can run without them — see the module comment.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
    held: &Held,
    // **The verb queue, for the six drag verbs alone.** Every other write in
    // this client is registered once by [`self::verbs::register`] and never
    // needs the world; `PutItemInBag` answers *and* records, so it has to be a
    // scoped function with the queue in reach. See [`super::panels::container`].
    queue: &'env self::verbs::Queue,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    // A unit token argument. Optional because `UnitHealth()` with no argument is
    // something an addon really writes, and the game answers it rather than
    // raising — so a missing token is an absent unit, not an error.
    macro_rules! unit {
        ($name:expr, |$token:ident| $body:expr) => {{
            let f = scope.create_function(move |_, token: Option<String>| {
                let $token: &str = token.as_deref().unwrap_or("");
                Ok($body)
            })?;
            globals.set($name, f)?;
        }};
    }
    // An action-slot argument, one-based as every action id in the game is.
    macro_rules! slot {
        ($name:expr, |$slot:ident| $body:expr) => {{
            let f = scope.create_function(move |_, slot: Option<u8>| {
                // **Slot 0 is not a slot**, so an absent argument reads as one
                // and every accessor answers "nothing there" rather than
                // pointing at the first button.
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

    // **`GetGameTime()` — the world's hour and minute**, which is a different
    // clock from `GetTime`'s: one counts seconds since this process started, the
    // other is what the sky is lit by. See [`UnitAnswers::game_time`] for why a
    // constant here stops `GameTimeFrame` rather than merely misreporting it.
    globals.set(
        "GetGameTime",
        scope.create_function(move |_, ()| Ok(answers.game_time()))?,
    )?;

    // **The hearthstone's two**, which have no panel of their own: one is read
    // by three `StaticPopupDialogs` entries and the other by one `OnUpdate`.
    // See [`crate::game::npc::binder`].
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

    // **`GetBonusBarOffset()` — which of the four bonus bars is on the screen.**
    // No arguments and no unit: it is always about the player, whose form
    // `SpellShapeshiftForm.dbc` turns into a bar. Zero is the ordinary paged
    // bar, which is what `BonusActionBar_OnEvent` hides the frame on.
    // **The rested pool, and `nil` when there is none** — which is
    // `ExhaustionTick_Update`'s own test (`if ( not exhaustionThreshold )`), so
    // a zero here parks the blue tick at the left edge of the bar instead of
    // hiding it. Always the player's: no unit argument, as the game takes none.
    globals.set(
        "GetXPExhaustion",
        scope.create_function(move |_, ()| Ok(answers.rested_experience()))?,
    )?;

    globals.set(
        "GetBonusBarOffset",
        scope.create_function(move |_, ()| Ok(answers.bonus_bar_offset()))?,
    )?;

    // **`GetActionBarToggles()` — the four extra action bars, and it is the one
    // read in this file that answers *four* values.**
    //
    // `UIParent.lua`'s `PLAYER_ENTERING_WORLD` arm is the only caller:
    //
    // ```lua
    // SHOW_MULTI_ACTIONBAR_1, ..._2, ..._3, ..._4 = GetActionBarToggles();
    // MultiActionBar_Update();
    // ```
    //
    // — so a client that answers nothing has four frames that exist, lay out
    // correctly and can never appear, which is exactly what this one had. Each
    // value is `1` or `nil` rather than a boolean, because `MultiBar1_IsVisible`
    // hands the same value back to a checkbox's `func` and `OptionsFrame` tests
    // it against `1`.
    //
    // The fifth toggle the *write* takes is deliberately not among them:
    // the client's read answers four toggles and stops at four, so
    // "Always Show ActionBars" is never read back from the wire. See
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

    // **`GetQuestGreenRange()` — one missing name, and it took the whole target
    // frame with it.** `TargetFrame_CheckLevel` colours the level number through
    // `GetDifficultyColor`, whose last question this is; with the name nil the
    // call raised, `TargetFrame_Update` died at its *second* line, and everything
    // after it never ran — the name plate kept its untinted art (which is the
    // "no friendly/hostile colour" report), the classification, the dead check
    // and `TargetPortrait:SetAlpha` never happened, and `TargetDebuffButton_Update`
    // never got to **hide** the twenty-one aura buttons the markup ships visible
    // (which is the "corrupted frame with blank buff slots" report). Two reports,
    // one absent global, and it only fires when `UnitCanAttack` — so a friendly
    // target repaired the frame and a hostile one broke it again, which is
    // exactly the "until you select yourself or another mob" the report ends on.
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
    // **The same name, with the PvP rank title in front of it** — "Sergeant
    // Bram". This client models no honour rank at all (`PLAYER_FIELD_*` for
    // the ranks is unread, and the rank titles are `PVP_RANK_*` in
    // `GlobalStrings.lua`), so it is the bare name, which is exactly what the
    // real client answers for a character below rank 1 — and that is most of
    // them. A **stated deviation**, not an alias: the day the rank is read this
    // has a prefix to compose.
    //
    // It is here rather than in [`self::stubs`] because it answers the world.
    // `CharacterFrame_OnShow`'s third line is `CharacterNameText:SetText(
    // UnitPVPName("player"))`, and with the name nil that body died there —
    // which is why the character sheet's title bar read "Name", the placeholder
    // its own `<FontString text="Name">` carries.
    unit!("UnitPVPName", |t| answers.unit_name(t));
    unit!("UnitLevel", |t| answers.unit_level(t));
    // **The one read `ReputationFrame_Update` makes before its loop**, and the
    // whole of why that panel drew fifteen empty bars: a nil here raised on
    // line 44 and every `Hide()` after it was never reached.
    unit!("UnitSex", |t| answers.unit_sex(t));
    unit!("UnitHealth", |t| answers.unit_health(t));
    unit!("UnitHealthMax", |t| answers.unit_health_max(t));
    unit!("UnitMana", |t| answers.unit_mana(t));
    // **The XP bar's two numbers, and its maximum is what decides it is drawn
    // at all.** `MainMenuExpBar_Update` feeds both to `SetMinMaxValues` and
    // `TextStatusBar_UpdateTextString` **hides** a bar whose maximum is zero —
    // so while these were stubs the game's own code took the whole XP bar off
    // the screen, which is the "XP bar is missing entirely" report.
    unit!("UnitXP", |t| answers.unit_experience(t).0);
    unit!("UnitXPMax", |t| answers.unit_experience(t).1);
    unit!("UnitManaMax", |t| answers.unit_mana_max(t));
    // **The two unspent-point pools**, which is the one read the talent panel
    // and the skills panel share. Both are numbers rather than nil for a unit
    // that is not there: `TalentFrame_UpdateTalentPoints` puts the first
    // straight into a `FontString` and `SkillFrame_UpdateSkills` compares the
    // second, and neither guards. See [`super::panels::talent`].
    globals.set(
        "UnitCharacterPoints",
        scope.create_function(move |_, token: Option<String>| {
            Ok(answers.unit_character_points(token.as_deref().unwrap_or("")))
        })?,
    )?;
    // **A unit that is not there still runs on mana**, and the reason is the
    // directory rather than a preference. `PetFrame`, `TargetofTargetFrame` and
    // the four `PartyMemberFrame`s all call `UnitFrame_UpdateManaType` from
    // their own `OnLoad`, at a login where none of those units exists, and the
    // next line is `ManaBarColor[UnitPowerType(unit)].r` — so a client that
    // answered nil there would break six of its own frames on every login, and
    // 1.12's does not. `0` is `ManaBarColor`'s own first row.
    unit!("UnitPowerType", |t| answers.unit_power_type(t).unwrap_or(0));
    // **Only a party member can be disconnected.** Every unit the world knows
    // about is connected by construction, which is the reference's answer too —
    // and the roster's status byte is the only thing that says otherwise for a
    // member across the zone. It was a constant `1` until the party arrived,
    // and `UnitFrameManaBar_Update` greys the whole bar out on a nil, so the
    // shape is unchanged for everything but `party<n>`.
    unit!("UnitIsConnected", |t| one_or_nil(
        answers.unit_is_connected(t)
    ));
    unit!("UnitIsDead", |t| one_or_nil(answers.unit_is_dead(t)));
    // **The three are three different answers now**, and the deviation this
    // comment used to record is retired: a corpse run really is "ghost and not
    // dead" (`PLAYER_FLAGS_GHOST`, health 1) and the client models it, so
    // `UnitIsDeadOrGhost` is the union rather than a synonym for the first.
    // Both `FriendsFrameAddFriendButton` and `PetitionFrameRenameButton` open
    // with it, and a dead-or-ghost check is the game's own way of refusing an
    // action to a corpse — which a ghost, walking about with 1 hit point, would
    // otherwise have passed.
    unit!("UnitIsGhost", |t| one_or_nil(answers.unit_is_ghost(t)));
    unit!("UnitIsDeadOrGhost", |t| one_or_nil(
        answers.unit_is_dead(t) || answers.unit_is_ghost(t)
    ));
    unit!("UnitAffectingCombat", |t| one_or_nil(
        answers.unit_affecting_combat(t)
    ));

    // **Being dead: the two clocks and the offer** — written into [`Held`]
    // rather than registered here. See that type, which says why these five
    // cannot be scoped functions like every other read in this file.
    *held.borrow_mut() = HeldReads {
        release: answers.release_time_remaining(),
        recovery: answers.corpse_recovery_delay(),
        offerer: answers.resurrect_offerer(),
        sickness: answers.resurrect_has_sickness(),
        timer: answers.resurrect_has_timer(),
    };
    // …and the spirit healer's two, which are scoped because both are called
    // *during* a chunk rather than stored by one: `UIParent_OnEvent` reads the
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

    // **Friend or foe — five names and one reading.** All of them take two
    // tokens, so they take the same shape as `UnitIsUnit` above rather than the
    // `unit!` macro's.
    //
    // These were stubs answering nil for nine rounds, and the note left with
    // them said the cost was "a target frame draws no attackable border". It was
    // larger than that: `TargetDebuffButton_Update` branches on
    // `UnitIsFriend("player", "target")` to decide **where the aura rows go**,
    // so with it nil every target in the game took the hostile layout and a
    // friendly target's buffs were drawn 46 units — two rows — below the frame
    // they belong under. That is the standing example of this directory's own
    // first rule: a constant answer is indistinguishable from a working one.
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

    // **The rank, and it is an index rather than a measurement.**
    // `TargetFrame_CheckFaction`'s only use of it is `UnitReactionColor[reaction]`
    // — an eight-row table that is red at 1..2, orange at 3, yellow at 4 and
    // green at 5..8 — and the value is the reaction plus one, so
    // this is the rank the rule answers with `1` added and nothing else.
    // `nil` for an absent unit, which that body has its own arm for.
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
    // **`UnitCanAssist` and `UnitCanCooperate` are the friendly reading, and
    // that is a stated deviation rather than an alias.** The game's two are
    // narrower than "friendly": cooperate is may-we-party-trade-and-duel, which
    // turns on the other party being a player of a faction group we can group
    // with, and assist is may-I-heal-you. Every consumer in the directory asks
    // them about a *player* target (`UnitPopup`'s trade and invite entries,
    // `FriendsFrame`'s add-friend), and for a player the friendly reading and
    // the real one agree — a hostile-faction player is not friendly and cannot
    // be cooperated with either. What it gets wrong is a friendly *creature*,
    // which answers yes here and no in the real client; nothing in 1.12's own
    // interface asks about one.
    pair!("UnitCanAssist", |a, b| one_or_nil(
        answers.unit_rank(a, b).map(Reaction::from) == Some(Reaction::Friendly)
    ));
    pair!("UnitCanCooperate", |a, b| one_or_nil(
        answers.unit_rank(a, b).map(Reaction::from) == Some(Reaction::Friendly)
    ));
    unit!("UnitPlayerControlled", |t| one_or_nil(
        answers.unit_player_controlled(t)
    ));
    // **`UnitIsPlayer` is the same fact for this client and a different one in
    // the real game**: the game's answer is false for a *pet*, which is
    // player-controlled and is not a player, and this client has no pet — which
    // is exactly the deviation [`Units::player_controlled`] already records
    // against itself. Two names off one reading, and the day a pet exists both
    // change together.
    unit!("UnitIsPlayer", |t| one_or_nil(
        answers.unit_player_controlled(t)
    ));
    // …and the three the plate and the target frame's border are made of. All
    // three left [`self::stubs`] this round: `UnitClassification` was answering
    // the constant `"normal"`, which drew the ordinary border round every elite
    // in the game, and the other two were nil.
    unit!("UnitIsPVP", |t| one_or_nil(answers.unit_is_pvp(t)));
    // **Two returns, and the *first* is the one that matters.**
    // `PartyMemberFrame_UpdatePvPStatus` concatenates it into a texture path,
    // so an English word is required there whatever the client's locale — which
    // is why `FactionGroup.dbc` carries `internalName` beside the eight
    // localised ones at all.
    // Two returns, so this one cannot go through `unit!` — that macro answers a
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

    // **The spell cursor's two reads.** `UnitFrame_OnEnter` is
    // `if SpellIsTargeting() then SetCursor(SpellCanTargetUnit(this.unit) and
    // "CAST_CURSOR" or "CAST_ERROR_CURSOR") end` — the interface asking which
    // pointer to show over a *unit frame*, where this client's own world pick
    // (`game::target::spell_cursor_validity`) answers the same question about the
    // 3D scene. `SetCursor` itself is still a stub, so what changes on screen
    // today is the world half; the frames' answers are correct and unused.
    let f = scope.create_function(|_, ()| Ok(one_or_nil(answers.spell_is_targeting())))?;
    globals.set("SpellIsTargeting", f)?;

    // --- **`ToggleGameMenu`'s three, which answer *and* act** ---
    //
    // Escape is `TOGGLEGAMEMENU` in the game's own shipped defaults, and its
    // body is a seven-branch `elseif` chain: `StaticPopup_EscapePressed()`,
    // `OptionsFrame`, `GameMenuFrame`, `CloseMenus()`, `SpellStopCasting()`,
    // `SpellStopTargeting()`, `CloseAllWindows()`, `ClearTarget()` — and only
    // if none of them did anything does the menu open.
    //
    // **The return value is the whole point**, which is why these are here and
    // not in [`self::verbs`]: `elseif ( SpellStopCasting() )` means *I
    // cancelled a cast, so do not open the menu*, so a verb answering nil would
    // open the game menu on every Escape including mid-cast. They are the shape
    // `PutItemInBag` already has — a read off the world with a write pushed on
    // the same queue a verb writes to.
    //
    // This client read `just_pressed(KeyCode::Escape)` in two other files for
    // all three of these until the key-bindings panel landed, which meant
    // Escape did three things at once and could be rebound away from none of
    // them. See `game::combat::action::stop_casting` and
    // `game::combat::target`'s `ClearTarget` arm.
    {
        let queue = std::rc::Rc::clone(queue);
        let f = scope.create_function(move |_, ()| {
            let casting = answers.spell_is_casting();
            if casting {
                queue.borrow_mut().push(crate::game::bindings::Binding::SpellStopCasting);
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
                    .push(crate::game::bindings::Binding::SpellStopTargeting);
            }
            Ok(one_or_nil(targeting))
        })?;
        // **Registered here rather than in `verbs.rs`, where it used to be.**
        // It was a plain verb answering nothing, and nothing noticed because
        // its only call site outside this chain — `SpellButton_OnClick`'s own
        // `else` — throws the answer away. Inside the chain it is load-bearing.
        globals.set("SpellStopTargeting", f)?;
    }
    {
        let queue = std::rc::Rc::clone(queue);
        let f = scope.create_function(move |_, ()| {
            // **`UnitExists("target")` is the reading**, which is the same
            // question the chain's own branch is asking and needs no state of
            // its own.
            let had = answers.unit_exists("target");
            if had {
                queue.borrow_mut().push(crate::game::bindings::Binding::ClearTarget);
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
    // **The range pair, and the second of them is the one place in this file
    // where `nil` and `0` are not the same answer.** `ActionButton_OnUpdate`
    // tests `IsActionInRange(button) == 0` for the red hotkey and `== 1` for the
    // range dot, so an unanswerable question — no target, or a unit the renderer
    // has not placed — must come back nil and not `one_or_nil(false)`, which is
    // the same nil arrived at by a route that also swallows a real "out of
    // range". Hence `Option<bool>` all the way down rather than a bool.
    slot!("ActionHasRange", |s| one_or_nil(answers.action_has_range(s)));
    slot!("IsActionInRange", |s| answers
        .is_action_in_range(s)
        .map(u32::from));
    // The item slot's own three — the count under the icon, and the green
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

    // The spellbook's seven, in their own file — one subject, one module, the
    // same rule the rest of this directory follows. The character sheet's
    // fourteen are the next of them.
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
    // …and the stable master's seven, one of which aims a `<PlayerModel>` and
    // is scoped for that reason — see [`super::panels::stable`].
    super::panels::stable::install(lua, scope, answers)?;
    // …and the bank's three, whose thirty buttons otherwise read the paper
    // doll's — see [`super::panels::bank`].
    super::panels::bank::install(lua, scope, answers)?;
    // …and the six a sign, a plaque or a book asks — the one window in the
    // directory whose subject is a thing. See [`super::panels::pagetext`].
    super::panels::pagetext::install(lua, scope, answers)?;
    // …and the summon popup's three. See [`super::panels::summon`].
    super::panels::summon::install(lua, scope, answers)?;
    super::panels::trade::install(lua, scope, answers)?;
    // …and the two profession windows, whose create buttons ride the same
    // queue the bags' drag does — see [`super::panels::tradeskill`].
    super::panels::tradeskill::install(lua, scope, answers, queue)?;
    super::panels::craft::install(lua, scope, answers, queue)?;
    super::panels::talent::install(lua, scope, answers)?;
    super::panels::taxi::install(lua, scope, answers)?;
    super::panels::worldmap::install(lua, scope, answers)?;
    // …and who else is in the group, which is the one unit subject whose
    // members may not be in the world at all — see [`super::panels::party`].
    super::panels::party::install(lua, scope, answers)?;
    // …and the raid's eight, which are the same roster asked a different set of
    // questions — see [`super::panels::raid`].
    super::panels::raid::install(lua, scope, answers)?;
    // …and their pets, which are the one unit family whose token is derived
    // from another unit's fields rather than held — see [`super::panels::pet`].
    super::panels::pet::install(lua, scope, answers)?;
    super::panels::shapeshift::install(lua, scope, answers)?;
    // …and the two screens before there is a world, whose reads are about a
    // session that has not started — see [`super::panels::glue`].
    super::panels::glue::install(lua, scope, answers)?;

    // …and the two tooltip populations, which are the same kind of read one
    // level down: they answer *onto a widget* rather than into an expression,
    // but the value still has to be the world's at the moment of the call.
    super::widgets::tooltip::install_scoped(lua, scope, answers)?;

    Ok(())
}

/// The stub world every test in this directory answers from.
///
/// `pub(crate)` rather than `pub(super)`: [`crate::game::cvars`]'s own test
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
        /// **The zone line a party member's plate gets**, empty for every other
        /// unit — see [`crate::game::api::UnitTip::zone`].
        pub zone: String,
        pub level: i32,
        pub health: u32,
        pub health_max: u32,
        pub dead: bool,
        /// `UnitIsGhost` — released, which is not the same as [`Self::dead`].
        pub ghost: bool,
        pub in_combat: bool,
        /// The identity `UnitIsUnit` compares — two tokens naming one creature
        /// share it, which is how `TARGETSELF`'s branch is tested.
        pub guid: u64,
        /// `UnitPlayerControlled` — a person behind it rather than the server.
        pub player: bool,
        /// The three the unit plate's second and fourth lines are made of: a
        /// `CreatureType.dbc` row, a classification (0 normal … 4 rare), and the
        /// PvP flag. Zero and false is a plain, unflagged creature.
        pub creature_type: u32,
        pub classification: u32,
        pub pvp: bool,
        /// `<Innkeeper>`.
        pub sub_name: String,
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
        /// Whether this slot is the ranged attack currently repeating — the
        /// state `ActionButton_UpdateState` reads beside [`Self::current`].
        pub auto_repeat: bool,
        /// The item half of a slot: how many are carried, whether the stack
        /// count is drawn, and whether it is being worn. All three are `0`/false
        /// for a spell, which is what every slot a test builds through
        /// [`Stub::action`] is.
        pub count: u32,
        pub consumable: bool,
        pub equipped: bool,
        /// The range pair, and it is **two** fields for one subject because the
        /// interface reads three states out of them: no range at all, in range,
        /// out of range. `in_range` is `None` for a slot that has a range and
        /// nothing to measure it against — no target — which is the answer a
        /// single bool cannot express and the reason the trait method returns an
        /// `Option`.
        pub has_range: bool,
        pub in_range: Option<bool>,
    }

    /// One row of a [`Stub`]'s spellbook.
    pub struct StubSpell {
        pub name: String,
        pub rank: String,
        pub passive: bool,
    }

    /// A stub world, which is the second thing [`Answers`] bought — see the
    /// module comment. Everything a test needs to set is a public field.
    #[derive(Default)]
    pub struct Stub {
        pub now: f64,
        pub units: Vec<StubUnit>,
        pub actions: Vec<StubAction>,
        /// The book, flat, in the order [`vale_assets::tables::book`] would have
        /// sorted it — a test states the order it wants rather than building one.
        pub spells: Vec<StubSpell>,
        /// `(name, offset, count)` per tab, which is what `GetSpellTabInfo`
        /// answers. Not derived from `spells`: the whole thing the panel's
        /// arithmetic stands on is that these two agree, so a test that wants to
        /// check the agreement has to be able to state both.
        pub tabs: Vec<(String, usize, usize)>,
        /// `(token, aura)` pairs in the order they were added, which is the
        /// order the player's own bar keeps — see [`crate::game::combat::auras`].
        pub auras: Vec<(String, crate::lua::panels::auras::AuraInfo)>,
        /// The character sheet's block, for whoever answers `"player"` — built
        /// by [`Stub::stats`] out of real update fields, so the tests over
        /// `lua::paperdoll` exercise the same decode the client runs.
        pub stats: Option<vale_protocol::play::stats::UnitStats>,
        /// What every item answers for its cooldown — see
        /// [`Stub::container_item_cooldown`]. `(0, 0, false)` by `Default`,
        /// which is the "nothing there" triple with the swirl disabled.
        pub item_cooldown: (f64, f64, bool),
        /// **Which of the four extra action bars are on**, as the same
        /// `multi_bar` mask the wire carries. `0` by `Default`, which is a fresh
        /// account: four bars off. See [`super::ActionAnswers::action_bar_toggles`].
        pub bar_toggles: u8,
        /// **The rows of an open loot window**, one-based when read, with the
        /// coins first if there are any — see [`super::loot`]. Empty by
        /// `Default`, which is the "nothing has been right-clicked" state and
        /// what `GetNumLootItems` answers 0 for.
        pub loot: Vec<crate::lua::panels::loot::LootRow>,
        /// **…and the one group roll a test may have open**, under id 0 — which
        /// is a real id, since the roll counter starts there. `None` by
        /// `Default`, which is what every id answers nothing for.
        pub roll: Option<crate::lua::panels::lootroll::RollItem>,
        /// Parallel to [`Stub::auras`] — see [`Stub::buff`].
        helpful: Vec<bool>,
        /// How every unit in this world stands towards every other. One value
        /// rather than a matrix because the questions the directory asks are
        /// all `("player", "target")` in one order or the other, and a test that
        /// wants the other reading builds a second world.
        standing: vale_assets::tables::faction::Reaction,
        /// What `UpdateMapHighlight` answers. `None` is the ordinary case — the
        /// pointer over open water, and every point on a zone map.
        highlight: Option<crate::lua::panels::worldmap::Highlight>,
        /// …and the explored overlays on it. Empty is the ordinary case — a
        /// client with no world, and a zone nobody has walked.
        overlays: Vec<crate::lua::panels::worldmap::OverlayArt>,
        /// **The party**: `(name, is the leader)` per member, in `party1..N`
        /// order. Empty is a solo character, which is what most of this
        /// directory's tests are.
        party: Vec<(String, bool)>,
        /// **The raid**: `(name, subgroup byte)` for the members the server
        /// names, *without* the local player — who is appended as the last slot
        /// by [`crate::game::session::raid`]'s rule, and by this stub's own
        /// answer for the same reason. Empty is a party.
        raid: Vec<(String, u8)>,
        /// **Being dead**, as five plain fields — the two clocks and the three
        /// halves of an offer. Flat rather than an `Option<ResurrectOffer>`
        /// because every test that touches them states one number.
        release_remaining: i32,
        recovery_delay: i32,
        offerer: Option<String>,
        offer_sickness: bool,
        offer_timer: bool,
        /// **The bags: `(bag id, [(entry, count)])`**, one-based when read.
        /// An entry of 0 is an empty slot, which is how a bag with a hole in
        /// the middle is written.
        containers: Vec<(i32, Vec<(u32, u32)>)>,
        /// `(inventory slot id, entry, count)` — the worn slots, in the
        /// interface's numbering.
        worn: Vec<(u32, u32, u32)>,
        /// Which item is *in* each bag slot, for `GetBagName`.
        bag_entries: Vec<(i32, u32)>,
        /// **Templates, kept apart from the slots on purpose** — see
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
        /// …and the two the *requirement* lines are drawn from, which the stub
        /// carries because their whole point is the colour they are drawn in.
        pub required_level: u32,
        pub level_met: bool,
    }

    impl Stub {
        /// **A character carrying something** — a backpack with twenty linen
        /// in slot one and an unresolved stack in slot two, a worn sword, and
        /// templates for two of the three.
        ///
        /// The third is deliberately template-less: it is what makes the
        /// `-1` quality and the missing icon checkable, and it is the state
        /// every bag is in for the first second of a login. See
        /// [`crate::game::character::items`], where the two-phase fill is written up.
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
                // **A potion the character is too low for**, which is the
                // report the requirement lines were added for: the level is
                // stated and it is *not* met, so the plate must say so in red.
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

        /// One container slot, **one-based**, `None` for an empty one.
        fn stub_slot(&self, bag: i32, slot: usize) -> Option<(u32, u32)> {
            let held = *self.container(bag)?.get(slot.checked_sub(1)?)?;
            (held.0 != 0).then_some(held)
        }

        /// …and one worn slot.
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
                // Nothing is on the stub's cursor: a lock is a state only a
                // real drag reaches, and `cursor_has_item` below says so once.
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
                // A plain unflagged humanoid, which is the ordinary case and
                // the one every existing test was written against.
                creature_type: 7,
                classification: 0,
                pvp: false,
                sub_name: String::new(),
                // Empty, which is every unit but a group mate somewhere else.
                zone: String::new(),
            });
            self
        }

        /// **Where the unit added last is standing**, for the one plate line
        /// that only a party member gets.
        pub fn in_zone(mut self, zone: &str) -> Stub {
            if let Some(unit) = self.units.last_mut() {
                unit.zone = zone.to_string();
            }
            self
        }

        /// …and the three the unit plate's own lines are made of, for the tests
        /// that are about the plate rather than about the unit.
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

        /// Make the unit added last a **player**, which is the branch that
        /// composes "Human Warrior" out of a race and a class.
        pub fn player_controlled(mut self) -> Stub {
            if let Some(unit) = self.units.last_mut() {
                unit.player = true;
                unit.creature_type = 0;
            }
            self
        }

        /// **How this world's units stand towards each other** — the one fact
        /// `UnitIsFriend`, `UnitIsEnemy`, `UnitReaction` and `UnitCanAttack` are
        /// all answered from.
        pub fn standing(mut self, standing: vale_assets::tables::faction::Reaction) -> Stub {
            self.standing = standing;
            self
        }

        /// **What the pointer is over on the world map**, name and art — the
        /// two halves `WorldMapButton_OnUpdate` reads as one call.
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

        /// **The explored overlays on the parchment**, for the two reads
        /// `WorldMapFrame_Update` walks them with.
        pub fn overlays(mut self, overlays: Vec<crate::lua::panels::worldmap::OverlayArt>) -> Stub {
            self.overlays = overlays;
            self
        }

        /// **The party the reads answer off**, one entry per `party<n>`.
        pub fn party(mut self, members: &[(&str, bool)], count: usize) -> Stub {
            let _ = count;
            self.party = members
                .iter()
                .map(|(name, leader)| ((*name).to_string(), *leader))
                .collect();
            self
        }

        /// **A raid**, as the server's own list plus one: the members named
        /// here are `raid1..N` and the *local player* is `raid<N+1>`, which is
        /// the join the wire does not make — see
        /// [`crate::game::session::raid`]. `own_flags` is our own subgroup byte,
        /// which is the only place the reader's column is stated.
        pub fn raid(mut self, members: &[(&str, u8)], own_flags: u8) -> Stub {
            self.raid = members
                .iter()
                .map(|(name, flags)| ((*name).to_string(), *flags))
                .collect();
            self.raid.push(("Alden".to_string(), own_flags));
            self
        }

        /// **A highlight with a name and no art** — a zone map's answer, which
        /// is the other of the two shapes `UpdateMapHighlight` has and the one
        /// that hides `WorldMapHighlight`.
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

        /// A tab, and the spells on it — appended, so the offsets follow from
        /// the order the calls are made in the way a real book's do.
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
                // No range and nothing to measure — the untinted, dotless
                // button, which is what a spell like Battle Shout draws and
                // what a test that is not about range wants.
                has_range: false,
                in_range: None,
            });
            self
        }

        /// …and the same slot with a range on it. `in_range` is the whole point
        /// of the pair: `Some(false)` is the red hotkey and `None` is a range
        /// the client cannot measure right now, which the interface draws
        /// differently — see [`crate::game::api::is_action_in_range`].
        pub fn ranged_action(mut self, slot: u8, text: &str, in_range: Option<bool>) -> Stub {
            self = self.action(slot, text);
            if let Some(action) = self.actions.last_mut() {
                action.has_range = true;
                action.in_range = in_range;
            }
            self
        }

        /// **A level-60 character sheet's worth of update fields**, decoded
        /// the way the client decodes a real one — 60 Strength with a +10/-4
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

        // **The bags**, out of the three flat lists [`Stub::bags`] fills. The
        // stub keeps *slots* and *templates* apart exactly as the client does,
        // so a test can put a stack in a bag whose template has not arrived —
        // which is the state every bag is in for the first second of a login.
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
        /// **The `item_cooldown` field, verbatim, whatever the slot holds.** A
        /// test that is about a swirl states the triple it wants; the *join*
        /// from a slot to an item to a spell to a record is the live world's
        /// and is checked there.
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
            // The two rows the tests name, with the same shape the real table
            // gives them — see [`vale_assets::tables::inventory`].
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
        fn bag_item_tip(&self, bag: i32, slot: usize) -> Option<crate::game::api::ItemTip> {
            let (entry, _) = self.stub_slot(bag, slot)?;
            self.item_tip(entry)
        }
        fn inventory_item_tip(&self, token: &str, id: u32) -> Option<crate::game::api::ItemTip> {
            if token != "player" {
                return None;
            }
            self.item_tip(self.stub_worn(id)?.0)
        }
        fn item_tip(&self, entry: u32) -> Option<crate::game::api::ItemTip> {
            let template = self.template(entry)?;
            Some(crate::game::api::ItemTip {
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

        // **Empty by default**, which is what the tests in this directory want: a
        // stub world has no conversation and no log unless the test states one.
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
        fn quest_reward_spell_tip(&self, _from_log: bool) -> Option<crate::game::api::SpellTip> {
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
    
        // …and an untracked one, which is every quest in a log nobody has
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


        // --- talking to an NPC: a probe-sized shop and menu ---
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
        /// **Two rows, sold in the order the world sold them** — enough for a
        /// test to tell "the last one" from "the first one", which is the whole
        /// of what the front tab's `GetBuybackItemInfo(GetNumBuybackItems())`
        /// depends on.
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
        fn repairs(&self) -> crate::game::npc::merchant::Repairs {
            crate::game::npc::merchant::Repairs {
                can_repair: true,
                cost: 1234,
                priced: true,
                mode: false,
            }
        }
    }

    impl crate::lua::panels::taxi::TaxiAnswers for Stub {
        // …and a probe-sized flight map: where you are, and one place one hop
        // away. The numbers are the ones `lua::taxi`'s own tests read back.
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
        // **A sample profession**, so the panel probes exercise the window:
        // one header, one recipe under it, one reagent half-met. The shapes
        // mirror what a real Blacksmithing line answers.
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
        // **A sample Enchanting window**, one recipe, no header — the flat
        // shape the craft list really has.
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
        fn craft_spell_tip(&self, _index: usize) -> Option<crate::game::api::SpellTip> {
            None
        }
    }

    impl crate::lua::panels::mail::MailAnswers for Stub {
        // **An empty mailbox**, which is what a character who has never been
        // sent anything has — and the shape the panel's own tests check the
        // zero-versus-nil answers against.
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

    /// …and a probe-sized stable: one slot bought, a pet in the current stall
    /// and one in the bought stall, so `PetStable_Update` walks the occupied,
    /// the bought-but-empty and the unbought branches in one pass.
    /// …and a shut trade window, which is every default.
    impl crate::lua::panels::trade::TradeAnswers for Stub {}
    impl crate::lua::panels::summon::SummonAnswers for Stub {}
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

        // …and a probe-sized trainer: a header and a green spell under it.
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
        fn trainer_tooltip(&self, _row: usize) -> Option<crate::game::api::SpellTip> {
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

        // **The map: a client with no world.** Every one of these is the answer
        // a character screen gives, which is the state these tests run in and
        // the one the "nothing" answers on the trait are written for.
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

    /// **No pet**, which is the ordinary case and the one every non-pet test in
    /// this file is written against. The pet reads have their own doubles in
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
        fn pet_action_info(&self, _slot: usize) -> Option<crate::game::combat::pet::PetSlot> {
            None
        }
        fn pet_action_cooldown(&self, _slot: usize) -> (f64, f64, u32) {
            (0.0, 0.0, 0)
        }
        fn pet_action_tooltip(&self, _slot: usize) -> Option<crate::game::api::SpellTip> {
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
        fn creature_family(&self, _unit: crate::game::api::UnitId) -> Option<String> {
            None
        }
        fn has_pet_spells(&self) -> Option<(u32, &'static str)> {
            None
        }
    }

    /// **No forms**, which is every class but four and is what the bare
    /// interpreter's tests are about — the stance bar has its own double in
    /// [`crate::lua::audit`].
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
                // **The last row leads**, which is the local player: a stub
                // where `raid1` led would never exercise the branch a raid
                // panel spends its whole time in.
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
            !self.raid.is_empty() && crate::game::api::UnitId::parse(token).is_some()
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
        // **No handshake**, which is what the client holds at a login screen
        // and in the world — the two states this stub stands in for.
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

    /// **Nothing at all**, which is the only honest talent tree for a stub
    /// world with no class in it — `GetNumTalentTabs()` answering 0 is exactly
    /// what a character below the level that has any reads.
    ///
    /// The *populated* case is [`crate::lua::audit`]'s `Login`, which is where
    /// the panel is actually driven.
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
        fn talent_tooltip(&self, _: usize, _: usize) -> Option<crate::game::api::SpellTip> {
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
        fn spell_tooltip(&self, index: usize) -> Option<crate::game::api::SpellTip> {
            let (name, rank) = self.spell_name(index)?;
            Some(crate::game::api::SpellTip {
                name,
                rank,
                ..Default::default()
            })
        }
    }

    impl crate::lua::panels::auras::AuraAnswers for Stub {
        fn player_buff(&self, index: usize, filter: &str) -> i32 {
            // The stub's own filter is the two halves and nothing else, which
            // is what every shipped button passes.
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
            // Only the two tokens the live client keeps lists for, so the
            // "a unit with no list answers nothing" test means something.
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
        /// tests can assert on every line the law composes.
        fn action_tooltip(&self, slot: u8) -> Option<crate::game::api::SpellTip> {
            self.slot(slot).map(|action| crate::game::api::SpellTip {
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
        /// **No stub slot is an item**, so a bar plate here is always the
        /// spell's — which is what keeps the tooltip tests below asserting on
        /// the composition they are about.
        fn action_item_tooltip(&self, _: u8) -> Option<crate::game::api::ItemTip> {
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
        /// (`game::action`).
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
        /// The reference's own fallback for a character with no bind point,
        /// which is what a harness with no session is.
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
        /// The player's, and nobody else's — see [`Stub::stats`].
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
        /// No side: the tests here are about creatures and the two the
        /// interface asks about are answered in [`crate::lua::audit`].
        fn unit_faction_group(&self, _token: &str) -> Option<(String, String)> {
            None
        }
        fn unit_is_pvp(&self, token: &str) -> bool {
            self.find(token).is_some_and(|unit| unit.pvp)
        }
        fn unit_tooltip(&self, token: &str) -> Option<crate::game::api::UnitTip> {
            let unit = self.find(token)?;
            Some(crate::game::api::UnitTip {
                name: unit.name.clone(),
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
        /// Whatever [`Stub::standing`] was told, and `None` for a token this
        /// world has no unit at — which is the answer that matters, since every
        /// consumer in the directory has a separate arm for it.
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

    /// Run a chunk with the reads installed, and get its `return` back as a
    /// string — which is the shortest way to assert on what Lua actually saw.
    pub fn eval(answers: &dyn Answers, chunk: &str) -> String {
        let lua = mlua::Lua::new();
        // The five held reads are registered once on the state, exactly as
        // `LuaHost::new` does it — see [`Held`].
        let (held, queue) = held_for_test(&lua);
        lua.scope(|scope| {
            install(&lua, scope, answers, &held, &queue)?;
            let value: mlua::Value = lua.load(chunk).eval()?;
            Ok(format!("{value:?}"))
        })
        .expect("the chunk runs")
    }

    /// **A read is answered during the call**, which is the whole claim of this
    /// module: the value comes back into a Lua expression rather than onto a
    /// queue somebody drains later.
    #[test]
    fn a_read_answers_inside_the_expression() {
        let world = Stub::default().unit("target", "Kobold Vermin", 7);
        assert_eq!(
            eval(&world, r#"return UnitName("target") .. " at " .. UnitHealth("target")"#),
            r#"String("Kobold Vermin at 100")"#
        );
    }

    /// **A boolean is `1` or `nil`, not `true`/`false`** — the game's own shape,
    /// and the one an addon can compare against. See the module comment.
    #[test]
    fn a_boolean_comes_back_as_the_games_one_or_nil() {
        let world = Stub::default().unit("player", "Alden", 1);
        assert_eq!(eval(&world, r#"return UnitExists("player")"#), "Integer(1)");
        assert_eq!(eval(&world, r#"return UnitExists("target")"#), "Nil");
        // …and the comparison that only works if it really is a number.
        assert_eq!(
            eval(&world, r#"return UnitExists("player") == 1"#),
            "Boolean(true)"
        );
    }

    /// **`GetActionBarToggles()` answers four values in the interface's own
    /// order**, and the order is the whole of what this test is for.
    ///
    /// `UIParent.lua` assigns them straight across —
    /// `SHOW_MULTI_ACTIONBAR_1, ..._2, ..._3, ..._4 = GetActionBarToggles()` —
    /// and `MultiActionBar_Update` spends the four on the bottom-left, the
    /// bottom-right, the right and the left column in that order. So a
    /// transposition here does not fail: it puts the right number of bars on the
    /// screen in the wrong places, and only a relog would ever show it up.
    #[test]
    fn the_four_extra_bars_come_back_in_the_interfaces_own_order() {
        use vale_protocol::play::spells::multi_bar;
        let one = |mask: u8| Stub { bar_toggles: mask, ..Default::default() };
        let read = "local a, b, c, d = GetActionBarToggles(); \
                    return tostring(a)..tostring(b)..tostring(c)..tostring(d)";
        // A fresh character: four bars off, four nils — which is what
        // `MultiActionBar_Update`'s `else` branch hides on.
        assert_eq!(eval(&one(0), read), r#"String("nilnilnilnil")"#);
        // …and one at a time, each landing on its own return.
        assert_eq!(eval(&one(multi_bar::BOTTOM_LEFT), read), r#"String("1nilnilnil")"#);
        assert_eq!(eval(&one(multi_bar::BOTTOM_RIGHT), read), r#"String("nil1nilnil")"#);
        assert_eq!(eval(&one(multi_bar::RIGHT), read), r#"String("nilnil1nil")"#);
        assert_eq!(eval(&one(multi_bar::LEFT), read), r#"String("nilnilnil1")"#);
        assert_eq!(eval(&one(multi_bar::ALL), read), r#"String("1111")"#);
    }

    /// …and each of the four is **`1` rather than `true`**, for the reason the
    /// boolean test above gives — `MultiBar1_IsVisible` hands its answer to an
    /// options checkbox, and `OptionsFrame` tests those against `1`.
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

    /// **An absent unit is `nil`, never `""`** — `if ( UnitName(u) )` is how
    /// FrameXML asks whether a unit is there, and an empty string is true.
    #[test]
    fn an_absent_unit_is_nil_rather_than_empty() {
        let world = Stub::default();
        assert_eq!(eval(&world, r#"return UnitName("target")"#), "Nil");
        assert_eq!(
            eval(&world, r#"if ( UnitName("target") ) then return 1 else return 0 end"#),
            "Integer(0)"
        );
        // A token this client has no state for at all is the same answer as an
        // empty one, rather than an error.
        assert_eq!(eval(&world, r#"return UnitName("party3")"#), "Nil");
        // …and a call with no argument, which addons write.
        assert_eq!(eval(&world, "return UnitName()"), "Nil");
    }

    /// **`TargetDebuffButton_Update`'s own branch**, which is where a nil
    /// `UnitIsFriend` cost a picture: the whole of what decides whether a
    /// target's *buffs* sit against the frame or two rows under its debuffs is
    /// this one call, and answering nothing put every friendly target in the
    /// game on the hostile layout.
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
        // …and `TargetFrame_OnShow`'s three-way, whose first arm is the other
        // name: a neutral critter is neither.
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
        // **…and it is still attackable**, which is the surprising half of the
        // rule `vale_assets::tables::faction::Reaction::is_attackable` states: a
        // client that required hostility could not attack a rabbit.
        assert_eq!(
            eval(&neutral, r#"return UnitCanAttack("player", "target")"#),
            "Integer(1)"
        );
    }

    /// **`UnitReaction` is an index into `UnitReactionColor`**, and the three
    /// rows this client can name are the three colours the game paints: red at
    /// 2, yellow at 4, green at 5. A unit that is not there answers `nil`, which
    /// `TargetFrame_CheckFaction` has its own arm for — and which the constant
    /// `4` this replaces could never produce.
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

    /// `UnitPlayerControlled` — the first branch of `TargetFrame_CheckFaction`,
    /// which is what puts a *player* target's name plate on the PvP colours
    /// instead of the reaction's.
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

    /// **`TARGETSELF`'s read, on the real body's terms**: two absent tokens are
    /// not the same unit.
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

    /// **The cooldown comes back as the game's three values**, which is what
    /// `local start, duration, enable = GetActionCooldown(n)` needs — a single
    /// value silently leaves `duration` nil and every cooldown swirl at zero
    /// length.
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
        // …and the same for the usable pair, whose second value is the whole
        // reason a button can read blue rather than grey.
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

    /// **`IsActionInRange` has three answers and the interface tells them
    /// apart by `== 1` and `== 0`**, so a bool would be a bug that draws
    /// plausibly: `ActionButton_OnUpdate` reads `nil` as "say nothing" and `0`
    /// as "colour the hotkey red", and `one_or_nil(false)` collapses the two.
    ///
    /// Asserted on the raw values rather than on truthiness, because that
    /// collapse is exactly what Lua's own `if` would hide.
    #[test]
    fn the_range_read_answers_one_zero_and_nil() {
        let inside = Stub::default().ranged_action(1, "Fireball", Some(true));
        assert_eq!(eval(&inside, "return IsActionInRange(1)"), "Integer(1)");
        assert_eq!(eval(&inside, "return ActionHasRange(1)"), "Integer(1)");

        let outside = Stub::default().ranged_action(1, "Fireball", Some(false));
        assert_eq!(eval(&outside, "return IsActionInRange(1)"), "Integer(0)");
        // The distinction the whole signature exists for: an explicit 0 is not
        // the same value as nothing, and the interface compares against both.
        assert_eq!(
            eval(
                &outside,
                "if ( IsActionInRange(1) == 0 ) then return 1 end return 0"
            ),
            "Integer(1)"
        );

        // A range the client cannot measure — nothing targeted — and a slot with
        // no range at all both answer nil, and neither draws a dot.
        let unmeasurable = Stub::default().ranged_action(1, "Fireball", None);
        assert_eq!(eval(&unmeasurable, "return IsActionInRange(1)"), "Nil");
        assert_eq!(eval(&unmeasurable, "return ActionHasRange(1)"), "Integer(1)");

        let rangeless = Stub::default().action(1, "Battle Shout");
        assert_eq!(eval(&rangeless, "return IsActionInRange(1)"), "Nil");
        assert_eq!(eval(&rangeless, "return ActionHasRange(1)"), "Nil");
    }

    /// **`GetTime` is the base every timer is in.** Not a wall clock and not a
    /// second origin — see [`crate::game::api::get_time`].
    #[test]
    fn get_time_is_the_clock_the_bars_scrub_against() {
        let world = Stub {
            now: 1234.5,
            ..Stub::default()
        };
        assert_eq!(eval(&world, "return GetTime()"), "Number(1234.5)");
    }

    /// **`GetGameTime` is the world's clock and returns two numbers**, which is
    /// what `GameTime.lua` unpacks into `hour, minute`.
    ///
    /// Asserted as two values rather than one, because a single return reads in
    /// Lua as the hour with a `nil` minute, and the frame's arithmetic
    /// (`hour * 60 + minute`) raises rather than drawing wrongly — which the
    /// load probe would report and the panel probe would not.
    #[test]
    fn the_game_clock_answers_an_hour_and_a_minute() {
        let world = Stub::default();
        assert_eq!(
            eval(&world, "local h, m = GetGameTime(); return h * 60 + m"),
            "Integer(720)"
        );
    }

    /// **`GetQuestGreenRange` is the player's, whoever is targeted** — no
    /// argument, and the table is indexed by *our* level and nothing else.
    ///
    /// The interface calls it inside `GetDifficultyColor(targetLevel)`, which is
    /// enough to read it as being about the target; it is not. The level the
    /// colour is *for* is the argument, and this is the width of the green band
    /// under the player.
    #[test]
    fn the_green_range_reads_the_players_level_and_not_the_targets() {
        let mut world = Stub::default().unit("player", "Alden", 1);
        world.units[0].level = 60;
        let mut world = world.unit("target", "Roc", 2);
        world.units[1].level = 42;
        assert_eq!(eval(&world, "return GetQuestGreenRange()"), "Integer(7)");

        // Drop the player twenty levels and the band narrows, with the target
        // untouched — which is the direction that proves which unit it reads.
        world.units[0].level = 39;
        assert_eq!(eval(&world, "return GetQuestGreenRange()"), "Integer(5)");
    }

    /// **The whole of `GetDifficultyColor`'s cascade, on the directory's own
    /// body** — because the bug this closes was not in the arithmetic, it was
    /// that the last question in the chain had no answer at all.
    ///
    /// `QuestLogFrame.lua`'s five branches are transcribed here rather than
    /// loaded, which is deliberate: a test that loads 175 files to reach four
    /// comparisons is an acceptance run, and that is what `--audit --events` is.
    /// What this pins is the pair — that a level 18 below the player reaches the
    /// grey branch *and gets there*, which before this round raised on a nil
    /// global and took `TargetFrame_Update` down with it.
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

        // …and the boundary the table decides: 60 - 7 = 53 is still green, 52 is
        // grey. Nothing else in the cascade can move that line.
        let at = |level: i32| cascade.replace("local level = 42", &format!("local level = {level}"));
        assert_eq!(eval(&world, &at(53)), "String(\"standard\")");
        assert_eq!(eval(&world, &at(52)), "String(\"trivial\")");
    }

    /// A slot outside the bar answers "nothing there" rather than pointing at
    /// the first button — and neither `0` nor a missing argument is a slot.
    #[test]
    fn a_slot_outside_the_bar_is_empty() {
        let world = Stub::default().action(1, "Rend");
        assert_eq!(eval(&world, "return HasAction(1)"), "Integer(1)");
        assert_eq!(eval(&world, "return HasAction(0)"), "Nil");
        assert_eq!(eval(&world, "return HasAction(13)"), "Nil");
        assert_eq!(eval(&world, "return HasAction()"), "Nil");
    }

    /// **[`READS`] and what [`install`] registers are the same set** — checked in
    /// both directions, and the list is sorted.
    ///
    /// One direction was checked here for two rounds and the other was not, and
    /// the missing half cost exactly what it was always going to: `UnitName` was
    /// registered from the first day and absent from the list, so `vale
    /// bindings` reported 18 reads where there were 19. That error is in the safe
    /// direction — a client that looks *less* complete than it is — which is
    /// precisely why nothing noticed it for two rounds.
    ///
    /// **The client's own boolean coercion**, case by case — see
    /// [`to_boolean`], where the switch is written out.
    ///
    /// The rows that matter are the two Lua gets wrong: `0` and `"false"` are
    /// **false** here and true under `lua_toboolean`, and the directory writes
    /// both. Getting them wrong drew a checked border on every action button and
    /// every spell in the book.
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
        // …and the four the string compares catch, which are outside the
        // class table's `'0'..'y'` window or fall to `2` inside it.
        assert!(!s("off"));
        assert!(!s("disabled"));
        assert!(s("on"));
        assert!(s("enabled"));

        // Anything else — including the empty string — takes the caller's own
        // default, which every widget setter in the client pushes as true.
        assert!(s(""));
        assert!(s("wombat"));
        let lua = mlua::Lua::new();
        let empty = mlua::Value::String(lua.create_string("").expect("a string"));
        assert!(!to_boolean(Some(&empty), false), "the default is the caller's");
        assert!(t(mlua::Value::Table(lua.create_table().expect("a table"))));
    }

    /// The reverse test is a diff of the globals table across the install rather
    /// than a second hand-written list, because a second hand-written list has
    /// the same failure as the first.
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
        // **Registered after the snapshot on purpose.** These five are
        // persistent rather than scoped ([`Held`]) but they are still `READS`
        // this client answers, so they have to appear in the difference the
        // check below compares against the list.
        let (held, queue) = held_for_test(&lua);
        let installed = lua
            .scope(|scope| {
                install(&lua, scope, &world as &dyn Answers, &held, &queue)?;
                Ok(global_names(&lua))
            })
            .expect("the scope runs");
        let mut added: Vec<String> = installed.difference(&before).cloned().collect();
        added.sort();
        // **Both lists**, because `install` also opens the spellbook's — one
        // scope, two files, and the check has to cover what the scope really
        // holds rather than what this file put in it.
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


/// **What the interface may ask about the action bar**, and about a spell
/// waiting to be aimed.
///
/// Split from [`UnitAnswers`] rather than left beside it because they are two
/// subjects that happened to share a file: one is about a creature in the
/// world, the other about twelve slots and a cursor. Both stay in this module
/// because their `Live` bodies read `crate::game::combat::action` and there is no
/// `lua::action` for them to move to — the bar's own registration is here.
pub trait ActionAnswers {

    // --- the action bar ---

    fn has_action(&self, slot: u8) -> bool;
    /// `GetBonusBarOffset` — 0 for the ordinary bar, 1..4 for a form's own.
    /// See [`crate::game::combat::action::ActionBar::bonus_bar`].
    fn bonus_bar_offset(&self) -> u8;
    /// `GetActionBarToggles` — which of the four **extra** bars are switched on,
    /// as a mask of [`vale_protocol::play::spells::multi_bar`] bits.
    ///
    /// A mask rather than the four values the Lua call returns, because the four
    /// are a *presentation* of one byte the server keeps: splitting them here
    /// would put the bit order in three implementations instead of one. See
    /// [`crate::game::combat::action::ActionBar::toggles`].
    fn action_bar_toggles(&self) -> u8;
    /// `GameTooltip:SetAction` — the data the spell tooltip is composed from,
    /// or `None` for a slot with nothing to say. See
    /// [`crate::game::api::action_tooltip`].
    fn action_tooltip(&self, slot: u8) -> Option<api::SpellTip>;
    /// …and the same call for a slot holding an **item**, which is a different
    /// plate entirely. `None` for a spell slot, so the two cannot both answer.
    fn action_item_tooltip(&self, slot: u8) -> Option<api::ItemTip>;
    fn action_text(&self, slot: u8) -> Option<String>;
    fn action_texture(&self, slot: u8) -> Option<String>;
    /// `(start, duration, enable)`, in [`Answers::now`]'s base.
    fn action_cooldown(&self, slot: u8) -> (f64, f64, bool);
    /// `(usable, not_enough_mana)` — the game's own pair, and the reason a
    /// button can read blue rather than grey.
    fn action_usable(&self, slot: u8) -> (bool, bool);
    fn is_attack_action(&self, slot: u8) -> bool;
    fn is_current_action(&self, slot: u8) -> bool;
    /// `IsAutoRepeatAction` — is this the ranged attack currently repeating?
    /// Beside [`Self::is_current_action`] because `ActionButton.lua` asks the
    /// two together; see [`crate::game::api::is_auto_repeat_action`].
    fn is_auto_repeat_action(&self, slot: u8) -> bool;
    /// `ActionHasRange` — is range a question about this button at all? See
    /// [`crate::game::api::action_has_range`].
    fn action_has_range(&self, slot: u8) -> bool;
    /// `IsActionInRange` — **three answers, not two**: `Some(true)`,
    /// `Some(false)` and `None`, which the interface reads as `1`, `0` and
    /// `nil` and treats as three different things. See
    /// [`crate::game::api::is_action_in_range`].
    fn is_action_in_range(&self, slot: u8) -> Option<bool>;
    /// `IsConsumableAction` — does `ActionButton_UpdateCount` write a stack
    /// count under this button? The rule is the item's, not the bar's; see
    /// [`vale_protocol::state::query::ItemInfo::is_consumable`].
    fn is_consumable_action(&self, slot: u8) -> bool;
    /// `IsEquippedAction` — is the item in this slot being worn? The green
    /// border round the button.
    fn is_equipped_action(&self, slot: u8) -> bool;
    /// `GetActionCount` — how many of the slot's item the character carries,
    /// across every container and the worn slots. `0` for a spell or a macro,
    /// which is what the real client answers and what
    /// [`Self::is_consumable_action`] keeps off the screen.
    fn action_count(&self, slot: u8) -> u32;

    // --- the spell cursor ---
    //
    // 1.12's targeting mode, and the interface's half of it is exactly these two
    // reads plus two writes. `UnitFrame_OnEnter` asks both to choose between
    // `SetCursor("CAST_CURSOR")` and `SetCursor("CAST_ERROR_CURSOR")`, and
    // `TargetFrame_OnClick` asks the first to decide whether a click casts or
    // retargets. See [`crate::game::combat::action::SpellTargeting`].

    /// `SpellIsTargeting()` — is a cast waiting to be pointed at something?
    fn spell_is_targeting(&self) -> bool;
    /// **Is there a cast to stop?** — the read behind `SpellStopCasting()`,
    /// which answers whether it cancelled anything and is what decides whether
    /// `ToggleGameMenu`'s chain stops at that branch or goes on to open the
    /// menu.
    ///
    /// All three of the states a cancel can end: a wind-up in progress, a
    /// channel, and an *ask* the server has not answered yet — the third
    /// matters because the bar waits for `SMSG_SPELL_START`, so there is a
    /// round trip in which a player who presses Escape means it and nothing is
    /// yet "casting". See [`crate::game::combat::action::Casting`].
    fn spell_is_casting(&self) -> bool;
    /// `SpellCanTargetUnit(unit)` — would the waiting cast take *this* one?
    ///
    /// **False when nothing is waiting**, which is the directory's own reading:
    /// every call site is already inside an `if SpellIsTargeting()`.
    fn spell_can_target_unit(&self, token: &str) -> bool;
}

impl Live<'_, '_, '_> {
    /// **The spell an item slot's cooldown is actually on.**
    ///
    /// A bar slot holding an item carries only the entry (`SMSG_ACTION_BUTTONS`
    /// carries nothing else), so the swirl's timer has to be found the long way:
    /// entry -> the item's cached template -> its on-use spell -> `Spell.dbc`.
    /// The timer itself is on that spell's **category** — every healing potion in
    /// the game shares one — which is why asking about the entry would find
    /// nothing even if this client kept per-item timers.
    ///
    /// `None` for three real states, and all three draw no swirl, which is right:
    /// the slot is not an item, the template has not arrived yet (every item's
    /// prototype is a round trip away — see
    /// [`crate::game::character::items`]), or the item has no on-use spell at
    /// all, which is a garment sitting on the bar.
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
                // A *spell* plate has no requirement lines; see
                // [`Self::item_context`], which is the one that pays for them.
                race: 0,
                class: 0,
                catalog: self.tables.as_ref().and_then(|t| t.spellbook()),
                item_names: &names,
                home: Some(self.home.clone()),
            },
        )
    }

    /// **The item plate for a bar slot** — composed from the template alone,
    /// with no stack behind it, because a bar slot names an entry and not a
    /// square: the same entry may sit in three bags and there is no one stack
    /// to report a count or a durability off.
    fn action_item_tooltip(&self, slot: u8) -> Option<api::ItemTip> {
        self.tip_from(api::action_item(self.bar, slot)?, None)
    }

    fn action_text(&self, slot: u8) -> Option<String> {
        api::get_action_text(self.bar, slot)
    }

    /// **A spell's icon or an item's**, which is one line of branch and the
    /// whole of whether a mount on the bar is drawn at all. An item slot has no
    /// `SpellInfo` in it, so the spell path answers `None` and
    /// `ActionButton_Update` hides the icon — which is what an item button
    /// looked like: an empty quickslot that did nothing.
    fn action_texture(&self, slot: u8) -> Option<String> {
        if let Some(entry) = api::action_item(self.bar, slot) {
            let template = self.inventory.template(entry)?;
            return self.tables.as_ref()?.item_icon(template.display_id);
        }
        api::get_action_texture(self.bar, slot).map(str::to_string)
    }

    /// …and the same split for the swirl: **an item's cooldown is its `ON_USE`
    /// spell's**, through the same clocks a bag square reads it through, so the
    /// two cannot run at different rates.
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
            // **The cast in flight, which is `Casting`'s own answer to "is one
            // running"** — `started` is set at the send and cleared when the
            // cast lands, is interrupted or is refused, which is exactly when
            // the border should go out. The same pair
            // [`crate::lua::panels::spellbook`] asks for a book button.
            self.casting.started.map(|_| self.casting.spell_id),
            // …and the cursor waiting to be pointed at something, which is the
            // reference's own gate — see [`api::is_current_action`].
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
        // Re-asked through the same [`resolve_aim`] the click will run, rather
        // than off a cached answer — see
        // [`crate::game::combat::target::spell_cursor_validity`], which asks the same
        // question about the *world* pick and for the same reason. `Unit(_)` is
        // the only yes: a spell that would bind implicitly is not one this frame
        // can be the target of.
        let answer = || {
            let tables = self.tables.as_ref()?;
            let info = tables.spellbook()?.info(self.targeting.spell()?)?;
            let me = self.units.get(crate::game::api::UnitId::Player)?;
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
