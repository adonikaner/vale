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
//! twenty lines. Every test in this file does, through `stub::Stub`.
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
//! One case is unresolved: the 1.12 API reference this file follows answers
//! `UnitExists` with a Lua boolean and its neighbours with `1`/`nil`, and
//! which of the two the client returns for that function is not known. This
//! file answers `1`/`nil` throughout, because `== 1` is a comparison an addon
//! can write and `== true` is not.
//!
//! An absent value is `nil`, never a placeholder: `UnitName("party1")` with no
//! party is `nil` and not `""`, because FrameXML asks whether a unit is there
//! with `if ( UnitName(u) ) then`.
//!
//! ```text
//! mod.rs      the `Answers` trait (the sum of the subject traits the panels
//!             declare), the live world that answers it, and the
//!             registration of the reads that belong to no panel
//! unit.rs     `UnitAnswers`: the unit reads, and the live world's answers
//! action.rs   `ActionAnswers`: the action bar reads, and the live answers
//! date.rs     `date` and `time`, and the calendar arithmetic behind `date`
//! stub.rs     the stub world this directory's tests answer from (tests only)
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

pub mod action;
pub mod cvars;
pub mod date;
pub mod events;
pub mod keyboard;
pub mod mouse;
pub mod portrait;
pub mod savedvars;
pub mod sound;
#[cfg(test)]
pub(crate) mod stub;
pub mod stubs;
pub mod unit;
pub mod update;
pub mod verbs;

pub use self::action::ActionAnswers;
pub use self::unit::UnitAnswers;
pub use super::panels::Answers;

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
    /// The world state table and its listed rows. See
    /// [`crate::interface::worldstate`].
    pub world_states: &'a crate::interface::worldstate::WorldStates,
    /// The tutorial masks. See [`crate::interface::tutorial`].
    pub tutorials: &'a crate::interface::tutorial::Tutorials,
    /// The eight raid target icons. See [`crate::interface::raidtarget`].
    pub raid_targets: &'a crate::interface::raidtarget::RaidTargets,
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
    /// The realm's type from the realm list, 0 (normal) outside the world.
    /// See [`crate::world::session::ActiveSession::realm_type`].
    pub realm_type: u32,
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
    /// The live session, for reads of server-owned names and fields the ECS
    /// does not mirror, and for queueing the queries that fill them. A
    /// spell's reagents are item entries and `Item.dbc` is not in the
    /// archives, so a name such as "Rune of Teleportation" lives in the
    /// templates `CMSG_ITEM_QUERY_SINGLE` fills. Reads lock
    /// [`LiveSession::world`](vale_protocol::socket::session::LiveSession::world);
    /// a miss is queued through the session's own `want_*` methods.
    pub live: Option<&'a vale_protocol::socket::session::LiveSession>,
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
    pub world_states: Res<'w, crate::interface::worldstate::WorldStates>,
    pub tutorials: Res<'w, crate::interface::tutorial::Tutorials>,
    pub raid_targets: Res<'w, crate::interface::raidtarget::RaidTargets>,
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
    pub status: Res<'w, crate::world::status::WorldStatus>,
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
            .init_resource::<crate::interface::worldstate::WorldStates>()
            .init_resource::<crate::interface::tutorial::Tutorials>()
            .init_resource::<crate::interface::raidtarget::RaidTargets>()
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
            .init_resource::<crate::world::status::WorldStatus>()
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
            world_states: &self.world_states,
            tutorials: &self.tutorials,
            raid_targets: &self.raid_targets,
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
            realm_type: self.session.active.as_ref().map_or(0, |active| active.realm_type),
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
            live: self.session.active.as_ref().map(|active| &active.live),
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
    /// Safe to call from a hover: a miss queues the entry
    /// ([`ObjectManager::want_item`], through the session) and never touches
    /// the socket, so the query goes out on the session's own interval with
    /// the equipment queries, and a hundred hovers cost one packet.
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
        let live = self.live?;
        let known = live.world().lock().unwrap_or_else(|e| e.into_inner()).items.get(&entry).cloned();
        if known.is_none() {
            live.want_item(entry);
        }
        known
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
        let live = self.live?;
        let known = {
            let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
            match target {
                Target::Creature(entry) => world.creatures.get(&entry).map(|info| info.name.clone()),
                Target::GameObject(entry) => world.gameobjects.get(&entry).map(|info| info.name.clone()),
            }
        };
        if known.is_none() {
            match target {
                Target::Creature(entry) => live.want_creature(entry),
                Target::GameObject(entry) => live.want_gameobject(entry),
            }
        }
        known
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
    /// No packet this client reads carries another unit's bags or durability.
    /// Another player's worn entries come only from the visible-item update
    /// fields (see [`Self::worn_by_other`]), so every read that needs a carried
    /// item answers the absent value for any other token. Answering with the
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
        let world = self.live?.world();
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
        let Some(live) = self.live else {
            return 0;
        };
        let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
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
    ) -> Option<crate::interface::plate::ItemTip> {
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
            let Some(live) = self.live else {
                return false;
            };
            let world = live.world().lock().unwrap_or_else(|e| e.into_inner());
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
            let live = self.live?;
            let known = live
                .world()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .players
                .get(&guid)
                .map(|info| info.name.clone());
            if known.is_none() {
                live.want_social_guid(guid);
            }
            known
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
        let wearer = crate::interface::plate::Wearer {
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
        Some(crate::interface::plate::item_tip(
            template,
            carried,
            self.tables.as_deref(),
            &context,
            &wearer,
        ))
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
    // "Sergeant Bram". This read does not look up the rank (the rank titles
    // are `PVP_RANK_*` in `GlobalStrings.lua`), so it answers the bare name.
    // The 1.12.1 client answers the same for a character below rank 1, which
    // is most characters. This is a stated deviation, not an alias: once the
    // rank is read, this read must add the prefix.
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
    // here. That type says why these five cannot be scoped functions like
    // every other read in this file.
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
    // buffs were drawn 46 units (two rows) below the frame. The first line of
    // [`self::stubs`] states the rule this shows: a constant answer cannot be
    // told apart from a working one.
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
    // a player. [`Units::player_controlled`] answers from the object type
    // alone, so a pet answers false to both; that function records the
    // deviation. When it reads the flag, both reads must change together.
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
    // for the 3D scene. `SetCursor` is a verb, but `crate::ui::cursor` puts
    // the targeting mode ahead of a cursor the interface asked for, so while
    // a spell waits only the world pick changes the pointer on screen; the
    // frames' answers are correct but do not reach it.
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

    // Each panel's reads, registered from that panel's own module in the
    // order [`super::panels`] lists them.
    super::panels::install(lua, scope, answers, queue)?;

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
    use super::stub::*;

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

    /// `UnitReaction` is an index into `UnitReactionColor`. The stub's three
    /// standings give the game's three main colours: red at 2, yellow at 4,
    /// green at 5. A unit that is not there answers `nil`, which
    /// `TargetFrame_CheckFaction` handles in a separate branch. A constant
    /// answer, such as the `4` this read gave before reactions were read,
    /// cannot produce `nil`.
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
    /// A check in one direction only missed `UnitName`, which was registered
    /// and not listed, so `vale bindings` reported 18 reads where there were
    /// 19. An undercount makes the client look less complete than it is, and
    /// no other check catches it.
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
            .chain(crate::lua::panels::worldstate::READS.iter())
            .chain(crate::lua::panels::tutorial::READS.iter())
            .chain(crate::lua::panels::raidtarget::READS.iter())
            .chain(crate::lua::panels::questshare::READS.iter())
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
