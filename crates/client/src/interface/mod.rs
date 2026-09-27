//! The client's side of the game's interface: the state behind each FrameXML
//! panel, and the interface code the panels share.
//!
//! The 1.12.1 client keeps this code in one flat directory, one file per panel,
//! and this directory has the same shape. The interpreter and the widget tree
//! that run the panels' own Lua and XML are in `crate::lua`, and the painting is
//! in `crate::ui`. Nothing here draws.
//!
//! ```text
//! the code every panel uses:
//! events.rs      the game's own event names: one write, many readers
//! api.rs         how the interface addresses state: unit tokens, slots
//! messages.rs    what the client tells the player, in the game's own words
//! plate.rs       an item's tooltip as lines of words and colours
//! cursor.rs      what the pointer is carrying: an item, a spell, money
//! target.rs      what the player is pointing at, and everything that changes it
//! object.rs      what a click on a game object does: a door, a chest, a vein
//!
//! the panels:
//! action.rs      the action bar, the cooldowns, the cast in progress, the swing
//! auras.rs       the buff bar and the target frame's aura rows
//! bank.rs        the bank window
//! binder.rs      making an inn the hearthstone's home
//! channels.rs    chat channels: the file, the packets, the zone channels
//! chat.rs        what was said, on its way to the chat frame
//! death.rs       the death, corpse and resurrect popups
//! duel.rs        the duel popup, countdown and result
//! emotetext.rs   text emotes: `/dance` out, the sentence in
//! gossip.rs      the gossip window, and the "npc" unit token
//! items.rs       the bags and the paper doll's item slots
//! log.rs         the combat log's lines
//! logout.rs      Logout(), Quit() and the packets under them
//! loot.rs        the loot window
//! lootroll.rs    group loot rolls
//! mail.rs        the mailbox
//! merchant.rs    the vendor window
//! minimap.rs     where the minimap is looking
//! pagetext.rs    the text of signs, plaques and readable items
//! party.rs       the party roster and its verbs
//! pet.rs         the pet action bar
//! played.rs      `/played`
//! quest.rs       the quest log and the quest giver's windows
//! raid.rs        the raid roster
//! received.rs    the "You receive loot" lines
//! reputation.rs  the reputation panel
//! shapeshift.rs  the stance bar
//! skills.rs      the skills panel
//! social.rs      the friends, ignore and /who lists
//! spellbook.rs   the spellbook's pages
//! stable.rs      the stable window
//! stats.rs       the character sheet's stat events
//! summon.rs      the summon popup
//! supersede.rs   replacing lower spell ranks already on the action bar
//! talents.rs     the talent trees
//! taxi.rs        the flight map
//! timers.rs      the breath meter and the two bars beside it
//! trade.rs       the trade window
//! tradeskill.rs  the trade-skill and craft windows
//! trainer.rs     the training window
//! untrainer.rs   resetting a pet's skills
//! vitals.rs      the unit frames' health, power and level events
//! worldmap.rs    where the character is, and which world map is showing
//! ```
//!
//! ## The boundary with FrameXML
//!
//! The 1.12.1 client is a C core that owns the state and a Lua and XML
//! interface that draws it. The boundary between them is three mechanisms:
//!
//! ```text
//! the API       C functions the interface calls   api.rs                UnitHealth("target")
//! the events    what the interface waits for       events.rs             "PLAYER_TARGET_CHANGED"
//! the bindings  a key -> a name -> a verb          input/bindings.rs     "ACTIONBUTTON1"
//! ```
//!
//! The names in all three are the game's own, read from the archives
//! (`vale extract 'Interface\FrameXML\Bindings.xml'` and the other files),
//! not chosen here. An addon's `frame:RegisterEvent("PLAYER_TARGET_CHANGED")`
//! passes a string, and the only names it can match are the ones the game
//! shipped.
//!
//! ## The boundary with `assets`
//!
//! A decision that needs no renderer is a rule and is in `vale-assets`, where
//! it is unit-tested with no window and where the CLI checks the same code:
//! [`vale_assets::tables::spellbook::resolve_aim`] (what a cast is aimed at)
//! and [`vale_assets::tables::faction::can_attack`] (what may be attacked).
//! What is here is the client's own state: a ray against a pick box, which key
//! is down, which clock a cooldown runs on.
//!
//! ## Leaving the world
//!
//! The state here belongs to one session: the target is a guid on a map the
//! player has left, the bar is the last character's, the cooldown clocks run
//! against a `GetTime` that keeps ticking. A second login overwrites most of it
//! within a second or two (`SMSG_ACTION_BUTTONS` refills the bar, the first
//! `SMSG_UPDATE_OBJECT` resolves the units again), so it looked correct. In
//! between, the screen showed the last character's state, and anything the
//! server does not send again stayed.
//!
//! Each module therefore resets its own resources when
//! [`events::PlayerLeavingWorld`] arrives. [`leaving`] writes that message once,
//! on the frame the session ends. `crate::render::residency::leave_world` does
//! the same for the world's entities.
//!
//! ## Ordering
//!
//! [`GameSet`] runs after the world has been placed. A mouse pick against the
//! previous frame's transforms misses a running creature by its own stride, and
//! a cast aimed at a selection cleared in the same frame sends a packet for a
//! dead guid. Inside the set, `crate::input::bindings::BindingSet` runs first,
//! because a verb reads binding messages; then the targeting chain; then the
//! action chain. Each module states the order within its own chain.

