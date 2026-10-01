//! The interpreter: one Lua 5.1 state, the game's interface code running in it,
//! and the handling of calls to functions this client does not implement.
//!
//! ## Which standard libraries are opened
//!
//! 1.12 opens base, `string`, `table` and `math`, and does not open `io`, `os`,
//! `package` or `debug`. `Interface\AddOns\` holds third-party code downloaded
//! from the internet and run in-process, so a library that reaches the
//! filesystem must not be available to it. This host opens the same four.
//!
//! Two globals are added: `getglobal` and `setglobal`. Stock 5.1 does not have
//! them and FrameXML cannot load without them. See
//! [`super::widgets::frames::install`].
//!
//! ## The libraries are also exposed as flat globals
//!
//! 1.12's Lua reaches those three libraries as bare globals: `format`, not
//! `string.format`; `strlen`, `floor`, `tinsert`, `getn`, `mod`. The whole
//! directory is written that way and stock 5.1 has none of the names. Without
//! [`FLATTENED`], every `format(…)` in the interface raises `attempt to call a
//! nil value`; there are 141 call sites for that one name, and each error aborts
//! the body it is in, so whole panels stop working.
//!
//! ## `LuaHost` is a non-send resource
//!
//! `mlua::Lua` is not `Sync`, so it lives as a Bevy non-send resource and every
//! system that touches it runs on the main thread. A Lua state has one caller,
//! so a mutex around the interpreter would add cost and no benefit.
//!
//! ## `LuaHost::run` is the only entry point for running a chunk
//!
//! [`LuaHost::run`] is the only place a chunk is executed, and it takes an
//! [`Answers`]. The reason is in [`super::api`]: the reads are scoped functions
//! that exist for the length of one call and are destroyed after it, so a chunk
//! run outside a scope would find `UnitHealth` pointing at a destroyed callback.
//! With one entry point, no chunk can run outside a scope.
//!
//! A verb records a request and the caller drains it after the call; a read
//! answers immediately.
//!
//! ## Each failure is reported once
//!
//! 234 bindings call 122 distinct functions and this client implements a
//! fraction of them, so most keys, if bound, would raise `attempt to call a nil
//! value` at key-repeat rate. [`LuaHost::missing`] records the function name the
//! first time and ignores repeats, and the count is shown on the HUD. The name
//! is recorded rather than dropped so that a key that does nothing can be
//! matched to the missing function.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use bevy::prelude::*;

use super::api::{self, Answers};
use super::widgets::frames;
use super::api::verbs::{self, Queue};
use super::xml;
use crate::input::bindings::Binding;
use crate::interface::events::{EventArg, FIRED};
#[cfg(feature = "diagnostics")]
use crate::ui::report::{HudReport, Slot};

/// The HUD line this file writes. See [`crate::ui::report`] for the
/// convention. 30 sits under the sky and celestial lines and above residency's
/// 90.
#[cfg(feature = "diagnostics")]
const REPORT: Slot = Slot(30);

/// How many distinct failures to keep. A cap, because a broken addon could
/// otherwise name a new function on every frame and this set is unbounded.
const MAX_FAILURES: usize = 64;

/// Which of the game's two interface directories is loaded.
///
/// 1.12 ships two, and only one is loaded at a time:
/// `Interface\GlueXML\` from the moment the window opens until a character
/// enters the world, and `Interface\FrameXML\` from then until the session ends.
/// Each has its own `.toc`, strings file, fonts and `$parent` root
/// (`GlueParent` or `UIParent`). Both declare `MasterFont` and
/// `GameFontNormal`, so loading them into one state would leave whichever came
/// second holding the other's typefaces.
///
/// The whole `mlua::Lua` is discarded on leaving the world
/// ([`unload_interface`]), and the fresh state loads the glue directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Directory {
    /// `Interface\GlueXML\` — the login screen and character select.
    Glue,
    /// `Interface\FrameXML\` — everything after `CMSG_PLAYER_LOGIN`.
    Frame,
}

/// What the character-create screen has chosen, copied out of the Lua state
/// once a frame. See [`LuaHost::char_create_choice`].
///
/// Holds the values the packet and the renderer use: `race` and `class` are
/// `ChrRaces` and `ChrClasses` ids and `gender` is `PLAYER_BYTES`' 0 and 1. The
/// interface uses different numbering; [`super::panels::charcreate`] converts
/// between the two.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharCreateChoice {
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    /// Skin, face, hair style, hair colour, facial hair — `PLAYER_BYTES` order,
    /// which is `CMSG_CHAR_CREATE`'s.
    pub appearance: [u8; 5],
    /// The same appearance as the type `vale_assets::look::dress` takes, so the
    /// model on the screen is dressed by the same call the world uses.
    pub look: vale_assets::look::character::Appearance,
    /// The items the model wears: on this screen, the class's starting outfit.
    /// See [`vale_assets::tables::charcreate::CharCreate::outfit`].
    pub outfit: Vec<vale_assets::tables::charcreate::OutfitPiece>,
}

impl Directory {
    /// The `.toc` to load for this directory.
    fn toc(self) -> &'static str {
        match self {
            Directory::Glue => vale_assets::interface::toc::GLUEXML_TOC,
            Directory::Frame => vale_assets::interface::toc::FRAMEXML_TOC,
        }
    }
}

/// The Lua state, the game's binding table, and the queue verbs write to.
///
/// Non-send: taken as `NonSendMut<LuaHost>`, which keeps its readers on the
/// main thread. See the module comment.
pub struct LuaHost {
    lua: mlua::Lua,
    queue: Queue,
    /// Chat lines, for the one verb whose argument is a sentence. See
    /// [`verbs::Said`]. Separate from `queue` because [`Binding`] is `Copy` and
    /// a chat line is a `String`, and because a different system drains it.
    said: verbs::SaidQueue,
    /// Text emotes. See [`verbs::Emoted`].
    emoted: verbs::EmoteQueue,
    /// The one pet verb that carries a name. See [`verbs::PetRenameQueue`].
    renamed: verbs::PetRenameQueue,
    /// `Bindings.xml`, once the archives are open. `None` until then, which
    /// covers the whole login screen.
    bindings: Option<std::sync::Arc<vale_assets::interface::bindings::Bindings>>,
    /// The world map's three writes. See [`super::panels::worldmap`].
    map: super::panels::worldmap::MapQueue,
    /// The interface's four sound verbs. See [`super::api::sound`].
    sound: super::api::sound::SoundQueue,
    /// The settings the interface writes. See [`super::api::cvars`]. Every
    /// options panel in the game is three calls over this store, so the
    /// options panels' effects come out of this queue.
    cvars: super::api::cvars::CVarQueue,
    /// The second settings store, which is neither a queue nor CVars: the
    /// names `RegisterForSave` has been given. See [`super::api::savedvars`];
    /// forty-four of the options panel's rows are these rather than CVars.
    saved: super::api::savedvars::SavedNames,
    /// The saved values read from the file, held so that a registration can
    /// apply its own value. See [`super::api::savedvars`], whose module note
    /// explains the ordering.
    saved_values: super::api::savedvars::SavedValues,
    /// The addon board: every addon the folder and the archives carry, which of
    /// them each character has enabled, which have loaded, and their saved
    /// files until they load. See [`super::panels::addons`]. Seeded by
    /// `crate::settings::addons`; read by [`Self::load_interface`] for the
    /// load order.
    addons: super::panels::addons::Held,
    /// Whether `SavedVariables.lua` has been applied to this host yet.
    ///
    /// Kept on the host rather than beside the file because a session builds
    /// more than one host: the glue directory is swapped for
    /// `Interface\FrameXML\` on the way into the world and back again on the
    /// way out. A one-shot flag kept in the resource fired on the login screen's
    /// host and never on the world's, so every `uvar` in the options panel
    /// reverted at login. It is `false` at construction, so every fresh host
    /// asks to be seeded. See [`crate::settings::savedvars`], and
    /// [`super::super::game::session::keybindings`], which had the same bug and
    /// the same fix.
    saved_applied: bool,
    /// The party's six verbs. See [`super::panels::party`].
    party: super::panels::party::Queue,
    /// The raid's nine verbs. A separate queue from the party's because a
    /// separate module drains it. See [`super::panels::raid`] and
    /// [`crate::interface::raid`].
    raid: super::panels::raid::Queue,
    /// The reputation panel's state. It is state rather than a queue for the
    /// same reason as [`super::panels::charcreate`]'s board:
    /// `ReputationBar_OnClick` re-reads the whole list inside the handler that
    /// changed it. The three writes that send a packet go in the queue beside
    /// it.
    reputation: super::panels::reputation::Held,
    reputation_queue: super::panels::reputation::Queue,
    /// The social panel's state and queue, held for the same reason. See
    /// [`super::panels::social`].
    social: super::panels::social::Held,
    social_queue: super::panels::social::Queue,
    /// The chat channels' state and queue. See [`super::panels::channels`].
    channels: super::panels::channels::Held,
    channels_queue: super::panels::channels::Queue,
    /// The skills panel's state, held for the same reason. It has no queue
    /// because none of its writes sends a packet. See
    /// [`super::panels::skills`], which explains why `AbandonSkill` is absent.
    skills: super::panels::skills::Held,
    /// The key bindings' state. It is state because it is the table the
    /// keyboard reads: there is one copy in the client, and both [`Self::fire`]
    /// and the panel read it. See [`super::panels::keybindings`].
    keybindings: super::panels::keybindings::Held,
    /// The two screens before the world. See [`super::panels::glue`]. Logging
    /// in needs a socket, so `DefaultServerLogin` can only record a request.
    glue: super::panels::glue::GlueQueue,
    /// The character-create screen: one request queue and one board. See
    /// [`super::panels::charcreate`]. The board is state rather than a queue
    /// because every read on that screen must answer inside the handler that
    /// wrote it.
    char_create: super::panels::charcreate::Held,
    char_create_queue: super::panels::charcreate::Queue,
    /// Looting a body. See [`super::panels::loot`].
    loot: super::panels::loot::Queue,
    /// Group loot rolls: which way the player rolled. See
    /// [`super::panels::lootroll`].
    loot_roll: super::panels::lootroll::Queue,
    /// The quest conversation. See [`super::panels::quest`].
    quest: super::panels::quest::Queue,
    /// The two windows a right-click on an NPC opens. See
    /// [`super::panels::gossip`] and [`super::panels::merchant`].
    gossip: super::panels::gossip::Queue,
    merchant: super::panels::merchant::Queue,
    /// The mailbox window, which has no NPC behind it. See
    /// [`super::panels::mail`].
    mail: super::panels::mail::Queue,
    /// The trainer window. See [`super::panels::trainer`].
    trainer: super::panels::trainer::Queue,
    /// The stable master's four verbs. See [`super::panels::stable`].
    stable: super::panels::stable::Queue,
    /// The bank's two verbs. See [`super::panels::bank`].
    bank: super::panels::bank::Queue,
    /// The page window's three verbs. See [`super::panels::pagetext`].
    pagetext: super::panels::pagetext::Queue,
    /// The trade window's ten verbs. See [`super::panels::trade`].
    trade: super::panels::trade::Queue,
    /// The duel's four verbs. See [`super::panels::duel`].
    duel: super::panels::duel::Queue,
    /// The inspect window's three verbs and `CanInspect`'s refusal line. See
    /// [`super::panels::inspect`].
    inspect: super::panels::inspect::Queue,
    /// The talent panel's one verb, drained by
    /// [`crate::interface::talents`].
    talent: super::panels::talent::Queue,
    /// The flight master's map. See [`super::panels::taxi`].
    taxi: super::panels::taxi::Queue,
    /// The unit frames' portraits. See [`super::api::portrait`]. Drawing a
    /// portrait needs a camera and a model, which belong to the renderer and
    /// not to this state.
    portrait: super::api::portrait::PortraitQueue,
    /// The five reads the directory stores rather than calls. See
    /// [`api::Held`] for the reason.
    held: api::Held,
    /// Verb names the interface asked for and this client does not have, and
    /// handler errors, recorded once each. See the module comment for why they
    /// are recorded.
    missing: BTreeSet<String>,
    /// The text a Ctrl-C asked to put on the clipboard, until the frame's
    /// keyboard system takes it. See [`LuaHost::take_copied`]. A plain field
    /// rather than an `Rc<RefCell<_>>` queue like the others, because the only
    /// writer is [`LuaHost::keyboard`] and no Lua closure holds it.
    copied: Option<String>,
    /// What loading a directory of markup produced, and which directory it
    /// was. `None` before either has been loaded.
    ///
    /// One pair rather than two fields because only one directory is loaded at
    /// a time: this client runs `Interface\GlueXML\` before there is a world and
    /// `Interface\FrameXML\` after, and discards the state between them. See
    /// [`LuaHost::load_glue`].
    interface: Option<(Directory, xml::Report)>,
    /// The heap size [`LuaHost::pace_collector`] recorded on its last call, so
    /// that the next call can measure how much was allocated since. A `Cell`
    /// because pacing takes `&self` and is called from a system that has one.
    settled: Cell<usize>,
    /// The heap size below which [`LuaHost::pace_collector`] does nothing:
    /// [`LuaHost::live_floor`] plus [`LuaHost::GC_PAUSE_BYTES`], set when a
    /// cycle finishes. Zero while a cycle is in progress.
    paused_below: Cell<usize>,
    /// The smallest heap seen after a full collection or a finished cycle: an
    /// estimate of the live set. Set from the full collection that ends each
    /// interface load, and lowered by any finished cycle that ends below it.
    live_floor: Cell<usize>,
    /// The layout, pile and paint generations as the last walk left them, for
    /// [`LuaHost::drawn_if_changed`]. `None` forces the next walk.
    painted: Cell<Option<(i64, i64, i64)>>,
}

impl LuaHost {
    /// A fresh state with the four libraries 1.12 opens, this client's verbs
    /// registered, and the frame object model installed.
    ///
    /// The reads are not registered here, because they borrow the world. See
    /// [`LuaHost::run`].
    pub fn new() -> mlua::Result<LuaHost> {
        let lua = mlua::Lua::new_with(
            mlua::StdLib::STRING | mlua::StdLib::TABLE | mlua::StdLib::MATH,
            mlua::LuaOptions::default(),
        )?;
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let said: verbs::SaidQueue = Rc::new(RefCell::new(Vec::new()));
        let emoted: verbs::EmoteQueue = Rc::new(RefCell::new(Vec::new()));
        let renamed: verbs::PetRenameQueue = Rc::new(RefCell::new(Vec::new()));
        let map: super::panels::worldmap::MapQueue = Rc::new(RefCell::new(Vec::new()));
        let sound: super::api::sound::SoundQueue = Rc::new(RefCell::new(Vec::new()));
        let cvars: super::api::cvars::CVarQueue = Rc::new(RefCell::new(Vec::new()));
        let saved: super::api::savedvars::SavedNames = Rc::new(RefCell::new(Vec::new()));
        let saved_values: super::api::savedvars::SavedValues = Rc::new(RefCell::new(Vec::new()));
        let party: super::panels::party::Queue = Rc::new(RefCell::new(Vec::new()));
        let raid: super::panels::raid::Queue = Rc::new(RefCell::new(Vec::new()));
        let reputation: super::panels::reputation::Held = Rc::default();
        let skills: super::panels::skills::Held = Rc::default();
        let keybindings: super::panels::keybindings::Held = Rc::default();
        let addons: super::panels::addons::Held = Rc::default();
        let reputation_queue: super::panels::reputation::Queue = Rc::new(RefCell::new(Vec::new()));
        let social: super::panels::social::Held = Rc::default();
        let social_queue: super::panels::social::Queue = Rc::new(RefCell::new(Vec::new()));
        let channels: super::panels::channels::Held = Rc::default();
        let channels_queue: super::panels::channels::Queue = Rc::new(RefCell::new(Vec::new()));
        let glue: super::panels::glue::GlueQueue = Rc::new(RefCell::new(Vec::new()));
        let char_create: super::panels::charcreate::Held = Rc::default();
        let char_create_queue: super::panels::charcreate::Queue = Rc::new(RefCell::new(Vec::new()));
        let portrait: super::api::portrait::PortraitQueue = Rc::new(RefCell::new(Vec::new()));
        let loot: super::panels::loot::Queue = Rc::new(RefCell::new(Vec::new()));
        let loot_roll: super::panels::lootroll::Queue = Rc::new(RefCell::new(Vec::new()));
        let quest: super::panels::quest::Queue = Rc::new(RefCell::new(Vec::new()));
        let gossip: super::panels::gossip::Queue = Rc::new(RefCell::new(Vec::new()));
        let merchant: super::panels::merchant::Queue = Rc::new(RefCell::new(Vec::new()));
        let mail: super::panels::mail::Queue = Rc::new(RefCell::new(Vec::new()));
        let trainer: super::panels::trainer::Queue = Rc::new(RefCell::new(Vec::new()));
        let stable: super::panels::stable::Queue = Rc::new(RefCell::new(Vec::new()));
        let bank: super::panels::bank::Queue = Rc::new(RefCell::new(Vec::new()));
        let pagetext: super::panels::pagetext::Queue = Rc::new(RefCell::new(Vec::new()));
        let trade: super::panels::trade::Queue = Rc::new(RefCell::new(Vec::new()));
        let duel: super::panels::duel::Queue = Rc::new(RefCell::new(Vec::new()));
        let inspect: super::panels::inspect::Queue = Rc::new(RefCell::new(Vec::new()));
        let talent: super::panels::talent::Queue = Rc::new(RefCell::new(Vec::new()));
        let taxi: super::panels::taxi::Queue = Rc::new(RefCell::new(Vec::new()));
        let held: api::Held = Rc::new(RefCell::new(api::HeldReads::default()));
        api::install_held(&lua, &held)?;
        // Before the flatten, so that `tremove` is the 5.0 body too.
        restore_5_0_table_remove(&lua)?;
        flatten_libraries(&lua)?;
        verbs::register(&lua, &queue, &said, &renamed, &emoted)?;
        // The map's three writes work like the chat verbs: a handler cannot
        // move the view directly, because the world is borrowed for the length
        // of the call, so it records a request and `interface::worldmap`
        // applies it. See `super::panels::worldmap`.
        super::panels::worldmap::register(&lua, &map)?;
        // The four sound verbs, drained by `crate::sound::interface`.
        super::api::sound::register(&lua, &sound)?;
        // The client's settings, drained by `crate::settings::cvars`.
        // Registered early because it seeds the store as well as registering
        // the three globals, and an `OnLoad` that reads a CVar runs after both.
        super::api::cvars::register(&lua, &cvars)?;
        // The second settings store, used by the interface's `uvar` rows where
        // the CVar store serves `cvar` rows. See [`super::api::savedvars`].
        super::api::savedvars::register(&lua, &saved, &saved_values)?;
        // The party's six verbs, drained by `crate::interface::party`.
        super::panels::party::register(&lua, &party)?;
        // The raid's nine verbs, drained by `crate::interface::raid`.
        super::panels::raid::register(&lua, &raid)?;
        // The reputation panel's eleven functions. The state is held here and
        // `crate::interface::reputation` drains the three packets.
        super::panels::reputation::register(&lua, &reputation, &reputation_queue)?;
        // The social panel's sixteen functions. The state is held here and
        // `crate::interface::social` drains the six packets.
        super::panels::social::register(&lua, &social, &social_queue)?;
        // The chat channels' twenty-five functions. The board is held here and
        // `crate::interface::channels` drains the two packets.
        super::panels::channels::register(&lua, &channels, &channels_queue)?;
        // The skills panel's eleven functions, over a board the same module
        // feeds.
        super::panels::skills::register(&lua, &skills)?;
        // The key bindings panel's eight functions. Their board is also what
        // [`Self::fire`] resolves a key press through. `RunBinding`, the ninth,
        // is not registered here: it runs a `<Binding>` body, which needs this
        // type rather than the board, and `crate::input::bindings` registers it.
        super::panels::keybindings::register(&lua, &keybindings)?;
        // The addon list's sixteen functions, over a board
        // `crate::settings::addons` seeds. Unscoped, because `LoadAddOn` runs
        // the loader itself and the loader cannot run inside a scoped read.
        super::panels::addons::register(&lua, &addons)?;
        // The glue screens' functions, drained by `crate::glue::glue`.
        // Registered unconditionally rather than only while the glue is loaded:
        // nothing in `Interface\FrameXML\` calls them, so they cost nothing
        // there, and registering and removing them with the screen would be a
        // second piece of state to keep in step with the directory swap.
        //
        // After the sound verbs, because `PlayGlueMusic` forwards to
        // `PlayMusic`.
        super::panels::glue::register(&lua, &glue)?;
        // The character-create screen's functions. Its reads are unscoped
        // because none of them touches the world: the available choices come
        // entirely from `vale_assets::tables::charcreate`.
        super::panels::charcreate::register(&lua, &char_create, &char_create_queue)?;
        // The unit frames' portrait function, drained by
        // `crate::render::portraits`. Placed before the object model by
        // convention only (it registers a global, not a method). The ordering
        // rule that matters is that [`super::api::stubs`] goes last, so a stub
        // name cannot shadow a real function.
        super::api::portrait::register(&lua, &portrait)?;
        // The loot window's two verbs, drained by `crate::interface::loot`.
        super::panels::loot::register(&lua, &loot)?;
        // The roll frame's two verbs, drained by `crate::interface::lootroll`.
        super::panels::lootroll::register(&lua, &loot_roll)?;
        // The quest panel's verbs, drained by `crate::interface::quest`.
        super::panels::quest::register(&lua, &quest)?;
        // The gossip and merchant windows' verbs, drained by their own modules.
        super::panels::gossip::register(&lua, &gossip)?;
        super::panels::merchant::register(&lua, &merchant)?;
        // The mail window's ten verbs, drained by `crate::interface::mail`.
        super::panels::mail::register(&lua, &mail)?;
        // The trainer's seven verbs, drained by `crate::interface::trainer`.
        super::panels::trainer::register(&lua, &trainer)?;
        // The stable's four verbs, drained by `crate::interface::stable`.
        super::panels::stable::register(&lua, &stable)?;
        // The bank's two verbs, drained by `crate::interface::bank`.
        super::panels::bank::register(&lua, &bank)?;
        // The page window's three verbs, drained by `crate::interface::pagetext`.
        super::panels::pagetext::register(&lua, &pagetext)?;
        // The trade window's ten verbs, drained by `crate::interface::trade`.
        super::panels::trade::register(&lua, &trade)?;
        // The duel's four verbs, drained by `crate::interface::duel`.
        super::panels::duel::register(&lua, &duel)?;
        // The inspect window's three verbs, drained by
        // `crate::interface::inspect`.
        super::panels::inspect::register(&lua, &inspect)?;
        // The talent panel's one verb. See [`super::panels::talent`] for the
        // five reads beside it and why they are scoped.
        super::panels::talent::register(&lua, &talent)?;
        // The flight map's three verbs, drained by `crate::interface::taxi`.
        super::panels::taxi::register(&lua, &taxi)?;
        frames::install(&lua)?;
        // After the object model, because two of these take a frame, and last,
        // so that a stub name cannot shadow a real verb.
        super::api::stubs::install(&lua)?;
        Ok(LuaHost {
            lua,
            queue,
            said,
            emoted,
            renamed,
            held,
            map,
            sound,
            cvars,
            saved,
            saved_values,
            addons,
            saved_applied: false,
            party,
            raid,
            glue,
            reputation,
            reputation_queue,
            social,
            social_queue,
            channels,
            channels_queue,
            skills,
            keybindings,
            char_create,
            char_create_queue,
            portrait,
            loot,
            loot_roll,
            quest,
            gossip,
            merchant,
            mail,
            trainer,
            stable,
            bank,
            pagetext,
            trade,
            duel,
            inspect,
            talent,
            taxi,
            bindings: None,
            missing: BTreeSet::new(),
            copied: None,
            interface: None,
            settled: Cell::new(0),
            paused_below: Cell::new(0),
            live_floor: Cell::new(usize::MAX),
            painted: Cell::new(None),
        })
    }

