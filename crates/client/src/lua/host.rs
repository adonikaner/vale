//! The interpreter: one Lua 5.1 state, the game's own code running in it, and
//! what happens when that code asks for something this client does not have.
//!
//! ## Which libraries, and the two that are missing on purpose
//!
//! 1.12 opens base, `string`, `table` and `math` and **not** `io`, `os`,
//! `package` or `debug`. That is the difference between an addon language and a
//! scripting language with a filesystem in it, and it is load-bearing rather
//! than cosmetic: `Interface\AddOns\` is third-party code, downloaded from the
//! internet, running in-process. This host opens the same four.
//!
//! Two globals are *added* back: `getglobal` and `setglobal`, which stock 5.1
//! does not have and which FrameXML cannot load without — see
//! [`super::widgets::frames::install`].
//!
//! ## …and the libraries are **flat**, which is not a detail
//!
//! 1.12's Lua reaches those three libraries as bare globals: `format`, not
//! `string.format`; `strlen`, `floor`, `tinsert`, `getn`, `mod`. The whole
//! directory is written that way and stock 5.1 has none of the names, so before
//! [`FLATTENED`] every `format(…)` in the interface was `attempt to call a nil
//! value` — 141 call sites for that one name, each of them aborting the body it
//! was in. That is the shape of failure this file's last section is about: not a
//! function that does nothing, a *panel* that does nothing.
//!
//! ## A non-send resource, which is not a compromise
//!
//! `mlua::Lua` is not `Sync`, so it lives as a Bevy **non-send** resource and
//! every system that touches it runs on the main thread. That is what we want
//! anyway — a single Lua state is single-threaded in the real client too, and
//! the alternative (a mutex around the interpreter) would buy nothing, since
//! there is exactly one caller.
//!
//! ## There is exactly one way in, and it takes the world with it
//!
//! [`LuaHost::run`] is the only place a chunk is executed, and it takes an
//! [`Answers`]. The reason is in [`super::api`]: the reads are **scoped**
//! functions that exist for the length of one call and are destroyed after it,
//! so a chunk run outside a scope would find `UnitHealth` pointing at a
//! destroyed callback. Keeping the entry points to one makes that window
//! impossible rather than merely unlikely.
//!
//! It also means the write side and the read side no longer work differently for
//! any interesting reason. A verb records and the caller drains it; a read
//! answers immediately. That is the boundary the real client has, where a Lua
//! call reaches C and C decides which of the two it is.
//!
//! ## Failure is reported once and then suppressed
//!
//! 234 bindings call 122 distinct functions and this client answers a fraction
//! of them, so most keys, if bound, would raise `attempt to call a nil value` —
//! at key-repeat rate. [`LuaHost::missing`] records the *name* the first time and
//! says nothing after, and the count reaches the HUD. Silence would be worse
//! than noise here: a key that does nothing with no explanation is the hardest
//! interface fault there is to diagnose, and this is a client that is going to
//! spend a long time with most of its verbs unimplemented.

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

/// The HUD line this file writes — see [`crate::ui::report`] for the
/// convention. 30 sits under the sky and celestial lines and well above
/// residency's 90.
#[cfg(feature = "diagnostics")]
const REPORT: Slot = Slot(30);

/// How many distinct failures to keep. A cap, because a broken addon could
/// otherwise name a new function on every frame and this set is unbounded.
const MAX_FAILURES: usize = 64;

/// **Which of the game's two interface directories is loaded.**
///
/// 1.12 ships two, and they are alternatives rather than layers:
/// `Interface\GlueXML\` is up from the moment the window opens until a character
/// enters the world, and `Interface\FrameXML\` from then until the session ends.
/// Each has its own `.toc`, its own strings file, its own fonts and its own
/// `$parent` root — `GlueParent` against `UIParent` — and both declare
/// `MasterFont` and `GameFontNormal`, so loading them into one state would leave
/// whichever came second holding the other's typefaces.
///
/// This client already threw the whole `mlua::Lua` away on leaving the world
/// ([`unload_interface`]), so the swap costs nothing new: what changes is that
/// the fresh state gets the *glue* rather than nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Directory {
    /// `Interface\GlueXML\` — the login screen and character select.
    Glue,
    /// `Interface\FrameXML\` — everything after `CMSG_PLAYER_LOGIN`.
    Frame,
}

/// **What the character-create screen has chosen**, copied out of the Lua state
/// once a frame — see [`LuaHost::char_create_choice`].
///
/// The wire's numbers and the renderer's in one value: `race` and `class` are
/// `ChrRaces` and `ChrClasses` ids and `gender` is `PLAYER_BYTES`' 0 and 1, none
/// of which is what the interface counts in. See [`super::panels::charcreate`], where
/// the two numberings cross.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharCreateChoice {
    pub race: u8,
    pub class: u8,
    pub gender: u8,
    /// Skin, face, hair style, hair colour, facial hair — `PLAYER_BYTES` order,
    /// which is `CMSG_CHAR_CREATE`'s.
    pub appearance: [u8; 5],
    /// …and the same thing as the type `vale_assets::look::dress` takes, so the
    /// plinth is dressed by exactly the call the world uses.
    pub look: vale_assets::look::character::Appearance,
    /// …and what it is **wearing**, which on this screen is the class's own
    /// starting outfit rather than nothing — see
    /// [`vale_assets::tables::charcreate::CharCreate::outfit`].
    pub outfit: Vec<vale_assets::tables::charcreate::OutfitPiece>,
}

impl Directory {
    /// The `.toc` to load, which is the whole difference between the two.
    fn toc(self) -> &'static str {
        match self {
            Directory::Glue => vale_assets::interface::toc::GLUEXML_TOC,
            Directory::Frame => vale_assets::interface::toc::FRAMEXML_TOC,
        }
    }
}

/// The Lua state, the game's binding table, and the queue verbs write to.
///
/// **Non-send**: taken as `NonSendMut<LuaHost>`, which pins its readers to the
/// main thread. See the module comment.
pub struct LuaHost {
    lua: mlua::Lua,
    queue: Queue,
    /// …and the second queue, for the one verb whose argument is a sentence —
    /// see [`verbs::Said`]. Separate because [`Binding`] is `Copy` and a chat
    /// line is a `String`, and because what drains it is a different system.
    said: verbs::SaidQueue,
    /// …and the text emotes — see [`verbs::Emoted`].
    emoted: verbs::EmoteQueue,
    /// …and the second queue of the same kind, for the one pet verb that
    /// carries a name — see [`verbs::PetRenameQueue`].
    renamed: verbs::PetRenameQueue,
    /// `Bindings.xml`, once the archives are open. `None` until then, which is
    /// the whole of the login screen.
    bindings: Option<std::sync::Arc<vale_assets::interface::bindings::Bindings>>,
    /// …and the third, for the world map's own three writes — see
    /// [`super::panels::worldmap`].
    map: super::panels::worldmap::MapQueue,
    /// …and the fourth, for the interface's four sound verbs — see
    /// [`super::api::sound`].
    sound: super::api::sound::SoundQueue,
    /// …and the settings the interface writes — see [`super::api::cvars`].
    /// Every options panel in the game is three calls over this one store, so
    /// this is the queue a *panel that does something* comes out of.
    cvars: super::api::cvars::CVarQueue,
    /// …and the *other* settings store, which is not a queue and not CVars:
    /// the names `RegisterForSave` has been given. See
    /// [`super::api::savedvars`] — forty-four of the options panel's rows are
    /// these rather than CVars.
    saved: super::api::savedvars::SavedNames,
    /// **…and what the file said they were**, held so that a registration can
    /// apply its own value. See [`super::api::savedvars`], whose module note is
    /// the ordering argument.
    saved_values: super::api::savedvars::SavedValues,
    /// **The addon board** — every addon the folder and the archives carry,
    /// which of them each character has on, which have loaded, and their
    /// saved files until they do. See [`super::panels::addons`]. Seeded by
    /// `crate::settings::addons`; read by [`Self::load_interface`] for the
    /// order to load in.
    addons: super::panels::addons::Held,
    /// **Whether `SavedVariables.lua` has been put onto *this* host yet.**
    ///
    /// On the host rather than beside the file, and that is the whole point: a
    /// session builds more than one of these — the glue directory is swapped for
    /// `Interface\FrameXML\` on the way into the world and back again on the way
    /// out — so a one-shot kept in the resource fires on the login screen's host
    /// and never on the world's, which is every `uvar` in the options panel
    /// silently reverting at login. `false` by construction here, so a fresh host
    /// asks to be seeded and cannot be forgotten. See
    /// [`crate::settings::savedvars`], and [`super::super::game::session::keybindings`],
    /// which had the identical bug and fixed it the identical way.
    saved_applied: bool,
    /// …and the party's six — see [`super::panels::party`].
    party: super::panels::party::Queue,
    /// …and the raid's nine, which are a second queue rather than more of the
    /// first because the module that drains them is a second one — see
    /// [`super::panels::raid`] and [`crate::interface::raid`].
    raid: super::panels::raid::Queue,
    /// …and the reputation panel's, which is **state rather than a queue** for
    /// the reason [`super::panels::charcreate`]'s board is: `ReputationBar_OnClick`
    /// re-reads the whole list inside the handler that changed it. The three
    /// writes that owe a packet queue beside it.
    reputation: super::panels::reputation::Held,
    reputation_queue: super::panels::reputation::Queue,
    /// …and the social panel's, held for exactly the same reason — see
    /// [`super::panels::social`], whose module note says so at length.
    social: super::panels::social::Held,
    social_queue: super::panels::social::Queue,
    /// …and the chat channels' — see [`super::panels::channels`].
    channels: super::panels::channels::Held,
    channels_queue: super::panels::channels::Queue,
    /// …and the skills panel's, which is state for the same reason and has no
    /// queue at all: not one of its writes owes a packet — see
    /// [`super::panels::skills`], where `AbandonSkill`'s absence is argued.
    skills: super::panels::skills::Held,
    /// …and the key bindings', which is state for a third reason: it is the
    /// table the *keyboard* reads, so there is only one copy of it in the
    /// client and both [`Self::fire`] and the panel are looking at that one.
    /// See [`super::panels::keybindings`].
    keybindings: super::panels::keybindings::Held,
    /// …and the fifth, for the two screens before the world — see
    /// [`super::panels::glue`]. Logging in owns a socket, so `DefaultServerLogin` cannot
    /// be anything but a record.
    glue: super::panels::glue::GlueQueue,
    /// …and the third glue screen's, which is one request and one board — see
    /// [`super::panels::charcreate`]. The board is **state rather than a queue**,
    /// because every read on that screen has to answer inside the handler that
    /// wrote it.
    char_create: super::panels::charcreate::Held,
    char_create_queue: super::panels::charcreate::Queue,
    /// …and the seventh, for what is taken off a body — see [`super::panels::loot`].
    loot: super::panels::loot::Queue,
    /// …and its group half: which way the player rolled — see
    /// [`super::panels::lootroll`].
    loot_roll: super::panels::lootroll::Queue,
    /// …and the eighth, for the quest conversation — see [`super::panels::quest`].
    quest: super::panels::quest::Queue,
    /// …and the ninth and tenth, for the two windows a right-click on an NPC
    /// opens — see [`super::panels::gossip`] and [`super::panels::merchant`].
    gossip: super::panels::gossip::Queue,
    merchant: super::panels::merchant::Queue,
    /// …and the mailbox's, which is a window with no NPC behind it — see
    /// [`super::panels::mail`].
    mail: super::panels::mail::Queue,
    /// …and the eleventh, for the third of those windows — see
    /// [`super::panels::trainer`].
    trainer: super::panels::trainer::Queue,
    /// …and the stable master's four — see [`super::panels::stable`].
    stable: super::panels::stable::Queue,
    /// …and the bank's two — see [`super::panels::bank`].
    bank: super::panels::bank::Queue,
    /// …and the page window's three — see [`super::panels::pagetext`].
    pagetext: super::panels::pagetext::Queue,
    /// …and the trade window's ten — see [`super::panels::trade`].
    trade: super::panels::trade::Queue,
    /// …and the duel's four — see [`super::panels::duel`].
    duel: super::panels::duel::Queue,
    /// …and the talent panel's one, drained by
    /// [`crate::interface::talents`].
    talent: super::panels::talent::Queue,
    /// …and the twelfth, for the flight master's map — see [`super::panels::taxi`].
    taxi: super::panels::taxi::Queue,
    /// …and the sixth, for the unit frames' faces — see [`super::api::portrait`].
    /// Taking a picture of somebody needs a camera and a model, which are the
    /// renderer's and not this state's.
    portrait: super::api::portrait::PortraitQueue,
    /// The five reads the directory *stores* rather than calls — see
    /// [`api::Held`], which is where the reason is.
    held: api::Held,
    /// Verb names the interface asked for and this client does not have, and
    /// handler errors, recorded once each. See the module comment on why not
    /// silently.
    missing: BTreeSet<String>,
    /// **What a Ctrl-C asked to be put on the clipboard**, until the frame's
    /// keyboard system takes it — see [`LuaHost::take_copied`]. A plain field
    /// rather than one of the `Rc<RefCell<_>>` queues beside it, because the
    /// only writer is [`LuaHost::keyboard`] itself and no Lua closure holds it.
    copied: Option<String>,
    /// What loading a directory of markup produced, and **which** directory it
    /// was. `None` before either has been loaded.
    ///
    /// The pair rather than two fields because the two are never both up: this
    /// client runs `Interface\GlueXML\` before there is a world and
    /// `Interface\FrameXML\` after, in one state, with the state thrown away
    /// between — see [`LuaHost::load_glue`].
    interface: Option<(Directory, xml::Report)>,
    /// The heap as [`LuaHost::pace_collector`] left it last frame, so that the
    /// next pace can see **how much this frame allocated**. A `Cell` because
    /// pacing takes `&self` and is called from a system that has one.
    settled: Cell<usize>,
}

