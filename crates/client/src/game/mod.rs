//! **What the player can do**: pick a target, swing at it, cast at it.
//!
//! ```text
//! the four mechanisms the interface is written against, and the queue behind
//! them — these are the directory's own and belong to no one subject:
//! events.rs    the game's own event names — one write, N readers
//! api.rs       …and how the interface addresses any of it: tokens, slots
//! bindings.rs  a key is not an action: the key table, and the names it maps to
//! messages.rs  …and what to tell the player, in the game's own words
//! plate.rs     …and an item's tooltip as lines of words and colours, which the
//!              interface's GameTooltip draws and any other painter can
//! cvars.rs     …and what the player has *set*: the settings every options
//!              panel writes, mirrored out of the interpreter
//! savedvars.rs …and its twin, for the forty-four rows of that panel that are
//!              *not* CVars: `RegisterForSave` and `config-cache.wtf`
//! incoming.rs  …and the one place a packet becomes any of the above
//!
//! combat/      pressing a button, and what happens when it lands
//! character/   what the character *is*: vitals, stats, bags, timers, death
//! npc/         everything a right-click on somebody else opens
//! place/       where the character is, and what happens when that changes
//! session/     getting in, being in it with other people, getting out
//! ```
//!
//! Until this directory existed the client could *watch* a fight in complete
//! detail — a swing, a flinch, a wind-up held while a cast bar ran, a missile,
//! a burst, an aura — and could not start one. Everything here is the other
//! direction: an input becomes a packet, and what comes back becomes state.
//!
//! ## This is the client's half of FrameXML, and it is shaped like it on purpose
//!
//! The real client is a C core that owns the state and a Lua/XML interface that
//! draws it, and the boundary between them is not a suggestion — it is three
//! specific mechanisms, all of which are now here:
//!
//! ```text
//! the API      C functions the interface calls    api.rs      UnitHealth("target")
//! the events   what the interface waits on        events.rs   "PLAYER_TARGET_CHANGED"
//! the bindings a key -> a name -> a verb          bindings.rs "ACTIONBUTTON1"
//! ```
//!
//! **The names in all three are the game's own, extracted from the archives**
//! rather than chosen here — `vale extract 'Interface\FrameXML\Bindings.xml'`
//! and its siblings, the same move that put `GlobalStrings.lua` behind every
//! message this client shows. That matters for one concrete reason: an addon's
//! `frame:RegisterEvent("PLAYER_TARGET_CHANGED")` is a *string*, and the only
//! names it can ever match are the ones the game shipped. Inventing a tidier set
//! would mean translating at the boundary forever.
//!
//! **The Lua host arrived and it binds to exactly these**, which is the claim
//! this directory was making: [`crate::lua`] runs the archive's own
//! `Bindings.xml` bodies and its registered verbs produce
//! [`bindings::Binding`] values, so the whole of `action.rs` and `target.rs`
//! changed by nothing at all when an interpreter appeared underneath them. What
//! is still not here is the widget tree — that is the interface's own half.
//!
//! ## The split against `ui/` is state versus pixels, and it is load-bearing
//!
//! Nothing in here draws. [`target::Selection`] is a guid, [`action::ActionBar`]
//! is twelve resolved slots, [`events::UiErrorMessage`] is a finished line — and
//! the frames, bars and buttons that show them are in `ui/`, which is getting
//! replaced wholesale. **This half is not.** A target frame written in egui is
//! scaffolding; the rule that a cast with no target commits against the caster is
//! not, and it would have to be written again if it lived inside the widget that
//! happened to need it first.
//!
//! ## …and the split against `assets/` is the same rule one level down
//!
//! The two decisions here that are *rules* rather than plumbing are both in
//! `vale-assets`, where they can be unit-tested with no window and where
//! `vale spellbook` checks the same copy this runs:
//! [`vale_assets::tables::spellbook::resolve_aim`] (what a cast is aimed at) and
//! [`vale_assets::tables::faction::can_attack`] (what may be attacked). What is left
//! here is genuinely the client's: a ray against a pick box, which key is down,
//! and which clock a cooldown runs on.
//!
//! ## Leaving the world is a state change like any other
//!
//! A session ends and this directory's state is about the session: the target is
//! a guid on a map nobody is on any more, the bar is the character's who logged
//! out, the cooldown clocks are running against a `GetTime` that will keep
//! ticking. None of it was reset until [`events::PlayerLeavingWorld`], and the
//! reason it was never *visibly* wrong is that a second login overwrites most of
//! it within a second or two — `SMSG_ACTION_BUTTONS` refills the bar, the first
//! `SMSG_UPDATE_OBJECT` re-resolves the units. "Most of it" and "within a second
//! or two" are the two halves of the bug: what is on screen in between is the
//! last character's, and anything the server does not re-send stays for ever.
//!
//! So each module resets **its own** resources off that one message, which is
//! the same shape [`crate::render::residency::leave_world`] has for the world's
//! entities and the same reason: the directory that owns the state owns its
//! teardown, and the signal is one message with N readers rather than N systems
//! each testing the session.
//!
//! ## Ordering
//!
//! One set, [`GameSet`], after the world has been placed — a mouse pick against
//! last frame's transforms misses a running mob by its own stride, and a cast
//! aimed at a selection cleared this frame is a packet sent at a dead guid.
//! Inside it, [`bindings::BindingSet`] runs first (a verb reads binding
//! *messages*, so they have to have been written), then the targeting chain, then
//! the action chain. The chain within each module is stated there.