    /// Whether the game's own binding table has been read yet.
    pub fn loaded(&self) -> bool {
        self.bindings.is_some()
    }

    /// Load all of `Interface\FrameXML\`. Called once, at the first login.
    ///
    /// Runs inside a scope like every other chunk, because `OnLoad` handlers
    /// call reads: `TargetFrame_OnLoad` and its neighbours call them, and
    /// outside a scope those names are nil. For the same reason this cannot run
    /// at host construction, when there is no world to answer from.
    ///
    /// The [`xml::Report`] is kept for the HUD and its errors are added to the
    /// same `missing` set a binding's are, so the one HUD line counts every
    /// call the interface made that this client could not answer.
    pub fn load_interface(
        &mut self,
        answers: &dyn Answers,
        read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    ) {
        // The typefaces are installed first, because the load measures text
        // with them. An `OnLoad` that sizes a tab to its label (six of them do)
        // asks how wide its text is before this function returns, so faces
        // installed afterwards would leave those six sized by the fallback
        // estimate for the whole session. See [`super::widgets::text::width`].
        super::widgets::text::install_faces(&self.lua, vale_assets::interface::font::Faces::load(read));
        let addons = Rc::clone(&self.addons);
        let report = self
            .run(answers, |lua| {
                // Load `FrameXML`, then every addon the board says to load, in
                // the board's order. The seven shipped `Blizzard_*` addons come
                // first and load at login, a departure from 1.12 that
                // `assets::toc`'s constants document; the folder's addons
                // follow, each after its dependencies. Every addon uses the same
                // template registry `FrameXML` filled, so its `inherits`
                // resolve. See [`xml::Loader::load_tocs`]. Each addon's load
                // runs its saved files and raises `ADDON_LOADED` for it; that is
                // done in [`super::panels::addons::load_with`] so that
                // `LoadAddOn` from Lua does the same thing.
                //
                // A board nothing has seeded (a bare test host) loads
                // `FrameXML` alone.
                let mut report = xml::Loader::new(read).load_toc(lua, Directory::Frame.toc());
                let order = addons.borrow().load_order();
                for name in order {
                    match super::panels::addons::load_with(lua, &addons, &name, read) {
                        Ok(loaded) => report.merge(loaded),
                        Err(reason) => {
                            report.errors.insert(format!("{name}: not loaded ({})", reason.key()));
                        }
                    }
                }
                // The first of three events the load raises. The 1.12.1
                // client fires `UPDATE_CHAT_WINDOWS` at login once the windows
                // exist, and `FloatingChatFrame_Update` applies a window's
                // colour, alpha and shown state only on that event. Without it
                // every chat texture stays untinted, and an untinted
                // `ChatFrameBackground` is an opaque white rectangle. Raised
                // inside the same scope, so the handlers' reads answer.
                report
                    .errors
                    .extend(frames::fire(lua, "UPDATE_CHAT_WINDOWS", &[])?);
                // The second event, `UPDATE_CHAT_COLOR`, once per chat type.
                // `ChatTypeInfo["SAY"]` ships with `sticky` and nothing else:
                // a type's `r`, `g` and `b` are written only by
                // `ChatFrame_OnEvent`'s `UPDATE_CHAT_COLOR` branch, and
                // `AddMessage` with a nil colour draws white. Without this,
                // every chat line (system messages, whispers, loot) is white.
                // The colour table is in `assets::interface::chattype`, which
                // the combat log's routing also indexes (94 rows).
                //
                // The ten numbered channels take `CHANNEL`'s colour.
                // `ChatTypeInfo["CHANNEL1".."CHANNEL10"]` are rows in the
                // directory with no colour in the table or in the 1.12.1
                // client's `chat-cache.txt`, and a channel line is drawn under
                // `"CHANNEL"..arg8`, so without these ten every channel line
                // is white.
                let channel = vale_assets::interface::chattype::TYPES
                    .iter()
                    .find(|chat| chat.name == "CHANNEL")
                    .map(|chat| chat.colour);
                let numbered: Vec<(String, [u8; 3])> = channel
                    .into_iter()
                    .flat_map(|colour| {
                        (1..=super::panels::channels::SLOTS).map(move |n| (format!("CHANNEL{n}"), colour))
                    })
                    .collect();
                let rows = vale_assets::interface::chattype::TYPES
                    .iter()
                    .map(|chat| (chat.name.to_string(), chat.colour))
                    .chain(numbered);
                for (name, [r, g, b]) in rows {
                    report.errors.extend(frames::fire(
                        lua,
                        "UPDATE_CHAT_COLOR",
                        &[
                            EventArg::Text(name),
                            EventArg::Number(f64::from(r) / 255.0),
                            EventArg::Number(f64::from(g) / 255.0),
                            EventArg::Number(f64::from(b) / 255.0),
                        ],
                    )?);
                }
                // Channels are sticky in this client, as a deliberate
                // deviation. The shipped `ChatTypeInfo["CHANNEL"]` is
                // `{ sticky = 0 }` and the 1.12.1 client never changes the
                // field, so 1.12's `ChatEdit_OnEnterPressed` sends `/2 hello`
                // and opens the next line as a say. This client keeps the
                // channel, and `/s` switches back. One field in the
                // directory's table, set after it loads.
                lua.load("ChatTypeInfo.CHANNEL.sticky = 1").exec()?;
                // The third event, `RAID_ROSTER_UPDATE`, needed because this
                // client loads the `Blizzard_*` addons at login.
                // `Blizzard_RaidUI` is the first of the five whose frames are
                // shown by default: forty `RaidGroupButton`s parented to
                // `RaidFrame`, hidden only by `RaidGroupFrame_Update`'s `else`
                // branch. In 1.12, `RaidFrame_OnEvent` loads that addon and
                // calls `RaidFrame_Update()` on the next line. Here the addon
                // loads at login, and `RaidFrame_OnLoad`, which would call the
                // update, runs before the addon exists. Without this event a
                // solo character's Raid tab shows forty empty buttons stacked
                // at the origin. `RAID_ROSTER_UPDATE` is the event
                // `RaidFrame_OnEvent` loads the addon on.
                report
                    .errors
                    .extend(frames::fire(lua, "RAID_ROSTER_UPDATE", &[])?);
                Ok(report)
            })
            .unwrap_or_default();
        for error in &report.errors {
            self.note(error.clone());
        }
        self.interface = Some((Directory::Frame, report));
        // One full collection after the load. The load's
        // temporary allocations are tens of megabytes, and the paced
        // collection that runs from then on ([`Self::pace_collector`]) does
        // too little work per call to clear them. A full collection here
        // starts pacing from the live set, during the loading transition.
        let _ = self.lua.gc_collect();
        self.reset_collector_floor();
    }

    /// Load `Interface\GlueXML\`: the login screen and character select.
    ///
    /// The same code as [`LuaHost::load_interface`] with the other `.toc`. Only
    /// one of the two directories is loaded at a time; see [`Directory`].
    ///
    /// `FRAMES_LOADED` is raised at the end. `GlueParent_OnEvent`'s first branch
    /// answers it by calling `LocalizeFrames()`, which is
    /// `GlueLocalization.lua`'s pass over the screens' captions and anchors;
    /// without it both screens are drawn un-localised. It is raised here rather
    /// than through `interface::events`, like `UPDATE_CHAT_WINDOWS`, because it
    /// belongs to the load and there is no session to write a message from.
    ///
    /// `SET_GLUE_SCREEN` is not raised here. Nothing is shown until
    /// `crate::glue::glue`'s watcher sees which screen the session is on, on
    /// the next frame, so that one mechanism alone decides what is on screen.
    pub fn load_glue(
        &mut self,
        answers: &dyn Answers,
        read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    ) {
        super::widgets::text::install_faces(&self.lua, vale_assets::interface::font::Faces::load(read));
        let report = self
            .run(answers, |lua| {
                let mut report = xml::Loader::new(read).load_toc(lua, Directory::Glue.toc());
                report
                    .errors
                    .extend(frames::fire(lua, "FRAMES_LOADED", &[])?);
                Ok(report)
            })
            .unwrap_or_default();
        for error in &report.errors {
            self.note(error.clone());
        }
        self.interface = Some((Directory::Glue, report));
        let _ = self.lua.gc_collect();
        self.reset_collector_floor();
    }

    /// Record the heap right after a full collection as the live set that
    /// [`Self::pace_collector`] pauses above, and end any pause.
    fn reset_collector_floor(&self) {
        self.live_floor.set(self.lua.used_memory());
        self.paused_below.set(0);
        // A new interface: the first walk after a load always runs.
        self.painted.set(None);
    }

    /// Set the screen size in the state, before anything is loaded into it.
    ///
    /// [`LuaHost::drawn`] does this every frame, which is enough for layout,
    /// because a rectangle is solved on demand. It is not enough for values the
    /// interface computes once: an `OnLoad` that reads `GetScreenWidth()` and
    /// sizes a texture from it runs once, and without this call it would run
    /// against [`super::widgets::layout::DEFAULT_SCREEN`]. See
    /// [`super::widgets::layout::UI_SIZE`], which is the size passed instead,
    /// and [`super::widgets::layout::VIRTUAL_WIDTH`] for why that is a constant.
    pub fn set_screen(&self, width: f64, height: f64) {
        let _ = super::widgets::layout::set_screen(&self.lua, width, height);
    }

    /// What the last load produced, or `None` before there was one.
    pub fn interface(&self) -> Option<&xml::Report> {
        self.interface.as_ref().map(|(_, report)| report)
    }

    /// Which directory the last load was: `Some(Glue)` at a login screen,
    /// `Some(Frame)` in the world, `None` before either.
    pub fn directory(&self) -> Option<Directory> {
        self.interface.as_ref().map(|(which, _)| *which)
    }

    /// The state itself, for this directory's instruments: the audit's `--draw`
    /// dump walks tables the way [`LuaHost::drawn`] does. `pub(super)` because
    /// nothing outside `lua/` may hold the `mlua::Lua`, for the reason the
    /// module comment gives, and no caller given this reference may run a chunk
    /// with it.
    pub(super) fn state(&self) -> &mlua::Lua {
        &self.lua
    }

    /// The number of bytes the interpreter's heap holds, for an instrument
    /// outside this directory to report. [`Self::pace_collector`] paces against
    /// this number; see [`Self::GC_BEHIND_BYTES`] for where it settles.
    pub fn heap_bytes(&self) -> usize {
        self.lua.used_memory()
    }

    /// Everything the interface would draw this frame, back to front.
    ///
    /// This read is not a Lua call. It is a method here rather than a `lua()`
    /// accessor for the reason the module comment gives: if the rest of the
    /// client could hold the `mlua::Lua`, it could run a chunk outside a scope.
    /// This borrows the state, walks tables, and returns plain data. See
    /// [`super::widgets::draw`], which decides what is drawn and is testable
    /// with no window.
    pub fn drawn(&self, screen: (f32, f32), now: f64) -> Vec<super::widgets::draw::Item> {
        // The screen first, because every rectangle in the interface is measured
        // from `UIParent`, and `UIParent` is the screen.
        let _ = super::widgets::layout::set_screen(&self.lua, screen.0 as f64, screen.1 as f64);
        // Then the clock, which determines, for example, when an error message
        // is five seconds old. One timestamp a frame, read by `AddMessage` and
        // by the draw. See [`super::widgets::messages::set_now`].
        super::widgets::messages::set_now(&self.lua, now);
        super::widgets::draw::collect(&self.lua)
    }

    /// [`Self::drawn`], or `None` when the walk would return `last` unchanged.
    ///
    /// The walk is skipped when the layout, pile and paint generations are
    /// where the last walk left them and that walk drew nothing that changes
    /// with the clock alone (see `widgets::widget::mark_animating`). The screen
    /// and the clock are written first either way, since a changed screen bumps
    /// the layout generation.
    ///
    /// With `VALE_PAINT_VERIFY` set, and always in tests, a skipped walk is run
    /// anyway and compared with `last`. A difference means some write reached a
    /// field the walk reads without bumping a counter: it is logged once with
    /// the first differing item (a test panics), and the fresh list is returned.
    pub fn drawn_if_changed(
        &self,
        screen: (f32, f32),
        now: f64,
        last: &[super::widgets::draw::Item],
    ) -> Option<Vec<super::widgets::draw::Item>> {
        let _ = super::widgets::layout::set_screen(&self.lua, screen.0 as f64, screen.1 as f64);
        super::widgets::messages::set_now(&self.lua, now);
        let stamp = || {
            (
                super::widgets::layout::generation(&self.lua),
                super::widgets::widget::pile_generation(&self.lua),
                super::widgets::widget::paint_generation(&self.lua),
            )
        };
        let unchanged = !super::widgets::widget::animating(&self.lua)
            && self.painted.get() == Some(stamp());
        if unchanged {
            if !paint_verify() {
                return None;
            }
            let fresh = super::widgets::draw::collect(&self.lua);
            if fresh == last {
                return None;
            }
            report_stale_paint(last, &fresh);
            self.painted.set(Some(stamp()));
            return Some(fresh);
        }
        let items = super::widgets::draw::collect(&self.lua);
        // Read after the walk, which writes its own caches (the layout memo, a
        // button's worn font) and must not force the next walk by doing so.
        self.painted.set(Some(stamp()));
        Some(items)
    }

    /// The 3D scene the glue screens are showing as their backdrop, or `None`.
    ///
    /// A method here rather than a `lua()` accessor for the reason the module
    /// comment gives about [`LuaHost::drawn`]: nothing outside this directory
    /// may hold the `mlua::Lua`. It borrows the state, walks one small registry
    /// list, and returns plain data. See [`super::widgets::model`], which
    /// decides what is visible and is testable with no window.
    ///
    /// Always `None` while `Interface\FrameXML\` is loaded, checked here
    /// explicitly. `Interface\FrameXML\` does show `<Model>` frames in the
    /// world: `CooldownFrameTemplate` (`Cooldown.xml`) is a `<Model>` holding
    /// `Interface\Cooldown\UI-Cooldown-Indicator.mdx`, one instance per action
    /// button, and `CooldownFrame_SetTimer` shows it when anything goes on
    /// cooldown. The caller, `crate::render::glue`, builds a full-screen scene
    /// from whatever this returns, aims the world camera through the model's
    /// camera, and hides the sky, stars, moons and fog while it is up. Without
    /// this check, the first spell cast with a cooldown replaced the world with
    /// the cooldown indicator model.
    ///
    /// The check is on the directory rather than on a list of frame names
    /// because this answers a question about the screen: the backdrop of the two
    /// screens before the world. `Interface\FrameXML\`'s model frames (fourteen
    /// portraits, plus the cooldown indicators) are drawn inside their own
    /// rectangles, which this client does not do yet, and are not a scene.
    pub fn glue_scene(&self) -> Option<super::widgets::model::Scene> {
        if self.directory() != Some(Directory::Glue) {
            return None;
        }
        super::widgets::model::visible(&self.lua)
    }

    /// Pass the keyboard's modifier state to the interface. See
    /// [`super::api::stubs::set_modifiers`] for why.
    ///
    /// A method here rather than a free function so that nothing outside this
    /// directory holds the `mlua::Lua`, the rule the module comment states and
    /// [`LuaHost::drawn`] follows.
    pub fn set_modifiers(&self, shift: bool, control: bool, alt: bool) {
        super::api::stubs::set_modifiers(&self.lua, shift, control, alt);
    }

    pub fn set_bindings(&mut self, bindings: std::sync::Arc<vale_assets::interface::bindings::Bindings>) {
        // Both uses are set from one call. The declarations decide what a key
        // does (here) and what the key-bindings panel lists
        // (`keybindings::Keys`); setting only the first would draw an empty
        // panel over a working keyboard.
        self.keybindings
            .borrow_mut()
            .set_declarations(std::sync::Arc::clone(&bindings));
        self.bindings = Some(bindings);
    }

    /// The declaration for a name, if the game declares one.
    pub fn declaration(&self, name: &str) -> Option<&vale_assets::interface::bindings::BindingDecl> {
        self.bindings.as_ref()?.get(name)
    }

    /// The only entry point for running Lua. Opens a scope, lends the interface
    /// the world through [`Answers`], and runs `body` inside it.
    ///
    /// Every read the chunk makes is answered from `answers` during the call
    /// and every write it makes goes on the verb queue. Nothing else in this
    /// client may execute a chunk; see the module comment.
    pub(super) fn run<R>(
        &self,
        answers: &dyn Answers,
        body: impl FnOnce(&mlua::Lua) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        self.lua.scope(|scope| {
            let filled = api::install(&self.lua, scope, answers, &self.held, &self.queue);
            let outcome = filled.and_then(|()| body(&self.lua));
            // Cleared here, inside the scope, while its functions are still
            // alive. A store cleared after the scope closed would hold destroyed
            // callbacks for one statement, and a call through one in that
            // interval is an error the caller cannot handle. Also cleared after
            // a failed install, which is why the install result is chained with
            // `and_then` rather than returned with `?`. See [`super::scoped`].
            super::scoped::clear(&self.lua);
            outcome
        })
    }