pub mod action;
pub mod api;
pub mod auras;
pub mod bank;
pub mod binder;
pub mod channels;
pub mod chat;
pub mod cursor;
pub mod death;
pub mod duel;
pub mod emotetext;
pub mod events;
pub mod gossip;
pub mod items;
pub mod log;
pub mod logout;
pub mod loot;
pub mod lootroll;
pub mod mail;
pub mod merchant;
pub mod messages;
pub mod minimap;
pub mod object;
pub mod pagetext;
pub mod party;
pub mod pet;
pub mod plate;
pub mod played;
pub mod quest;
pub mod raid;
pub mod received;
pub mod reputation;
pub mod shapeshift;
pub mod skills;
pub mod social;
pub mod spellbook;
pub mod stable;
pub mod stats;
pub mod summon;
pub mod supersede;
pub mod talents;
pub mod target;
pub mod taxi;
pub mod timers;
pub mod trade;
pub mod tradeskill;
pub mod trainer;
pub mod untrainer;
pub mod vitals;
pub mod worldmap;

use bevy::prelude::*;

/// The systems that turn input and packets into client state: those in this
/// directory, in `crate::input`, `crate::glue` and `crate::settings`, and the
/// state modules of `crate::world` that order themselves in it. `crate::ui`
/// and the interface's event dispatch order themselves after the whole set
/// rather than after whichever system they read.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameSet;

/// Every plugin in this directory, as one group. See
/// [`crate::render::RenderPlugins`] for why each directory registers its own.
pub struct InterfacePlugins;

impl Plugin for InterfacePlugins {
    fn build(&self, app: &mut App) {
        events::register(app);
        // `add_plugins` accepts tuples of at most sixteen, and the bound fails
        // on the whole call rather than on the seventeenth entry, so the list
        // is nested. The grouping follows the file tree above.
        app.add_plugins((
            (
                messages::MessagesPlugin,
                target::TargetPlugin,
                cursor::CursorPlugin,
                object::ObjectPlugin,
            ),
            (
                action::ActionPlugin,
                auras::AurasPlugin,
                spellbook::SpellbookPlugin,
                supersede::SupersedePlugin,
                pet::PetBarPlugin,
                shapeshift::ShapeshiftPlugin,
                log::CombatLogPlugin,
            ),
            (
                vitals::VitalsPlugin,
                stats::StatsPlugin,
                items::ItemsPlugin,
                received::ReceivedPlugin,
                timers::TimersPlugin,
                death::DeathPlugin,
                reputation::ReputationPlugin,
                skills::SkillsPlugin,
                talents::TalentsPlugin,
                tradeskill::TradeSkillPlugin,
            ),
            (
                gossip::GossipPlugin,
                merchant::MerchantPlugin,
                trainer::TrainerPlugin,
                untrainer::UntrainerPlugin,
                stable::StablePlugin,
                binder::BinderPlugin,
                bank::BankPlugin,
                mail::MailPlugin,
                pagetext::PageTextPlugin,
                taxi::TaxiPlugin,
                quest::QuestPlugin,
                loot::LootPlugin,
                lootroll::LootRollPlugin,
                trade::TradePlugin,
            ),
            (
                chat::ChatPlugin,
                channels::ChannelsPlugin,
                emotetext::EmoteTextPlugin,
                party::PartyPlugin,
                raid::RaidPlugin,
                social::SocialPlugin,
                duel::DuelPlugin,
                summon::SummonPlugin,
                played::PlayedPlugin,
                logout::LogoutPlugin,
            ),
            (worldmap::WorldMapPlugin, minimap::MinimapPlugin),
        ))
        // First in the set, so every reader of the message, here and in
        // `crate::lua`, sees it in the frame the session ended rather than the
        // frame after.
        .add_systems(
            Update,
            leaving
                .in_set(GameSet)
                .before(crate::input::bindings::BindingSet),
        );
    }
}