pub mod api;
pub mod bindings;
pub mod cvars;
pub mod events;
pub mod incoming;
pub mod messages;
pub mod plate;
pub mod savedvars;

pub mod character;
pub mod combat;
pub mod npc;
pub mod place;
pub mod session;
use bevy::prelude::*;

/// Everything in this directory, so `ui/` can order itself after the whole of it
/// rather than after whichever system it happens to read from.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct GameSet;

/// Every pass in this directory, as one plugin — see
/// [`crate::render::RenderPlugins`] for why the registration lives here rather
/// than in `lib.rs`.
pub struct GamePlugins;

impl Plugin for GamePlugins {
    fn build(&self, app: &mut App) {
        events::register(app);
        app.add_plugins((
            bindings::BindingsPlugin,
            // **Paired to stay inside `add_plugins`' sixteen**, and they belong
            // together: one says what the client tells the player, the other
            // what the player has told the client.
            // **Three rather than two now**, and the third is the second's
            // twin: `cvars` owns `WTF\Config.wtf` and `savedvars` owns
            // `config-cache.wtf`, which is where the *other* forty-four rows of
            // the options panel live. See `savedvars`' own first line.
            (
                messages::MessagesPlugin,
                cvars::CVarsPlugin,
                savedvars::SavedVariablesPlugin,
                session::cameracache::CameraCachePlugin,
                // …and the third file of the same kind, which is neither
                // CVars nor uvars: the key bindings, per account and per
                // character. See `session::keybindings`.
                session::keybindings::KeybindingsPlugin,
                // …and the fourth: which addons each character has on,
                // per character. See `session::addons`.
                session::addons::AddonsPlugin,
            ),
            combat::target::TargetPlugin,
            combat::action::ActionPlugin,
            // **Paired in a nested tuple to stay inside `add_plugins`' sixteen**
            // — see the note further down, which this list has already hit once.
            // They belong together anyway: one builds the book, the other
            // repairs the bar the book is read against.
            (
                combat::spellbook::SpellbookPlugin,
                // …and the bar's stale ranks, repaired once per login — see
                // [`combat::supersede`], which is this client's own repair
                // rather than anything 1.12 does.
                combat::supersede::SupersedePlugin,
                // …and the *other* bar, which the server states outright where
                // this one's is the client's own — see [`combat::pet`].
                combat::pet::PetBarPlugin,
                // …and the *third*, which no packet mentions at all: the stance
                // buttons are a read of `Spell.dbc` over the book the first
                // plugin here builds — see [`combat::shapeshift`].
                combat::shapeshift::ShapeshiftPlugin,
            ),
            // **Paired to stay inside `add_plugins`' sixteen**, which this
            // list has now hit four times. They belong together anyway: one is
            // where the character is being told to go and the other is the
            // parchment that says where they are.
            (
                // …and the character's own controls, which are bindings now
                // rather than raw keys — see `place::controls`, and note the
                // ordering it states against `bindings::BindingSet`.
                place::controls::ControlsPlugin,
                place::worldmap::WorldMapPlugin,
            ),
            (
                character::vitals::VitalsPlugin,
                // …and what the *log* says about the fight, which is a reader
                // of the world rather than a writer and has no ordering of its
                // own — see [`combat::log`]. Paired with the vitals to stay
                // inside `add_plugins`' sixteen, which this list has now hit
                // four times; the pairing is arithmetic and says so.
                combat::log::CombatLogPlugin,
            ),
            combat::auras::AurasPlugin,
            character::stats::StatsPlugin,
            // …and what the character is *carrying*, which is the other half of
            // the sheet and the whole of the bags — see [`items`].
            character::items::ItemsPlugin,
            // …and what is on the *pointer*, which is the one piece of state
            // every drag in the interface is written against — see [`cursor`].
            combat::cursor::CursorPlugin,
            // **Paired to stay inside `add_plugins`' sixteen**, which this
            // list has now hit three times. They are not one subject; the
            // pairing is arithmetic and says so.
            (
                // …and the population a right click could never reach at all:
                // the doors, the chests and the ore veins — see
                // [`npc::object`].
                npc::object::ObjectPlugin,
                session::chat::ChatPlugin,
            ),
            // …and the way out, which is the one verb in this directory whose
            // answer arrives a round trip later — see [`logout`].
            session::logout::LogoutPlugin,
            // …and the way back *in*, whose two clocks are the client's own —
            // see [`death`].
            character::death::DeathPlugin,
            // **A nested tuple, and it is not a style choice.** `add_plugins`
            // is implemented for tuples up to sixteen and this list was at
            // exactly sixteen; the seventeenth does not fail where it is
            // written, it fails as an unsatisfied `Plugins<_>` bound on the
            // whole call. Nesting costs nothing and moves the ceiling.
            (
                // …and the bars the *server* counts down while you are in the
                // water — the one clock in this directory that is nobody's here
                // at all; see [`timers`].
                character::timers::TimersPlugin,
                // …and the one outcome of walking into a dungeon portal that
                // has to be *said* — see [`areatrigger`], where the rest of the
                // subject is deliberately not.
                place::areatrigger::AreaTriggerPlugin,
                // …and the two screens before any of the above exists — see
                // [`glue`], which is this directory's shape one screen earlier.
                session::glue::GluePlugin,
                // …and the way past both of those screens for a caller that
                // already knows who it wants to be — see [`autologin`]. Idle
                // unless something asks, so a player still gets the screens.
                session::autologin::AutoLoginPlugin,
                // …and the third of those screens, which is the only one with a
                // decision in it — see [`charcreate`], and note that its state
                // is deliberately not in this directory.
                session::charcreate::CharCreatePlugin,
                // …and what is on a body, which is the one panel in this
                // directory whose whole content arrives in a single packet —
                // see [`loot`]. **Paired for the arithmetic reason above**, and
                // they belong together: the second decides who gets a row the
                // first is already drawing, and it is the only thing that ever
                // makes such a row clickable — see [`lootroll`], whose whole
                // `rollID` is this client's own invention.
                (npc::loot::LootPlugin, npc::lootroll::LootRollPlugin),
                // …and the log the character carries and the conversation in
                // front of a giver, which are two different kinds of state in
                // one subject — see [`quest`].
                npc::quest::QuestPlugin,
                // …and the windows a right-click opens, **nested for the
                // arithmetic reason the tuple above is**: `add_plugins`'
                // sixteen is a bound on the whole call, and the mailbox was
                // the seventeenth entry here.
                //
                // The two an NPC owns — see [`gossip`] and [`merchant`], and
                // the `"npc"` token the first of them derives for all four
                // panels; the third, which is the same shape one door along and
                // whose panel is the game's own load-on-demand addon — see
                // [`trainer`]; and the fourth, which **no person owns at all**:
                // the box on the corner, whose window this client opens with no
                // packet behind it — see [`mail`].
                (
                    npc::gossip::GossipPlugin,
                    npc::merchant::MerchantPlugin,
                    npc::trainer::TrainerPlugin,
                    npc::mail::MailPlugin,
                    // …and the window two *players* open at each other, which
                    // borrows the same token — see [`session::trade`].
                    session::trade::TradePlugin,
                    // …and the fifth, which is the trainer's shape again with
                    // one difference: its four verbs share one answer byte that
                    // does not say which of them it answers, so the window
                    // re-asks — see [`npc::stable`].
                    npc::stable::StablePlugin,
                    // …and the sixth, which has no window of its own at all:
                    // the innkeeper's *"make this inn your home"* is a gossip
                    // option whose answer arrives after the gossip window has
                    // already closed, and is a popup rather than a panel — see
                    // [`npc::binder`].
                    npc::binder::BinderPlugin,
                    npc::pagetext::PageTextPlugin,
                    // …and its twin one trainer over: resetting a pet's skills
                    // is a gossip option whose confirm also arrives after the
                    // window has closed — see [`npc::untrainer`].
                    npc::untrainer::UntrainerPlugin,
                    // …and the bank, whose window is a guid and whose
                    // contents are the inventory's own fields — see
                    // [`npc::bank`].
                    npc::bank::BankPlugin,
                ),
                // …and the one edge none of the above can see: an item template
                // arriving, which changes no field anybody polls and is the
                // whole of why a first-seen item has no name — see
                // [`templates`], which is the reference's own cache callback.
                character::templates::TemplatesPlugin,
                // …and the one packet that says something *arrived* in a bag,
                // which no field does — see [`character::received`].
                character::received::ReceivedPlugin,
                // …and where the *little* map is looking, which is four facts
                // the widget cannot know and the world already holds — see
                // [`minimap`].
                place::minimap::MinimapPlugin,
                // …and who else is in the group, which is the one unit subject
                // whose members may not be in the world at all — see [`party`].
                // **Paired for the same reason the tuples above are**, and
                // they belong together: the second is the first's roster with
                // one byte different, and the join the server does not send —
                // see [`raid`].
                (
                    session::party::PartyPlugin,
                    session::raid::RaidPlugin,
                    session::social::SocialPlugin,
                    session::channels::ChannelsPlugin,
                    session::emotetext::EmoteTextPlugin,
                    // …and the three things another player can ask of us that
                    // are not a window: a duel, a summon — and `/played`,
                    // which is the server rather than a player and is here
                    // because it is the same shape, a question and an event.
                    session::duel::DuelPlugin,
                    session::summon::SummonPlugin,
                    session::played::PlayedPlugin,
                ),
                // …and the way *out* of a zone, which is the one window in this
                // directory whose whole content the client works out for itself
                // — see [`taxi`].
                npc::taxi::TaxiPlugin,
                // …and what is on the screen while the far side of any of it
                // arrives, which is the one subject here that is *not* about
                // the world's state but about whether the player may see it —
                // see [`loading`].
                place::loading::LoadingPlugin,
                // …and the one entry here that is an *instrument* rather than a
                // subject: what this client measured when the server refused
                // for distance — see [`combat::desync`], which exists because
                // "everything says out of range" has three causes and no check
                // in this project could tell them apart.
                // **Nested again, and for the fourth time** — see the note on
                // the tuple above. `add_plugins`' sixteen is a bound on the
                // whole call, so the seventeenth entry fails at `mod.rs` and
                // not where it is written.
                (
                    combat::desync::DesyncPlugin,
                    // …and what the character has *earned*, which is the one
                    // panel here whose whole shape is a client-side rule: the
                    // wire carries 64 pairs of numbers and nothing about rows,
                    // headings or order — see [`character::reputation`].
                    character::reputation::ReputationPlugin,
                    // …and what it has *learned*, which is the one subject in
                    // this directory with no packet of its own at all — see
                    // [`character::skills`].
                    character::skills::SkillsPlugin,
                    // …and what it *chose*, which is the same again one step
                    // further: the talent trees are read entirely out of two
                    // DBCs and the known-spell set, and the only number the
                    // server sends is how many points are left — see
                    // [`character::talents`].
                    character::talents::TalentsPlugin,
                    // …and what it can *make*: the trade-skill and craft
                    // windows, whose rows are the known-spell set crossed with
                    // the bags and whose thresholds are `SkillLineAbility.dbc`
                    // — see [`character::tradeskill`].
                    character::tradeskill::TradeSkillPlugin,
                    // …and what it may *hold*, which is one packet per item
                    // class and is in no shipped file — see
                    // [`character::proficiency`].
                    character::proficiency::ProficiencyPlugin,
                    // …and what a *talent* does to a spell's numbers, which is
                    // two packets and is likewise in no file — see
                    // [`combat::spellmods`].
                    combat::spellmods::SpellModPlugin,
                ),
            ),
        ))
        // **First in the set**, so that every reader of it — in this directory
        // and in `lua/` — sees the news in the frame the session ended rather
        // than the frame after.
        .add_systems(Update, leaving.in_set(GameSet).before(bindings::BindingSet));
    }
}