    /// Run a binding's Lua body, and return the verbs it called.
    ///
    /// `down` becomes the `keystate` global, which the hundred `runOnUp`
    /// bodies branch on. A binding without `runOnUp` runs on the press only;
    /// the caller enforces that, since it knows which edge this is.
    ///
    /// An error is recorded and not propagated: a bad chunk must not stop the
    /// frame, and the cause is almost always a verb this client has not
    /// implemented.
    pub fn fire(&mut self, name: &str, down: bool, answers: &dyn Answers) -> Vec<Binding> {
        let Some(decl) = self.bindings.as_ref().and_then(|b| b.get(name)) else {
            // A key bound to a name the game does not declare. Recorded under
            // the binding name rather than a verb's, because nothing was
            // called.
            self.note(name.to_string());
            return Vec::new();
        };
        let body = decl.body.clone();
        self.queue.borrow_mut().clear();
        let run = self.run(answers, |lua| {
            lua.globals()
                .set("keystate", if down { "down" } else { "up" })?;
            // Compiled, then run through Lua's `pcall`. See
            // [`frames::protected`]. A key bound to a verb this client has not
            // implemented raises on every press, and once FrameXML is loaded a
            // failing call costs 8 µs through `pcall` against 31 ms through
            // `mlua::Function::call`.
            let body = lua
                .load(super::dialect::to_5_1(&body).as_ref())
                .set_name(name)
                .into_function()?;
            frames::protected(lua, &body)
        });
        if let Err(e) = run {
            self.note(format!("{name}: {}", first_line(&e)));
        }
        self.drain_swallowed();
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// Raise `event` on every frame that registered for it.
    ///
    /// A binding carries input from the player to the interface; an event
    /// carries state changes from the world to it. See
    /// [`super::widgets::frames::fire`] for the calling convention and the order.
    ///
    /// Returns the verbs the handlers called, on the same queue a binding body
    /// writes to, because a handler may call one. `TargetFrame.lua`'s `OnEvent`
    /// does not, but an addon's may, and dropping those verbs would make a verb
    /// work from a key and not from an event.
    pub fn fire_event(
        &mut self,
        event: &str,
        args: &[EventArg],
        answers: &dyn Answers,
    ) -> Vec<Binding> {
        self.fire_events(std::slice::from_ref(&(event, args)), answers)
    }

    /// Raise a whole frame's events at once, in one scope.
    ///
    /// Opening a scope re-installs the entire scoped read API, about three
    /// hundred `scope.create_function` calls and three hundred global writes,
    /// and costs 0.13 ms and 27 KB of Lua garbage each time, measured by the
    /// `api scope` line of `--audit --spin`, which reports that fixed cost
    /// separately from every other phase. A handler body costs about 0.03 ms.
    ///
    /// With one scope per event, the interface's cost grew with the number of
    /// events. A group of four changes seven watched units' health on the same
    /// tick ([`crate::interface::vitals`]' `WATCHED`), and seven scopes cost
    /// 0.9 ms and 190 KB of garbage for about 0.2 ms of handler work. Measured
    /// with `--audit --spin --party 4` before and after the change.
    ///
    /// A failing event does not stop the rest: `frames::fire` is called per
    /// event and its `Err` is recorded and skipped, so events after a failure in
    /// the middle of a batch are still delivered. The verbs come back as one
    /// list rather than per event; the caller writes each one as a
    /// `BindingPressed` in the order they were called.
    pub fn fire_events(
        &mut self,
        news: &[(&str, &[EventArg])],
        answers: &dyn Answers,
    ) -> Vec<Binding> {
        if news.is_empty() {
            return Vec::new();
        }
        self.queue.borrow_mut().clear();
        let outcome = self.run(answers, |lua| {
            let mut errors = Vec::new();
            for (event, args) in news {
                match frames::fire(lua, event, args) {
                    Ok(from_handlers) => errors.extend(from_handlers),
                    Err(e) => errors.push(format!("{event}: {}", first_line(&e))),
                }
            }
            Ok(errors)
        });
        match outcome {
            Ok(errors) => {
                for error in errors {
                    self.note(error);
                }
            }
            // The scope itself would not open, which is not about any one event.
            Err(e) => self.note(format!("events: {}", first_line(&e))),
        }
        self.drain_swallowed();
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// How many frames have an `OnUpdate` handler: the number the HUD shows,
    /// and the number of frames a tick walks.
    pub fn ticking(&self) -> usize {
        super::api::update::tracked(&self.lua)
    }

    /// Whether any frame has an `OnUpdate` handler.
    ///
    /// Checked once a frame before anything else, so that a client at the
    /// login screen, or one whose interface has not loaded, pays a single table
    /// read rather than opening a scope.
    pub fn has_updates(&self) -> bool {
        self.ticking() > 0
    }

    /// One tick of the interface's animation, in one scope: every visible
    /// frame's `OnUpdate` with the elapsed seconds in `arg1`, then every
    /// `<Model>`'s clock.
    ///
    /// Returns the verbs those bodies called, on the same queue a key's binding
    /// writes to, for the same reason as [`LuaHost::fire_event`]: a verb must
    /// work the same from an `OnUpdate` as from a key. `UIParent`'s handler
    /// calls `RequestBattlefieldPositions()` on every tick.
    ///
    /// Both walks share one scope because a scope costs about 0.13 ms and
    /// 27 KB of Lua garbage before any body runs (see [`Self::fire_events`] for
    /// the measurement), and both walks happen at the same instant of the same
    /// clock: `InterfaceClock::due` gates both.
    ///
    /// Model loading stays outside, in [`super::widgets::model`]'s own system,
    /// because it reads an archive and needs `UiModels` mutably, and neither
    /// belongs inside a Lua scope. That system is ordered `.before` this one,
    /// so a file is loaded by the time the tick asks how long its sequence is.
    pub fn fire_tick(
        &mut self,
        elapsed: f64,
        answers: &dyn Answers,
        models: &super::widgets::model::UiModels,
    ) -> Vec<Binding> {
        self.queue.borrow_mut().clear();
        let length_of = |path: &str, sequence: u32| models.sequence_length(path, sequence);
        let outcome = self.run(answers, |lua| {
            let mut errors = super::api::update::fire(lua, elapsed)?;
            errors.extend(super::widgets::model::tick(lua, elapsed, &length_of)?);
            Ok(errors)
        });
        match outcome {
            Ok(errors) => {
                for error in errors {
                    self.note(error);
                }
            }
            Err(e) => self.note(format!("OnUpdate: {}", first_line(&e))),
        }
        self.drain_swallowed();
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// Every visible frame's `OnUpdate`, with the elapsed seconds in `arg1`, in
    /// its own scope. [`LuaHost::fire_tick`] runs this and the model ticks in
    /// one scope.
    pub fn fire_updates(&mut self, elapsed: f64, answers: &dyn Answers) -> Vec<Binding> {
        self.queue.borrow_mut().clear();
        match self.run(answers, |lua| super::api::update::fire(lua, elapsed)) {
            Ok(errors) => {
                for error in errors {
                    self.note(error);
                }
            }
            Err(e) => self.note(format!("OnUpdate: {}", first_line(&e))),
        }
        self.drain_swallowed();
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// Which files the visible `<Model>` frames hold. See
    /// [`super::widgets::model::files`], whose caller loads them.
    pub fn model_files(&self) -> Vec<String> {
        super::widgets::model::files(&self.lua)
    }

    /// Every visible `<PlayerModel>` frame that is set to a unit, for
    /// [`crate::render::paperdoll`]. These are the paper dolls;
    /// [`Self::model_files`] returns the frames that show a model file.
    pub fn unit_models(&self) -> Vec<super::widgets::model::UnitFrame> {
        super::widgets::model::unit_frames(&self.lua)
    }

    /// Tick every visible `<Model>` frame: `OnUpdateModel` every frame, and
    /// `OnAnimFinished` when the sequence it is playing has ended.
    ///
    /// Works like [`LuaHost::fire_updates`] for a different script kind, and
    /// returns verbs for the same reason. `models` answers how long a sequence
    /// is; nothing else in this file knows what a model contains.
    pub fn fire_model_ticks(
        &mut self,
        delta: f64,
        answers: &dyn Answers,
        models: &super::widgets::model::UiModels,
    ) -> Vec<Binding> {
        self.queue.borrow_mut().clear();
        let length_of = |path: &str, sequence: u32| models.sequence_length(path, sequence);
        match self.run(answers, |lua| super::widgets::model::tick(lua, delta, &length_of)) {
            Ok(errors) => {
                for error in errors {
                    self.note(error);
                }
            }
            Err(e) => self.note(format!("OnUpdateModel: {}", first_line(&e))),
        }
        self.drain_swallowed();
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// Whether a mouse pass for `pointer` would change nothing; see
    /// `api::mouse::unchanged`. Reads the interpreter without opening a scope.
    pub fn mouse_unchanged(&self, pointer: &super::api::mouse::Pointer) -> bool {
        super::api::mouse::unchanged(&self.lua, pointer)
    }

    /// Pass the pointer to the interface, and return which frame it is over.
    ///
    /// The second element is what [`super::api::mouse::MouseFocus`] carries to
    /// `interface::target`: the world must not also act on a click the
    /// interface took. The third is whether a frame took the wheel; see
    /// [`super::api::mouse::wheel_was_taken`], which stops the camera zooming
    /// while a list is being scrolled.
    pub fn mouse(
        &mut self,
        pointer: &super::api::mouse::Pointer,
        answers: &dyn Answers,
    ) -> (Vec<Binding>, Option<String>, bool) {
        self.queue.borrow_mut().clear();
        let over = self.run(answers, |lua| {
            let errors = super::api::mouse::dispatch(lua, pointer)?;
            // Read back rather than computed again: the dispatch has just
            // decided it, and a second hit test could disagree with the one the
            // handlers were fired for.
            Ok((
                errors,
                super::api::mouse::focused_name(lua)?,
                super::api::mouse::wheel_was_taken(lua)?,
            ))
        });
        let (over, wheel) = match over {
            Ok((errors, over, wheel)) => {
                for error in errors {
                    self.note(error);
                }
                (over, wheel)
            }
            Err(e) => {
                self.note(format!("mouse: {}", first_line(&e)));
                (None, false)
            }
        };
        self.drain_swallowed();
        (std::mem::take(&mut *self.queue.borrow_mut()), over, wheel)
    }

    /// Pass keyboard strokes to the interface, and return which edit box has
    /// the focus afterwards.
    ///
    /// The keyboard counterpart of [`LuaHost::mouse`]: the strokes go to
    /// whichever [`super::widgets::editbox`] holds the focus, and the second
    /// element is what [`super::api::keyboard::KeyboardFocus`] carries to the
    /// key table, so that no binding fires for a key the interface took.
    ///
    /// Verbs are returned because a stroke runs the game's handlers:
    /// `OnEnterPressed` calls `SendChatMessage`, and `ChatEdit_ParseText`'s
    /// `SlashCmdList` entries call `TargetUnit` and similar functions.
    pub fn keyboard(
        &mut self,
        strokes: &[super::widgets::editbox::Stroke],
        answers: &dyn Answers,
    ) -> (Vec<Binding>, Option<String>) {
        self.queue.borrow_mut().clear();
        self.copied = None;
        let focus = self.run(answers, |lua| {
            let (errors, copied) = super::widgets::editbox::dispatch(lua, strokes)?;
            Ok((errors, super::widgets::editbox::focused_name(lua), copied))
        });
        let focus = match focus {
            Ok((errors, focus, copied)) => {
                for error in errors {
                    self.note(error);
                }
                self.copied = copied;
                focus
            }
            Err(e) => {
                self.note(format!("keyboard: {}", first_line(&e)));
                None
            }
        };
        self.drain_swallowed();
        (std::mem::take(&mut *self.queue.borrow_mut()), focus)
    }

    /// Pass key edges to whichever frame has the keyboard, and return the name
    /// of the frame that took one, if any.
    ///
    /// The middle case of the three in [`super::api::keyboard`]: an edit box
    /// with the focus takes a character, a frame with `enableKeyboard` takes a
    /// key, and a key neither takes goes to the key table. A `None` here lets
    /// `W` move the character when no panel is up.
    ///
    /// One scope for the whole frame's keys rather than one per key, for the
    /// reason given in [`Self::fire_events`]: a scope costs 0.12 ms and
    /// 27.8 KB, and a key repeat produces several edges.
    pub fn keys_to_frame(
        &mut self,
        keys: &[(String, bool)],
        answers: &dyn Answers,
    ) -> (Vec<Binding>, Option<String>) {
        self.queue.borrow_mut().clear();
        let outcome = self.run(answers, |lua| {
            let mut errors = Vec::new();
            let mut taken = None;
            for (key, down) in keys {
                if super::widgets::keyboard::deliver(lua, key, *down, &mut errors) {
                    taken = super::widgets::keyboard::receiver_name(lua);
                }
            }
            Ok((errors, taken))
        });
        let taken = match outcome {
            Ok((errors, taken)) => {
                for error in errors {
                    self.note(error);
                }
                taken
            }
            Err(e) => {
                self.note(format!("keyboard: {}", first_line(&e)));
                None
            }
        };
        self.drain_swallowed();
        (std::mem::take(&mut *self.queue.borrow_mut()), taken)
    }

    /// How many frames have ever declared themselves keyboard receivers: the
    /// count `--audit` prints. See [`super::widgets::keyboard`].
    pub fn keyboard_receivers(&self) -> usize {
        super::widgets::keyboard::receiver_count(&self.lua)
    }

    /// The keyboard receivers' names, with a `*` on the ones currently
    /// visible. The visible ones are the ones to check, since a receiver that
    /// is always visible would take every key away from the key table.
    pub fn keyboard_receiver_names(&self) -> Vec<String> {
        super::widgets::keyboard::receiver_names(&self.lua)
    }

    /// The frame holding the keyboard now, if any. Checked once a frame, so
    /// that an open panel releases the movement controls even on a frame where
    /// no key changed.
    ///
    /// Cheap: it reads a registry of the frames that declared themselves
    /// receivers rather than walking the frame tree. See
    /// [`super::widgets::keyboard`].
    pub fn keyboard_frame(&self) -> Option<String> {
        super::widgets::keyboard::receiver_name(&self.lua)
    }

    /// Take the text the last keystrokes asked to put on the clipboard.
    ///
    /// A Ctrl-C decides what is copied inside [`super::widgets::editbox`], which
    /// does not open the window system's clipboard; [`super::api::keyboard`]
    /// does the copying. `None` covers both "nothing was copied" and "nothing
    /// was selected", which the 1.12.1 client treats the same way: it leaves
    /// the clipboard unchanged.
    pub fn take_copied(&mut self) -> Option<String> {
        self.copied.take()
    }

    /// The name of the edit box holding the keyboard, if one is.
    ///
    /// Checked once a frame before any key is examined, so that a client with
    /// nothing focused, which is most of the time, pays one registry read
    /// rather than opening a scope.
    pub fn keyboard_focus(&self) -> Option<mlua::Table> {
        super::widgets::editbox::focused(&self.lua)
    }

    /// Take and clear the chat lines the interface asked the client to say.
    ///
    /// One caller, [`crate::interface::chat::send`], for the same reason
    /// `take_chat` has one caller: this empties the queue.
    pub fn take_said(&mut self) -> Vec<verbs::Said> {
        std::mem::take(&mut *self.said.borrow_mut())
    }

    /// Take and clear the requested text emotes. One caller,
    /// [`crate::interface::emotetext`].
    pub fn take_emoted(&mut self) -> Vec<verbs::Emoted> {
        std::mem::take(&mut *self.emoted.borrow_mut())
    }

    /// Take and clear the names typed into the pet rename box. See
    /// [`verbs::PetRenameQueue`]. One caller, [`crate::interface::pet`].
    pub fn take_pet_renames(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.renamed.borrow_mut())
    }

    /// Take and clear the world map requests. See
    /// [`super::panels::worldmap`]. One caller, [`crate::interface::worldmap`].
    pub fn take_map_requests(&mut self) -> Vec<super::panels::worldmap::MapRequest> {
        std::mem::take(&mut *self.map.borrow_mut())
    }

    /// Take and clear the party requests. See [`super::panels::party`]. One
    /// caller, [`crate::interface::party`].
    pub fn take_party_verbs(&mut self) -> Vec<super::panels::party::PartyRequest> {
        std::mem::take(&mut *self.party.borrow_mut())
    }

    /// Take and clear the raid requests. See [`super::panels::raid`]. One
    /// caller, [`crate::interface::raid`].
    pub fn take_raid_verbs(&mut self) -> Vec<super::panels::raid::RaidRequest> {
        std::mem::take(&mut *self.raid.borrow_mut())
    }

    /// Take and clear the sound requests. See [`super::api::sound`]. One
    /// caller, `crate::sound::interface`.
    pub fn take_sound_requests(&mut self) -> Vec<super::api::sound::SoundRequest> {
        std::mem::take(&mut *self.sound.borrow_mut())
    }

    /// Run a chunk, for a test in another module that needs a real interpreter
    /// rather than a bare `mlua::Lua`. The registry `RegisterForSave` writes
    /// into belongs to this type, so a round-trip test needs one.
    ///
    /// `#[cfg(test)]` so it cannot become a second way to drive the interface;
    /// the only way is `run_scripts`, which reports what it did.
    #[cfg(test)]
    pub(crate) fn run_for_test(&mut self, chunk: &str) {
        self.lua.load(chunk).exec().expect("the chunk runs");
    }

    /// Evaluate one expression and return it as a string, for tests.
    #[cfg(test)]
    pub(crate) fn eval_for_test(&self, expression: &str) -> String {
        self.lua
            .load(format!("return {expression}"))
            .eval::<String>()
            .expect("the expression reads")
    }

    /// The current value of every global passed to `RegisterForSave`. Read at
    /// logout, which is also when the 1.12.1 client saves them. See
    /// [`super::api::savedvars`], whose module note explains why this is not a
    /// queue.
    pub fn saved_variables(&self) -> Vec<(String, vale_assets::interface::wtf::SavedValue)> {
        super::api::savedvars::snapshot(&self.lua, &self.saved)
    }

    /// Each loaded addon's saved variables, serialised: `(addon, scope, file
    /// text)` for every addon that has loaded and whose `.toc` declares names
    /// at that scope. An addon that has not loaded is left out, so its file on
    /// disk is not overwritten with nothing. See
    /// [`super::api::savedvars::render_addon`].
    pub fn addon_saved_variables(&self) -> Vec<(String, super::panels::addons::Scope, String)> {
        let board = self.addons.borrow();
        let mut out = Vec::new();
        for addon in board.loaded_addons() {
            for (scope, names) in [
                (super::panels::addons::Scope::Account, &addon.saved),
                (super::panels::addons::Scope::Character, &addon.saved_per_character),
            ] {
                if !names.is_empty() {
                    out.push((
                        addon.name.clone(),
                        scope,
                        super::api::savedvars::render_addon(&self.lua, names),
                    ));
                }
            }
        }
        out
    }

    /// Pass an addon's saved file to this host. It runs now if the addon has
    /// loaded; otherwise it is held on the board until the addon loads,
    /// because an addon's saved variables are applied over its defaults, which
    /// do not exist before its files have run.
    pub fn apply_addon_saved_variables(&mut self, addon: &str, scope: super::panels::addons::Scope, text: &str) {
        let loaded = self.addons.borrow().is_loaded(addon);
        if loaded {
            if let Err(e) = super::api::savedvars::apply_addon_chunk(&self.lua, text) {
                warn!("SavedVariables\\{addon}.lua would not run ({e})");
            }
        } else {
            self.addons.borrow_mut().put_saved_file(addon, scope, text);
        }
    }

    /// The addon board, borrowed. See [`super::panels::addons`]. Seeded by
    /// `crate::settings::addons` and read by [`Self::load_interface`].
    pub fn addons(&self) -> &super::panels::addons::Held {
        &self.addons
    }

    /// Whether the saved variables still need to be applied to this host. See
    /// [`Self::saved_applied`], and [`super::super::game::session::keybindings`]'
    /// `Keys::needs_seeding`, which answers the same question for key bindings.
    ///
    /// The caller asks the host rather than keeping its own flag, because a
    /// flag outside the host outlives it and goes stale when the host is
    /// replaced.
    pub fn needs_saved_variables(&self) -> bool {
        !self.saved_applied
    }

    /// Apply saved variable values to this host, before `VARIABLES_LOADED`.
    ///
    /// Marks the host as applied even when the values are empty: a fresh
    /// install has nothing to apply, and asking again every frame for the whole
    /// session would never succeed.
    pub fn apply_saved_variables(
        &mut self,
        values: &[(String, vale_assets::interface::wtf::SavedValue)],
    ) {
        self.saved_applied = true;
        if let Err(e) =
            super::api::savedvars::apply(&self.lua, &self.saved, &self.saved_values, values)
        {
            warn!("the saved variables would not load ({e})");
        }
    }

    /// Take and clear every `SetCVar` since the last call. See
    /// [`super::api::cvars`]. One caller, [`crate::settings::cvars`].
    pub fn take_cvar_writes(&mut self) -> Vec<super::api::cvars::CVarWrite> {
        std::mem::take(&mut *self.cvars.borrow_mut())
    }

    /// The whole CVar store, read once when the settings resource is built.
    pub fn cvar_snapshot(&self) -> Vec<(String, String)> {
        super::api::cvars::snapshot(&self.lua)
    }

    /// Write the values from `WTF\Config.wtf` into the CVar store, once,
    /// before the interface loads.
    ///
    /// One caller, [`crate::settings::cvars`], from a `Startup` system, which
    /// is early enough because all of `Interface\` loads in `Update`. See
    /// [`super::api::cvars::seed`] for why this does not go on the queue.
    pub fn seed_cvars(&mut self, values: &[(String, String)]) {
        if let Err(e) = super::api::cvars::seed(&self.lua, values) {
            warn!("the saved settings would not load ({e})");
        }
    }

    /// Take and clear the login and character-select screens' requests. See
    /// [`super::panels::glue`]. One caller, [`crate::glue::glue`].
    pub fn take_glue_requests(&mut self) -> Vec<super::panels::glue::GlueRequest> {
        std::mem::take(&mut *self.glue.borrow_mut())
    }

    /// Take and clear the character-create screen's Accept presses. See
    /// [`super::panels::charcreate`]. One caller, [`crate::glue::charcreate`].
    pub fn take_create_requests(&mut self) -> Vec<super::panels::charcreate::CreateRequest> {
        std::mem::take(&mut *self.char_create_queue.borrow_mut())
    }

    /// Give the character-create screen the tables it chooses from.
    ///
    /// Called once when `Interface\GlueXML\` loads, because that is the first
    /// moment there is both an archive chain and a Lua state; the host itself
    /// is built before either. Until then every read on that screen returns an
    /// empty list, which draws a create screen with no buttons rather than one
    /// with wrong buttons.
    pub fn set_char_create_tables(
        &mut self,
        tables: std::sync::Arc<vale_assets::tables::charcreate::CharCreate>,
    ) {
        self.char_create.borrow_mut().tables = Some(tables);
    }

    /// Whether it already has them, so the supplying system asks the archives
    /// once per interface rather than once per frame.
    pub fn char_create_ready(&self) -> bool {
        self.char_create.borrow().tables.is_some()
    }

    /// What the create screen has chosen, for the preview model and for the
    /// packet.
    ///
    /// `(race id, class id, gender, appearance)` plus the composed
    /// [`vale_assets::look::character::Appearance`], copied out rather than
    /// borrowed: the caller is a Bevy system and this is a `RefCell` the Lua
    /// state can re-enter.
    pub fn char_create_choice(&self) -> CharCreateChoice {
        let board = self.char_create.borrow();
        CharCreateChoice {
            race: board.race_id(),
            class: board.class_id(),
            gender: board.gender,
            appearance: board.appearance(),
            look: board.look(),
            outfit: board.outfit(),
        }
    }

    /// Take and clear the requested portraits. See [`super::api::portrait`].
    /// One caller, [`crate::render::portraits`].
    pub fn take_portrait_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.portrait.borrow_mut())
    }

    /// The reputation panel's board, borrowed. See
    /// [`super::panels::reputation`]. One caller,
    /// [`crate::interface::reputation`], which writes the server's updates
    /// into it and reads the version back.
    pub fn reputation(&self) -> &super::panels::reputation::Held {
        &self.reputation
    }

    /// The key table, borrowed. See [`super::panels::keybindings`].
    ///
    /// Two callers: `crate::input::bindings`, which reads the live set to turn
    /// a keystroke into a binding name, and `crate::settings::keybindings`,
    /// which fills the saved sets from the two files at login and writes them
    /// back at logout.
    pub fn keybindings(&self) -> &super::panels::keybindings::Held {
        &self.keybindings
    }

    /// The skills panel's board, borrowed. See [`super::panels::skills`]. One
    /// caller, [`crate::interface::skills`].
    pub fn skills(&self) -> &super::panels::skills::Held {
        &self.skills
    }

    /// The social panel's board, borrowed. See [`super::panels::social`].
    /// One caller, [`crate::interface::social`], which writes the server's
    /// answers into it and reads what the panel asked for.
    pub fn social(&self) -> &super::panels::social::Held {
        &self.social
    }

    /// Take and clear the social panel's six verbs that send a packet.
    pub fn take_social_verbs(&mut self) -> Vec<vale_protocol::socket::session::SocialVerb> {
        std::mem::take(&mut *self.social_queue.borrow_mut())
    }

    /// The chat channels' board, borrowed. See [`super::panels::channels`].
    /// Two callers: [`crate::interface::channels`], which fills it, and
    /// [`crate::interface::chat`], which numbers a channel line from it.
    pub fn channels(&self) -> &super::panels::channels::Held {
        &self.channels
    }

    /// Take and clear the channel verbs the chat frame queued.
    pub fn take_channel_verbs(&mut self) -> Vec<vale_protocol::socket::session::ChannelVerb> {
        std::mem::take(&mut *self.channels_queue.borrow_mut())
    }

    /// Take and clear the reputation panel's three verbs that send a packet.
    pub fn take_reputation_verbs(
        &mut self,
    ) -> Vec<vale_protocol::socket::session::ReputationVerb> {
        std::mem::take(&mut *self.reputation_queue.borrow_mut())
    }

    /// Take and clear the loot window's item takes. See
    /// [`super::panels::loot`]. One caller, [`crate::interface::loot`].
    pub fn take_loot_presses(&mut self) -> Vec<crate::interface::loot::TakeLoot> {
        std::mem::take(&mut *self.loot.borrow_mut())
    }

    /// Take and clear the loot roll choices. See [`super::panels::lootroll`].
    /// One caller, [`crate::interface::lootroll`].
    pub fn take_roll_presses(&mut self) -> Vec<crate::interface::lootroll::RollPress> {
        std::mem::take(&mut *self.loot_roll.borrow_mut())
    }

    /// Take and clear the quest panel's presses. See [`super::panels::quest`].
    /// One caller, [`crate::interface::quest`].
    pub fn take_quest_presses(&mut self) -> Vec<crate::interface::quest::QuestPress> {
        std::mem::take(&mut *self.quest.borrow_mut())
    }

    /// Take and clear the gossip window's presses. This and
    /// `take_merchant_presses` have one caller each.
    pub fn take_gossip_presses(&mut self) -> Vec<crate::interface::gossip::GossipPress> {
        std::mem::take(&mut *self.gossip.borrow_mut())
    }

    pub fn take_merchant_presses(&mut self) -> Vec<crate::interface::merchant::MerchantPress> {
        std::mem::take(&mut *self.merchant.borrow_mut())
    }

    /// Take and clear the mail window's presses. See [`crate::interface::mail`].
    pub fn take_mail_presses(&mut self) -> Vec<crate::interface::mail::MailPress> {
        std::mem::take(&mut *self.mail.borrow_mut())
    }

    /// Take and clear the trainer window's presses. See
    /// [`crate::interface::trainer`].
    pub fn take_trainer_presses(&mut self) -> Vec<crate::interface::trainer::TrainerPress> {
        std::mem::take(&mut *self.trainer.borrow_mut())
    }

    /// Take and clear the stable's presses. See [`crate::interface::stable`].
    pub fn take_stable_presses(&mut self) -> Vec<crate::interface::stable::StablePress> {
        std::mem::take(&mut *self.stable.borrow_mut())
    }

    /// Take and clear the bank's presses. See [`crate::interface::bank`].
    pub fn take_bank_presses(&mut self) -> Vec<crate::interface::bank::BankPress> {
        std::mem::take(&mut *self.bank.borrow_mut())
    }

    /// Take and clear the page window's presses. See
    /// [`crate::interface::pagetext`].
    pub fn take_page_presses(&mut self) -> Vec<crate::interface::pagetext::PagePress> {
        std::mem::take(&mut *self.pagetext.borrow_mut())
    }

    /// Take and clear the trade window's presses. See
    /// [`crate::interface::trade`].
    pub fn take_trade_presses(&mut self) -> Vec<crate::interface::trade::TradePress> {
        std::mem::take(&mut *self.trade.borrow_mut())
    }

    /// Take and clear the duel's presses. See [`crate::interface::duel`].
    pub fn take_duel_presses(&mut self) -> Vec<crate::interface::duel::DuelPress> {
        std::mem::take(&mut *self.duel.borrow_mut())
    }

    /// Take and clear the inspect window's presses. See
    /// [`crate::interface::inspect`].
    pub fn take_inspect_presses(&mut self) -> Vec<crate::interface::inspect::InspectPress> {
        std::mem::take(&mut *self.inspect.borrow_mut())
    }

    /// Take and clear the talent panel's presses, as `(tab, index)` pairs the
    /// ECS resolves against the tree. See [`crate::interface::talents`].
    pub fn take_talent_presses(&mut self) -> Vec<(usize, usize)> {
        std::mem::take(&mut *self.talent.borrow_mut())
    }

    /// Take and clear the flight map's presses. See [`crate::interface::taxi`].
    pub fn take_taxi_presses(&mut self) -> Vec<crate::interface::taxi::TaxiPress> {
        std::mem::take(&mut *self.taxi.borrow_mut())
    }

    /// Run one `/script` body, and return the verbs it called or its error.
    ///
    /// `/script` is the game's command, and the only way in this client to
    /// call the interface directly: `/script ActionButton1:Click()`,
    /// `/script CastingBarFrame:Show()`. It goes through [`LuaHost::run`] like
    /// everything else, so a script sees the same reads a handler does.
    ///
    /// Errors are returned rather than recorded, because the person who typed
    /// the script is looking at the chat pane. They are not added to
    /// `missing`: a typo at a prompt is not a missing function, and it would
    /// stay on the HUD for the rest of the session as if it were one.
    pub fn script(&mut self, body: &str, answers: &dyn Answers) -> Result<Vec<Binding>, String> {
        self.queue.borrow_mut().clear();
        let run = self.run(answers, |lua| {
            let chunk = lua
                .load(super::dialect::to_5_1(body).as_ref())
                .set_name("script")
                .into_function()?;
            frames::protected(lua, &chunk)
        });
        self.drain_swallowed();
        match run {
            Ok(()) => Ok(std::mem::take(&mut *self.queue.borrow_mut())),
            Err(e) => Err(first_line(&e)),
        }
    }

    /// Advance the collector by a time budget rather than a byte budget. Called
    /// once a frame by the `pace` system. After a finished cycle it does
    /// nothing until the heap has grown by [`Self::GC_PAUSE_BYTES`].
    ///
    /// Lua 5.1's automatic collector spreads a cycle over allocation and does
    /// the expensive final part wherever the cycle happens to finish. The
    /// audit's `--spin` measured this as clusters of 18–25 ms frames every
    /// couple of seconds, with ~4 MB freed from a ~50 MB heap across each; on
    /// screen the frame rate swung 90→50→90.
    ///
    /// Three other forms of manual pacing were measured first. A single
    /// `gc_step_kbytes(1024)` per frame held the heap level but ran a whole
    /// cycle every ten frames, and the call that finished the cycle still did
    /// its expensive final part: a regular 16–25 ms frame every sixth of a
    /// second, which was worse than the clusters it replaced. A time-budgeted
    /// loop of minimal `gc_step()`s halved the number of spikes but not their
    /// size, because 5.1's `LUA_GCSTEP` re-arms the automatic threshold as a
    /// side effect, so the collector still ran its final part at allocation
    /// sites mid-frame. This version does both: the smallest step the API has,
    /// repeated until the budget is spent, then `gc_stop()` to push the
    /// automatic threshold back out, so every increment of the cycle, atomic
    /// phase and sweep included, runs here and nowhere else.
    ///
    /// When a step finishes a cycle, the loop stops and sets the field
    /// `paused_below` to the live-set floor (`live_floor`) plus
    /// [`Self::GC_PAUSE_BYTES`]. Later calls return at once while the heap is
    /// below that size, so the collector does not re-mark the whole live set
    /// to reclaim a few kilobytes of garbage.
    ///
    /// The budget rises to `BEHIND_BUDGET` while the heap is over
    /// [`Self::GC_BEHIND_BYTES`], because with the automatic collector stopped
    /// this loop is the only limit on the heap during an allocation burst.
    ///
    /// Below that, the budget follows the allocation since the last call rather
    /// than being a constant. The 1,200 µs `BUDGET` was tuned against ~580 KB
    /// of Lua heap allocated a frame, almost all of it by `mlua` itself: two C
    /// closures per protected table read, on every field the walks touch (see
    /// [`super::widgets::widget`]'s note on `raw_get`). With that removed the
    /// interface allocates ~29 KB a frame, and a fixed 1.2 ms of collection was
    /// 36% of the whole interface frame, spent on a heap that was no longer
    /// growing. So every call pays `FLOOR`, and the allocation since the last
    /// call adds `PER_KB` microseconds per kilobyte, up to `BUDGET`. A frame
    /// that allocates nothing pays the floor; an allocation burst is still
    /// paced, and `GC_BEHIND_BYTES` is still the upper limit.
    pub fn pace_collector(&self) {
        /// The minimum budget of a call that runs, so that a cycle in progress
        /// always advances.
        const FLOOR: std::time::Duration = std::time::Duration::from_micros(300);
        const BUDGET: std::time::Duration = std::time::Duration::from_micros(1200);
        const BEHIND_BUDGET: std::time::Duration = std::time::Duration::from_micros(3000);
        /// Microseconds of collection per kilobyte allocated since the last
        /// call.
        ///
        /// Higher than the ratio `BUDGET` was tuned at (1,200 µs / 580 KB ≈ 2).
        /// At the ~29 KB a quiet frame allocates, the proportional term is
        /// under the floor at either rate, so this number only matters during
        /// a burst. Measured over 3,000 frames, the heap grew 1.6 KB a frame at
        /// 2, and 12 at 8, the value used here.
        const PER_KB: u64 = 8;
        // Before the interface loads there is nothing worth pacing, and the
        // load needs the automatic collector: stopping it during a 175-file
        // load would grow the heap by all of the load's temporary allocations.
        if self.interface.is_none() {
            return;
        }
        let live = self.lua.used_memory();
        // After a finished cycle the heap is close to the live set, and a new
        // cycle would re-mark the whole live set for a few kilobytes of
        // garbage. So pacing waits until the heap is `GC_PAUSE_BYTES` above the
        // live-set floor, which at ~29 KB a frame is a few seconds of frames
        // that pay nothing here.
        if live < self.paused_below.get() {
            self.settled.set(live);
            return;
        }
        self.paused_below.set(0);
        let budget = if live > Self::GC_BEHIND_BYTES {
            BEHIND_BUDGET
        } else {
            // Growth since the last call, which is the last frame, because
            // nothing else collects. Saturating, because a sweep that finishes
            // during the frame can leave the heap below where this call left
            // it.
            let grew = live.saturating_sub(self.settled.get()) as u64 / 1024;
            std::time::Duration::from_micros(grew * PER_KB).clamp(FLOOR, BUDGET)
        };
        let deadline = std::time::Instant::now() + budget;
        loop {
            if self.lua.gc_step().unwrap_or(true) {
                // The pause is measured from the live-set floor, not from the
                // heap now: the heap at the end of a cycle includes what was
                // allocated while the cycle ran, and pausing above that let
                // the heap ratchet upward cycle by cycle. A cycle that ends at
                // or above `floor + GC_PAUSE_BYTES` sets no effective pause,
                // so pacing continues until the heap comes back down.
                let floor = self.live_floor.get().min(self.lua.used_memory());
                self.live_floor.set(floor);
                self.paused_below.set(floor.saturating_add(Self::GC_PAUSE_BYTES));
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
        }
        self.lua.gc_stop();
        self.settled.set(self.lua.used_memory());
    }

    /// How far the heap may grow after a finished collection cycle before
    /// [`Self::pace_collector`] starts the next one. Small against the ~34 MB
    /// live set, because the walks slow as the heap grows, and well under the
    /// gap to [`Self::GC_BEHIND_BYTES`].
    const GC_PAUSE_BYTES: usize = 4 * 1024 * 1024;

    /// The heap size above which [`Self::pace_collector`] uses its larger
    /// budget. Above the ~34 MB a freshly loaded interface holds live, and
    /// kept close to it because the walks measurably slow as the heap grows
    /// (memory locality), so the heap should stay near the live set.
    const GC_BEHIND_BYTES: usize = 48 * 1024 * 1024;

    /// The distinct failures recorded so far: functions the interface called
    /// that this client does not have, and handler errors, up to
    /// `MAX_FAILURES`.
    pub fn missing(&self) -> &BTreeSet<String> {
        &self.missing
    }

    /// The events frames have registered for that this client never raises.
    ///
    /// The count falls as events are implemented. `ActionButton_OnLoad` alone
    /// registers seven events this client does not raise, and a frame waiting
    /// on one of them is not updated until something raises it.
    pub fn unfired_events(&self) -> Vec<String> {
        frames::registered_events(&self.lua)
            .unwrap_or_default()
            .into_iter()
            .filter(|event| !FIRED.contains(&event.as_str()))
            .collect()
    }

    /// How many event names any frame is listening on at all.
    pub fn registered_events(&self) -> usize {
        frames::registered_events(&self.lua).map_or(0, |set| set.len())
    }

    /// Record a failure, once, up to the cap.
    fn note(&mut self, failure: String) {
        if self.missing.len() < MAX_FAILURES {
            self.missing.insert(failure);
        }
    }

    /// Take the errors raised by handlers whose errors were caught and not
    /// propagated, and record them like any other failure.
    ///
    /// Called after every entry point that runs interface code. A `Show()`
    /// inside a chunk cannot return an error list to Rust, because it is a
    /// method on a table called from Lua, so [`frames::swallowed`] stores the
    /// error and this collects it; [`super::api::mouse::dispatch`] does the same
    /// one layer up. Without it, an `OnShow` that fails leaves a panel's
    /// contents unset and reports nothing.
    fn drain_swallowed(&mut self) {
        for failure in frames::take_swallowed(&self.lua) {
            self.note(failure);
        }
    }
}

/// The library functions 1.12 has as bare globals, as `(global, library,
/// member)`.
///
/// `format(…)` and `strlen(…)`, not `string.format` and `string.len`. The
/// directory is written entirely that way (141 `format`, 62 `strlen`, 50
/// `gsub`, 34 `floor`, 33 `ceil`, 31 `strsub`, 25 `strupper`, 20 `mod`, 19
/// `tinsert`) and stock 5.1 has none of them, so without this list every one
/// of those call sites raises `attempt to call a nil value` and aborts the rest
/// of its body.
///
/// The list comes from the interface files, not from Lua's history. It does
/// not claim whether 5.0 exported these names or the game added them; it
/// relies on `Interface\FrameXML\` calling each name at the top level, which
/// means the game's Lua environment has them.
///
/// A member the vendored 5.1 does not have is skipped rather than raising an
/// error: `table.foreach` and `table.setn` are compile-time options in 5.1 and
/// the directory calls neither. [`REQUIRED_FLAT_NAMES`] is the set that must
/// be installed, and it is the set the files call.
const FLATTENED: [(&str, &str, &str); 26] = [
    ("abs", "math", "abs"),
    ("ceil", "math", "ceil"),
    ("floor", "math", "floor"),
    ("foreach", "table", "foreach"),
    ("foreachi", "table", "foreachi"),
    ("format", "string", "format"),
    ("getn", "table", "getn"),
    ("gfind", "string", "gmatch"),
    ("gsub", "string", "gsub"),
    ("max", "math", "max"),
    ("min", "math", "min"),
    // `fmod`, not the `%` operator. 5.0's `mod` is C's `fmod` and 5.1
    // renamed it; `mod(counter, 0.5)` in `PlayerFrame_OnUpdate` is a call that
    // must keep working.
    ("mod", "math", "fmod"),
    ("random", "math", "random"),
    ("setn", "table", "setn"),
    ("sort", "table", "sort"),
    ("sqrt", "math", "sqrt"),
    ("strbyte", "string", "byte"),
    ("strchar", "string", "char"),
    ("strfind", "string", "find"),
    ("strlen", "string", "len"),
    ("strlower", "string", "lower"),
    ("strrep", "string", "rep"),
    ("strsub", "string", "sub"),
    ("strupper", "string", "upper"),
    ("tinsert", "table", "insert"),
    ("tremove", "table", "remove"),
];

/// The flattened names `Interface\FrameXML\` actually calls, which is the set
/// [`flatten_libraries`] must not silently fail to install.
///
/// Separate from [`FLATTENED`] because that list includes names the shipped
/// interface does not call (an addon may call `strrep`), so it cannot serve as
/// a check. These are the names counted in the files.
pub const REQUIRED_FLAT_NAMES: [&str; 15] = [
    "abs", "ceil", "floor", "format", "getn", "gsub", "max", "min", "mod", "sort", "sqrt",
    "strfind", "strlen", "strsub", "strupper",
];

/// The base-library names the directory calls. Sorted.
///
/// Not installed by anything here: `mlua` opens the base library whatever
/// `StdLib` flags are given, and 1.12's Lua has these too. They are listed so
/// that `vale framexml` counts them as implemented and does not report
/// `tonumber` and the others as missing functions.
pub const BASE: [&str; 12] = [
    "assert", "error", "getmetatable", "ipairs", "next", "pairs", "pcall", "rawget", "rawset",
    "setmetatable", "tonumber", "tostring",
];

/// Replace `table.remove` with 5.0's version, which 5.1 replaced with a
/// stricter one.
///
/// The two differ in one case, and `QuestLogTitleButton_OnClick` reaches it.
/// 5.1 opens with
///
/// ```c
/// if (!(1 <= pos && pos <= e))  /* position is outside bounds? */
///   return 0;                   /* nothing to remove */
/// ```
///
/// and 5.0 has no such guard: it shortens the table by one and sets `t[n]` to
/// nil whatever `pos` was, returning `t[pos]`. For an out-of-range `pos` that
/// returns `nil` and drops the last entry.
///
/// Untracking a quest runs
///
/// ```lua
/// tremove(QUEST_WATCH_LIST, questIndex);   -- a quest log *row*, as a table index
/// RemoveQuestWatch(questIndex);
/// ```
///
/// and a row is almost never a valid index into a list of at most five watches.
/// Under 5.0 the list still shrinks, by the wrong entry, so
/// `getn(QUEST_WATCH_LIST)` equals the number of watches and
/// `AutoQuestWatch_Insert`'s `getn(QUEST_WATCH_LIST) < MAX_WATCHABLE_QUESTS`
/// keeps admitting quests. Under 5.1 the list never shrinks: it grows by one
/// on every track, so after five the guard fails and every later shift-click
/// does nothing, with no error and no message. This was reported as "you
/// cannot re-enable tracking in certain cases": it happens only when the row
/// is past the list's length, which is almost always.
///
/// Written in Lua rather than Rust because it is four lines of Lua and the
/// original is four lines of C over the same primitives. It replaces
/// `table.remove` itself, so [`FLATTENED`]'s `tremove` alias is this one; the
/// directory calls both spellings.
fn restore_5_0_table_remove(lua: &mlua::Lua) -> mlua::Result<()> {
    lua.load(
        r#"
        table.remove = function(t, pos)
            local n = table.getn(t)
            if n <= 0 then return end
            if pos == nil then pos = n end
            local removed = t[pos]
            for i = pos, n - 1 do t[i] = t[i + 1] end
            t[n] = nil
            return removed
        end
        "#,
    )
    .set_name("=[dialect] table.remove")
    .exec()
}

/// Install [`FLATTENED`] into the globals table.
///
/// Called before the verbs and the object model, and well before the loader:
/// `Fonts.xml`'s inline `<Script>` is the third file in the `.toc` and it
/// already calls them.
fn flatten_libraries(lua: &mlua::Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    for (flat, library, member) in FLATTENED {
        let Some(table) = globals.get::<Option<mlua::Table>>(library)? else {
            continue;
        };
        if let Some(function) = table.get::<Option<mlua::Function>>(member)? {
            globals.set(flat, function)?;
        }
    }
    // `PI`, the one flat name that is a number. `Model_OnUpdate` in
    // `UIParent.lua` turns a paper doll by `elapsedTime * 2 * PI` while a
    // rotate button is held, and `InspectPaperDollFrame.lua` does the same.
    // Without it that body raised on every frame, and a held button turned the
    // doll by its click's 0.03 and no further.
    globals.set("PI", std::f64::consts::PI)?;
    // `date` and `time`, the two `os` functions 1.12 exposes as flat
    // globals. 1.12's state has them and no `os` table, and addons read the
    // clock through them (pfUI's first line after load is `date("%d")`).
    // `date(format, t)` supports the `strftime` subset below, or returns a
    // table for `"*t"`; the zone is UTC, since the interpreter has no zone
    // table, and a leading `!` (5.1's explicit UTC) gives the same result.
    // `time()` is seconds since the epoch; `time(table)` does not convert the
    // table and returns the current time.
    let date = lua.create_function(|lua, (format, at): (Option<String>, Option<f64>)| {
        let seconds = at.map_or_else(now_seconds, |t| t as i64);
        let civil = Civil::from_seconds(seconds);
        let format = format.unwrap_or_else(|| "%c".to_string());
        let format = format.trim_start_matches('!');
        if format.starts_with("*t") {
            let table = lua.create_table()?;
            table.set("year", civil.year)?;
            table.set("month", civil.month)?;
            table.set("day", civil.day)?;
            table.set("hour", civil.hour)?;
            table.set("min", civil.minute)?;
            table.set("sec", civil.second)?;
            table.set("wday", civil.weekday + 1)?;
            table.set("yday", civil.yearday + 1)?;
            table.set("isdst", false)?;
            return Ok(mlua::Value::Table(table));
        }
        Ok(mlua::Value::String(lua.create_string(civil.format(format))?))
    })?;
    globals.set("date", date)?;
    let time = lua.create_function(|_, _: mlua::MultiValue| Ok(now_seconds()))?;
    globals.set("time", time)?;
    Ok(())
}

/// Seconds since the Unix epoch, for `time()` and a bare `date()`.
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// A calendar date and time of day, UTC, for the `date` function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::lua) struct Civil {
    pub year: i64,
    pub month: i64,
    pub day: i64,
    pub hour: i64,
    pub minute: i64,
    pub second: i64,
    /// 0 Sunday .. 6 Saturday.
    pub weekday: i64,
    /// 0-based day of the year.
    pub yearday: i64,
}

impl Civil {
    /// The proleptic Gregorian date of a Unix time: the days-from-civil
    /// inverse, as in Howard Hinnant's algorithms.
    pub(in crate::lua) fn from_seconds(seconds: i64) -> Civil {
        let days = seconds.div_euclid(86_400);
        let of_day = seconds.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = if month <= 2 { y + 1 } else { y };
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let before: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        let yearday = before[(month - 1) as usize] + day - 1 + i64::from(leap && month > 2);
        Civil {
            year,
            month,
            day,
            hour: of_day / 3_600,
            minute: of_day % 3_600 / 60,
            second: of_day % 60,
            weekday: (days + 4).rem_euclid(7),
            yearday,
        }
    }

    /// The `strftime` codes an addon uses: the numeric fields, the English
    /// names, and the three composite forms. An unknown code is kept as
    /// written.
    pub(in crate::lua) fn format(&self, format: &str) -> String {
        const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
        const MONTHS: [&str; 12] = [
            "January", "February", "March", "April", "May", "June", "July", "August", "September",
            "October", "November", "December",
        ];
        let mut out = String::new();
        let mut chars = format.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('Y') => out.push_str(&self.year.to_string()),
                Some('y') => out.push_str(&format!("{:02}", self.year.rem_euclid(100))),
                Some('m') => out.push_str(&format!("{:02}", self.month)),
                Some('d') => out.push_str(&format!("{:02}", self.day)),
                Some('H') => out.push_str(&format!("{:02}", self.hour)),
                Some('I') => out.push_str(&format!("{:02}", if self.hour % 12 == 0 { 12 } else { self.hour % 12 })),
                Some('M') => out.push_str(&format!("{:02}", self.minute)),
                Some('S') => out.push_str(&format!("{:02}", self.second)),
                Some('p') => out.push_str(if self.hour < 12 { "AM" } else { "PM" }),
                Some('j') => out.push_str(&format!("{:03}", self.yearday + 1)),
                Some('w') => out.push_str(&self.weekday.to_string()),
                Some('A') => out.push_str(DAYS[self.weekday as usize]),
                Some('a') => out.push_str(&DAYS[self.weekday as usize][..3]),
                Some('B') => out.push_str(MONTHS[(self.month - 1) as usize]),
                Some('b') => out.push_str(&MONTHS[(self.month - 1) as usize][..3]),
                Some('x') => out.push_str(&self.format("%m/%d/%y")),
                Some('X') => out.push_str(&self.format("%H:%M:%S")),
                Some('c') => out.push_str(&self.format("%a %b %d %H:%M:%S %Y")),
                Some('%') => out.push('%'),
                Some(other) => {
                    out.push('%');
                    out.push(other);
                }
                None => out.push('%'),
            }
        }
        out
    }
}