impl LuaHost {
    /// A fresh state with the four libraries 1.12 opens, this client's verbs
    /// registered, and the frame object model installed.
    ///
    /// The reads are **not** registered here — they cannot be, since they borrow
    /// the world. See [`LuaHost::run`].
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
        let talent: super::panels::talent::Queue = Rc::new(RefCell::new(Vec::new()));
        let taxi: super::panels::taxi::Queue = Rc::new(RefCell::new(Vec::new()));
        let held: api::Held = Rc::new(RefCell::new(api::HeldReads::default()));
        api::install_held(&lua, &held)?;
        // **Before the flatten**, so that `tremove` is the 5.0 body too.
        restore_5_0_table_remove(&lua)?;
        flatten_libraries(&lua)?;
        verbs::register(&lua, &queue, &said, &renamed, &emoted)?;
        // **The map's three writes, on the same terms as the chat's**: a
        // handler cannot move the view directly, because the world is borrowed
        // for the length of the call — so it records and `interface::worldmap`
        // applies. See `super::panels::worldmap`.
        super::panels::worldmap::register(&lua, &map)?;
        // …and the sound's four, drained by `crate::sound::interface`.
        super::api::sound::register(&lua, &sound)?;
        // …and the client's own settings, drained by `crate::settings::cvars`.
        // **Early**, because it seeds the store as well as registering the three
        // globals, and an `OnLoad` that reads a CVar runs long after either.
        super::api::cvars::register(&lua, &cvars)?;
        // …and the *other* settings store, beside it because it is the same
        // subject seen from the interface's other habit — a `uvar` row rather
        // than a `cvar` one. See [`super::api::savedvars`].
        super::api::savedvars::register(&lua, &saved, &saved_values)?;
        // …and the party's six, drained by `crate::interface::party`.
        super::panels::party::register(&lua, &party)?;
        // …and the raid's nine, drained by `crate::interface::raid`.
        super::panels::raid::register(&lua, &raid)?;
        // …and the reputation panel's eleven, whose state is held here and whose
        // three packets `crate::interface::reputation` drains.
        super::panels::reputation::register(&lua, &reputation, &reputation_queue)?;
        // …and the social panel's sixteen, whose state is held here and whose
        // six packets `crate::interface::social` drains.
        super::panels::social::register(&lua, &social, &social_queue)?;
        // …and the chat channels' twenty-five, whose board is here and whose
        // two packets `crate::interface::channels` drains.
        super::panels::channels::register(&lua, &channels, &channels_queue)?;
        // …and the skills panel's eleven, whose board the same module feeds.
        super::panels::skills::register(&lua, &skills)?;
        // …and the key bindings panel's eight, whose board is also what
        // [`Self::fire`] resolves a key press through. `RunBinding` is the
        // ninth and is not here — it runs a `<Binding>` body, which needs this
        // type rather than the board; `crate::input::bindings` registers it.
        super::panels::keybindings::register(&lua, &keybindings)?;
        // …and the addon list's sixteen, over a board `crate::settings::addons`
        // seeds. Unscoped, because `LoadAddOn` runs the loader itself and the
        // loader cannot run inside a scoped read.
        super::panels::addons::register(&lua, &addons)?;
        // …and the glue's, drained by `crate::glue::glue`. Registered
        // unconditionally rather than only while the glue is up: the names cost
        // nothing when `Interface\FrameXML\` is the loaded directory (nothing in
        // it calls one), and a registration that came and went with the screen
        // would be a second piece of state to keep in step with the swap.
        //
        // **After the sound**, because `PlayGlueMusic` forwards to `PlayMusic`.
        super::panels::glue::register(&lua, &glue)?;
        // …and the third glue screen's, whose reads are unscoped because none of
        // them touches the world: what a character *may* be made of is entirely
        // `vale_assets::tables::charcreate`'s. See that module's own first line.
        super::panels::charcreate::register(&lua, &char_create, &char_create_queue)?;
        // …and the unit frames' one, drained by `crate::render::portraits`.
        // **Before the object model**, which is only a convention here — it
        // registers a global rather than a method — but keeps the one rule that
        // matters visible: [`super::api::stubs`] goes last, so a name it still
        // carries cannot shadow a real function.
        super::api::portrait::register(&lua, &portrait)?;
        // …and the loot window's two, drained by `crate::interface::loot`.
        super::panels::loot::register(&lua, &loot)?;
        // …and the roll frame's two, drained by `crate::interface::lootroll`.
        super::panels::lootroll::register(&lua, &loot_roll)?;
        // …and the quest panel's, drained by `crate::interface::quest`.
        super::panels::quest::register(&lua, &quest)?;
        // …and the gossip and merchant windows', drained by their own modules.
        super::panels::gossip::register(&lua, &gossip)?;
        super::panels::merchant::register(&lua, &merchant)?;
        // …and the mail window's ten, drained by `crate::interface::mail`.
        super::panels::mail::register(&lua, &mail)?;
        // …and the trainer's seven, drained by `crate::interface::trainer`.
        super::panels::trainer::register(&lua, &trainer)?;
        // …and the stable's four, drained by `crate::interface::stable`.
        super::panels::stable::register(&lua, &stable)?;
        // …and the bank's two, drained by `crate::interface::bank`.
        super::panels::bank::register(&lua, &bank)?;
        // …and the page window's three, drained by `crate::interface::pagetext`.
        super::panels::pagetext::register(&lua, &pagetext)?;
        // …and the trade window's ten, drained by `crate::interface::trade`.
        super::panels::trade::register(&lua, &trade)?;
        // …and the duel's four, drained by `crate::interface::duel`.
        super::panels::duel::register(&lua, &duel)?;
        // …and the talent panel's one — see [`super::panels::talent`], where
        // the five reads beside it are and why they are scoped instead.
        super::panels::talent::register(&lua, &talent)?;
        // …and the flight map's three, drained by `crate::interface::taxi`.
        super::panels::taxi::register(&lua, &taxi)?;
        frames::install(&lua)?;
        // **After the object model**, because two of these take a frame — and
        // last, so that a name here can never shadow a real verb by accident.
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
            talent,
            taxi,
            bindings: None,
            missing: BTreeSet::new(),
            copied: None,
            interface: None,
            settled: Cell::new(0),
        })
    }

    /// Whether the game's own binding table has been read yet.
    pub fn loaded(&self) -> bool {
        self.bindings.is_some()
    }

    /// **Load `Interface\FrameXML\` — all of it.** Once, at the first login.
    ///
    /// Run inside the same scope every other chunk is, because an `OnLoad` asks
    /// questions: `TargetFrame_OnLoad` and its neighbours reach for the reads,
    /// and outside a scope those names are nil. That is also why this cannot
    /// happen at host construction, which is before there is a world to answer
    /// from.
    ///
    /// The [`xml::Report`] is kept for the HUD and its errors are folded into the
    /// same `missing` set a binding's are, so the one line on screen counts
    /// everything the interface has asked for and not got.
    pub fn load_interface(
        &mut self,
        answers: &dyn Answers,
        read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    ) {
        // **The typefaces first, because the load measures with them.** An
        // `OnLoad` that sizes a tab to its label (six of them do) asks how wide
        // its text is before this function returns, so a set installed
        // afterwards would leave those six sized by the fallback estimate for
        // the life of the session. See [`super::widgets::text::width`].
        super::widgets::text::install_faces(&self.lua, vale_assets::interface::font::Faces::load(read));
        let addons = Rc::clone(&self.addons);
        let report = self
            .run(answers, |lua| {
                // **`FrameXML`, then every addon the board says to load, in
                // the board's order.** The seven shipped `Blizzard_*` addons
                // come first and load eagerly, which is the departure
                // `assets::toc`'s own constants state; the folder's own follow,
                // each after its dependencies. Every addon goes through the same
                // template registry `FrameXML` filled, so its `inherits` resolve
                // — see [`xml::Loader::load_tocs`]. Each addon's load runs its
                // saved files and raises `ADDON_LOADED` for it, which is
                // [`super::panels::addons::load_with`]'s job so that
                // `LoadAddOn` from Lua does exactly the same thing.
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
                // **The first of three events the load itself raises.** The real client
                // fires `UPDATE_CHAT_WINDOWS` at login once the windows exist,
                // and `FloatingChatFrame_Update` applies a window's colour,
                // alpha and shown state *only* on that event — a client that
                // never raises it leaves every chat texture untinted, and
                // `ChatFrameBackground` untinted is an opaque white sheet.
                // Inside the same scope, so the handlers' reads answer.
                report
                    .errors
                    .extend(frames::fire(lua, "UPDATE_CHAT_WINDOWS", &[])?);
                // **…and the second, for the same reason and from the same
                // place.** `ChatTypeInfo["SAY"]` ships with `sticky` and
                // nothing else: a type's `r`, `g` and `b` are written *only*
                // by `ChatFrame_OnEvent`'s `UPDATE_CHAT_COLOR` arm, and
                // `AddMessage` with a nil colour draws white. So every line
                // in the game came out white until this ran — the system
                // messages, the whispers, the loot. The table is the
                // client's own; see `assets::interface::chattype`, which is
                // where it lives now that the combat log's routing indexes the
                // same 94 rows.
                // **…and the ten numbered channels take `CHANNEL`'s colour.**
                // `ChatTypeInfo["CHANNEL1".."CHANNEL10"]` are rows of the
                // directory's own with no colour of their own in the table
                // or in the reference's `chat-cache.txt`, and a channel line
                // is drawn under `"CHANNEL"..arg8`, so without these ten
                // every channel line is white.
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
                // **A channel is sticky here, by request.** The shipped
                // `ChatTypeInfo["CHANNEL"]` is `{ sticky = 0 }` and the client
                // never touches the field, so 1.12's own
                // `ChatEdit_OnEnterPressed` sends `/2 hello` and opens the
                // next line as a say. This client keeps the channel, and
                // `/s` is the way back. One field in the directory's own
                // table, set after it loads; a stated deviation.
                lua.load("ChatTypeInfo.CHANNEL.sticky = 1").exec()?;
                // **…and the third, which is the eager addon load's own bill.**
                // `Blizzard_RaidUI` is the first of the five whose frames are
                // *shown* by default: forty `RaidGroupButton`s parented to
                // `RaidFrame`, hidden only by `RaidGroupFrame_Update`'s `else`
                // branch. The reference never sees them, because it loads that
                // addon from `RaidFrame_OnEvent` and the line after the load is
                // `RaidFrame_Update()`. This client loads it at login instead,
                // and `RaidFrame_OnLoad` — which would have called the update —
                // ran while the addon did not exist, so nothing ever hid them
                // and a solo character's Raid tab showed forty empty buttons
                // stacked on the origin. `RAID_ROSTER_UPDATE` is the name the
                // reference's own load pairs with; raised here it means what it
                // says.
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
        // **One full collection, here and nowhere else.** The load's transients
        // are tens of megabytes and the paced regime that takes over from the
        // next frame ([`Self::pace_collector`]) is deliberately too gentle to
        // dig out from under them — so the regime starts from the live set,
        // once, at a moment the player is looking at a loading transition.
        let _ = self.lua.gc_collect();
    }

    /// **Load `Interface\GlueXML\`** — the login screen and character select.
    ///
    /// The same machinery as [`LuaHost::load_interface`] pointed at the other
    /// `.toc`, and the two are alternatives rather than layers — see
    /// [`Directory`], which is where the argument for that is.
    ///
    /// **One event is raised at the end and it is not optional.**
    /// `GlueParent_OnEvent`'s first arm answers `FRAMES_LOADED` by calling
    /// `LocalizeFrames()`, which is `GlueLocalization.lua`'s pass over the
    /// screens' captions and anchors — so a client that never raises it draws
    /// both screens un-localised. It is raised here rather than through
    /// `interface::events` for the same reason `UPDATE_CHAT_WINDOWS` is: it is about
    /// the *load* and there is no session to write a message from.
    ///
    /// `SET_GLUE_SCREEN` is deliberately **not** raised here. Nothing is shown
    /// until `crate::glue::glue`'s own watcher notices which screen the session
    /// is on, which is the frame after — and that is one mechanism deciding what
    /// is on screen rather than two that can disagree.
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
    }

    /// **Tell the state how big the screen is**, before anything is loaded into
    /// it.
    ///
    /// [`LuaHost::drawn`] does this every frame, which is enough for *layout* —
    /// a rectangle is solved on demand — and is not enough for the interface's
    /// **latches**: an `OnLoad` that reads `GetScreenWidth()` and sizes a texture
    /// from it runs exactly once, and it ran against
    /// [`super::widgets::layout::DEFAULT_SCREEN`]. See
    /// [`super::widgets::layout::UI_SIZE`], which is what it is told instead,
    /// and [`super::widgets::layout::VIRTUAL_WIDTH`] for why that is a constant.
    pub fn set_screen(&self, width: f64, height: f64) {
        let _ = super::widgets::layout::set_screen(&self.lua, width, height);
    }

    /// What the last load produced, or `None` before there was one.
    pub fn interface(&self) -> Option<&xml::Report> {
        self.interface.as_ref().map(|(_, report)| report)
    }

    /// **…and which directory it was**, which is the one question the swap makes
    /// answerable: `Some(Glue)` at a login screen, `Some(Frame)` in the world.
    pub fn directory(&self) -> Option<Directory> {
        self.interface.as_ref().map(|(which, _)| *which)
    }

    /// The state itself, for this directory's own instruments — the audit's
    /// `--draw` dump walks tables the way [`LuaHost::drawn`] does. `pub(super)`
    /// on purpose: nothing outside `lua/` may hold the `mlua::Lua`, for the
    /// reason the module comment gives, and nothing given this reference may
    /// run a chunk with it.
    pub(super) fn state(&self) -> &mlua::Lua {
        &self.lua
    }

    /// **How many bytes the interpreter's heap holds live**, for an instrument
    /// outside this directory to report. The number [`Self::pace_collector`]
    /// paces against; see [`Self::GC_BEHIND_BYTES`] for where it settles.
    pub fn heap_bytes(&self) -> usize {
        self.lua.used_memory()
    }

    /// **Everything the interface would draw this frame, back to front.**
    ///
    /// The one read that is not a Lua call, and it stays a method here rather
    /// than becoming a `lua()` accessor for the reason the module comment gives:
    /// letting the rest of the client hold the `mlua::Lua` would reopen the
    /// window where a chunk runs outside a scope. This borrows the state, walks
    /// tables, and hands back plain data — see [`super::widgets::draw`], which is where
    /// the decision is and which is testable with no window at all.
    pub fn drawn(&self, screen: (f32, f32), now: f64) -> Vec<super::widgets::draw::Item> {
        // The screen first, because every rectangle in the interface is measured
        // from `UIParent`, and `UIParent` *is* the screen.
        let _ = super::widgets::layout::set_screen(&self.lua, screen.0 as f64, screen.1 as f64);
        // …and the clock, which is what says an error message is five seconds
        // old. One stamp a frame, read by `AddMessage` and by the draw — see
        // [`super::widgets::messages::set_now`].
        super::widgets::messages::set_now(&self.lua, now);
        super::widgets::draw::collect(&self.lua)
    }

    /// **What 3D scene the interface is showing**, or `None`.
    ///
    /// A method here rather than a `lua()` accessor for the reason the module
    /// comment gives about [`LuaHost::drawn`]: nothing outside this directory
    /// may hold the `mlua::Lua`. It borrows the state, walks one small registry
    /// list, and hands back plain data — see [`super::widgets::model`], which is where
    /// the decision is and which is testable with no window.
    ///
    /// **`None` for the whole of a session in the world, and that is enforced
    /// here rather than assumed.**
    ///
    /// This used to say that no `<Model>` in `Interface\FrameXML\` is ever
    /// visible in the world — fourteen portraits, all inside hidden panels — and
    /// that was **wrong by one, in the worst possible place**.
    /// `CooldownFrameTemplate` (`Cooldown.xml`) is a `<Model>` holding
    /// `Interface\Cooldown\UI-Cooldown-Indicator.mdx`, one instance per action
    /// button, and `CooldownFrame_SetTimer` *shows* it the moment anything goes
    /// on cooldown. The reader is `crate::render::glue`, which builds a
    /// **full-screen** scene out of whatever this answers, aims the world camera
    /// through the model's own camera and takes the sky, the stars, the moons
    /// and the fog down while it is up — so the first spell cast with a
    /// cooldown replaced the world with a two-centimetre clock face.
    ///
    /// The gate is the directory rather than a list of frame names, because the
    /// claim being made is about a *screen*: this answers the backdrop of the
    /// two screens before the world, and `Interface\FrameXML\`'s own model
    /// frames are portraits that want drawing inside their own rectangles —
    /// which is work this client has not done, and not a scene.
    pub fn glue_scene(&self) -> Option<super::widgets::model::Scene> {
        if self.directory() != Some(Directory::Glue) {
            return None;
        }
        super::widgets::model::visible(&self.lua)
    }

    /// **Hand the interface the keyboard's modifier state** — see
    /// [`super::api::stubs::set_modifiers`], which is where the argument for it is.
    ///
    /// A method here rather than a free function so that nothing outside this
    /// directory holds the `mlua::Lua`, which is the rule the module comment
    /// makes about [`LuaHost::drawn`] for the same reason.
    pub fn set_modifiers(&self, shift: bool, control: bool, alt: bool) {
        super::api::stubs::set_modifiers(&self.lua, shift, control, alt);
    }

    pub fn set_bindings(&mut self, bindings: std::sync::Arc<vale_assets::interface::bindings::Bindings>) {
        // **Both halves, from one call.** The declarations decide what a key
        // *does* (here) and what the key-bindings panel *lists*
        // (`keybindings::Keys`), and a client that set one and forgot the other
        // would draw an empty panel over a working keyboard.
        self.keybindings
            .borrow_mut()
            .set_declarations(std::sync::Arc::clone(&bindings));
        self.bindings = Some(bindings);
    }

    /// The declaration for a name, if the game declares one.
    pub fn declaration(&self, name: &str) -> Option<&vale_assets::interface::bindings::BindingDecl> {
        self.bindings.as_ref()?.get(name)
    }

    /// **The one way into Lua.** Opens a scope, lends the interface the world
    /// through [`Answers`], and runs `body` inside it.
    ///
    /// Every read the chunk makes is answered from `answers` *during* the call
    /// and every write it makes lands on the verb queue. Nothing else in this
    /// client may execute a chunk — see the module comment.
    pub(super) fn run<R>(
        &self,
        answers: &dyn Answers,
        body: impl FnOnce(&mlua::Lua) -> mlua::Result<R>,
    ) -> mlua::Result<R> {
        self.lua.scope(|scope| {
            let filled = api::install(&self.lua, scope, answers, &self.held, &self.queue);
            let outcome = filled.and_then(|()| body(&self.lua));
            // **Emptied here, inside the scope, while the functions in it are
            // still alive.** A store emptied after the scope closed would hold
            // destructed callbacks for the width of one statement, and what
            // reaches through it in that window is not an error the caller can
            // do anything about. Cleared on the way out of a failed install
            // too, which is why the `?` above became an `and_then`. See
            // [`super::scoped`].
            super::scoped::clear(&self.lua);
            outcome
        })
    }

    /// **Run a binding's own Lua body**, and return whatever verbs it called.
    ///
    /// `down` becomes the `keystate` global, which is what the hundred
    /// `runOnUp` bodies branch on. A binding without `runOnUp` is run on the
    /// press only — the caller's business, since it is the caller that knows
    /// which edge this is.
    ///
    /// An error is recorded and swallowed: a bad chunk must not take the frame
    /// down, and the *reason* it is bad is almost always a verb this client has
    /// not written.
    pub fn fire(&mut self, name: &str, down: bool, answers: &dyn Answers) -> Vec<Binding> {
        let Some(decl) = self.bindings.as_ref().and_then(|b| b.get(name)) else {
            // A key bound to a name the game does not declare. Recorded under
            // the *binding* name rather than a verb's, which is right: nothing
            // was called.
            self.note(name.to_string());
            return Vec::new();
        };
        let body = decl.body.clone();
        self.queue.borrow_mut().clear();
        let run = self.run(answers, |lua| {
            lua.globals()
                .set("keystate", if down { "down" } else { "up" })?;
            // Compiled, then run through Lua's own `pcall` — see
            // [`frames::protected`]. A key bound to a verb this client has not
            // written raises on **every press**, and once FrameXML is loaded the
            // difference between the two ways of catching that is 31 ms and 8 µs.
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

    /// **Tell every frame that registered for `event`.**
    ///
    /// The other half of the interface's shape: a binding is the player pushing
    /// something in, and this is the world pushing something out. See
    /// [`super::widgets::frames::fire`] for the calling convention and the order.
    ///
    /// Returns whatever verbs the handlers called, on the same queue a binding
    /// body writes to — because a handler *may* call one. `TargetFrame.lua`'s
    /// own `OnEvent` does not, but nothing stops an addon's from doing so, and
    /// dropping them silently would be a verb that works from a key and not from
    /// an event.
    pub fn fire_event(
        &mut self,
        event: &str,
        args: &[EventArg],
        answers: &dyn Answers,
    ) -> Vec<Binding> {
        self.fire_events(std::slice::from_ref(&(event, args)), answers)
    }

    /// **…and a whole frame's news at once, in one scope.**
    ///
    /// The batching is not tidiness, it is the largest single per-frame cost the
    /// interface had. Opening a scope means re-installing the entire scoped read
    /// API — some three hundred `scope.create_function` calls and three hundred
    /// global writes — and that costs **0.13 ms and 27 KB of Lua garbage every
    /// time**, measured by `--audit --spin`'s own `api scope` line, which exists
    /// to be the floor under every other phase. A handler body is ~0.03 ms
    /// against it.
    ///
    /// So one scope per *event* made the interface's cost scale with how much
    /// news the world had, which is exactly how a party is felt: a group of four
    /// moves seven watched units' health on the same tick
    /// ([`crate::interface::vitals`]' `WATCHED`), and seven scopes is
    /// **0.9 ms and 190 KB of garbage** for about 0.2 ms of handler. Measured
    /// with `--audit --spin --party 4` before and after; the numbers are in the
    /// rendering facts.
    ///
    /// Two things this keeps that a naive batch would lose. **One bad event does
    /// not eat the rest** — `frames::fire` is called per event and its `Err` is
    /// noted and stepped over, so a scope-level failure in the middle of a batch
    /// still delivers what comes after it. And the verbs come back as one list
    /// rather than per event, which is what the caller wanted anyway: it writes
    /// every one of them as a `BindingPressed` in the order they were called.
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

    /// How many frames are carrying an `OnUpdate` — the HUD's number, and the
    /// count of what a tick actually walks.
    pub fn ticking(&self) -> usize {
        super::api::update::tracked(&self.lua)
    }

    /// Is any frame carrying an `OnUpdate` at all?
    ///
    /// Asked once a frame before anything else happens, so that a client at the
    /// login screen — or one whose interface has not loaded — pays a single
    /// table read rather than opening a scope.
    pub fn has_updates(&self) -> bool {
        self.ticking() > 0
    }

    /// **Tick the interface's own clock**: every visible frame's `OnUpdate`, with
    /// the elapsed seconds in `arg1`.
    ///
    /// Returns whatever verbs those bodies called, on the same queue a key's
    /// binding writes to — and for the same reason [`LuaHost::fire_event`] does:
    /// a verb that works from a key and not from an animation would be a silent
    /// asymmetry. `UIParent`'s own handler calls `RequestBattlefieldPositions()`
    /// on every tick, so this is not hypothetical.
    /// **One tick of the interface's animation, in one scope** — every visible
    /// frame's `OnUpdate` and then every `<Model>`'s own clock.
    ///
    /// The two used to be a scope each, in two systems a `.after()` apart, and
    /// the second one bought nothing for it: a scope is ~0.13 ms and ~27 KB of
    /// Lua garbage before a single body runs (see [`Self::fire_events`], where
    /// that number comes from), and these two walks are the *same instant* of
    /// the *same clock* — `InterfaceClock::due` gates both.
    ///
    /// The model **loading** is deliberately still outside, in
    /// [`super::widgets::model`]'s own system: it reads an archive and wants
    /// `UiModels` mutably, neither of which belongs inside a Lua scope. That
    /// system is ordered `.before` this one, so a file is in hand by the time
    /// the tick asks how long its sequence is — the order the two halves were
    /// already in when they shared a system.
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

    /// Which files the visible `<Model>` frames are holding — see
    /// [`super::widgets::model::files`], whose caller loads them.
    pub fn model_files(&self) -> Vec<String> {
        super::widgets::model::files(&self.lua)
    }

    /// **Every visible `<PlayerModel>` frame that is pointed at a unit**, for
    /// [`crate::render::paperdoll`] — the paper dolls, against
    /// [`Self::model_files`]'s pictures of files.
    pub fn unit_models(&self) -> Vec<super::widgets::model::UnitFrame> {
        super::widgets::model::unit_frames(&self.lua)
    }

    /// **Tick every visible `<Model>` frame**: `OnUpdateModel` every frame, and
    /// `OnAnimFinished` when the sequence it is playing has run out.
    ///
    /// The same shape as [`LuaHost::fire_updates`] one script kind over, and it
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

    /// **Hand the interface the pointer**, and say which frame it landed on.
    ///
    /// The second half of the answer is what [`super::api::mouse::MouseFocus`] carries
    /// down to `interface::target`: a click the interface took is a click the world
    /// must not also act on.
    /// The **third** is whether a frame took the wheel — see
    /// [`super::api::mouse::wheel_was_taken`], which is what stops the camera zooming
    /// while a list is being scrolled.
    pub fn mouse(
        &mut self,
        pointer: &super::api::mouse::Pointer,
        answers: &dyn Answers,
    ) -> (Vec<Binding>, Option<String>, bool) {
        self.queue.borrow_mut().clear();
        let over = self.run(answers, |lua| {
            let errors = super::api::mouse::dispatch(lua, pointer)?;
            // Read back rather than solved again: the dispatch has just decided
            // it, and a second hit test could disagree with the one the handlers
            // were fired for.
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

    /// **Hand the interface the keyboard**, and say which box still has it.
    ///
    /// The twin of [`LuaHost::mouse`], one input device over: the strokes go to
    /// whichever [`super::widgets::editbox`] holds the focus, and the second half of the
    /// answer is what [`super::api::keyboard::KeyboardFocus`] carries down to the key
    /// table — a key the interface took is a key no binding may also fire.
    ///
    /// Verbs come back because the handlers a stroke runs are the game's own:
    /// `OnEnterPressed` reaches `SendChatMessage`, and `ChatEdit_ParseText`'s
    /// `SlashCmdList` arms reach `TargetUnit` and its neighbours.
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

    /// **Hand a key edge to whichever frame has the keyboard**, and say whether
    /// one did.
    ///
    /// The middle branch of the switch in [`super::api::keyboard`]: an edit box
    /// with the focus takes a *character*, a frame with `enableKeyboard` takes a
    /// *key*, and anything neither of them wanted goes to the key table. `false`
    /// here is what lets `W` still walk the character when no panel is up.
    ///
    /// One scope for the whole frame's keys rather than one per key, which is
    /// the arithmetic [`Self::fire_events`] makes and for the same reason: a
    /// scope is 0.12 ms and 27.8 KB, and a key repeat is several edges.
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

    /// **How many frames ever declared themselves keyboard receivers** — the
    /// census `--audit` prints. See [`super::widgets::keyboard`].
    pub fn keyboard_receivers(&self) -> usize {
        super::widgets::keyboard::receiver_count(&self.lua)
    }

    /// …and their names, with a `*` on the ones visible right now — which is
    /// the half that matters, since a receiver that is always up would take
    /// every key in the game away from the key table.
    pub fn keyboard_receiver_names(&self) -> Vec<String> {
        super::widgets::keyboard::receiver_names(&self.lua)
    }

    /// **Is a frame holding the keyboard right now?** — asked once a frame, so
    /// that a panel being up releases the movement controls even on a frame
    /// where no key changed.
    ///
    /// Cheap by construction: the population is a registry of the frames that
    /// declared themselves receivers, not a walk of the tree. See
    /// [`super::widgets::keyboard`].
    pub fn keyboard_frame(&self) -> Option<String> {
        super::widgets::keyboard::receiver_name(&self.lua)
    }

    /// **What the last keystrokes asked to be put on the clipboard**, taken.
    ///
    /// A Ctrl-C decides *what* is copied inside [`super::widgets::editbox`], which has no
    /// business opening a window-system clipboard, and the copying itself is
    /// [`super::api::keyboard`]'s — the same split every other outside edge in this
    /// directory has. `None` covers both "nothing was copied" and "nothing was
    /// selected", which the reference treats identically: it leaves whatever is
    /// on the clipboard alone.
    pub fn take_copied(&mut self) -> Option<String> {
        self.copied.take()
    }

    /// The name of the edit box holding the keyboard, if one is.
    ///
    /// Asked once a frame before any key is looked at, so a client with nothing
    /// focused — which is all of it, most of the time — pays one registry read
    /// rather than opening a scope.
    pub fn keyboard_focus(&self) -> Option<mlua::Table> {
        super::widgets::editbox::focused(&self.lua)
    }

    /// **What the interface asked the client to say**, drained.
    ///
    /// One caller, [`crate::interface::chat::send`], for the same reason there is one
    /// caller of `take_chat`: this empties itself.
    pub fn take_said(&mut self) -> Vec<verbs::Said> {
        std::mem::take(&mut *self.said.borrow_mut())
    }

    /// …and the text emotes asked for, taken and cleared. One caller,
    /// [`crate::interface::emotetext`].
    pub fn take_emoted(&mut self) -> Vec<verbs::Emoted> {
        std::mem::take(&mut *self.emoted.borrow_mut())
    }

    /// **…and the names typed into the rename box**, on the same terms — see
    /// [`verbs::PetRenameQueue`]. One caller,
    /// [`crate::interface::pet`].
    pub fn take_pet_renames(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.renamed.borrow_mut())
    }

    /// **…and what it asked the world map to show**, on the same terms — see
    /// [`super::panels::worldmap`]. One caller, [`crate::interface::worldmap`].
    pub fn take_map_requests(&mut self) -> Vec<super::panels::worldmap::MapRequest> {
        std::mem::take(&mut *self.map.borrow_mut())
    }

    /// **…and what it asked the party to do**, on the same terms — see
    /// [`super::panels::party`]. One caller, [`crate::interface::party`].
    pub fn take_party_verbs(&mut self) -> Vec<super::panels::party::PartyRequest> {
        std::mem::take(&mut *self.party.borrow_mut())
    }

    /// **…and the raid**, likewise — see [`super::panels::raid`]. One caller,
    /// [`crate::interface::raid`].
    pub fn take_raid_verbs(&mut self) -> Vec<super::panels::raid::RaidRequest> {
        std::mem::take(&mut *self.raid.borrow_mut())
    }

    /// **…and what it asked to hear**, on the same terms — see
    /// [`super::api::sound`]. One caller, `crate::sound::interface`.
    pub fn take_sound_requests(&mut self) -> Vec<super::api::sound::SoundRequest> {
        std::mem::take(&mut *self.sound.borrow_mut())
    }

    /// **…and every `SetCVar` since the last frame**, on the same terms — see
    /// [`super::api::cvars`]. One caller, [`crate::settings::cvars`].
    /// **Run a chunk and read a global back**, for a test in another module
    /// that needs a real interpreter rather than a bare `mlua::Lua` — the
    /// registry `RegisterForSave` writes into is this type's, so a test of the
    /// round trip cannot be written without one.
    ///
    /// `#[cfg(test)]` so it cannot become a second way to drive the interface;
    /// the one way is `run_scripts`, which announces what it did.
    #[cfg(test)]
    pub(crate) fn run_for_test(&mut self, chunk: &str) {
        self.lua.load(chunk).exec().expect("the chunk runs");
    }

    /// …and read one expression back as a string.
    #[cfg(test)]
    pub(crate) fn eval_for_test(&self, expression: &str) -> String {
        self.lua
            .load(format!("return {expression}"))
            .eval::<String>()
            .expect("the expression reads")
    }

    /// **What every `RegisterForSave`d global holds now** — read on the way
    /// out, which is where the reference reads them too. See
    /// [`super::api::savedvars`], whose module note is why this is not a queue.
    pub fn saved_variables(&self) -> Vec<(String, vale_assets::interface::wtf::SavedValue)> {
        super::api::savedvars::snapshot(&self.lua, &self.saved)
    }

    /// **…and each loaded addon's, serialised** — `(addon, scope, file text)`
    /// for every addon that has loaded and whose `.toc` declares names at that
    /// scope. An addon that has not loaded is left out, so its file on disk
    /// is not overwritten with nothing. See
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

    /// **Hand an addon's saved file to this host.** Run now if the addon has
    /// loaded, held on the board until its load otherwise — an addon's saved
    /// variables are applied over its own defaults, which do not exist before
    /// its files have run.
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

    /// **The addon board**, borrowed — see [`super::panels::addons`]. Seeded by
    /// `crate::settings::addons` and read by [`Self::load_interface`].
    pub fn addons(&self) -> &super::panels::addons::Held {
        &self.addons
    }

    /// **Has `config-cache.wtf` been put onto this host yet?** — see
    /// [`Self::saved_applied`], and [`super::super::game::session::keybindings`]'
    /// `Keys::needs_seeding`, which is the same question about the same hazard.
    ///
    /// Asked rather than remembered by the caller, because the thing that goes
    /// stale is a latch that outlives the host and there is exactly one way to
    /// make that impossible: keep it on the host.
    pub fn needs_saved_variables(&self) -> bool {
        !self.saved_applied
    }

    /// …and the way back in, before `VARIABLES_LOADED`.
    ///
    /// **Marks the host either way**, including when the values are empty: a
    /// fresh install has nothing to apply and asking again every frame for the
    /// life of the session would be a lookup that can never succeed.
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

    pub fn take_cvar_writes(&mut self) -> Vec<super::api::cvars::CVarWrite> {
        std::mem::take(&mut *self.cvars.borrow_mut())
    }

    /// …and the whole store, read once when the settings resource is built.
    pub fn cvar_snapshot(&self) -> Vec<(String, String)> {
        super::api::cvars::snapshot(&self.lua)
    }

    /// …and the other direction, once: **what `WTF\Config.wtf` carried**, put
    /// back before the interface loads.
    ///
    /// One caller, [`crate::settings::cvars`], from a `Startup` system — which is
    /// early enough because the whole of `Interface\` loads in `Update`. See
    /// [`super::api::cvars::seed`] for why this does not go on the queue.
    pub fn seed_cvars(&mut self, values: &[(String, String)]) {
        if let Err(e) = super::api::cvars::seed(&self.lua, values) {
            warn!("the saved settings would not load ({e})");
        }
    }

    /// **…and what the login and character screens asked for**, on the same
    /// terms — see [`super::panels::glue`]. One caller, [`crate::glue::glue`].
    pub fn take_glue_requests(&mut self) -> Vec<super::panels::glue::GlueRequest> {
        std::mem::take(&mut *self.glue.borrow_mut())
    }

    /// **…and whether the third glue screen pressed Accept**, on the same terms
    /// — see [`super::panels::charcreate`]. One caller, [`crate::glue::charcreate`].
    pub fn take_create_requests(&mut self) -> Vec<super::panels::charcreate::CreateRequest> {
        std::mem::take(&mut *self.char_create_queue.borrow_mut())
    }

    /// **Hand the character-create screen the tables it chooses out of.**
    ///
    /// Called once when `Interface\GlueXML\` loads, because that is the first
    /// moment there is both an archive chain and a Lua state — the host itself
    /// is built before either. Until then every read on that screen answers an
    /// empty list, which draws a create screen with no buttons rather than one
    /// with wrong buttons on it.
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

    /// **What the create screen has chosen**, for the plinth and for the packet.
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

    /// **…and whose faces it asked for**, on the same terms — see
    /// [`super::api::portrait`]. One caller, [`crate::render::portraits`].
    pub fn take_portrait_requests(&mut self) -> Vec<String> {
        std::mem::take(&mut *self.portrait.borrow_mut())
    }

    /// **The reputation panel's board**, borrowed — see
    /// [`super::panels::reputation`]. One caller,
    /// [`crate::interface::reputation`], which both writes the server's
    /// news into it and reads the version back out.
    pub fn reputation(&self) -> &super::panels::reputation::Held {
        &self.reputation
    }

    /// **The key table**, borrowed — see [`super::panels::keybindings`].
    ///
    /// Two callers and they are the two ends of it: `crate::input::bindings`,
    /// which reads the live set to turn a keystroke into a binding name, and
    /// `crate::settings::keybindings`, which fills the saved sets from the
    /// two files at login and writes them back on the way out.
    pub fn keybindings(&self) -> &super::panels::keybindings::Held {
        &self.keybindings
    }

    /// **…and the skills panel's board**, on the same terms — see
    /// [`super::panels::skills`]. One caller,
    /// [`crate::interface::skills`].
    pub fn skills(&self) -> &super::panels::skills::Held {
        &self.skills
    }

    /// …and the three packets it owes, drained.
    /// **The social panel's board**, borrowed — see [`super::panels::social`].
    /// One caller, [`crate::interface::social`], which both writes the
    /// server's answers into it and reads what the panel asked for.
    pub fn social(&self) -> &super::panels::social::Held {
        &self.social
    }

    /// …and the six verbs it owes the server, taken and cleared.
    pub fn take_social_verbs(&mut self) -> Vec<vale_protocol::socket::session::SocialVerb> {
        std::mem::take(&mut *self.social_queue.borrow_mut())
    }

    /// **The chat channels' board**, borrowed — see [`super::panels::channels`].
    /// Two callers: [`crate::interface::channels`], which fills it, and
    /// [`crate::interface::chat`], which numbers a channel line from it.
    pub fn channels(&self) -> &super::panels::channels::Held {
        &self.channels
    }

    /// …and the verbs the chat frame queued about a channel, taken and cleared.
    pub fn take_channel_verbs(&mut self) -> Vec<vale_protocol::socket::session::ChannelVerb> {
        std::mem::take(&mut *self.channels_queue.borrow_mut())
    }

    pub fn take_reputation_verbs(
        &mut self,
    ) -> Vec<vale_protocol::socket::session::ReputationVerb> {
        std::mem::take(&mut *self.reputation_queue.borrow_mut())
    }

    /// **…and what it took off a body**, on the same terms — see
    /// [`super::panels::loot`]. One caller, [`crate::interface::loot`].
    pub fn take_loot_presses(&mut self) -> Vec<crate::interface::loot::TakeLoot> {
        std::mem::take(&mut *self.loot.borrow_mut())
    }

    /// **…and which way it rolled**, on the same terms — see
    /// [`super::panels::lootroll`]. One caller,
    /// [`crate::interface::lootroll`].
    pub fn take_roll_presses(&mut self) -> Vec<crate::interface::lootroll::RollPress> {
        std::mem::take(&mut *self.loot_roll.borrow_mut())
    }

    /// **…and what the quest panel pressed**, on the same terms — see
    /// [`super::panels::quest`]. One caller, [`crate::interface::quest`].
    pub fn take_quest_presses(&mut self) -> Vec<crate::interface::quest::QuestPress> {
        std::mem::take(&mut *self.quest.borrow_mut())
    }

    /// **…and what the gossip and merchant windows pressed**, on the same
    /// terms. One caller apiece.
    pub fn take_gossip_presses(&mut self) -> Vec<crate::interface::gossip::GossipPress> {
        std::mem::take(&mut *self.gossip.borrow_mut())
    }

    pub fn take_merchant_presses(&mut self) -> Vec<crate::interface::merchant::MerchantPress> {
        std::mem::take(&mut *self.merchant.borrow_mut())
    }

    /// …and the mail window's — see [`crate::interface::mail`].
    pub fn take_mail_presses(&mut self) -> Vec<crate::interface::mail::MailPress> {
        std::mem::take(&mut *self.mail.borrow_mut())
    }

    /// …and the trainer's — see [`crate::interface::trainer`].
    pub fn take_trainer_presses(&mut self) -> Vec<crate::interface::trainer::TrainerPress> {
        std::mem::take(&mut *self.trainer.borrow_mut())
    }

    /// …and the stable's — see [`crate::interface::stable`].
    pub fn take_stable_presses(&mut self) -> Vec<crate::interface::stable::StablePress> {
        std::mem::take(&mut *self.stable.borrow_mut())
    }

    /// …and the bank's — see [`crate::interface::bank`].
    pub fn take_bank_presses(&mut self) -> Vec<crate::interface::bank::BankPress> {
        std::mem::take(&mut *self.bank.borrow_mut())
    }

    /// …and the page window's — see [`crate::interface::pagetext`].
    pub fn take_page_presses(&mut self) -> Vec<crate::interface::pagetext::PagePress> {
        std::mem::take(&mut *self.pagetext.borrow_mut())
    }

    /// …and the trade window's — see [`crate::interface::trade`].
    pub fn take_trade_presses(&mut self) -> Vec<crate::interface::trade::TradePress> {
        std::mem::take(&mut *self.trade.borrow_mut())
    }

    /// …and the duel's — see [`crate::interface::duel`].
    pub fn take_duel_presses(&mut self) -> Vec<crate::interface::duel::DuelPress> {
        std::mem::take(&mut *self.duel.borrow_mut())
    }

    /// …and the talent panel's, which are `(tab, index)` pairs the ECS resolves
    /// against the tree — see [`crate::interface::talents`].
    pub fn take_talent_presses(&mut self) -> Vec<(usize, usize)> {
        std::mem::take(&mut *self.talent.borrow_mut())
    }

    /// …and the flight map's — see [`crate::interface::taxi`].
    pub fn take_taxi_presses(&mut self) -> Vec<crate::interface::taxi::TaxiPress> {
        std::mem::take(&mut *self.taxi.borrow_mut())
    }

    /// **Run one `/script` body**, and say what it did.
    ///
    /// The game's own command, and the only way in this client to reach the
    /// interface directly: `/script ActionButton1:Click()`,
    /// `/script CastingBarFrame:Show()`. It goes through [`LuaHost::run`] like
    /// everything else, so a script sees the same reads a handler does.
    ///
    /// Errors come back rather than being recorded, because unlike a handler's
    /// this one has somewhere to go — the person who typed it is looking at the
    /// chat pane. They are deliberately **not** folded into `missing`: a typo at
    /// a prompt is not a gap in the client's API and would sit on the HUD for
    /// the rest of the session claiming to be one.
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

    /// **Advance the collector by a time budget, not a byte budget.** Called
    /// once a frame.
    ///
    /// Lua 5.1's automatic collector spreads a cycle over allocation and pays
    /// the heavy end of it wherever the cycle happens to finish — measured in
    /// the audit's `--spin` as clusters of 18–25 ms frames every couple of
    /// seconds with ~4 MB falling off a ~50 MB heap across each, which on
    /// screen is the frame rate sawing 90→50→90.
    ///
    /// Three shapes of manual pacing were measured before this one. A single
    /// `gc_step_kbytes(1024)` per frame plateaued the heap but ran a whole
    /// cycle every ten frames, and one *call* still swallows the cycle's heavy
    /// end — a clockwork 16–25 ms frame every sixth of a second, which is
    /// worse to sit in front of than the cluster it replaced. A time-budgeted
    /// loop of minimal `gc_step()`s halved the spikes but not their size,
    /// because 5.1's `LUA_GCSTEP` **re-arms the automatic threshold** as a
    /// side effect — so the collector kept doing its heavy end at allocation
    /// sites mid-frame anyway. What works is both halves: the smallest step
    /// the API has, repeated until the budget is spent, and then `gc_stop()`
    /// to push the automatic threshold back out — so every increment of the
    /// cycle, atomic and sweep included, runs in this slot and nowhere else.
    /// The break on a finished cycle stops this from immediately re-marking a
    /// heap it has just swept.
    ///
    /// The budget doubles while the heap is over [`Self::GC_BEHIND_BYTES`],
    /// because with the automatic collector parked this loop is the only thing
    /// standing between an allocation burst and an unbounded heap.
    ///
    /// **And the budget follows the garbage rather than being a constant.** The
    /// 1,200 µs above was tuned against ~580 KB of Lua heap a frame, which was
    /// almost entirely `mlua`'s own — two C closures per protected table read,
    /// on every field the walks touch (see [`super::widgets::widget`]'s note on
    /// `raw_get`). With that gone the interface allocates ~29 KB a frame and a
    /// fixed 1.2 ms of collector was **36% of the whole interface frame**, spent
    /// chewing a heap that was no longer moving. So the entry price is the
    /// *floor*, and the frame's own allocation buys the rest of it at the same
    /// rate the old constant implied — 1,200 µs for 580 KB, near enough 2 µs a
    /// kilobyte. A frame that allocates nothing pays the floor; an allocation
    /// burst is still paced, and the heap ceiling above is still the backstop.
    pub fn pace_collector(&self) {
        /// What every frame pays, so that a cycle in progress always advances.
        const FLOOR: std::time::Duration = std::time::Duration::from_micros(300);
        const BUDGET: std::time::Duration = std::time::Duration::from_micros(1200);
        const BEHIND_BUDGET: std::time::Duration = std::time::Duration::from_micros(3000);
        /// Microseconds of collector per kilobyte this frame allocated.
        ///
        /// Steeper than the ratio the old constant implied (1,200 µs / 580 KB ≈
        /// 2), and deliberately: at the ~29 KB a quiet frame now allocates the
        /// proportional term is under the floor either way, so this number does
        /// nothing at idle and everything during a burst — which is the only
        /// time it is asked. Measured at 2 the heap drifted up 1.6 KB a frame
        /// over 3,000 frames; the shipped constant drifted 12.
        const PER_KB: u64 = 8;
        // Before the interface loads there is nothing worth pacing and the
        // *load* needs the automatic collector — parking it under a 175-file
        // load would balloon the heap by the load's own transients.
        if self.interface.is_none() {
            return;
        }
        let live = self.lua.used_memory();
        let budget = if live > Self::GC_BEHIND_BYTES {
            BEHIND_BUDGET
        } else {
            // Since the last pace — which is the frame, since nothing else
            // collects. Saturating, because a sweep landing inside the frame
            // leaves the heap *below* where this left it.
            let grew = live.saturating_sub(self.settled.get()) as u64 / 1024;
            std::time::Duration::from_micros(grew * PER_KB).clamp(FLOOR, BUDGET)
        };
        let deadline = std::time::Instant::now() + budget;
        loop {
            if self.lua.gc_step().unwrap_or(true) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                break;
            }
        }
        self.lua.gc_stop();
        self.settled.set(self.lua.used_memory());
    }

    /// The heap size past which [`Self::pace_collector`] stops being polite.
    /// Above the ~34 MB a freshly-loaded interface holds live, and low on
    /// purpose: the walks measurably slow as the heap grows (locality), so the
    /// equilibrium wants to sit near the live set rather than merely below
    /// anything the process would care about.
    const GC_BEHIND_BYTES: usize = 48 * 1024 * 1024;

    /// How many distinct things the interface has asked for and not got, and
    /// the first few of them.
    pub fn missing(&self) -> &BTreeSet<String> {
        &self.missing
    }

    /// **What the interface asked to be told about and nothing fires.**
    ///
    /// The one measurement this round adds, and it counts *down* as work is
    /// done: `ActionButton_OnLoad` alone registers seven events this client has
    /// never heard of, and each of them is a frame that will sit stale until
    /// something raises it.
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

    /// **Take whatever a swallowed handler raised**, and record it like anything
    /// else that broke.
    ///
    /// Called after every entry point that runs interface code. A `Show()` deep
    /// inside a chunk cannot return an error list to Rust — it is a method on a
    /// table, called from Lua — so [`frames::swallowed`] parks it and this
    /// collects it, which is the same shape [`super::api::mouse::dispatch`] uses one
    /// layer up. Without it an `OnShow` that dies takes a whole panel's contents
    /// with it and reports nothing at all.
    fn drain_swallowed(&mut self) {
        for failure in frames::take_swallowed(&self.lua) {
            self.note(failure);
        }
    }
}

/// **The library functions 1.12 has as bare globals**, as `(global, library,
/// member)`.
///
/// `format(…)` and `strlen(…)`, not `string.format` and `string.len`. The
/// directory is written entirely in that dialect — 141 `format`, 62 `strlen`, 50
/// `gsub`, 34 `floor`, 33 `ceil`, 31 `strsub`, 25 `strupper`, 20 `mod`, 19
/// `tinsert` — and stock 5.1 has none of them, so every one of those call sites
/// was `attempt to call a nil value` and took the rest of its body with it.
///
/// **This is a measurement of the files rather than a reconstruction of Lua's
/// history.** Whether 5.0 exported these itself or the client added them does
/// not matter and is not claimed here: what is checked is that
/// `Interface\FrameXML\` calls each of these names at the top level and that the
/// shipped client runs, which leaves exactly one possible answer about the
/// environment it runs in.
///
/// A member the vendored 5.1 does not have is **skipped rather than an error** —
/// `table.foreach` and `table.setn` are compile-time options in 5.1 and neither
/// is called by the directory. [`REQUIRED_FLAT_NAMES`] is the set that must
/// actually arrive, and it is the set the files call.
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
    // **`fmod`, not the `%` operator.** 5.0's `mod` is C's `fmod` and 5.1
    // renamed it; `mod(counter, 0.5)` in `PlayerFrame_OnUpdate` is the shape
    // that has to keep working.
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
/// Separate from [`FLATTENED`] because that list is deliberately generous — an
/// addon may reach for `strrep` even though the shipped interface never does —
/// and a generous list is not a check. These are the ones counted in the files.
pub const REQUIRED_FLAT_NAMES: [&str; 15] = [
    "abs", "ceil", "floor", "format", "getn", "gsub", "max", "min", "mod", "sort", "sqrt",
    "strfind", "strlen", "strsub", "strupper",
];

/// **The base-library names the directory calls.** Sorted.
///
/// Not installed by anything here — `mlua` opens the base library whatever
/// `StdLib` flags are given, and 1.12's Lua has these too. They are listed so
/// that `vale framexml` counts them as answered: a census that reports
/// `tonumber` as a function this client owes is one whose work list has to be
/// read past rather than worked down.
pub const BASE: [&str; 12] = [
    "assert", "error", "getmetatable", "ipairs", "next", "pairs", "pcall", "rawget", "rawset",
    "setmetatable", "tonumber", "tostring",
];

/// Install [`FLATTENED`] into the globals table.
///
/// Deliberately **before** the verbs and the object model, and long before the
/// loader: `Fonts.xml`'s inline `<Script>` is the third file in the `.toc` and it
/// is already using them.
/// **5.0's `table.remove`, which 5.1 replaced with a stricter one.**
///
/// The two differ in exactly one place, and it is the place
/// `QuestLogTitleButton_OnClick` lands in. 5.1 opens with
///
/// ```c
/// if (!(1 <= pos && pos <= e))  /* position is outside bounds? */
///   return 0;                   /* nothing to remove */
/// ```
///
/// and 5.0 has no such guard: it shortens the table by one and nils `t[n]`
/// whatever `pos` was, returning `t[pos]` — which for an out-of-range `pos` is
/// `nil` and drops the *last* entry.
///
/// That is not a curiosity. Untracking a quest runs
///
/// ```lua
/// tremove(QUEST_WATCH_LIST, questIndex);   -- a quest log *row*, as a table index
/// RemoveQuestWatch(questIndex);
/// ```
///
/// and a row is almost never a valid index into a list of at most five watches.
/// Under 5.0 the list still shrinks — by the wrong entry, but it shrinks — so
/// `getn(QUEST_WATCH_LIST)` tracks the number of watches and
/// `AutoQuestWatch_Insert`'s `getn(QUEST_WATCH_LIST) < MAX_WATCHABLE_QUESTS`
/// keeps letting quests in. Under 5.1 it shrinks *never*: the list grows by one
/// on every track and never falls, so after five the guard fails and every
/// later shift-click does **nothing at all, silently** — no error, no message,
/// the quest simply refuses to be tracked. That is the report *"you cannot
/// re-enable tracking in certain cases"*, and "certain" is the whole of it: it
/// only bites once the row happens to be past the list's length, which is
/// almost always but not quite.
///
/// Written in Lua rather than Rust because it is four lines of Lua and the
/// original is four lines of C over the same primitives. It replaces
/// `table.remove` itself, so [`FLATTENED`]'s `tremove` alias is this one — the
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
    // **`date` and `time`, the flat `os` pair.** 1.12's state has them as
    // globals and no `os` table, and an addon reads the clock through them
    // (pfUI's first line after load is `date("%d")`). `date(format, t)`
    // answers the `strftime` subset below, or a table for `"*t"`; the zone is
    // UTC, since the interpreter carries no zone table, and a leading `!`
    // (5.1's explicit UTC) reads the same. `time()` is seconds since the
    // epoch; `time(table)` is not composed and answers now.
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

/// A Lua error's first line, which is the useful one — the rest is a traceback
/// through a chunk that is three lines long.
pub(in crate::lua) fn first_line(e: &mlua::Error) -> String {
    e.to_string().lines().next().unwrap_or_default().to_string()
}

/// `--script`: one Lua chunk to run once the interface has settled, and whether
/// it has been run.
///
/// **An interface path that cannot be driven from a script cannot be checked
/// without a person at the keyboard**, which is the same argument `--tune` makes
/// for the render settings one directory over. It is how the casting regression
/// this file's neighbours document was confirmed fixed:
///
/// ```powershell
/// cargo run -p vale-client -- Alden --script "ActionButtonDown(1); ActionButtonUp(1)"
/// ```
#[derive(Resource, Default)]
pub struct StartupScript {
    body: Option<String>,
    /// When to run it, set the moment the interface finishes loading.
    at: Option<f32>,
    /// …and chunks handed over **while the client is running**, drained on the
    /// next frame in the order they were queued.
    ///
    /// The debug panel's Lua console is the only writer. It is a queue rather
    /// than a direct call for one reason: running a chunk needs the host (a
    /// non-send resource), the whole of [`api::LuaWorld`], and a writer for the
    /// bindings a chunk can press — which is [`run_scripts`]'s parameter list
    /// exactly, and adding a second copy of it to the egui pass would both blow
    /// Bevy's sixteen and give the interface two places that can call into the
    /// interpreter in one frame.
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
/// A resource rather than a `Local` on the panel because the answer is produced
/// by [`run_scripts`] a frame later — the console types into a queue and reads
/// the log, and the two are different systems on purpose. See
/// [`StartupScript::typed`].
#[derive(Resource, Default)]
pub struct ScriptLog {
    lines: Vec<(String, Result<String, String>)>,
}

/// How many console exchanges to keep. Enough to scroll back through a session
/// of poking at a panel, small enough that a script in a loop cannot grow the
/// heap.
const SCRIPT_LOG_LIMIT: usize = 100;

impl ScriptLog {
    /// Record one chunk and what it produced — `Ok` with a note, or the Lua
    /// error's own first line.
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
/// The interface is up within a frame of login and the *world* is not: the
/// spellbook, the action bar and the first entities arrive over the next few
/// seconds, and a script that casts before `SMSG_INITIAL_SPELLS` is a script
/// testing an empty bar. Five seconds is comfortably past all three and well
/// inside `--shot`'s 25-second default, so the picture shows the result.
const SCRIPT_DELAY_SECS: f32 = 5.0;

pub struct HostPlugin;

impl Plugin for HostPlugin {
    fn build(&self, app: &mut App) {
        // **Registered whether or not the interpreter starts**, because
        // `run_scripts` is not the only reader: the debug panel's console holds
        // it too, and a resource that exists only on the happy path is a panic
        // on the unhappy one. `StartupScript` itself comes from `lib.rs`, which
        // fills it from `--script`.
        app.init_resource::<ScriptLog>();
        // …and the two flags the loaders' run conditions read. Beside
        // `ScriptLog` and registered on the same terms: whether or not the
        // interpreter starts, because a condition on a missing resource is a
        // panic on the unhappy path. See [`LoadWanted`].
        app.init_resource::<LoadWanted>();
        // …and whether the interface is running at all, on the same terms and
        // for the same reason. See [`InterfaceAwake`].
        app.init_resource::<InterfaceAwake>();
        match LuaHost::new() {
            Ok(host) => {
                app.insert_non_send(host);
                // **Before `GameSet` reads anything.** `run_scripts` writes a
                // `BindingPressed`, and a verb typed at the chat line has to
                // reach the same systems in the same frame a key's does — a
                // message that lands after its reader is a `/script` that takes
                // a frame longer than the key it is standing in for, which is
                // exactly the kind of difference a debugging tool must not have.
                app.add_systems(
                    Update,
                    (
                        // **Both loaders are behind a condition, and it is a
                        // measurement rather than tidiness.** Each does its one
                        // job once and returns on a flag for the rest of the
                        // session — but a system that returns on its first line
                        // has still had *every parameter fetched*, and
                        // [`api::LuaWorld`] is a `SystemParam` of twenty-nine
                        // fields. Measured with Tracy at the floor framing:
                        // `load_bindings` 0.196 ms and `load_glue` 0.102 ms of
                        // self time **per frame**, for two functions whose
                        // bodies had not run since login. A run condition that
                        // reads two cheap resources skips the fetch entirely.
                        // See [`interface_wanted`] and [`glue_wanted`].
                        load_glue.run_if(glue_wanted),
                        load_bindings.run_if(interface_wanted),
                        run_scripts,
                        pace,
                    )
                        .chain()
                        .before(crate::interface::GameSet),
                );
                // …and this file's own HUD line, after the chain rather than
                // inside it: `report` is an instrument and the three above it
                // are the interface's own clock. See `ui::debug`.
                #[cfg(feature = "diagnostics")]
                app.add_systems(
                    Update,
                    report
                        .after(run_scripts)
                        .before(crate::interface::GameSet)
                        .run_if(crate::ui::report::watched),
                );
                // **After the news has been delivered**, which is the one
                // ordering this system has and the whole of why it is not in
                // the chain above: the interface is told `PLAYER_LEAVING_WORLD`
                // and *then* thrown away. See [`unload_interface`].
                app.add_systems(Update, unload_interface.after(super::api::events::dispatch));
                // …and the same teardown without the leaving, which is what
                // `ReloadUI()` is. After the dispatch for the same reason, and
                // after `unload_interface` so that a frame carrying both — a
                // logout that an addon answered with a reload — tears down once
                // and comes back up as the login screen rather than as a
                // FrameXML the session no longer has.
                app.add_systems(Update, reload_interface.after(unload_interface));
                // …and the third thing that is the same teardown: a host that
                // has stopped showing the interface at all. **After both**, so
                // a frame carrying a logout *and* a sleep tears down once — see
                // [`sleep_the_interface`].
                app.add_systems(Update, sleep_the_interface.after(reload_interface));
                // …and the one panel that has to be poked by hand rather than
                // told — see [`refresh_loot`].
                app.add_systems(Update, refresh_loot.after(super::api::events::dispatch));
            }
            // There is no recovering from this and no reason for it to happen —
            // the interpreter is vendored and opening it allocates. Say so
            // loudly and carry on with no host, which is a client whose keys do
            // nothing rather than one that will not start.
            Err(e) => error!("the Lua host would not start ({e}) — no key will do anything"),
        }
    }
}

/// **Whether the game's interface is running at all.**
///
/// True in the client, where the interface is most of what a player looks at.
/// A host that is showing the world without it — and there is no such host in
/// this crate — sets it false, and then **neither directory is loaded**: there
/// is no tree to walk, no `OnUpdate` to tick, no edit box to type into and no
/// button under the pointer.
///
/// It is not [`crate::render::tuning::WorldTuning::interface`], and the
/// difference is the whole reason it exists. That switch subtracts the walk and
/// the paint from an interface that is still *there*: the frames still exist,
/// the events still fire, and — the part that is easy to miss until somebody
/// reports it — the keyboard still reaches whichever edit box has the focus and
/// the pointer still finds whichever button is under it. An interface that is
/// merely invisible is an interface you can still type your movement keys into
/// and still press Enter on.
///
/// **Turning it off tears down whatever is loaded**, on the next frame, by the
/// same rebuild a logout does — see [`sleep_the_interface`]. Turning it back on
/// loads the directory again, which costs what the first load cost.
#[derive(Resource, Debug, Clone, Copy)]
pub struct InterfaceAwake(pub bool);

impl Default for InterfaceAwake {
    fn default() -> InterfaceAwake {
        InterfaceAwake(true)
    }
}

/// **Whether either directory still has to be loaded**, as a plain `Resource`
/// the run conditions below can read.
///
/// ## Why this exists rather than asking the host
///
/// The obvious condition is the loaders' own first line — `host.directory()`,
/// `host.loaded()` — and it **panics**:
///
/// ```text
/// Attempted to access or drop non-send resource LuaHost
/// from thread ThreadId(1) on a thread ThreadId(11)
/// ```
///
/// A `NonSend` parameter forces the *system* holding it onto the main thread.
/// A **run condition is not that system**: the multithreaded executor evaluates
/// conditions on whatever worker is free, so a `NonSend<LuaHost>` in one is read
/// off the main thread and Bevy is right to abort. The client came up, painted
/// the login screen, and died on the next frame. It is not a subtle failure and
/// it is not caught by any test in this repo, because no test runs the
/// multithreaded executor with a real host in it.
///
/// ## …and why a mirror of a fact is safe *here* specifically
///
/// A second copy of a truth is normally a bug waiting to happen. This one has
/// an invariant that makes the drift harmless in the only direction that
/// matters: **both flags start `true` and are only ever cleared by the body
/// that just did the work.** So a write that is forgotten, or a state change
/// nobody thought of, leaves the flag saying "still wanted" — which runs the
/// system and lets it return on its own check, i.e. exactly the behaviour this
/// replaced. The dangerous direction — a flag saying "done" when it is not,
/// which is a login screen that never appears — cannot be reached by omission,
/// only by writing `false` somewhere that has not loaded anything.
///
/// The bodies keep their own checks for the same reason. This is an
/// optimisation over them, never a replacement for them.
#[derive(Resource)]
pub struct LoadWanted {
    /// `Interface\GlueXML\` — the login screen.
    glue: bool,
    /// …and `Interface\FrameXML\` — the interface proper.
    interface: bool,
}

impl Default for LoadWanted {
    /// **Both true**, which is the whole of the invariant above: a fresh client
    /// has loaded neither, and anything that resets this resource resets it to
    /// "do the work" rather than to "the work is done".
    fn default() -> Self {
        LoadWanted { glue: true, interface: true }
    }
}

impl LoadWanted {
    /// Whether the interface on screen is the world's (`Interface\FrameXML\`)
    /// rather than the login screens'. False from launch and again after a
    /// logout resets this resource; true from the moment [`load_bindings`]
    /// has done its work. [`crate::ui::scale`] reads it because the `uiScale`
    /// pair is a game-UI setting and the glue draws at scale 1.0 outright —
    /// and it reads this mirror rather than the host for the reason the
    /// resource exists at all: the host is `NonSend`.
    pub fn world_interface_loaded(&self) -> bool {
        !self.interface
    }
}

/// Is there a login screen to put up?
///
/// The cheap half of [`load_glue`]'s own first two lines, so that the expensive
/// half — [`api::LuaWorld`], a `SystemParam` of twenty-nine fields — is not
/// fetched on the frames the answer is no, which is every frame of every
/// session after the first second. See the registration above for the
/// measurement and [`LoadWanted`] for why it reads a mirror.
fn glue_wanted(
    wanted: Res<LoadWanted>,
    awake: Res<InterfaceAwake>,
    session: Res<crate::world::session::Session>,
) -> bool {
    awake.0 && wanted.glue && session.active.is_none()
}

/// …and is there an interface to put up? The same argument, for
/// [`load_bindings`].
///
/// The player check is deliberately **not** here. It is a query into
/// [`api::LuaWorld`], which is the thing this exists to avoid fetching, and it
/// is only false for the one or two frames between the login burst landing and
/// the player entity being spawned. Leaving it in the body costs those two
/// frames and keeps the condition to two resource reads.
fn interface_wanted(
    wanted: Res<LoadWanted>,
    awake: Res<InterfaceAwake>,
    session: Res<crate::world::session::Session>,
) -> bool {
    awake.0 && wanted.interface && session.active.is_some()
}

/// **Load `Interface\GlueXML\` — the login screen** — as soon as there is a
/// window and nothing else is loaded.
///
/// Unlike its neighbour this is *not* deferred, and the reason is the whole
/// point of the round: the login screen is not a form drawn over a black
/// rectangle any more, it is 39 of the game's own files and a full-screen M2
/// scene, so "paint the window first and open the archives later" would mean
/// painting an empty window. The chain is ~19 archives and about a second, once,
/// at the moment nothing else is happening.
///
/// **Reloaded after a logout as well as loaded at startup**, because
/// [`unload_interface`] throws the whole state away — so the condition is "no
/// directory is loaded" rather than a one-shot latch, and coming back from the
/// world puts the login screen up again the way the real client does.
///
/// The session is not consulted at all: this runs at the login screen, at
/// character select, and during the moment between `CMSG_PLAYER_LOGIN` and the
/// world arriving. [`load_bindings`] takes over the moment there is a session,
/// and it is chained after this so a login that lands on the same frame gets the
/// interface rather than the glue.
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
    // **Cleared here and nowhere else** — by the body that just did the work,
    // which is the whole of [`LoadWanted`]'s invariant.
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

/// **Tell the state how big the screen is before a directory is loaded into
/// it.**
///
/// One helper for both loaders, because the failure it prevents is the same one
/// at either end and it is silent: a body that reads `GetScreenWidth()` at load
/// keeps whatever it read, and there is no second event to correct it.
///
/// **It takes the window's *shape*, and the scale the interface is drawn at.**
/// The space is [`super::widgets::layout::ui_height`] units tall and as many
/// across as the window's ratio asks for — see
/// [`super::widgets::layout::units_wide`] — so a latch read at load is true for
/// the whole life of the load however the window is *dragged*, because the
/// window is held to 16:9 in the one mode it can be dragged in. What it is not
/// true across is a fullscreen toggle or a move of the UI Scale slider, and the
/// two frames that care are re-sized by `layout::set_screen` itself when either
/// moves.
///
/// Both of those replace a `reload_on_resize` that dropped the entire Lua state
/// and rebuilt it whenever the window settled at a new size, which is what the
/// reference's own resolution change does and cost what one costs: ~1.1 s of no
/// interface, every open panel closed and the chat frame's lines gone with the
/// state that held them, once per drag. Two latches drove it —
/// `GlueParent_OnLoad`'s absolute pillarbox width and `WorldMapFrame_OnLoad`'s
/// `BlackoutWorld` — and neither can go stale against a ratio a drag cannot
/// change.
fn tell_the_screen(host: &LuaHost, windows: &Windows, scale: f64) {
    let (width, height) = window_size(windows);
    host.set_screen(
        super::widgets::layout::units_wide(width, height, scale),
        super::widgets::layout::ui_height(scale),
    );
}

/// The primary window, for the two loaders — named because both take it and
/// neither wants anything off it but its size.
type Windows<'w, 's> =
    Query<'w, 's, &'static bevy::window::Window, With<bevy::window::PrimaryWindow>>;

/// …and that size in *logical* pixels, or the 16:9 space's own when there is no
/// window at all — which is the headless `--audit`, where a shape derived from
/// nothing would be worse than the stated default.
fn window_size(windows: &Windows) -> (f64, f64) {
    match windows.single() {
        Ok(window) => (f64::from(window.width()), f64::from(window.height())),
        Err(_) => super::widgets::layout::UI_SIZE,
    }
}

/// Hand the host the game's own binding table and **load the interface**, once
/// the archives are open.
///
/// Deferred until there is a session for the same reason `load_strings` is: the
/// chain is ~19 archives and the login screen should paint first. The two happen
/// together because they want the same moment and the same archive — and because
/// the interface load is the expensive one (175 files, 2.4 MB), so doing it
/// anywhere but once, off the critical path, would be felt.
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
    // **…and the player has to exist, not merely the session.** An `OnLoad`
    // reads the world, and a world that has not answered yet answers *zero* —
    // which is not always temporary, because some of the interface latches on
    // what it read. `MainMenuBar.xml`'s is the standing example and it cost the
    // whole XP bar:
    //
    // ```lua
    // <OnLoad>  MainMenuExpBar_Update()  </OnLoad>   -- SetMinMaxValues(0, 0)
    // <OnValueChanged>
    //     if (not this:IsShown()) then return; end   -- …and it is not, by then
    //     TextStatusBar_OnValueChanged();            -- -> :Hide() on max == 0
    // ```
    //
    // `TextStatusBar_UpdateTextString` hides a bar whose maximum is zero, and
    // the bar's own `OnValueChanged` declines to run once it is hidden — so the
    // XP bar hid itself at the load and **nothing in the directory could ever
    // bring it back**: `TextStatusBar_OnEvent` answers `CVAR_UPDATE` and
    // nothing else. The reference never meets this because it builds its
    // interface with the player already in hand.
    //
    // `session.active` is set the moment the login burst is in, which can be
    // the same frame — before `world::session`'s sync has spawned the
    // `WorldEntity` every read here goes through. One frame of loading screen
    // is the whole cost.
    if world.units.get(crate::interface::api::UnitId::Player).is_none() {
        return;
    }
    // **The glue comes down before the interface goes up**, and it is a fresh
    // state rather than a second `.toc` loaded on top. The two directories both
    // declare `MasterFont`, `GameFontNormal` and `GameFontHighlight`, both
    // define a `$parent` root, and `GlueStrings.lua` and `GlobalStrings.lua`
    // share hundreds of keys with different values — so a state holding both is
    // a state where whichever loaded second silently took the other's typefaces
    // and captions. It is the same teardown [`unload_interface`] does at the
    // other end of a session, for the same reason and at the same cost: the
    // moment a loading screen is already up.
    // **…and the addon board has to know which character this is.** Which
    // addons load is the character's `AddOns.txt`, which
    // `crate::settings::addons` reads once the session names a character.
    // That system runs after this chain in the same frame, so the first frame
    // with a player waits here and the next one loads. Without the wait every
    // addon would load under its default state whatever the file says.
    if !host.addons().borrow().ready_for_world() {
        return;
    }
    if host.directory() == Some(Directory::Glue) {
        // The board outlives the state: it was seeded on the glue's host and
        // is what the load one line down reads. `carry_over` forgets what that
        // host had loaded, which is nothing this one has.
        let board = host.addons().borrow_mut().carry_over();
        match LuaHost::new() {
            Ok(fresh) => *host = fresh,
            Err(e) => error!("the Lua host would not restart ({e}) — the interface is gone"),
        }
        *host.addons().borrow_mut() = board;
        // The focus went with the state that held it, exactly as at a logout.
        *focus = super::api::keyboard::KeyboardFocus::default();
        // …and so did the settings, which are the one thing in that state that
        // is *not* the interface's. A fresh store opens on the client's own
        // registrations, so without this the whole of `Interface\FrameXML\`
        // would load one line below reading the shipped volume rather than the
        // player's — and `UIOptionsFrame_Init` latches what it reads. Only the
        // ones that are not still at their default; see [`CVars::changed`].
        host.seed_cvars(&cvars.changed());
    }
    // **Before the load and not after it**, because the interface latches on
    // what it reads — `WorldMapFrame_OnLoad` sizes `BlackoutWorld` to the screen
    // and never looks again. The fresh state above is why this is not covered by
    // the painter's own per-frame call: the swap throws away everything the glue
    // had been told. See [`LuaHost::set_screen`].
    tell_the_screen(&host, &windows, ui_scale.get());
    // `--script` is armed here rather than at startup, because "the interface
    // has loaded" is the only moment it can be measured from — see
    // [`SCRIPT_DELAY_SECS`].
    startup.at = Some(time.elapsed_secs() + SCRIPT_DELAY_SECS);
    let mut read = |path: &str| {
        assets
            .with_archive(|archive| Ok(archive.read(path).ok()))
            .ok()
            .flatten()
    };
    // **The game's declarations, then each loading addon's `Bindings.xml`
    // after them.** An addon's `.toc` never names that file; the reference
    // reads it out of the directory of every addon it loads, which is how
    // pfUI's `PFPAGING1` reaches the key-bindings panel and the keyboard.
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
    // **…and a reader `LoadAddOn` can use from inside a Lua call**, which is
    // the one load that does not go through this system.
    host.addons().borrow_mut().set_reader(assets.reader());

    // **The interface, inside a scope that can answer it.** An `OnLoad` reaches
    // for the reads, so the load goes through `run` like everything else.
    // **`attacking: false` no longer stated here.** The load runs before any
    // swing can have started, so the bundle's own answer is the same one.
    let live = world.live();
    host.load_interface(&live, &mut read);
    // …and the same, by the body that did it. The glue is gone with the state
    // that held it, so its flag is cleared too — see the swap above.
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

    // **And now tell it the world is here, because it missed being told.**
    //
    // This client logs in and *then* loads the interface — 175 files, about a
    // second — where 1.12 loads the interface at startup and enters the world
    // afterwards. Every event `GameSet` raised in that second was written with
    // nothing registered to hear it, and a `bevy` message with no reader is a
    // message that expires: `PLAYER_ENTERING_WORLD`, the whole action bar's
    // `ACTIONBAR_SLOT_CHANGED`, the first `UNIT_*` of every frame.
    //
    // The cost was not subtle and it is what this round's "spells are not on
    // the bar" turned out to be. `ActionButton_OnLoad` calls
    // `ActionButton_Update` itself, so a button reads its slot once at load —
    // and *that* read happens before `interface::action` has resolved the server's
    // 120 buttons against `Spell.dbc`, so every one of them answered "empty",
    // hid its icon and unregistered the six events that would have corrected
    // it. Measured: re-running `ActionButton_Update` by hand from `--script`
    // five seconds later filled the whole bar.
    //
    // **`PLAYER_ENTERING_WORLD` is the game's own answer to this**, not a hook
    // invented here: it is the event the directory initialises on, and 34
    // frames register it — the action buttons, `PlayerFrame`, the chat windows,
    // the shapeshift bar. Raised here it means exactly what it says, one second
    // later than the real client would have said it.
    //
    // **And `VARIABLES_LOADED` before it, because that is the order.** The real
    // client reads `SavedVariables` at startup and enters the world afterwards;
    // this one has no `WTF`, so the option globals are final the moment the last
    // `OnLoad` has run — which is here. See
    // [`crate::interface::events::VariablesLoaded`] for the two widgets that were
    // waiting on it.
    variables.write(crate::interface::events::VariablesLoaded);
    entering.write(crate::interface::events::PlayerEnteringWorld);
}

/// Run whatever `/script` lines the chat pane queued, and put the result where
/// the person who typed them is looking.
///
/// **The verbs a script calls are announced exactly as a key's are.** `/script
/// ActionButton1:Click()` has to press the button, not merely look like it — a
/// second path that ran the Lua and dropped the queue would be a debugging tool
/// that lies about the thing being debugged.
/// **The typed `/script` does not come through here** and used to. It is
/// `SlashCmdList["SCRIPT"]` in `ChatFrame.lua`, whose body is `RunScript(msg)` —
/// a C function this client now registers, running the chunk where it stands the
/// way the real one does. So the command line is the only producer left, and the
/// resource that used to queue both is gone with the egui pane it lived on.
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
    // **The typed queue first, and it is not gated on the settle delay.**
    // `--script`'s five seconds exist because a chunk fired at login is a
    // chunk testing an empty action bar; a person typing into the console has
    // already waited, and making them wait again would read as the console
    // being broken.
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
                // **The chat line stays the channel for `--script`** — it is
                // where a scripted run's failure has always been visible and
                // where a `/script` typo belongs. A console error is echoed in
                // the console *as well*, because a person who just typed it is
                // looking at the console and not at the chat frame.
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

/// **Re-read the loot rows, because `LootFrame` has no event that would.**
///
/// The single departure in this client from "the world raises a name and the
/// interface decides what to do with it", and it is forced rather than chosen:
/// `LootFrame_OnEvent`'s only redraw is `ShowUIPanel`, which `UIParent.lua`
/// returns from for a frame that is already visible, and `LootFrame_Update` is
/// reachable from nothing but `OnShow` and the two page buttons. See
/// [`crate::interface::loot`], which carries the reading and the reference
/// addresses.
///
/// It is **almost never reached**. The window is held shut until its rows can be
/// named, so this covers only the case where that hold timed out and the
/// template turned up afterwards. The guard is the frame's own visibility, so
/// while the window is still held back this is a chunk that does nothing.
///
/// The chunk is the frame's own function called where it stands, which is what
/// the reference's completion callback ends in too — not a second drawing path
/// invented here.
fn refresh_loot(
    host: Option<NonSendMut<LuaHost>>,
    mut landed: MessageReader<crate::interface::loot::LootNamesLanded>,
    world: api::LuaWorld,
) {
    let Some(mut host) = host else { return };
    // Collapsed to one call however many templates arrived in the frame: the
    // update reads every row anyway.
    if landed.read().next().is_none() {
        return;
    }
    landed.clear();
    let live = world.live();
    // The verb queue `script` answers with is empty by construction — a redraw
    // presses nothing — which is why this drops it where [`run_scripts`] must
    // not.
    if let Err(e) = host.script(
        "if ( LootFrame and LootFrame:IsVisible() ) then LootFrame_Update(); end",
        &live,
    ) {
        warn!("lua: the loot rows could not be re-read ({e})");
    }
}

/// **Throw the whole interface away when the session ends**, and let the next
/// login build a fresh one.
///
/// The real client does exactly this — the interface is loaded on entering a
/// world and unloaded on leaving it, which is why an addon's `OnLoad` runs again
/// after a logout — and this client did not, for no better reason than that
/// `load_bindings` gates on "have I loaded" and nothing ever un-said it. What it
/// cost, all of it invisible until you looked twice:
///
/// * **the last character's interface on the next one's screen.** The frames,
///   their `RegisterEvent` lists, `DEFAULT_CHAT_FRAME`'s lines, every global a
///   `.lua` file set and every field a handler wrote — all of it survived into
///   the next session, so a mage's action-bar state greeted a warrior.
/// * **~34 MB of live Lua heap held at the character screen**, with the
///   collector paced against it every frame for a world nobody is in.
///
/// Dropping and rebuilding the `mlua::Lua` is the whole teardown: every widget,
/// handler and global in the interface is reachable only from that state, so
/// there is nothing to walk and nothing to forget. It costs the ~1.1 s load
/// again on the next login, which is the moment a loading screen is already up.
/// `pub(crate)` for one reason: `settings::keybindings` has to write its
/// two files **before** this runs, and that ordering is stated rather than
/// inherited — see there.
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

/// **Throw the interpreter away and start a fresh one**, which is what all three
/// of a logout, a `ReloadUI()` and the interface being put to sleep are.
///
/// One function because the six lines below were written out three times and the
/// third copy is the one that would have drifted: a state carried across one of
/// them and not the others is a setting that survives a reload and not a logout,
/// which is the kind of difference nobody notices until it is a bug report.
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
        // The same failure `HostPlugin` reports, at the same volume and for the
        // same reason: a client whose keys do nothing is worth saying out loud.
        Err(e) => error!("the Lua host would not restart ({e}) — no key will do anything"),
    }
    *host.addons().borrow_mut() = board;
    // The focus went with the state that held it — and so did the settings, for
    // the reason [`load_bindings`] gives beside its own copy of this.
    *focus = super::api::keyboard::KeyboardFocus::default();
    host.seed_cvars(&cvars.changed());
    // **Both, because the state that held them is gone.** A logout puts the
    // login screen back up the way the real client does, and the fresh host has
    // loaded neither directory — so this is the one place that writes `true`,
    // and it writes the same value [`LoadWanted::default`] does.
    *wanted = LoadWanted::default();
}

/// **Take the interface away when nobody is being shown it** — see
/// [`InterfaceAwake`], which is where the argument is.
///
/// **The state and not the falling edge**, and the first draft had it the other
/// way round. `load_glue` and this both run in `Update` with nothing ordering
/// them, so on the first frame of a host that starts asleep the glue loaded
/// *before* the switch was read — and an edge detector whose first sample is
/// already `false` has no edge to find, so the login screen stayed loaded and
/// live for the rest of the session. That is exactly the bug this resource
/// exists to fix, reintroduced one layer down.
///
/// It costs nothing to ask every frame because `loaded()` is the second half of
/// the condition and the rebuild clears it: a fresh interpreter has loaded
/// neither directory, so this fires once per sleep however long the sleep is.
/// The rising edge needs nothing at all — the two loaders' conditions are
/// already asking.
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
    // **`directory()` and not `loaded()`.** The second is `bindings.is_some()`,
    // which is `Interface\FrameXML\` alone — so a host sitting on the login
    // screen answers `false` to it, and the first draft of this left
    // `Interface\GlueXML\` loaded and taking keys in exactly the case the
    // switch is most often used in: a host that has stopped before it ever
    // reached a world.
    if host.directory().is_none() {
        return;
    }
    info!("lua: the interface is not being shown — unloading it");
    restart(&mut host, &mut focus, &cvars, &mut wanted);
}

/// **`ReloadUI()` — the interface again, from nothing, with the world left
/// alone.**
///
/// The same rebuild [`unload_interface`] does, minus the leaving: a fresh
/// `LuaHost`, the addon board carried across it, the focus and the settings put
/// back, and both `LoadWanted` flags set so that [`load_bindings`] loads
/// `Interface\FrameXML\` and every enabled addon into the new state on the next
/// frame. `glue_wanted` refuses while there is a session, so setting both is
/// safe here and is one fewer thing to keep in step than setting one.
///
/// **What has to happen before this and cannot happen inside it.** An addon
/// reloads to make a setting take effect, so the setting has to survive the
/// state that holds it: `pfUI:LoadConfig(); ReloadUI()` writes `pfUI_config`
/// into a Lua global and nothing else. `crate::settings::savedvars::save_on_reload`
/// and `crate::settings::keybindings::save_on_reload` write both files
/// first and are ordered `.before` this for that reason — and `savedvars::apply`
/// reads them straight back, because a fresh host answers
/// `needs_saved_variables`. Without that half the reload is a way of *undoing*
/// whatever asked for it, which for pfUI's first-run wizard is a wizard that
/// can never be finished.
///
/// **Why it is a system and not the C function.** The function runs inside
/// `LuaHost::run`'s scope, on the state this replaces — see
/// [`super::api::verbs`]. The reference does not return from `ReloadUI` at all;
/// this one does, so the rest of the calling body still runs against a state
/// that is about to be discarded. That is harmless and it is the only
/// difference: nothing the body does afterwards outlives the frame.
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
    // The board outlives the state, exactly as at a logout: which addons the
    // folder has and which this character has on are not the interpreter's, and
    // `carry_over` forgets only what had loaded.
    restart(&mut host, &mut focus, &cvars, &mut wanted);
}

/// One bounded slice of the interpreter's collector, every frame — see
/// [`LuaHost::pace_collector`], where the measurements are. A system of its own
/// so it runs whether or not anything else touched Lua this frame: the pacing
/// parks the automatic collector, and parked-with-no-pacing would be an
/// unbounded heap.
fn pace(host: Option<NonSend<LuaHost>>) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::LuaCollector);
    if let Some(host) = host {
        host.pace_collector();
    }
}

