//! The interpreter's lifecycle in the app: [`HostPlugin`] and the systems
//! that load, reload, unload and pace [`LuaHost`], run `--script` chunks, and
//! report the interface's state on the HUD.
//!
//! The interpreter itself, and the one way into it, are in [`super::host`].

use bevy::prelude::*;

use super::api::{self, verbs};
use super::host::{Directory, LuaHost};
#[cfg(feature = "diagnostics")]
use crate::ui::report::{HudReport, Slot};

/// The HUD line this file writes. See [`crate::ui::report`] for the
/// convention. 30 sits under the sky and celestial lines and above residency's
/// 90.
#[cfg(feature = "diagnostics")]
const REPORT: Slot = Slot(30);

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
pub(crate) struct StartupScript {
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
    #[cfg(any(test, feature = "diagnostics"))]
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
pub(crate) struct ScriptLog {
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

    #[cfg(any(test, feature = "diagnostics"))]
    pub fn lines(&self) -> &[(String, Result<String, String>)] {
        &self.lines
    }

    #[cfg(feature = "diagnostics")]
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
pub(crate) struct LoadWanted {
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
    pub(crate) fn world_interface_loaded(&self) -> bool {
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
    if !crate::lua::panels::addons::held(&host).borrow().ready_for_world() {
        return;
    }
    if host.directory() == Some(Directory::Glue) {
        // The board outlives the state: it was seeded on the glue's host and
        // the load below reads it. `carry_over` clears the record of what that
        // host had loaded, since none of it is loaded in the new one.
        let board = crate::lua::panels::addons::held(&host).borrow_mut().carry_over();
        match LuaHost::new() {
            Ok(fresh) => *host = fresh,
            Err(e) => error!("the Lua host would not restart ({e}) — the interface is gone"),
        }
        *crate::lua::panels::addons::held(&host).borrow_mut() = board;
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
    for name in crate::lua::panels::addons::held(&host).borrow().load_order() {
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
    crate::lua::panels::addons::held(&host).borrow_mut().set_reader(assets.reader());

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
    let board = crate::lua::panels::addons::held(&host).borrow_mut().carry_over();
    match LuaHost::new() {
        Ok(fresh) => *host = fresh,
        // The same failure `HostPlugin` reports, logged as an error for the
        // same reason: afterwards no key does anything.
        Err(e) => error!("the Lua host would not restart ({e}) — no key will do anything"),
    }
    *crate::lua::panels::addons::held(&host).borrow_mut() = board;
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
    #[cfg(feature = "diagnostics")]
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