/// A Lua error's first line, which holds the message; the rest is a traceback
/// through a chunk that is a few lines long.
pub(in crate::lua) fn first_line(e: &mlua::Error) -> String {
    e.to_string().lines().next().unwrap_or_default().to_string()
}

/// `--script`: one Lua chunk to run once the interface has settled, and whether
/// it has been run.
///
/// It lets an interface path be checked without a person at the keyboard, as
/// `--tune` does for the render settings. It was used to confirm the fix for
/// the casting regression documented in this directory's other files:
///
/// ```powershell
/// cargo run -p vale-client -- Alden --script "ActionButtonDown(1); ActionButtonUp(1)"
/// ```
#[derive(Resource, Default)]
pub struct StartupScript {
    body: Option<String>,
    /// When to run it, set the moment the interface finishes loading.
    at: Option<f32>,
    /// Chunks queued while the client is running, drained on the next frame in
    /// the order they were queued.
    ///
    /// The debug panel's Lua console is the only writer. It is a queue rather
    /// than a direct call because running a chunk needs the host (a non-send
    /// resource), the whole of [`api::LuaWorld`], and a writer for the bindings
    /// a chunk can press, which is exactly [`run_scripts`]'s parameter list.
    /// Adding a second copy of that list to the egui pass would exceed Bevy's
    /// limit of sixteen system parameters and give the interface two places
    /// that call into the interpreter in one frame.
    typed: Vec<String>,
}