/// Raise [`events::PlayerLeavingWorld`] once, on the frame the session goes.
///
/// **Keyed on an edge, not on a state**, because every reader of it resets
/// something: a message written on every frame with no session would clear the
/// bar sixty times a second at the character screen, and — worse — would clear
/// it again on the frames between `CMSG_PLAYER_LOGIN` and the world arriving.
/// The `Local` is the previous frame's answer and nothing else.
fn leaving(
    session: Res<crate::world::session::Session>,
    mut was_in_world: Local<bool>,
    mut out: MessageWriter<events::PlayerLeavingWorld>,
) {
    if left_world(session.active.is_some(), &mut was_in_world) {
        out.write(events::PlayerLeavingWorld);
    }
}

/// **The edge itself**, as a function of this frame's answer and last frame's.
///
/// Separate from the system because an [`crate::world::session::ActiveSession`]
/// owns a socket and cannot be forged in a test — so the *rule* would otherwise
/// be the one part of the teardown nothing checked, and it is the part with a
/// wrong answer on either side of it: `true` every frame clears the bar sixty
/// times a second at the character screen, and `false` always leaves the last
/// character's state on the next one's.
fn left_world(in_world: bool, was_in_world: &mut bool) -> bool {
    let left = *was_in_world && !in_world;
    *was_in_world = in_world;
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every system in this directory can run beside every other one.**
    ///
    /// A `SystemParam` conflict is Bevy's B0002 and it is a **runtime** panic,
    /// not a compile error — so a system that takes `ResMut<X>` beside a bundle
    /// holding `Res<X>` compiles, passes every unit test in the workspace,
    /// passes all four `--audit` probes, and then kills the client on the first
    /// frame with a window open.
    ///
    /// That is not hypothetical: [`api::Units`] grew a `Res<Party>` when the
    /// party arrived, and two systems in two files were already writing that
    /// resource. Nothing in this repo could have said so until this test
    /// existed.
    ///
    /// **It initialises the schedules rather than running them**, which is
    /// exactly where the check lives: `SystemParam::init_state` is what panics,
    /// and it happens before any body executes. Running one `update()` instead
    /// would work too and would then need every resource half this directory
    /// reads — an input plugin, a `Solids`, a `MouseFocus`, a dozen messages —
    /// which is a list that rots every time a system gains a parameter, for no
    /// extra coverage of the thing being checked.
    ///
    /// **`render/` and `world/` are not covered**, and that is a gap rather than
    /// a decision: `RenderPlugins` wants a GPU. The same mistake made in either
    /// of those would still reach a window before anything said so.
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
        .add_plugins(GamePlugins);

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

    /// **Announced once, and only on the way out.**
    #[test]
    fn leaving_the_world_is_an_edge_rather_than_a_state() {
        let mut was = false;
        // The login screen, however long it goes on.
        assert!(!left_world(false, &mut was));
        assert!(!left_world(false, &mut was));
        // In, and staying in.
        assert!(!left_world(true, &mut was));
        assert!(!left_world(true, &mut was));
        // Out — once.
        assert!(left_world(false, &mut was));
        assert!(!left_world(false, &mut was));
        // …and again on the next session, which is the case a one-shot latch
        // would get wrong: a character screen, a second login, a second logout.
        assert!(!left_world(true, &mut was));
        assert!(left_world(false, &mut was));
    }
}