/// Write [`events::PlayerLeavingWorld`] once, on the frame the session ends.
///
/// It is written on the change from in the world to not in the world, not
/// while there is no world. Every reader resets something: a message written
/// on every frame with no session would clear the action bar sixty times a
/// second at the character screen, and again on the frames between
/// `CMSG_PLAYER_LOGIN` and the world arriving. The `Local` holds the previous
/// frame's answer.
fn leaving(
    session: Res<crate::world::session::Session>,
    mut was_in_world: Local<bool>,
    mut out: MessageWriter<events::PlayerLeavingWorld>,
) {
    if left_world(session.active.is_some(), &mut was_in_world) {
        out.write(events::PlayerLeavingWorld);
    }
}

/// Whether the session ended this frame, from this frame's answer and the
/// previous frame's.
///
/// A separate function because an [`crate::world::session::ActiveSession`]
/// owns a socket and cannot be built in a test, and this rule has a wrong
/// answer on both sides: `true` every frame clears the bar sixty times a second
/// at the character screen, and `false` always leaves the last character's
/// state for the next one.
fn left_world(in_world: bool, was_in_world: &mut bool) -> bool {
    let left = *was_in_world && !in_world;
    *was_in_world = in_world;
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every system in the client's state directories can run beside every
    /// other one.
    ///
    /// A `SystemParam` conflict (Bevy's B0002) is a panic at run time, not a
    /// compile error. A system that takes `ResMut<X>` beside a bundle holding
    /// `Res<X>` compiles, passes every unit test and every `--audit` probe, and
    /// then stops the client on the first frame with a window. It happened when
    /// [`api::Units`] gained a `Res<Party>` while two systems in two files were
    /// already writing that resource.
    ///
    /// The test initialises the schedules without running them, because
    /// `SystemParam::init_state` is where the panic happens, before any system
    /// body runs. Running one `update()` would also work, but would need every
    /// resource these systems read (an input plugin, a `Solids`, a
    /// `MouseFocus`, a dozen messages), a list that goes out of date every time
    /// a system gains a parameter.
    ///
    /// `render/` and the entity passes of `world/` are not covered, because
    /// their plugins need a GPU. A conflict in either still reaches a window
    /// before anything reports it.
    #[test]
    fn every_game_system_can_run_beside_every_other() {
        use bevy::ecs::schedule::Schedules;
        let mut app = App::new();
        app.add_plugins((
            bevy::time::TimePlugin,
            bevy::asset::AssetPlugin::default(),
        ))
        .insert_resource(crate::assets::GameAssets::new(String::new()))
        .insert_resource(crate::world::session::ClientConfig(
            vale_config::Config::default(),
        ))
        .init_resource::<crate::world::session::Session>()
        .add_plugins((
            crate::input::InputPlugins,
            crate::settings::SettingsPlugins,
            InterfacePlugins,
            crate::glue::GluePlugins,
            crate::world::StatePlugins,
        ));

        let mut schedules = app
            .world_mut()
            .remove_resource::<Schedules>()
            .expect("the app has schedules");
        for (_, schedule) in schedules.iter_mut() {
            schedule
                .initialize(app.world_mut())
                .expect("every system's parameters can be built together");
        }
    }

    /// The message is written once, when the session ends.
    #[test]
    fn leaving_the_world_is_an_edge_rather_than_a_state() {
        let mut was = false;
        // The login screen, however long it lasts.
        assert!(!left_world(false, &mut was));
        assert!(!left_world(false, &mut was));
        // In the world, and staying there.
        assert!(!left_world(true, &mut was));
        assert!(!left_world(true, &mut was));
        // Leaving: once.
        assert!(left_world(false, &mut was));
        assert!(!left_world(false, &mut was));
        // And again for a second session, which a one-shot flag would miss: the
        // character screen, a second login, a second logout.
        assert!(!left_world(true, &mut was));
        assert!(left_world(false, &mut was));
    }
}