impl StartupScript {
    pub fn new(body: Option<String>) -> StartupScript {
        StartupScript {
            body,
            at: None,
            typed: Vec::new(),
        }
    }

    /// Run `body` on the next frame, on the same path `--script` and `/script`
    /// take.
    pub fn queue(&mut self, body: String) {
        self.typed.push(body);
    }
}

/// What the console has run and what came back, newest last.
///
/// A resource rather than a `Local` on the panel because [`run_scripts`]
/// produces the result a frame later: the console writes to a queue and reads
/// the log, and the two are separate systems. See [`StartupScript::typed`].
#[derive(Resource, Default)]
pub struct ScriptLog {
    lines: Vec<(String, Result<String, String>)>,
}

/// How many console exchanges to keep. Enough to scroll back through a session
/// of testing a panel, small enough that a script run repeatedly cannot grow
/// the heap without limit.
const SCRIPT_LOG_LIMIT: usize = 100;

impl ScriptLog {
    /// Record one chunk and what it produced: `Ok` with a note, or the first
    /// line of the Lua error.
    pub fn push(&mut self, body: String, outcome: Result<String, String>) {
        self.lines.push((body, outcome));
        if self.lines.len() > SCRIPT_LOG_LIMIT {
            // Drop the oldest. `remove(0)` returns the pair and the `Result`
            // inside it is `#[must_use]`, hence the discard.
            let _ = self.lines.remove(0);
        }
    }