/// Up to four HUD lines: what the interface has asked for and not got, what it
/// is waiting to be told and never will be, and what it is doing right now.
///
/// Each absent while there is nothing to say, which is the ordinary case — a
/// line reading "0 missing" is a line that trains the eye to skip it.
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

    // **What the interface actually is**, once it is loaded — the line that says
    // the widget tree exists at all.
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

    // **What the interface is doing rather than what it is made of**: how many
    // frames are being ticked, and what the pointer is on. The second is the
    // line to read when a click does nothing — a name here means the interface
    // took it, an empty one means the world did.
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

    /// **The scope can be opened as often as a session opens it**, which is the
    /// one property `run` has that no other test was asking about.
    ///
    /// Every entry into Lua — `OnUpdate`, the mouse, the keyboard, a binding's
    /// body, an event batch — is one `lua.scope`, so this client opens four or
    /// five a frame and a leak of a single Lua stack slot in any of them is a
    /// state that dies in about a minute of play. It died in exactly that way:
    /// `mlua`'s `Table::clear` pushes the table and never pops it, `scoped::clear`
    /// cleared two stores, and Lua 5.1 will not grow the stack past
    /// `LUAI_MAXCSTACK` — so the 3,998th `run` returned `StackError` and **every**
    /// entry point stopped at once. See [`crate::lua::scoped::clear`], which does
    /// the emptying in Lua now and says why.
    ///
    /// What made it expensive to find is that it looks like nothing this file
    /// owns: the interface freezes in whatever state it was last painted in, no
    /// key does anything, and the camera keeps working because it never touches
    /// Lua. The error is on the HUD (`lua: 3 unimplemented — OnUpdate: out of Lua
    /// stack, …`) and nowhere else.
    ///
    /// **12,000, and the number is the whole of the test's value.** Lua 5.1
    /// refuses the stack past `LUAI_MAXCSTACK` = 8,000, so a leak of *one* slot
    /// a call fails on the 8,000th and a count below that would miss it — the
    /// bug this pins leaked two and so died at 3,998. Anything past 8,000
    /// catches the general case; 12,000 is three seconds and leaves room.
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

    /// **The loading system runs, with every parameter it now takes.**
    ///
    /// It grew four when it learned to load the interface — the whole world, so
    /// that an `OnLoad` can ask questions — and Bevy validates a system's
    /// parameters at *init* rather than at compile time, so a conflicting or
    /// missing one is a panic on the first frame after login. That costs a run of
    /// the client to find; this costs a millisecond. The same trap
    /// `input::bindings::the_dispatch_can_be_scheduled_with_the_world_it_now_borrows`
    /// exists for, one directory over, and this is the second system to fall into
    /// its blast radius.
    ///
    /// There is no session, so nothing loads. That is the point: what is checked
    /// is that the system can be built and run at all.
    /// **A chunk typed into the console runs and the answer comes back**, which
    /// is the whole of the console's plumbing: the panel can only queue, and
    /// `run_scripts` is what turns a queued line into a `ScriptLog` entry a
    /// frame later.
    ///
    /// It also pins the two halves apart: a good chunk logs `Ok` and a bad one
    /// logs the error's own first line rather than raising. A console that ate
    /// its own errors would be worse than no console.
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

        // Nothing queued: the log stays empty and the system does no work at
        // all, which is what it does on every frame of a real session.
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

        // …and the queue is drained rather than replayed, which would run every
        // chunk of the session again on every frame.
        app.update();
        assert_eq!(app.world().resource::<ScriptLog>().lines().len(), 2);
    }

    #[test]
    fn the_loader_can_be_scheduled_with_the_world_it_now_borrows() {
        let mut app = App::new();
        api::LuaWorld::init(&mut app);
        app.insert_non_send(LuaHost::new().expect("the interpreter starts"))
            .insert_resource(crate::assets::GameAssets::new(String::new()))
            .init_resource::<HudReport>()
            .init_resource::<StartupScript>()
            .init_resource::<ScriptLog>()
            // The loader hands the keyboard back when it swaps the glue for the
            // interface, exactly as the logout teardown does — and re-seeds the
            // settings into the fresh state for the same reason.
            .init_resource::<crate::lua::api::keyboard::KeyboardFocus>()
            .init_resource::<crate::settings::cvars::CVars>()
            // …and it tells the fresh state how big the screen is, which is the
            // space at whatever `uiScale` is in force — see [`crate::ui::scale`].
            .init_resource::<crate::ui::scale::InterfaceScale>()
            .add_message::<crate::input::bindings::BindingPressed>()
            // `run_scripts` reports a failed chunk as a system chat line now —
            // see `interface::chat::system_note`.
            .add_message::<crate::interface::events::ChatMessageReceived>()
            // …and the loader announces `PLAYER_ENTERING_WORLD` when it
            // finishes, because the interface missed the first one.
            .add_message::<crate::interface::events::VariablesLoaded>()
            .add_message::<crate::interface::events::PlayerEnteringWorld>()
            .init_resource::<LoadWanted>()
            .add_systems(Update, (load_bindings, run_scripts, report).chain());

        app.update();
    }

    /// **Leaving the world throws the interface away**, so the next login builds
    /// a fresh one rather than inheriting the last character's.
    ///
    /// The check is a global set from Lua: everything the interface *is* lives in
    /// that state, so if a global survives, so did 3,746 frames, their event
    /// registrations and every field a handler wrote on them.

    /// `date` is 5.0's flat `os.date`, in UTC: a known epoch second reads
    /// back as the day it is, in every code an addon uses.
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
        // Nothing has left the world yet, so nothing is thrown away — the check
        // that this is an *edge* and not "reload whenever there is no session".
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

    /// **`/script` runs the game's own command and its verbs are not dropped.**
    ///
    /// `/script ActionButton1:Click()` has to press the button rather than
    /// merely look like it, which is why the queue is drained and written on as
    /// `BindingPressed` — a debugging path that ran the Lua and swallowed the
    /// result would lie about the thing being debugged.
    #[test]
    fn a_script_runs_and_its_verbs_come_back() {
        let mut host = host("<Bindings/>");
        let world = Stub::default();
        assert_eq!(
            host.script("ToggleSheath();", &world),
            Ok(vec![Binding::ToggleSheath])
        );

        // A body that raises answers with the message rather than recording it
        // as a gap in the API — see [`LuaHost::script`].
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

    /// **A `<Model>` frame in the *world* is not a glue scene**, and the one
    /// that proves it is the cooldown swirl.
    ///
    /// `CooldownFrameTemplate` is a `<Model>` holding
    /// `Interface\Cooldown\UI-Cooldown-Indicator.mdx`, one per action button,
    /// shown by `CooldownFrame_SetTimer` the instant anything goes on cooldown.
    /// `render::glue` builds a **full-screen** scene out of whatever
    /// [`LuaHost::glue_scene`] answers and takes the world's sky, stars, moons
    /// and fog down while it is up — so without the directory gate the first
    /// spell cast replaced the world with a clock face.
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

        // Nothing loaded at all — the state a `--script` run is in.
        assert!(host.directory().is_none());
        assert_eq!(host.glue_scene(), None);

        // …and with `Interface\FrameXML\` up, which is a session in the world:
        // the frame is visible and holding a file, and it is still not a scene.
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

        // At a glue screen the same frame *is* the scene, which is what the
        // login screen and character select are made of.
        host.interface = Some((Directory::Glue, xml::Report::default()));
        assert!(host.glue_scene().is_some());
    }

    /// **1.12's Lua has `format`, not `string.format`** — and every one of the
    /// names the directory actually calls has to arrive, because a missing one
    /// is not a missing function but a dead `OnLoad`. See [`FLATTENED`].
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
        // …and that they are the *right* functions rather than merely present.
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

    /// **`tremove` past the end still shortens the list**, which is 5.0's
    /// behaviour and not 5.1's — see [`restore_5_0_table_remove`], where the
    /// two bodies are side by side.
    ///
    /// The shape here is the one `QuestLogTitleButton_OnClick` runs: a quest
    /// log *row* handed to `tremove` as a table index. Under 5.1's guard the
    /// call returns without touching anything, `QUEST_WATCH_LIST` grows on
    /// every track and never falls, and the fifth one wedges quest tracking for
    /// the rest of the session with no message of any kind.
    ///
    /// Both spellings, because the directory uses both and only one of them is
    /// what the flatten aliases.
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

    /// **A flattened name must not shadow anything the interface defines**, which
    /// is the collision rule one layer down: `sort` and `format` are the client's
    /// to provide and `TEXT` is not. Nothing in [`FLATTENED`] may be a name
    /// `Interface\FrameXML\` writes a `function` for.
    ///
    /// Checked here as a *list* property rather than against the directory,
    /// which needs the archives — `vale framexml`'s collision count is the
    /// half that reads the files, and it must stay zero with these added.
    /// **The base library is open**, which is not obvious from
    /// `StdLib::STRING | TABLE | MATH` — `mlua` loads the base functions
    /// whatever is asked for, and 1.12 has them too. Named here because
    /// `vale framexml` counts them as owed otherwise, and a work list with
    /// `tonumber` near the top is a work list nobody trusts.
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

    /// `ActionButton.lua`'s own two functions and the button they work on —
    /// enough of the interface for a real binding body to run against.
    ///
    /// This is what makes the two tests below say what they claim: a key press
    /// does not reach a Rust closure any more, it reaches the *game's* Lua, and
    /// only what falls out the bottom of that is the client's. See
    /// [`crate::lua::widgets::button`], where the same bodies are checked in full.
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

    /// **The game's own body, run for real**, `keystate` and all — and *through
    /// the interface's own Lua*, which is the correction this round makes.
    ///
    /// The binding body calls `ActionButtonDown`/`ActionButtonUp`; those are
    /// `ActionButton.lua`'s, not this client's; and the C function at the bottom
    /// of the chain is `UseAction`. Registering the middle of that chain as a
    /// verb is what broke casting the day the interface started loading.
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

        // The press pushes the button in and casts nothing…
        assert!(host.fire("ACTIONBUTTON1", true, &world).is_empty());
        // …and the release is what reaches `UseAction`.
        assert_eq!(
            host.fire("ACTIONBUTTON1", false, &world),
            vec![Binding::ActionButton(1)]
        );
        assert!(host.missing().is_empty(), "{:?}", host.missing());
    }

    /// `SELFACTIONBUTTON1`'s real body: **the same path with the game's own
    /// `onSelf` flag**, which rides the release and comes out of
    /// `ActionButtonUp` as `UseAction`'s third argument.
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

    /// **A key that fires before the interface has loaded is refused, loudly.**
    ///
    /// The other side of the same correction: with `ActionButtonDown` no longer
    /// registered, a press with no `ActionButton.lua` in the state has nothing
    /// to call. That is the honest answer — the client does not implement that
    /// name and never should — and it has to be *reported* rather than silent,
    /// because "the interface did not load" and "this key does nothing" look
    /// identical from the keyboard.
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

    /// **`TARGETSELF` works out of its real two-branch body**, which needs a
    /// read — and the read is answered from the live world during the call.
    ///
    /// Both cases matter. Not already on yourself: target the player. Already on
    /// yourself: the body targets the *pet*, which this client does not have, so
    /// the verb resolves to nothing and the target is left alone — rather than
    /// being cleared, which is what a `_ => TargetSelf` catch-all would do.
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

        // **Nothing targeted is not `UnitIsUnit`.** Two absent tokens comparing
        // equal would send this down the pet branch on the commonest state a
        // player is ever in.
        let nothing = Stub::default().unit("player", "Alden", 7);
        assert_eq!(
            host.fire("TARGETSELF", true, &nothing),
            vec![Binding::TargetSelf]
        );
    }

    /// **A verb this client has not written is recorded once, not once per
    /// press** — and the binding is refused rather than half-run.
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
    /// reported — the same channel, because from the player's side it is the
    /// same symptom.
    #[test]
    fn a_name_the_game_does_not_declare_is_refused() {
        let mut host = host(r#"<Binding name="JUMP">Jump();</Binding>"#);
        assert!(host.fire("NOTABINDING", true, &Stub::default()).is_empty());
        assert!(host.missing().contains("NOTABINDING"));
    }

    /// **`io` and `os` are not there.** Addons are third-party code running
    /// in-process and 1.12 does not give them a filesystem; this is the check
    /// that says so, because the difference between `Lua::new()` and the four
    /// libraries the game opens is invisible until something reads a file.
    #[test]
    fn the_dangerous_libraries_are_absent() {
        let mut reaching = host(r#"<Binding name="X">local f = io.open("a", "w");</Binding>"#);
        reaching.fire("X", true, &Stub::default());
        assert_eq!(reaching.missing().len(), 1, "io was reachable");

        // …and the four that are there work, since a body may format a string.
        let mut ok = host(r#"<Binding name="Y">local s = string.format("%d", 1);</Binding>"#);
        ok.fire("Y", true, &Stub::default());
        assert!(ok.missing().is_empty(), "{:?}", ok.missing());
    }

    /// **A frame registers and is called back**, through the host rather than
    /// through `frames::fire` directly — which is the path the world's news
    /// actually takes.
    #[test]
    fn an_event_reaches_the_frame_that_registered_for_it() {
        let mut host = host("<Bindings/>");
        let world = Stub::default().unit("target", "Kobold Vermin", 4);
        // The chunk goes in through the one entry point too, so it sees the reads.
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
        // **The read inside the handler saw the live world**, which is the whole
        // point of the scope: `UnitName` is registered for the length of the
        // dispatch and answers from `world`.
        let name: String = host
            .run(&world, |lua| lua.globals().get("shown"))
            .expect("the global is set");
        assert_eq!(name, "Kobold Vermin");
    }

    /// **A verb called from an event handler is not dropped.** Nothing in the
    /// shipped FrameXML does it, and an addon may — a verb that works from a key
    /// and not from an event would be a silent asymmetry.
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

    /// **The events the interface is waiting on that nothing raises**, which is
    /// the measurement this round adds. `ActionButton_OnLoad`'s own list is where
    /// the number comes from in the real thing.
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
        // `GUILD_ROSTER_UPDATE` because the guild is still unread — this slot
        // has been re-picked twice already, when `MERCHANT_SHOW` and then
        // `TRADE_SHOW` became names this client raises, which is the
        // instrument working as intended.
        assert_eq!(
            unfired,
            ["GUILD_ROSTER_UPDATE"],
            "the other two are names this client raises"
        );
    }

    /// **The failure set is capped.** A broken addon naming a new function every
    /// frame would otherwise grow it without bound, and the thing it would take
    /// down is the HUD line that exists to report it.
    #[test]
    fn the_failure_set_does_not_grow_without_bound() {
        let mut host = LuaHost::new().expect("the interpreter starts");
        for n in 0..MAX_FAILURES * 2 {
            host.note(format!("failure {n}"));
        }
        assert_eq!(host.missing().len(), MAX_FAILURES);
    }

    /// **The loaders' run conditions must not touch the host**, and the reason
    /// is a crash rather than a style rule.
    ///
    /// The first draft of [`glue_wanted`] and [`interface_wanted`] read
    /// `NonSend<LuaHost>` directly, which is the obvious thing to do — it is the
    /// bodies' own first line. It aborts the client on the second frame:
    ///
    /// ```text
    /// Attempted to access or drop non-send resource LuaHost
    /// from thread ThreadId(1) on a thread ThreadId(11)
    /// ```
    ///
    /// A `NonSend` parameter forces the *system* onto the main thread; a run
    /// condition is not that system, and the multithreaded executor evaluates it
    /// on whatever worker is free. The client painted the login screen and died,
    /// and **every test in this file passed**, because none of them runs the
    /// multithreaded executor.
    ///
    /// So this asserts the property that made the crash impossible instead: both
    /// conditions are satisfiable from resources alone. It runs them through a
    /// real schedule with no host in the world at all — which a `NonSend`
    /// version could not have done, since it would have needed one.
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

    /// **An interface nobody is being shown is not loaded at all**, which is the
    /// whole of [`InterfaceAwake`] and the thing that separates it from
    /// `WorldTuning::interface`.
    ///
    /// Asserted on the conditions rather than on the bodies because that is
    /// where it has to hold: a loader whose condition said yes would load the
    /// directory and then the teardown would have to take it away again, which
    /// is a second of work and a frame of a login screen nobody asked for.
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
        // …and waking it puts the first of them back, with nothing else changed.
        app.insert_resource(InterfaceAwake(true));
        assert!(app.world_mut().run_system_cached(glue_wanted).expect("runs"));
    }

    /// **Awake by default**, because the client is the host that shows the
    /// interface and a resource that defaulted the other way would be a client
    /// with no login screen.
    #[test]
    fn the_interface_is_awake_unless_somebody_says_otherwise() {
        assert!(InterfaceAwake::default().0);
    }

    /// **The flags fail in the safe direction**, which is the whole argument for
    /// keeping a mirror of a fact the host already knows.
    ///
    /// [`LoadWanted::default`] is both-true and only a body that has just loaded
    /// something writes `false`. So a forgotten write leaves a loader running
    /// and returning on its own check — the cost this change removes, not a
    /// login screen that never appears. This pins the default, because it is the
    /// half of the invariant a reader would most plausibly "tidy up".
    #[test]
    fn a_fresh_load_state_wants_both_directories() {
        let fresh = LoadWanted::default();
        assert!(fresh.glue, "a client that has loaded no glue must load it");
        assert!(fresh.interface, "…and the same for the interface");
    }

}