    pub fn lines(&self) -> &[(String, Result<String, String>)] {
        &self.lines
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

/// How long after the interface loads to run `--script`.
///
/// The interface is loaded within a frame of login but the world is not: the
/// spellbook, the action bar and the first entities arrive over the next few
/// seconds, and a script that casts before `SMSG_INITIAL_SPELLS` runs against
/// an empty bar. Five seconds is past all three and inside `--shot`'s
/// 25-second default, so the screenshot shows the result.
const SCRIPT_DELAY_SECS: f32 = 5.0;

pub struct HostPlugin;

impl Plugin for HostPlugin {
    fn build(&self, app: &mut App) {
        // Registered whether or not the interpreter starts, because
        // `run_scripts` is not the only reader: the debug panel's console
        // reads it too, and it would panic if the host failed to start and the
        // resource were missing. `StartupScript` comes from `lib.rs`, which
        // fills it from `--script`.
        app.init_resource::<ScriptLog>();
        // The two flags the loaders' run conditions read. Registered whether
        // or not the interpreter starts, for the same reason: a run condition
        // on a missing resource panics. See [`LoadWanted`].
        app.init_resource::<LoadWanted>();
        // Whether the interface is running at all, registered the same way for
        // the same reason. See [`InterfaceAwake`].
        app.init_resource::<InterfaceAwake>();
        match LuaHost::new() {
            Ok(host) => {
                app.insert_non_send(host);
                // Before `GameSet` reads anything. `run_scripts` writes a
                // `BindingPressed`, and a verb typed at the chat line must
                // reach the same systems in the same frame as a key's. If the
                // message landed after its reader, a `/script` would take one
                // frame longer than the key it stands in for, and a debugging
                // tool must not behave differently from what it tests.
                app.add_systems(
                    Update,
                    (
                        // Both loaders run behind a condition. Each does its
                        // job once and then returns on a flag for the rest of
                        // the session, but a system that returns on its first
                        // line still has every parameter fetched, and
                        // [`api::LuaWorld`] is a `SystemParam` of twenty-nine
                        // fields. Measured with Tracy at the floor framing:
                        // `load_bindings` 0.196 ms and `load_glue` 0.102 ms of
                        // self time per frame, for two functions whose bodies
                        // had not run since login. A run condition that reads
                        // two cheap resources skips the fetch. See
                        // [`interface_wanted`] and [`glue_wanted`].
                        load_glue.run_if(glue_wanted),
                        load_bindings.run_if(interface_wanted),
                        // Behind a condition for the same reason as the two
                        // loaders: on a frame with no script to run, the
                        // condition skips the `LuaWorld` fetch.
                        run_scripts.run_if(scripts_pending),
                        pace,
                    )
                        .chain()
                        .before(crate::interface::GameSet),
                );
                // This file's HUD line, ordered after the chain rather than
                // inside it, because `report` is an instrument and the systems
                // in the chain are the interface's own work. See `ui::debug`.
                #[cfg(feature = "diagnostics")]
                app.add_systems(
                    Update,
                    report
                        .after(run_scripts)
                        .before(crate::interface::GameSet)
                        .run_if(crate::ui::report::watched),
                );
                // After the events have been delivered. That is its only
                // ordering constraint and the reason it is not in the chain
                // above: the interface receives `PLAYER_LEAVING_WORLD` and is
                // then discarded. See [`unload_interface`].
                app.add_systems(Update, unload_interface.after(super::api::events::dispatch));
                // The same teardown without leaving the world, which is what
                // `ReloadUI()` does. After the dispatch for the same reason, and
                // after `unload_interface` so that a frame carrying both (a
                // logout that an addon answered with a reload) tears down once
                // and comes back up as the login screen rather than as a
                // FrameXML the session no longer has.
                app.add_systems(Update, reload_interface.after(unload_interface));
                // The same teardown again, for a host that has stopped showing
                // the interface. After both others, so a frame carrying a
                // logout and a sleep tears down once. See
                // [`sleep_the_interface`].
                app.add_systems(Update, sleep_the_interface.after(reload_interface));
                // The loot panel, which has to be refreshed directly rather
                // than through an event, and only on a frame when
                // `LootNamesLanded` was written. See [`refresh_loot`].
                app.add_systems(
                    Update,
                    refresh_loot
                        .after(super::api::events::dispatch)
                        .run_if(on_message::<crate::interface::loot::LootNamesLanded>),
                );
            }
            // This cannot be recovered from and should not happen: the
            // interpreter is vendored and opening it only allocates. Log an
            // error and continue with no host, so the client starts with keys
            // that do nothing rather than failing to start.
            Err(e) => error!("the Lua host would not start ({e}) — no key will do anything"),
        }
    }
}

/// Whether the game's interface is running at all.
///
/// True in the client. A host that shows the world without the interface
/// (there is no such host in this crate) sets it false, and then neither
/// directory is loaded: there is no frame tree to walk, no `OnUpdate` to tick,
/// no edit box to type into and no button under the pointer.
///
/// It differs from [`crate::render::tuning::WorldTuning::interface`]. That
/// switch skips the walk and the drawing of an interface that is still
/// loaded: the frames still exist, the events still fire, the keyboard still
/// reaches whichever edit box has the focus, and the pointer still finds
/// whichever button is under it. An invisible interface still takes movement
/// keys typed into a focused edit box, and still responds to Enter.
///
/// Turning it off tears down whatever is loaded on the next frame, with the
/// same rebuild a logout does; see [`sleep_the_interface`]. Turning it back on
/// loads the directory again, at the cost of the first load.
#[derive(Resource, Debug, Clone, Copy)]
pub struct InterfaceAwake(pub bool);

impl Default for InterfaceAwake {
    fn default() -> InterfaceAwake {
        InterfaceAwake(true)
    }
}

/// Whether either directory still has to be loaded, as a plain `Resource` the
/// run conditions below can read.
///
/// ## Why run conditions read this rather than the host
///
/// Using the loaders' own first-line checks (`host.directory()`,
/// `host.loaded()`) as the condition panics:
///
/// ```text
/// Attempted to access or drop non-send resource LuaHost
/// from thread ThreadId(1) on a thread ThreadId(11)
/// ```
///
/// A `NonSend` parameter forces the system holding it onto the main thread.
/// A run condition is not part of that system: the multithreaded executor
/// evaluates conditions on whatever worker is free, so a `NonSend<LuaHost>` in
/// one is read off the main thread and Bevy aborts. The client started, drew
/// the login screen, and crashed on the next frame. No test in this repository
/// catches it, because no test runs the multithreaded executor with a real
/// host.
///
/// ## Why a copy of the host's state is safe here
///
/// A second copy of a state can drift from the original. Here drift can only
/// go in the harmless direction: both flags start `true` and are only cleared
/// by the system body that has just done the load. A missed write, or an
/// unforeseen state change, leaves a flag saying "still wanted", which runs the
/// system and lets it return on its own check, the same behaviour as without
/// the flags. The harmful direction, a flag saying "done" when the load has
/// not happened (a login screen that never appears), cannot be reached by
/// omission, only by writing `false` somewhere that has not loaded anything.
///
/// The system bodies keep their own checks for the same reason. The flags
/// only let the scheduler skip the systems; they do not replace the checks.
#[derive(Resource)]
pub struct LoadWanted {
    /// `Interface\GlueXML\`: the login screen.
    glue: bool,
    /// `Interface\FrameXML\`: the in-world interface.
    interface: bool,
}

impl Default for LoadWanted {
    /// Both true, as the invariant above requires: a fresh client has loaded
    /// neither, and anything that resets this resource resets it to "load
    /// needed" rather than to "loaded".
    fn default() -> Self {
        LoadWanted { glue: true, interface: true }
    }
}

impl LoadWanted {
    /// Whether the interface on screen is the world's (`Interface\FrameXML\`)
    /// rather than the login screens'. False from launch and again after a
    /// logout resets this resource; true from the moment [`load_bindings`]
    /// has loaded it. [`crate::ui::scale`] reads it because the `uiScale`
    /// pair is a game-UI setting and the glue always draws at scale 1.0. It
    /// reads this copy rather than the host because the host is `NonSend`.
    pub fn world_interface_loaded(&self) -> bool {
        !self.interface
    }
}

/// Whether there is a login screen to load.
///
/// The cheap part of [`load_glue`]'s first two checks, so that
/// [`api::LuaWorld`], a `SystemParam` of twenty-nine fields, is not fetched on
/// frames where the answer is no, which is every frame after the first second
/// of a session. See the registration above for the measurement and
/// [`LoadWanted`] for why it reads a copy of the host's state.
fn glue_wanted(
    wanted: Res<LoadWanted>,
    awake: Res<InterfaceAwake>,
    session: Res<crate::world::session::Session>,
) -> bool {
    awake.0 && wanted.glue && session.active.is_none()
}

/// Whether there is an in-world interface to load. The same reasoning as
/// [`glue_wanted`], for [`load_bindings`].
///
/// The player check is not here. It is a query into [`api::LuaWorld`], which
/// this condition exists to avoid fetching, and it is false only for the one
/// or two frames between the login packets arriving and the player entity
/// being spawned. Leaving it in the body costs those frames and keeps the
/// condition to cheap resource reads.
fn interface_wanted(
    wanted: Res<LoadWanted>,
    awake: Res<InterfaceAwake>,
    session: Res<crate::world::session::Session>,
) -> bool {
    awake.0 && wanted.interface && session.active.is_some()
}

/// Load `Interface\GlueXML\` (the login screen) as soon as there is a window
/// and no directory is loaded.
///
/// Unlike [`load_bindings`] this is not deferred: the login screen is 39 of
/// the game's files and a full-screen M2 scene, so drawing the window first and
/// opening the archives later would draw an empty window. Opening the chain is
/// ~19 archives and about a second, once, when nothing else is happening.
///
/// Loaded again after a logout as well as at startup, because
/// [`unload_interface`] discards the whole state. The condition is therefore
/// "no directory is loaded" rather than a one-shot flag, and returning from the
/// world shows the login screen again, as the 1.12.1 client does.
///
/// Apart from requiring no active session, the session is not consulted: this
/// runs at the login screen, at character select, and between
/// `CMSG_PLAYER_LOGIN` and the world arriving. [`load_bindings`] takes over as
/// soon as there is a session, and it is chained after this so that a login
/// on the same frame gets the in-world interface rather than the glue.
fn load_glue(
    assets: Res<crate::assets::GameAssets>,
    session: Res<crate::world::session::Session>,
    host: Option<NonSendMut<LuaHost>>,
    world: api::LuaWorld,
    windows: Windows,
    ui_scale: Res<crate::ui::scale::InterfaceScale>,
    mut wanted: ResMut<LoadWanted>,
) {
    let Some(mut host) = host else { return };
    if host.directory().is_some() || session.active.is_some() {
        return;
    }
    tell_the_screen(&host, &windows, ui_scale.get());
    let live = world.live();
    let mut read = |path: &str| {
        assets
            .with_archive(|archive| Ok(archive.read(path).ok()))
            .ok()
            .flatten()
    };
    host.load_glue(&live, &mut read);
    // Cleared here and nowhere else, by the body that just did the load, as
    // [`LoadWanted`]'s invariant requires.
    wanted.glue = false;
    if let Some(report) = host.interface() {
        info!(
            "lua: GlueXML loaded — {} xml + {} lua files, {} frames, {} regions, \
             {} templates, {} handlers; {} missing files, {} distinct errors",
            report.xml_files,
            report.lua_files,
            report.frames,
            report.regions,
            report.templates,
            report.handlers,
            report.missing.len(),
            report.errors.len(),
        );
        for path in report.missing.iter().take(5) {
            warn!("lua: {path} is named by the glue and is not in the archives");
        }
    }
}

/// Set the screen size in the state before a directory is loaded into it.
///
/// One helper for both loaders, because both need it and a mistake is silent:
/// a body that reads `GetScreenWidth()` at load keeps the value it read, and no
/// later event corrects it.
///
/// It uses the window's aspect ratio and the scale the interface is drawn at.
/// The space is [`super::widgets::layout::ui_height`] units tall and as many
/// wide as the window's ratio gives (see
/// [`super::widgets::layout::units_wide`]). A value read at load therefore
/// stays correct however the window is resized by dragging, because the
/// window is held to 16:9 in the one mode where it can be dragged. It does not
/// stay correct across a fullscreen toggle or a change of the UI Scale slider;
/// `layout::set_screen` resizes the two affected frames when either changes.
///
/// This replaced a `reload_on_resize` that discarded the entire Lua state and
/// rebuilt it whenever the window settled at a new size, as the 1.12.1 client
/// does on a resolution change. That cost ~1.1 s with no interface, closed
/// every open panel, and lost the chat frame's lines, once per drag. Two
/// values computed once at load required it: `GlueParent_OnLoad`'s absolute
/// pillarbox width and `WorldMapFrame_OnLoad`'s `BlackoutWorld`. Neither goes
/// stale when the ratio cannot change.
fn tell_the_screen(host: &LuaHost, windows: &Windows, scale: f64) {
    let (width, height) = window_size(windows);
    host.set_screen(
        super::widgets::layout::units_wide(width, height, scale),
        super::widgets::layout::ui_height(scale),
    );
}

/// The primary window, for the two loaders. A named type because both take it
/// and both need only its size.
type Windows<'w, 's> =
    Query<'w, 's, &'static bevy::window::Window, With<bevy::window::PrimaryWindow>>;

/// The primary window's size in logical pixels, or the 16:9 space's size when
/// there is no window, as in the headless `--audit`.
fn window_size(windows: &Windows) -> (f64, f64) {
    match windows.single() {
        Ok(window) => (f64::from(window.width()), f64::from(window.height())),
        Err(_) => super::widgets::layout::UI_SIZE,
    }
}

/// Give the host the game's binding table and load the in-world interface,
/// once there is a session and a player.
///
/// Deferred until there is a session for the same reason as `load_strings`:
/// the chain is ~19 archives and the login screen should draw first. The two
/// happen together because they need the same moment and the same archive.
/// The interface load is the expensive one (175 files, 2.4 MB), so it runs
/// once, off the critical path.
fn load_bindings(
    assets: Res<crate::assets::GameAssets>,
    session: Res<crate::world::session::Session>,
    host: Option<NonSendMut<LuaHost>>,
    world: api::LuaWorld,
    time: Res<Time>,
    mut startup: ResMut<StartupScript>,
    mut focus: ResMut<super::api::keyboard::KeyboardFocus>,
    cvars: Res<crate::settings::cvars::CVars>,
    mut variables: MessageWriter<crate::interface::events::VariablesLoaded>,
    mut entering: MessageWriter<crate::interface::events::PlayerEnteringWorld>,
    windows: Windows,
    ui_scale: Res<crate::ui::scale::InterfaceScale>,
    mut wanted: ResMut<LoadWanted>,
) {
    let Some(mut host) = host else { return };
    if host.loaded() || session.active.is_none() {
        return;
    }
    // The player entity must exist, not only the session. An `OnLoad` reads
    // the world, and before the player exists the reads return zero. Some of
    // the interface keeps the value it read at load, so a zero can be
    // permanent. `MainMenuBar.xml` is an example, and it hid the XP bar:
    //
    // ```lua
    // <OnLoad>  MainMenuExpBar_Update()  </OnLoad>   -- SetMinMaxValues(0, 0)
    // <OnValueChanged>
    //     if (not this:IsShown()) then return; end   -- …and it is not, by then
    //     TextStatusBar_OnValueChanged();            -- -> :Hide() on max == 0
    // ```
    //
    // `TextStatusBar_UpdateTextString` hides a bar whose maximum is zero, and
    // the bar's `OnValueChanged` returns early once it is hidden, so the XP
    // bar hid itself at load and nothing in the directory shows it again:
    // `TextStatusBar_OnEvent` handles `CVAR_UPDATE` and nothing else. The
    // 1.12.1 client does not hit this because the player's values are
    // available when its interface loads.
    //
    // `session.active` is set as soon as the login packets have arrived, which
    // can be the same frame, before `world::session`'s sync has spawned the
    // `WorldEntity` every read here goes through. The wait costs one frame of
    // loading screen.
    if world.units.get(crate::interface::api::UnitId::Player).is_none() {
        return;
    }
    // The glue is discarded before the in-world interface loads, into a fresh
    // state rather than a second `.toc` loaded on top. Both directories
    // declare `MasterFont`, `GameFontNormal` and `GameFontHighlight`, both
    // define a `$parent` root, and `GlueStrings.lua` and `GlobalStrings.lua`
    // share hundreds of keys with different values, so in a state holding both
    // whichever loaded second would replace the other's typefaces and
    // captions. It is the same teardown [`unload_interface`] does at the other
    // end of a session, for the same reason, and done while a loading screen is
    // already showing.
    //
    // The addon board must also know which character this is. Which addons
    // load is set by the character's `AddOns.txt`, which
    // `crate::settings::addons` reads once the session names a character.
    // That system runs after this chain in the same frame, so the first frame
    // with a player waits here and the next one loads. Without the wait every
    // addon would load in its default state regardless of the file.
    if !host.addons().borrow().ready_for_world() {
        return;
    }
    if host.directory() == Some(Directory::Glue) {
        // The board outlives the state: it was seeded on the glue's host and
        // the load below reads it. `carry_over` clears the record of what that
        // host had loaded, since none of it is loaded in the new one.
        let board = host.addons().borrow_mut().carry_over();
        match LuaHost::new() {
            Ok(fresh) => *host = fresh,
            Err(e) => error!("the Lua host would not restart ({e}) — the interface is gone"),
        }
        *host.addons().borrow_mut() = board;
        // The focus was discarded with the state that held it, as at a logout.
        *focus = super::api::keyboard::KeyboardFocus::default();
        // The settings were discarded too; they are the one part of that state
        // that does not belong to the interface. A fresh store holds only the
        // client's default registrations, so without this all of
        // `Interface\FrameXML\` would load below reading the default volume
        // rather than the player's, and `UIOptionsFrame_Init` keeps what it
        // reads. Only the values that differ from their default are written;
        // see [`CVars::changed`].
        host.seed_cvars(&cvars.changed());
    }
    // Before the load, because the interface keeps some values it reads at
    // load: `WorldMapFrame_OnLoad` sizes `BlackoutWorld` to the screen and
    // never reads it again. The per-frame call in [`LuaHost::drawn`] does not
    // cover this, because the new state above has lost the size the glue was
    // given. See [`LuaHost::set_screen`].
    tell_the_screen(&host, &windows, ui_scale.get());
    // `--script`'s time is set here rather than at startup, because its delay
    // is measured from the interface load. See [`SCRIPT_DELAY_SECS`].
    startup.at = Some(time.elapsed_secs() + SCRIPT_DELAY_SECS);
    let mut read = |path: &str| {
        assets
            .with_archive(|archive| Ok(archive.read(path).ok()))
            .ok()
            .flatten()
    };
    // The game's declarations, then each loading addon's `Bindings.xml`. An
    // addon's `.toc` never names that file; the 1.12.1 client reads it from
    // the directory of every addon it loads, which is how pfUI's `PFPAGING1`
    // reaches the key-bindings panel and the keyboard.
    let mut bindings = (*assets.bindings()).clone();
    for name in host.addons().borrow().load_order() {
        let path = vale_assets::interface::addons::Addon::bindings_path(&name);
        if let Some(raw) = read(&path) {
            bindings.extend(vale_assets::interface::bindings::Bindings::parse(&raw));
        }
    }
    let bindings = std::sync::Arc::new(bindings);
    info!(
        "lua: {} bindings declared, {} verbs called, {} of them registered",
        bindings.len(),
        bindings.verbs().len(),
        bindings
            .verbs()
            .iter()
            .filter(|v| verbs::REGISTERED.contains(&v.as_str())
                || api::READS.contains(&v.as_str()))
            .count()
    );
    host.set_bindings(bindings);
    // An archive reader `LoadAddOn` can use from inside a Lua call, since that
    // is the one load that does not go through this system.
    host.addons().borrow_mut().set_reader(assets.reader());

    // The interface, loaded inside a scope that can answer its reads. `OnLoad`
    // handlers call the reads, so the load goes through `run` like everything
    // else. `attacking` is not overridden to `false` here: the load runs
    // before any swing can have started, so the live value is already false.
    let live = world.live();
    host.load_interface(&live, &mut read);
    // Cleared by the body that did the load. The glue was discarded with the
    // state that held it, so its flag is cleared too; see the swap above.
    wanted.interface = false;
    wanted.glue = false;
    if let Some(report) = host.interface() {
        info!(
            "lua: FrameXML loaded — {} xml + {} lua files, {} frames, {} regions, \
             {} templates, {} handlers; {} missing files, {} distinct errors",
            report.xml_files,
            report.lua_files,
            report.frames,
            report.regions,
            report.templates,
            report.handlers,
            report.missing.len(),
            report.errors.len(),
        );
        for path in report.missing.iter().take(5) {
            warn!("lua: {path} is named by the interface and is not in the archives");
        }
    }

    // Raise the world-entry events the interface was not loaded to receive.
    //
    // This client logs in and then loads the interface (175 files, about a
    // second), where 1.12 loads the interface at startup and enters the world
    // afterwards. Every event `GameSet` raised in that second was written with
    // no handler registered, and a `bevy` message with no reader expires:
    // `PLAYER_ENTERING_WORLD`, the action bar's `ACTIONBAR_SLOT_CHANGED`, and
    // the first `UNIT_*` events for every frame.
    //
    // Without them, no spells appeared on the action bar. `ActionButton_OnLoad`
    // calls `ActionButton_Update` itself, so a button reads its slot once at
    // load, and that read happens before `interface::action` has resolved the
    // server's 120 buttons against `Spell.dbc`. Every button therefore read
    // "empty", hid its icon and unregistered the six events that would have
    // corrected it. Running `ActionButton_Update` from `--script` five seconds
    // later filled the whole bar.
    //
    // `PLAYER_ENTERING_WORLD` is the game's own initialisation event: 34
    // frames register it, including the action buttons, `PlayerFrame`, the
    // chat windows and the shapeshift bar. Raised here it has its normal
    // meaning, about one second later than the 1.12.1 client would raise it.
    //
    // `VARIABLES_LOADED` is raised first, because that is the 1.12.1 order: the
    // client reads `SavedVariables` at startup and enters the world afterwards.
    // Here the option globals are final once the last `OnLoad` has run, which
    // is at this point. See [`crate::interface::events::VariablesLoaded`] for
    // the two widgets that wait on it.
    variables.write(crate::interface::events::VariablesLoaded);
    entering.write(crate::interface::events::PlayerEnteringWorld);
}

/// Whether [`run_scripts`] has anything to run: its own early-out, computed
/// from resources alone so that a frame with nothing queued does not fetch
/// `LuaWorld`. It must not read the `NonSend` host; see [`LoadWanted`].
fn scripts_pending(startup: Res<StartupScript>, time: Res<Time>) -> bool {
    !startup.typed.is_empty()
        || startup
            .at
            .is_some_and(|at| time.elapsed_secs() >= at && startup.body.is_some())
}

/// Run the chunks the debug panel's Lua console queued, and the `--script`
/// chunk once it is due. Runs only when [`scripts_pending`] is true.
///
/// The verbs a script calls are written as `BindingPressed`, the same as a
/// key's. `/script ActionButton1:Click()` must press the button; a path that
/// ran the Lua and dropped the queue would behave differently from the thing
/// being debugged.
///
/// The `/script` chat command does not come through here. It is
/// `SlashCmdList["SCRIPT"]` in `ChatFrame.lua`, whose body is
/// `RunScript(msg)`, a function this client registers and which runs the chunk
/// directly, as the 1.12.1 client does.
fn run_scripts(
    host: Option<NonSendMut<LuaHost>>,
    mut startup: ResMut<StartupScript>,
    world: api::LuaWorld,
    time: Res<Time>,
    mut pressed: MessageWriter<crate::input::bindings::BindingPressed>,
    mut notes: MessageWriter<crate::interface::events::ChatMessageReceived>,
    mut log: ResMut<ScriptLog>,
) {
    let Some(mut host) = host else { return };
    // The console's queue first, without the startup delay. `--script`'s five
    // seconds exist because a chunk run at login runs against an empty action
    // bar; a person typing into the console has already waited, and a further
    // delay would look like the console not working.
    let typed: Vec<String> = std::mem::take(&mut startup.typed);
    let due = startup
        .at
        .is_some_and(|at| time.elapsed_secs() >= at && startup.body.is_some());
    if typed.is_empty() && !due {
        return;
    }
    let live = world.live();
    let mut run = |body: String, host: &mut LuaHost, echo: bool| {
        info!("script: {body}");
        let outcome = match host.script(&body, &live) {
            Ok(verbs) => {
                let count = verbs.len();
                for verb in verbs {
                    pressed.write(crate::input::bindings::BindingPressed(verb));
                }
                Ok(match count {
                    0 => "ok".to_string(),
                    1 => "ok — 1 binding pressed".to_string(),
                    n => format!("ok — {n} bindings pressed"),
                })
            }
            Err(e) => {
                // Errors are written to the chat frame, where a `--script`
                // failure is visible. A console error is also echoed in the
                // console, because the person who typed it is looking at the
                // console and not at the chat frame.
                crate::interface::chat::system_note(&mut notes, format!("script error: {e}"));
                Err(e)
            }
        };
        if echo {
            log.push(body, outcome);
        }
    };
    for body in typed {
        run(body, &mut host, true);
    }
    if due {
        let body = startup.body.take().unwrap_or_default();
        run(body, &mut host, false);
    }
}

/// Re-read the loot rows when item names arrive, because no `LootFrame` event
/// does it. Runs only on a frame when `LootNamesLanded` was written.
///
/// This is the one place where this client updates a panel directly instead of
/// raising an event and letting the interface respond. There is no event to
/// raise: `LootFrame_OnEvent`'s only redraw is `ShowUIPanel`, which
/// `UIParent.lua` returns from for a frame that is already visible, and
/// `LootFrame_Update` is called only from `OnShow` and the two page buttons.
/// See [`crate::interface::loot`].
///
/// It rarely has an effect. The window stays closed until its rows can be
/// named, so this only matters when that wait timed out and the item template
/// arrived afterwards. The chunk checks the frame's visibility, so while the
/// window is still closed it does nothing.
///
/// The chunk calls the frame's own `LootFrame_Update`, so there is no second
/// drawing path.
fn refresh_loot(
    host: Option<NonSendMut<LuaHost>>,
    mut landed: MessageReader<crate::interface::loot::LootNamesLanded>,
    world: api::LuaWorld,
) {
    let Some(mut host) = host else { return };
    // One call however many templates arrived in the frame, because the
    // update reads every row.
    if landed.read().next().is_none() {
        return;
    }
    landed.clear();
    let live = world.live();
    // The verb list `script` returns is always empty here, because a redraw
    // calls no verbs, so this discards it where [`run_scripts`] must not.
    if let Err(e) = host.script(
        "if ( LootFrame and LootFrame:IsVisible() ) then LootFrame_Update(); end",
        &live,
    ) {
        warn!("lua: the loot rows could not be re-read ({e})");
    }
}

/// Discard the whole interface when the session ends, so that the next login
/// builds a fresh one.
///
/// The 1.12.1 client does the same: the interface is loaded on entering a
/// world and unloaded on leaving it, which is why an addon's `OnLoad` runs
/// again after a logout. Without this, `load_bindings` checks only whether an
/// interface is loaded, so the old one stayed, with two effects:
///
/// * The last character's interface appeared for the next one. The frames,
///   their `RegisterEvent` lists, `DEFAULT_CHAT_FRAME`'s lines, every global a
///   `.lua` file set and every field a handler wrote survived into the next
///   session, so a warrior saw the previous mage's action-bar state.
/// * ~34 MB of live Lua heap stayed allocated at the character screen, with
///   the collector paced against it every frame.
///
/// Dropping and rebuilding the `mlua::Lua` is the whole teardown: every
/// widget, handler and global in the interface is reachable only from that
/// state, so nothing needs to be walked or cleared. It costs the ~1.1 s load
/// again on the next login, while a loading screen is already showing.
/// `pub(crate)` because `settings::keybindings` must write its two files before
/// this runs, and states that ordering itself; see there.
pub(crate) fn unload_interface(
    host: Option<NonSendMut<LuaHost>>,
    mut leaving: MessageReader<crate::interface::events::PlayerLeavingWorld>,
    mut focus: ResMut<super::api::keyboard::KeyboardFocus>,
    cvars: Res<crate::settings::cvars::CVars>,
    mut wanted: ResMut<LoadWanted>,
) {
    let Some(mut host) = host else { return };
    if leaving.read().next().is_none() {
        return;
    }
    if !host.loaded() {
        return;
    }
    restart(&mut host, &mut focus, &cvars, &mut wanted);
}

/// Discard the interpreter and start a fresh one. A logout, a `ReloadUI()` and
/// putting the interface to sleep all do this.
///
/// One function rather than three copies, so that all three carry the same
/// state across. If one copy carried something the others did not, a setting
/// could survive a reload and not a logout.
fn restart(
    host: &mut LuaHost,
    focus: &mut super::api::keyboard::KeyboardFocus,
    cvars: &crate::settings::cvars::CVars,
    wanted: &mut LoadWanted,
) {
    // The addon list outlives the state; what it had loaded and which
    // character it was for do not. See `panels::addons::Board::carry_over`.
    let board = host.addons().borrow_mut().carry_over();
    match LuaHost::new() {
        Ok(fresh) => *host = fresh,
        // The same failure `HostPlugin` reports, logged as an error for the
        // same reason: afterwards no key does anything.
        Err(e) => error!("the Lua host would not restart ({e}) — no key will do anything"),
    }
    *host.addons().borrow_mut() = board;
    // The focus and the settings were discarded with the state that held
    // them; see [`load_bindings`], which does the same.
    *focus = super::api::keyboard::KeyboardFocus::default();
    host.seed_cvars(&cvars.changed());
    // Both flags, because the fresh host has loaded neither directory. A
    // logout shows the login screen again, as the 1.12.1 client does. This is
    // the one place that writes `true`, and it writes the same value as
    // [`LoadWanted::default`].
    *wanted = LoadWanted::default();
}

/// Unload the interface while it is not being shown. See [`InterfaceAwake`].
///
/// Reacts to the state, not to its change from true to false. `load_glue` and
/// this both run in `Update` with nothing ordering them, so on the first frame
/// of a host that starts asleep the glue can load before the switch is read.
/// An edge detector whose first sample is already `false` sees no change, and
/// the login screen stayed loaded and accepting input for the rest of the
/// session.
///
/// Checking every frame is cheap because the second half of the condition is
/// whether a directory is loaded, and the rebuild clears it: a fresh
/// interpreter has loaded neither directory, so this fires once per sleep
/// however long the sleep lasts. Waking needs no handling here, because the
/// two loaders' conditions already check [`InterfaceAwake`].
pub(crate) fn sleep_the_interface(
    host: Option<NonSendMut<LuaHost>>,
    awake: Res<InterfaceAwake>,
    mut focus: ResMut<super::api::keyboard::KeyboardFocus>,
    cvars: Res<crate::settings::cvars::CVars>,
    mut wanted: ResMut<LoadWanted>,
) {
    if awake.0 {
        return;
    }
    let Some(mut host) = host else { return };
    // `directory()` rather than `loaded()`. `loaded()` is
    // `bindings.is_some()`, which is true only for `Interface\FrameXML\`, so a
    // host at the login screen returns `false`. Checking it left
    // `Interface\GlueXML\` loaded and taking keys in the most common use of the
    // switch: a host that stops before it reaches a world.
    if host.directory().is_none() {
        return;
    }
    info!("lua: the interface is not being shown — unloading it");
    restart(&mut host, &mut focus, &cvars, &mut wanted);
}

/// `ReloadUI()`: load the interface again from nothing, leaving the world as
/// it is.
///
/// The same rebuild [`unload_interface`] does, without leaving the world: a
/// fresh `LuaHost`, the addon board carried across, the focus and the settings
/// restored, and both `LoadWanted` flags set so that [`load_bindings`] loads
/// `Interface\FrameXML\` and every enabled addon into the new state on the next
/// frame. `glue_wanted` returns false while there is a session, so setting both
/// flags is safe here and matches [`LoadWanted::default`].
///
/// Saving must happen before this, not inside it. An addon reloads to make a
/// setting take effect, so the setting must survive the state that holds it:
/// `pfUI:LoadConfig(); ReloadUI()` writes `pfUI_config` into a Lua global and
/// nothing else. `crate::settings::savedvars::save_on_reload` and
/// `crate::settings::keybindings::save_on_reload` write both files first and
/// are ordered `.before` this for that reason, and `savedvars::apply` reads
/// them back, because a fresh host returns true from `needs_saved_variables`.
/// Without the save, the reload would undo whatever asked for it, and pfUI's
/// first-run wizard could never be finished.
///
/// A system rather than the Lua-callable function, because the function runs
/// inside `LuaHost::run`'s scope, on the state this replaces; see
/// [`super::api::verbs`]. In the 1.12.1 client, the code after a `ReloadUI()`
/// call does not run; here the function returns, so the rest of the calling
/// body still runs against a state about to be discarded. That is harmless,
/// because nothing the body does afterwards outlives the frame.
pub(crate) fn reload_interface(
    host: Option<NonSendMut<LuaHost>>,
    mut pressed: MessageReader<crate::input::bindings::BindingPressed>,
    mut focus: ResMut<super::api::keyboard::KeyboardFocus>,
    cvars: Res<crate::settings::cvars::CVars>,
    mut wanted: ResMut<LoadWanted>,
) {
    let asked = pressed
        .read()
        .any(|crate::input::bindings::BindingPressed(binding)| {
            matches!(binding, crate::input::bindings::Binding::ReloadUI)
        });
    let Some(mut host) = host else { return };
    if !asked || !host.loaded() {
        return;
    }
    info!("lua: ReloadUI — rebuilding the interpreter and loading the directory again");
    // The board outlives the state, as at a logout: which addons the folder
    // has and which this character has enabled do not belong to the
    // interpreter, and `carry_over` clears only the record of what had loaded.
    restart(&mut host, &mut focus, &cvars, &mut wanted);
}

/// Run [`LuaHost::pace_collector`] every frame; its doc comment has the
/// measurements. A separate system so that it runs whether or not anything
/// else used Lua this frame: pacing stops the automatic collector, and without
/// pacing the heap would grow without limit. After a finished cycle the call
/// returns at once until the heap has grown by `GC_PAUSE_BYTES`.
fn pace(host: Option<NonSend<LuaHost>>) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::LuaCollector);
    if let Some(host) = host {
        host.pace_collector();
    }
}

/// Up to four HUD lines: the functions the interface called that this client
/// does not have, the loaded interface's size, what it is ticking and where
/// the pointer is, and the registered events this client never raises.
///
/// Each line is hidden while it has nothing to report, which is the usual
/// case, so that a line is only present when it carries information.
#[cfg(feature = "diagnostics")]
fn report(
    host: Option<NonSendMut<LuaHost>>,
    focus: Option<Res<super::api::mouse::MouseFocus>>,
    mut hud: ResMut<HudReport>,
) {
    let Some(host) = host else { return };
    if host.missing().is_empty() {
        hud.clear(REPORT, "lua");
    } else {
        let first: Vec<&str> = host.missing().iter().take(3).map(String::as_str).collect();
        hud.set(
            crate::ui::report::Section::Interface,
            REPORT,
            "lua",
            format!(
                "lua: {} unimplemented — {}",
                host.missing().len(),
                first.join(", ")
            ),
        );
    }

    // The loaded interface's size, which shows that the widget tree exists.
    match host.interface() {
        Some(report) => hud.set(
            crate::ui::report::Section::Interface,
            REPORT,
            "lua-frames",
            format!(
                "lua: {} frames + {} regions from {} files, {} handlers",
                report.frames,
                report.regions,
                report.xml_files + report.lua_files,
                report.handlers
            ),
        ),
        None => hud.clear(REPORT, "lua-frames"),
    }

    // How many frames are being ticked, and which frame the pointer is on. The
    // second shows where a click that did nothing went: a name means the
    // interface took it, no frame means the world did.
    let ticking = host.ticking();
    let over = focus.as_ref().and_then(|f| f.name.clone());
    if ticking == 0 && over.is_none() {
        hud.clear(REPORT, "lua-tick");
    } else {
        hud.set(
            crate::ui::report::Section::Interface,
            REPORT,
            "lua-tick",
            match &over {
                Some(name) if !name.is_empty() => {
                    format!("lua: {ticking} frames ticking, pointer on {name}")
                }
                Some(_) => format!("lua: {ticking} frames ticking, pointer on an unnamed frame"),
                None => format!("lua: {ticking} frames ticking"),
            },
        );
    }

    let unfired = host.unfired_events();
    if unfired.is_empty() {
        hud.clear(REPORT, "lua-events");
    } else {
        let first: Vec<&str> = unfired.iter().take(3).map(String::as_str).collect();
        hud.set(
            crate::ui::report::Section::Interface,
            REPORT,
            "lua-events",
            format!(
                "lua: {} events registered, {} never fired — {}",
                host.registered_events(),
                unfired.len(),
                first.join(", ")
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;

    /// `run` can open a scope many more times than a session does without
    /// leaking Lua stack slots.
    ///
    /// Every entry into Lua (`OnUpdate`, the mouse, the keyboard, a binding's
    /// body, an event batch) is one `lua.scope`, so this client opens four or
    /// five a frame, and a leak of one Lua stack slot in any of them exhausts
    /// the stack in about a minute of play. That happened: `mlua`'s
    /// `Table::clear` pushes the table and never pops it, `scoped::clear`
    /// cleared two stores, and Lua 5.1 will not grow the stack past
    /// `LUAI_MAXCSTACK`, so the 3,998th `run` returned `StackError` and every
    /// entry point stopped at once. See [`crate::lua::scoped::clear`], which now
    /// clears the stores in Lua and explains why.
    ///
    /// The symptom does not point at this file: the interface freezes in the
    /// state it was last drawn in, no key does anything, and the camera keeps
    /// working because it does not use Lua. The error appears only on the HUD
    /// (`lua: 3 unimplemented — OnUpdate: out of Lua stack, …`).
    ///
    /// The count must exceed 8,000. Lua 5.1 refuses to grow the stack past
    /// `LUAI_MAXCSTACK` = 8,000, so a leak of one slot a call fails on the
    /// 8,000th call and a lower count would miss it; the leak this test covers
    /// was two slots and failed at 3,998. 12,000 takes three seconds and leaves
    /// a margin.
    #[test]
    fn the_scope_can_be_opened_far_more_often_than_a_session_opens_it() {
        let host = LuaHost::new().expect("the interpreter starts");
        let stub = Stub::default();
        for i in 0..12_000u32 {
            if let Err(e) = host.run(&stub, |_| Ok(())) {
                panic!("the scope stopped opening at call {i}: {e}");
            }
        }
    }
    use vale_assets::interface::bindings::Bindings;
    use std::sync::Arc;

    /// A chunk queued from the console runs and its result reaches the log.
    /// The panel can only queue, and `run_scripts` turns a queued line into a
    /// `ScriptLog` entry a frame later.
    ///
    /// It also checks both outcomes: a valid chunk logs `Ok` and an invalid one
    /// logs the first line of its error rather than raising.
    #[test]
    fn a_queued_chunk_runs_and_its_answer_reaches_the_log() {
        let mut app = App::new();
        api::LuaWorld::init(&mut app);
        app.insert_non_send(LuaHost::new().expect("the interpreter starts"))
            .insert_resource(crate::assets::GameAssets::new(String::new()))
            .init_resource::<StartupScript>()
            .init_resource::<ScriptLog>()
            .init_resource::<crate::lua::api::keyboard::KeyboardFocus>()
            .init_resource::<crate::settings::cvars::CVars>()
            .add_message::<crate::input::bindings::BindingPressed>()
            .add_message::<crate::interface::events::ChatMessageReceived>()
            .add_systems(Update, run_scripts);

        // Nothing queued: the log stays empty and the system does no work, as
        // on most frames of a real session.
        app.update();
        assert!(app.world().resource::<ScriptLog>().lines().is_empty());

        app.world_mut()
            .resource_mut::<StartupScript>()
            .queue("local x = 1 + 1".to_string());
        app.world_mut()
            .resource_mut::<StartupScript>()
            .queue("this is not lua".to_string());
        app.update();

        let log = app.world().resource::<ScriptLog>();
        let lines = log.lines();
        assert_eq!(lines.len(), 2, "both chunks ran in the one frame, in order");
        assert_eq!(lines[0].0, "local x = 1 + 1");
        assert!(lines[0].1.is_ok(), "{:?}", lines[0].1);
        assert_eq!(lines[1].0, "this is not lua");
        assert!(lines[1].1.is_err(), "a syntax error is reported, not swallowed");

        // The queue is drained rather than replayed; replaying would run every
        // chunk of the session again on every frame.
        app.update();
        assert_eq!(app.world().resource::<ScriptLog>().lines().len(), 2);
    }

    /// The loading system can be built and run with all its parameters.
    ///
    /// Loading the interface added the world's parameters, so that an
    /// `OnLoad` can call reads. Bevy validates a system's parameters at init
    /// rather than at compile time, so a conflicting or missing one panics on
    /// the first frame after login. This test finds that in a millisecond
    /// instead of a run of the client.
    /// `input::bindings::the_dispatch_can_be_scheduled_with_the_world_it_now_borrows`
    /// covers the same failure for the binding dispatch.
    ///
    /// There is no session, so nothing loads; the test only checks that the
    /// system can be built and run.
    #[test]
    fn the_loader_can_be_scheduled_with_the_world_it_now_borrows() {
        let mut app = App::new();
        api::LuaWorld::init(&mut app);
        app.insert_non_send(LuaHost::new().expect("the interpreter starts"))
            .insert_resource(crate::assets::GameAssets::new(String::new()))
            .init_resource::<HudReport>()
            .init_resource::<StartupScript>()
            .init_resource::<ScriptLog>()
            // The loader resets the keyboard focus when it swaps the glue for
            // the interface, as the logout teardown does, and writes the
            // settings into the fresh state for the same reason.
            .init_resource::<crate::lua::api::keyboard::KeyboardFocus>()
            .init_resource::<crate::settings::cvars::CVars>()
            // The loader sets the fresh state's screen size, which depends on
            // the `uiScale` in force; see [`crate::ui::scale`].
            .init_resource::<crate::ui::scale::InterfaceScale>()
            .add_message::<crate::input::bindings::BindingPressed>()
            // `run_scripts` reports a failed chunk as a system chat line; see
            // `interface::chat::system_note`.
            .add_message::<crate::interface::events::ChatMessageReceived>()
            // The loader raises `VARIABLES_LOADED` and `PLAYER_ENTERING_WORLD`
            // when it finishes, because the interface was not loaded for the
            // first ones.
            .add_message::<crate::interface::events::VariablesLoaded>()
            .add_message::<crate::interface::events::PlayerEnteringWorld>()
            .init_resource::<LoadWanted>()
            .add_systems(Update, (load_bindings, run_scripts, report).chain());

        app.update();
    }

    /// `date` is 5.0's flat `os.date`, in UTC: a known epoch second formats as
    /// the correct date, in every code an addon uses.
    #[test]
    fn date_reads_the_calendar_in_utc() {
        // 2026-09-07 17:44:05 UTC, a Monday, the 250th day of a common year.
        let civil = Civil::from_seconds(1_788_803_045);
        assert_eq!((civil.year, civil.month, civil.day), (2026, 9, 7));
        assert_eq!((civil.hour, civil.minute, civil.second), (17, 44, 5));
        assert_eq!((civil.weekday, civil.yearday), (1, 249));
        assert_eq!(civil.format("%Y-%m-%d %H:%M:%S %a %b %j %p %I %%"), "2026-09-07 17:44:05 Mon Sep 250 PM 05 %");
        assert_eq!(civil.format("%c"), "Mon Sep 07 17:44:05 2026");
        // The epoch itself, and a leap year's last day.
        let epoch = Civil::from_seconds(0);
        assert_eq!((epoch.year, epoch.month, epoch.day, epoch.weekday), (1970, 1, 1, 4));
        let leap = Civil::from_seconds(1_072_915_199);
        assert_eq!((leap.year, leap.month, leap.day, leap.yearday), (2003, 12, 31, 364));
        let leap = Civil::from_seconds(1_104_537_599);
        assert_eq!((leap.year, leap.month, leap.day, leap.yearday), (2004, 12, 31, 365));

        let host = LuaHost::new().expect("the interpreter starts");
        assert_eq!(host.eval_for_test("type(date)"), "function");
        assert_eq!(host.eval_for_test("date(\"%Y\", 0)"), "1970");
        assert_eq!(host.eval_for_test("date(\"*t\", 86400).day"), "2");
        assert_eq!(host.eval_for_test("tostring(time() > 1700000000)"), "true");
    }

    /// Leaving the world discards the interface, so the next login builds a
    /// fresh one rather than inheriting the last character's.
    ///
    /// The check is a global set from Lua: the whole interface lives in that
    /// state, so if a global survives, so did its 3,746 frames, their event
    /// registrations and every field a handler wrote on them.
    #[test]
    fn leaving_the_world_unloads_the_interface() {
        let mut app = App::new();
        crate::interface::events::register(&mut app);
        app.insert_non_send(LuaHost::new().expect("the interpreter starts"))
            .init_resource::<crate::lua::api::keyboard::KeyboardFocus>()
            .init_resource::<crate::settings::cvars::CVars>()
            .init_resource::<LoadWanted>()
            .add_systems(Update, unload_interface);
        {
            let mut host = app.world_mut().get_non_send_mut::<LuaHost>().expect("host");
            // What `load_bindings` does at a login, minus the archives.
            host.set_bindings(std::sync::Arc::new(
                vale_assets::interface::bindings::Bindings::default(),
            ));
            host.state()
                .load("ChatFrameEditBox = 'the last character\\'s'")
                .exec()
                .expect("the global is set");
            assert!(host.loaded());
        }
        // Nothing has left the world yet, so nothing is discarded. This checks
        // that the unload reacts to the leaving event, not to "no session".
        app.update();
        assert!(app.world().get_non_send::<LuaHost>().unwrap().loaded());

        app.world_mut()
            .write_message(crate::interface::events::PlayerLeavingWorld);
        app.update();
        let host = app.world().get_non_send::<LuaHost>().unwrap();
        assert!(!host.loaded(), "the next login reloads the directory");
        let survivor: mlua::Value = host
            .state()
            .load("return ChatFrameEditBox")
            .eval()
            .expect("the state answers");
        assert_eq!(survivor, mlua::Value::Nil, "the widget tree went with it");
    }

    /// A script runs and the verbs it called are returned, not dropped.
    ///
    /// `/script ActionButton1:Click()` must press the button, which is why the
    /// queue is drained and written as `BindingPressed`; a debugging path that
    /// ran the Lua and discarded the verbs would behave differently from what
    /// it debugs.
    #[test]
    fn a_script_runs_and_its_verbs_come_back() {
        let mut host = host("<Bindings/>");
        let world = Stub::default();
        assert_eq!(
            host.script("ToggleSheath();", &world),
            Ok(vec![Binding::ToggleSheath])
        );

        // A body that raises returns the message rather than recording it as a
        // missing function; see [`LuaHost::script`].
        let failed = host.script("NoSuchFunction();", &world);
        assert!(failed.is_err(), "{failed:?}");
        assert!(
            host.missing().is_empty(),
            "a typo at a prompt is not a missing verb: {:?}",
            host.missing()
        );
    }

    fn host(xml: &str) -> LuaHost {
        let mut host = LuaHost::new().expect("the interpreter starts");
        host.set_bindings(Arc::new(Bindings::parse(xml.as_bytes())));
        host
    }

    /// A visible `<Model>` frame is a glue scene only while the glue directory
    /// is loaded. The test uses the cooldown indicator.
    ///
    /// `CooldownFrameTemplate` is a `<Model>` holding
    /// `Interface\Cooldown\UI-Cooldown-Indicator.mdx`, one per action button,
    /// shown by `CooldownFrame_SetTimer` when anything goes on cooldown.
    /// `render::glue` builds a full-screen scene from whatever
    /// [`LuaHost::glue_scene`] returns and hides the world's sky, stars, moons
    /// and fog while it is up, so without the directory check the first spell
    /// cast replaced the world with the cooldown indicator.
    #[test]
    fn a_visible_model_frame_is_only_a_scene_while_the_glue_is_loaded() {
        let mut host = host("<Bindings/>");
        let world = Stub::default();
        host.script(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            cd = CreateFrame("Model", "ActionButton1Cooldown", UIParent);
            cd:SetModel("Interface\\Cooldown\\UI-Cooldown-Indicator.mdx");
            "#,
            &world,
        )
        .expect("the cooldown frame is made");

        // No directory loaded.
        assert!(host.directory().is_none());
        assert_eq!(host.glue_scene(), None);

        // With `Interface\FrameXML\` loaded, as in the world: the frame is
        // visible and holds a file, and it is still not a scene.
        host.interface = Some((Directory::Frame, xml::Report::default()));
        assert!(
            crate::lua::widgets::model::visible(&host.lua).is_some(),
            "the model frame really is visible and holding a file"
        );
        assert_eq!(
            host.glue_scene(),
            None,
            "a cooldown swirl is not the login screen's backdrop"
        );

        // At a glue screen the same frame is the scene, as the login screen
        // and character select backdrops are.
        host.interface = Some((Directory::Glue, xml::Report::default()));
        assert!(host.glue_scene().is_some());
    }

    /// 1.12's Lua has `format`, not `string.format`. Every name the directory
    /// calls must be installed, because a missing one aborts the `OnLoad` that
    /// calls it. See [`FLATTENED`].
    #[test]
    fn the_library_functions_are_there_under_the_games_own_names() {
        let mut host = host("<Bindings/>");
        let world = Stub::default();
        for name in REQUIRED_FLAT_NAMES {
            let ran = host.script(
                &format!(r#"if ( type({name}) ~= "function" ) then error("{name}") end"#),
                &world,
            );
            assert!(ran.is_ok(), "{name} is not installed: {ran:?}");
        }
        // Check that they are the right functions, not only present.
        assert!(host
            .script(
                r#"
                if ( format("%d/%s", 3, "x") ~= "3/x" ) then error("format") end
                if ( strlen("abcd") ~= 4 ) then error("strlen") end
                if ( strsub("abcd", 2, 3) ~= "bc" ) then error("strsub") end
                if ( strupper("ab") ~= "AB" ) then error("strupper") end
                if ( gsub("a-a", "-", "+") ~= "a+a" ) then error("gsub") end
                if ( floor(1.7) ~= 1 or ceil(1.2) ~= 2 ) then error("floor/ceil") end
                if ( max(1, 9) ~= 9 or min(1, 9) ~= 1 ) then error("max/min") end
                if ( abs(-2) ~= 2 ) then error("abs") end
                -- `mod` is fmod, which is what PlayerFrame_OnUpdate wants.
                if ( mod(1.2, 0.5) < 0.19 or mod(1.2, 0.5) > 0.21 ) then error("mod") end
                -- `PI`, which `Model_OnUpdate` turns a paper doll by.
                if ( PI < 3.14159 or PI > 3.1416 ) then error("PI") end
                local t = {};
                tinsert(t, "a"); tinsert(t, "b");
                if ( getn(t) ~= 2 ) then error("tinsert/getn") end
                tremove(t);
                if ( getn(t) ~= 1 ) then error("tremove") end
                "#,
                &world
            )
            .is_ok());
    }

    /// `tremove` past the end still shortens the list, which is 5.0's
    /// behaviour and not 5.1's. See [`restore_5_0_table_remove`], which
    /// compares the two.
    ///
    /// The case here is the one `QuestLogTitleButton_OnClick` runs: a quest
    /// log row passed to `tremove` as a table index. Under 5.1's guard the
    /// call returns without changing anything, `QUEST_WATCH_LIST` grows on
    /// every track and never shrinks, and after the fifth, quest tracking stops
    /// working for the rest of the session with no message.
    ///
    /// Both spellings are tested, because the directory uses both and the
    /// flatten aliases only `tremove`.
    #[test]
    fn tremove_is_the_5_0_one_and_shortens_a_list_whatever_the_position() {
        let mut host = host("<Bindings/>");
        let world = Stub::default();
        assert!(host
            .script(
                r#"
                -- The reported case: one watch, removed by a row number.
                local t = {"only"};
                tremove(t, 5);
                if ( getn(t) ~= 0 ) then error("out of range did not shorten") end
                if ( t[1] ~= nil ) then error("the last entry was left behind") end

                -- …and an in-range removal is still exactly a removal.
                local u = {"a", "b", "c"};
                if ( tremove(u, 1) ~= "a" ) then error("the removed value") end
                if ( getn(u) ~= 2 or u[1] ~= "b" or u[2] ~= "c" ) then error("the shift") end

                -- …and the last, which is the no-argument form.
                if ( tremove(u) ~= "c" ) then error("the default position") end
                if ( getn(u) ~= 1 ) then error("the default shortened") end

                -- An empty table is untouched rather than an error.
                local e = {};
                if ( tremove(e, 3) ~= nil ) then error("empty returned something") end
                if ( getn(e) ~= 0 ) then error("empty moved") end

                -- The library spelling is the same body.
                local v = {"x"};
                table.remove(v, 9);
                if ( getn(v) ~= 0 ) then error("table.remove is not the 5.0 one") end
                "#,
                &world
            )
            .is_ok());
    }

    /// The base library is open, although `StdLib::STRING | TABLE | MATH` does
    /// not name it: `mlua` loads the base functions whatever is asked for, and
    /// 1.12 has them too. They are listed in [`BASE`] because otherwise
    /// `vale framexml` reports them as missing functions.
    #[test]
    fn the_base_library_is_open_beside_the_three() {
        let mut host = host("<Bindings/>");
        let world = Stub::default();
        for name in BASE {
            let ran = host.script(
                &format!(r#"if ( type({name}) ~= "function" ) then error("{name}") end"#),
                &world,
            );
            assert!(ran.is_ok(), "{name} is claimed in BASE and is missing: {ran:?}");
        }
    }

    /// [`FLATTENED`] is sorted, has no duplicates, and contains every name in
    /// [`REQUIRED_FLAT_NAMES`].
    ///
    /// A flattened name must also not shadow anything the interface defines:
    /// `sort` and `format` are the client's to provide and `TEXT` is not, so
    /// nothing in [`FLATTENED`] may be a name `Interface\FrameXML\` defines a
    /// `function` for. That needs the archives, so it is not checked here;
    /// `vale framexml`'s collision count reads the files, and it must stay zero
    /// with these names added.
    #[test]
    fn the_flattened_names_are_sorted_and_distinct() {
        let names: Vec<&str> = FLATTENED.iter().map(|(flat, _, _)| *flat).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, names, "FLATTENED is kept sorted and has no duplicate");
        for required in REQUIRED_FLAT_NAMES {
            assert!(
                names.contains(&required),
                "{required} is required by the directory and is not in FLATTENED"
            );
        }
    }

    /// `ActionButton.lua`'s two functions and the button they act on: enough
    /// of the interface for a real binding body to run against.
    ///
    /// The tests below need it because a key press reaches the game's Lua, not
    /// a Rust closure, and only the verbs that Lua calls are handled by the
    /// client. See [`crate::lua::widgets::button`], where the same bodies are
    /// tested in full.
    const ACTION_BUTTON_LUA: &str = r#"
        CURRENT_ACTIONBAR_PAGE = 1;
        NUM_ACTIONBAR_BUTTONS = 12;
        function ActionButton_GetPagedID(button)
            return (button:GetID() + ((CURRENT_ACTIONBAR_PAGE - 1) * NUM_ACTIONBAR_BUTTONS));
        end
        function ActionButtonDown(id)
            local button = getglobal("ActionButton"..id);
            if ( button:GetButtonState() == "NORMAL" ) then
                button:SetButtonState("PUSHED");
            end
        end
        function ActionButtonUp(id, onSelf)
            local button = getglobal("ActionButton"..id);
            if ( button:GetButtonState() == "PUSHED" ) then
                button:SetButtonState("NORMAL");
                UseAction(ActionButton_GetPagedID(button), 0, onSelf);
            end
        end
        local b = CreateFrame("CheckButton", "ActionButton1");
        b:SetID(1);
    "#;

    /// The game's binding body runs with `keystate` set, through the
    /// interface's own Lua.
    ///
    /// The binding body calls `ActionButtonDown`/`ActionButtonUp`, which are
    /// defined in `ActionButton.lua`, not by this client; the function the
    /// client provides at the end of the chain is `UseAction`. When the
    /// interface started loading, `ActionButtonDown` and `ActionButtonUp` were
    /// also registered as verbs, and that broke casting.
    #[test]
    fn a_real_binding_body_runs_through_the_interfaces_own_lua() {
        let mut host = host(
            r#"<Binding name="ACTIONBUTTON1" runOnUp="true">
                if ( keystate == "down" ) then
                    ActionButtonDown(1);
                else
                    ActionButtonUp(1);
                end
            </Binding>"#,
        );
        let world = Stub::default();
        host.run(&world, |lua| lua.load(ACTION_BUTTON_LUA).exec())
            .expect("the interface's own file loads");

        // The press pushes the button in and casts nothing.
        assert!(host.fire("ACTIONBUTTON1", true, &world).is_empty());
        // The release reaches `UseAction`.
        assert_eq!(
            host.fire("ACTIONBUTTON1", false, &world),
            vec![Binding::ActionButton(1)]
        );
        assert!(host.missing().is_empty(), "{:?}", host.missing());
    }

    /// `SELFACTIONBUTTON1`'s real body: the same path with the game's `onSelf`
    /// flag, which is passed on the release and reaches `UseAction` as its
    /// third argument through `ActionButtonUp`.
    #[test]
    fn the_self_cast_flag_comes_off_the_files_own_argument() {
        let mut host = host(
            r#"<Binding name="SELFACTIONBUTTON1" runOnUp="true">
                if ( keystate == "down" ) then
                    ActionButtonDown(1);
                else
                    ActionButtonUp(1, 1);
                end
            </Binding>"#,
        );
        let world = Stub::default();
        host.run(&world, |lua| lua.load(ACTION_BUTTON_LUA).exec())
            .expect("the interface's own file loads");
        host.fire("SELFACTIONBUTTON1", true, &world);
        assert_eq!(
            host.fire("SELFACTIONBUTTON1", false, &world),
            vec![Binding::SelfActionButton(1)]
        );
    }

    /// A key pressed before the interface has loaded does nothing and is
    /// reported.
    ///
    /// `ActionButtonDown` is not registered by the client, so a press with no
    /// `ActionButton.lua` in the state has nothing to call. The client does not
    /// implement that name and should not. The failure must be reported,
    /// because "the interface did not load" and "this key does nothing" look
    /// the same from the keyboard.
    #[test]
    fn an_action_key_with_no_interface_loaded_is_reported() {
        let mut host = host(
            r#"<Binding name="ACTIONBUTTON1" runOnUp="true">
                if ( keystate == "down" ) then ActionButtonDown(1); end
            </Binding>"#,
        );
        assert!(host.fire("ACTIONBUTTON1", true, &Stub::default()).is_empty());
        assert_eq!(host.missing().len(), 1, "{:?}", host.missing());
    }

    /// `TARGETSELF` works through its real two-branch body, which calls a read
    /// that is answered from the live world during the call.
    ///
    /// Both branches are tested. When the target is not the player, the body
    /// targets the player. When it is, the body targets the pet, which this
    /// client does not have, so the verb resolves to nothing and the target is
    /// left unchanged, rather than cleared as a `_ => TargetSelf` catch-all
    /// would do.
    #[test]
    fn target_self_reads_the_world_before_it_branches() {
        let mut host = host(
            r#"<Binding name="TARGETSELF">
                if ( UnitIsUnit("player", "target") ) then
                    TargetUnit("pet");
                else
                    TargetUnit("player");
                end
            </Binding>"#,
        );
        let elsewhere = Stub::default()
            .unit("player", "Alden", 7)
            .unit("target", "Kobold Vermin", 99);
        assert_eq!(
            host.fire("TARGETSELF", true, &elsewhere),
            vec![Binding::TargetSelf]
        );

        let on_myself = Stub::default()
            .unit("player", "Alden", 7)
            .unit("target", "Alden", 7);
        assert!(
            host.fire("TARGETSELF", true, &on_myself).is_empty(),
            "there is no pet to fall through to"
        );

        // With no target, `UnitIsUnit` is false. If two absent tokens compared
        // equal, this would take the pet branch in the most common state a
        // player is in.
        let nothing = Stub::default().unit("player", "Alden", 7);
        assert_eq!(
            host.fire("TARGETSELF", true, &nothing),
            vec![Binding::TargetSelf]
        );
    }

    /// A verb this client has not implemented is recorded once, not once per
    /// press, and the binding returns no verbs rather than running partly.
    #[test]
    fn an_unimplemented_verb_is_reported_once() {
        let mut host = host(r#"<Binding name="TOGGLEWORLDMAP">ToggleWorldMap();</Binding>"#);
        for _ in 0..10 {
            assert!(host
                .fire("TOGGLEWORLDMAP", true, &Stub::default())
                .is_empty());
        }
        assert_eq!(host.missing().len(), 1, "{:?}", host.missing());
        assert!(
            host.missing()
                .iter()
                .next()
                .unwrap()
                .starts_with("TOGGLEWORLDMAP"),
            "{:?}",
            host.missing()
        );
    }

    /// A key bound to a name the game does not declare fires nothing and is
    /// reported in the same set, because for the player the symptom is the
    /// same.
    #[test]
    fn a_name_the_game_does_not_declare_is_refused() {
        let mut host = host(r#"<Binding name="JUMP">Jump();</Binding>"#);
        assert!(host.fire("NOTABINDING", true, &Stub::default()).is_empty());
        assert!(host.missing().contains("NOTABINDING"));
    }

    /// `io` and `os` are not available. Addons are third-party code running
    /// in-process and 1.12 does not give them a filesystem. Opening all
    /// libraries with `Lua::new()` instead of the game's four would not show up
    /// until something read a file, so this test checks it.
    #[test]
    fn the_dangerous_libraries_are_absent() {
        let mut reaching = host(r#"<Binding name="X">local f = io.open("a", "w");</Binding>"#);
        reaching.fire("X", true, &Stub::default());
        assert_eq!(reaching.missing().len(), 1, "io was reachable");

        // The four opened libraries work, since a body may format a string.
        let mut ok = host(r#"<Binding name="Y">local s = string.format("%d", 1);</Binding>"#);
        ok.fire("Y", true, &Stub::default());
        assert!(ok.missing().is_empty(), "{:?}", ok.missing());
    }

    /// A frame registers for an event and its handler is called, through the
    /// host rather than through `frames::fire` directly, which is the path the
    /// world's events take.
    #[test]
    fn an_event_reaches_the_frame_that_registered_for_it() {
        let mut host = host("<Bindings/>");
        let world = Stub::default().unit("target", "Kobold Vermin", 4);
        // The chunk also goes through the single entry point, so it sees the
        // reads.
        host.run(&world, |lua| {
            lua.load(
                r#"
                TargetFrame = CreateFrame("Button", "TargetFrame");
                TargetFrame:RegisterEvent("PLAYER_TARGET_CHANGED");
                TargetFrame:SetScript("OnEvent", function()
                    shown = UnitName("target");
                end);
                "#,
            )
            .exec()
        })
        .expect("the file loads");

        host.fire_event("PLAYER_TARGET_CHANGED", &[], &world);
        assert!(host.missing().is_empty(), "{:?}", host.missing());
        // The read inside the handler saw the live world: `UnitName` is
        // registered in the scope for the length of the dispatch and answers
        // from `world`.
        let name: String = host
            .run(&world, |lua| lua.globals().get("shown"))
            .expect("the global is set");
        assert_eq!(name, "Kobold Vermin");
    }

    /// A verb called from an event handler is not dropped. Nothing in the
    /// shipped FrameXML does this, but an addon may, and a verb must work the
    /// same from an event as from a key.
    #[test]
    fn a_handler_may_call_a_verb() {
        let mut host = host("<Bindings/>");
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"
                f = CreateFrame("Frame", "Sheather");
                f:RegisterEvent("ACTIONBAR_UPDATE_STATE");
                f:SetScript("OnEvent", function() ToggleSheath(); end);
                "#,
            )
            .exec()
        })
        .expect("loads");
        assert_eq!(
            host.fire_event("ACTIONBAR_UPDATE_STATE", &[], &world),
            vec![Binding::ToggleSheath]
        );
    }

    /// Registered events that this client never raises are counted. In the
    /// real interface most of the count comes from `ActionButton_OnLoad`'s
    /// list.
    #[test]
    fn an_event_nothing_fires_is_counted_as_a_gap() {
        let host = host("<Bindings/>");
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"
                f = CreateFrame("Frame", "Probe");
                f:RegisterEvent("PLAYER_TARGET_CHANGED");
                f:RegisterEvent("BAG_UPDATE");
                f:RegisterEvent("GUILD_ROSTER_UPDATE");
                "#,
            )
            .exec()
        })
        .expect("loads");
        assert_eq!(host.registered_events(), 3);
        let mut unfired = host.unfired_events();
        unfired.sort();
        // `GUILD_ROSTER_UPDATE` because this client does not read the guild
        // yet. This event was changed twice before, when `MERCHANT_SHOW` and
        // then `TRADE_SHOW` became names this client raises.
        assert_eq!(
            unfired,
            ["GUILD_ROSTER_UPDATE"],
            "the other two are names this client raises"
        );
    }

    /// The failure set is capped. A broken addon calling a new missing function
    /// every frame would otherwise grow it without limit, and the HUD line that
    /// reports it would grow with it.
    #[test]
    fn the_failure_set_does_not_grow_without_bound() {
        let mut host = LuaHost::new().expect("the interpreter starts");
        for n in 0..MAX_FAILURES * 2 {
            host.note(format!("failure {n}"));
        }
        assert_eq!(host.missing().len(), MAX_FAILURES);
    }

    /// The loaders' run conditions must not read the host, because doing so
    /// crashes the client.
    ///
    /// An earlier [`glue_wanted`] and [`interface_wanted`] read
    /// `NonSend<LuaHost>` directly, as the system bodies' first lines do. That
    /// aborts the client on the second frame:
    ///
    /// ```text
    /// Attempted to access or drop non-send resource LuaHost
    /// from thread ThreadId(1) on a thread ThreadId(11)
    /// ```
    ///
    /// A `NonSend` parameter forces the system onto the main thread; a run
    /// condition is not part of that system, and the multithreaded executor
    /// evaluates it on whatever worker is free. The client drew the login
    /// screen and crashed, and every test in this file passed, because none of
    /// them runs the multithreaded executor.
    ///
    /// This test therefore checks the property that prevents the crash: both
    /// conditions can be evaluated from resources alone. It runs them with no
    /// host in the world, which a `NonSend` version could not do.
    #[test]
    fn the_load_conditions_ask_nothing_of_the_host() {
        let mut app = App::new();
        app.init_resource::<LoadWanted>()
            .init_resource::<InterfaceAwake>()
            .init_resource::<crate::world::session::Session>();
        // No `LuaHost` is inserted, and none is needed. A condition that
        // reached for one would fail to run here.
        assert!(
            app.world_mut().run_system_cached(glue_wanted).expect("runs"),
            "a fresh client has no glue up and no session, so the login screen is wanted"
        );
        assert!(
            !app.world_mut().run_system_cached(interface_wanted).expect("runs"),
            "…and no interface, because there is no session to have one for"
        );
    }

    /// An interface that is not being shown is not loaded. That is what
    /// [`InterfaceAwake`] does, and what distinguishes it from
    /// `WorldTuning::interface`.
    ///
    /// Checked on the conditions rather than on the system bodies, because
    /// that is where it must hold: if a loader's condition were true, it would
    /// load the directory and the teardown would then unload it, costing a
    /// second of work and a frame of an unwanted login screen.
    #[test]
    fn a_sleeping_interface_loads_neither_directory() {
        let mut app = App::new();
        app.init_resource::<LoadWanted>()
            .init_resource::<crate::world::session::Session>()
            .insert_resource(InterfaceAwake(false));
        assert!(
            !app.world_mut().run_system_cached(glue_wanted).expect("runs"),
            "asleep, so there is no login screen to put up"
        );
        assert!(
            !app.world_mut().run_system_cached(interface_wanted).expect("runs"),
            "…and none of the interface either"
        );
        // Waking it makes the glue wanted again, with nothing else changed.
        app.insert_resource(InterfaceAwake(true));
        assert!(app.world_mut().run_system_cached(glue_wanted).expect("runs"));
    }

    /// Awake by default, because the client shows the interface; the other
    /// default would give the client no login screen.
    #[test]
    fn the_interface_is_awake_unless_somebody_says_otherwise() {
        assert!(InterfaceAwake::default().0);
    }

    /// The flags fail in the safe direction, which is why a copy of the host's
    /// state is acceptable.
    ///
    /// [`LoadWanted::default`] is both true, and only a body that has just
    /// loaded something writes `false`. A missed write leaves a loader running
    /// and returning on its own check, which costs time but does not stop the
    /// login screen from appearing. This test fixes the default, because it is
    /// the part of the invariant most likely to be changed by mistake.
    #[test]
    fn a_fresh_load_state_wants_both_directories() {
        let fresh = LoadWanted::default();
        assert!(fresh.glue, "a client that has loaded no glue must load it");
        assert!(fresh.interface, "…and the same for the interface");
    }

}

/// Whether a skipped draw walk is run anyway and checked; see
/// [`LuaHost::drawn_if_changed`]. Always on in tests.
fn paint_verify() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    cfg!(test) || *ON.get_or_init(|| std::env::var_os("VALE_PAINT_VERIFY").is_some())
}

/// Report a walk that was going to be skipped but would have drawn something
/// different: the first differing item on each side. Logged once per session;
/// a test panics instead.
fn report_stale_paint(
    last: &[super::widgets::draw::Item],
    fresh: &[super::widgets::draw::Item],
) {
    let at = last
        .iter()
        .zip(fresh)
        .position(|(a, b)| a != b)
        .unwrap_or(last.len().min(fresh.len()));
    let message = format!(
        "paint: a skipped walk would have drawn differently: {} items held, {} fresh;          first difference at {at}: held {:?}, fresh {:?}",
        last.len(),
        fresh.len(),
        last.get(at),
        fresh.get(at),
    );
    if cfg!(test) {
        panic!("{message}");
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| warn!("{message}"));
}
