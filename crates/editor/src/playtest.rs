//! Switching between editing and playing.
//!
//! ## The three playtest states
//!
//! The app contains the whole client, so playing an edit needs no second binary
//! and no mode to load. Without this file, reaching the login screen took five
//! steps: the editor switches the game's interface off at startup, and turning
//! it back on (press `F4`, find the world tab, tick `interface`) drew the game's
//! login over a viewport still showing the editor's own camera and panel. Three
//! of the five steps went through a diagnostics window that is not part of the
//! editor.
//!
//! [`Playtest`] replaces that procedure with one button and three states. Every
//! switch a playtest implies is written from the state rather than by hand:
//!
//! ```text
//! Editing    the free camera, the tools, the panel.
//!            The game's interface is off and its login scene is not built.
//! Entering   the client's own login and character screens, over the game's own
//!            backdrop. The editor keeps neither the keyboard nor the mouse.
//! Playing    a character is in the world. The session has the camera and the
//!            focus, exactly as it does in the client.
//! ```
//!
//! ## Which transitions the session makes and which a person makes
//!
//! A person chooses Editing -> Entering. The moves between `Entering` and
//! `Playing` are read off `vale_client::world::session::Session`: a
//! character arriving is `Playing`, and a character leaving (the game's own
//! Logout, a disconnect, a `/camp`) is back to `Entering`, which is where the
//! client itself goes. The return to `Editing` is chosen by a person again,
//! because a playtest may stay at the character screen.
//!
//! [`follow_the_session`] is therefore a guard rather than a state machine: it
//! never promotes out of `Editing`, so the client cannot start a playtest
//! nobody asked for.
//!
//! ## What a playtest switches
//!
//! [`arrange`] writes four things and they are all derived from the state:
//! `lua::host::InterfaceAwake` (whether the game's interface is loaded and
//! running at all), `WorldTuning::interface` (its walk and its paint),
//! `GlueScenes` (whether the login screen may put its own 3D scene on the
//! camera) and `world::camera::PointerTaken` (whose the world's mouse gestures
//! are). The editor's own camera, pick and brush read the state directly.
//!
//! `InterfaceAwake` and `WorldTuning::interface` are separate switches.
//! `WorldTuning::interface` removes the walk and the paint from an interface
//! that is still loaded. With only that one off, the login screen was invisible
//! but still took input: `W`, `A`, `S` and `D` went into the account name box
//! while the camera flew, and Enter attempted a logon. `InterfaceAwake` off
//! means the interface is not shown to anyone, and while it is off neither
//! directory is loaded: there is no tree to walk, no `OnUpdate` to tick, no edit
//! box to type into and no button under the pointer.
//!
//! Each mode has its own set of view bar toggles. [`keep_view_settings`] gives
//! each mode its own copy of `WorldTuning`, `RenderTuning` and the overlays:
//! crossing into a playtest stores the editor's set and restores whatever the
//! last playtest had (the client's own defaults the first time), and crossing
//! back does the reverse. Without it, fog, sky and MSAA switched off while
//! editing stayed off in the playtest, which does not match the game, and had to
//! be switched back by hand at every crossing. It runs before [`arrange`], which
//! then writes the one field (`interface`) that belongs to the state rather than
//! to the person.
//!
//! [`stand_the_world_down`] is not a switch: the editor's nine tiles are
//! despawned and forgotten on the way in, so the login screen has an empty world
//! behind it as the client's own does, and whatever the character streams is
//! read again out of the overlay. The live patch in [`crate::tools`] keeps a
//! tile's vertices up to date; only a re-read also rebuilds its foliage, its
//! bounding sphere and its water from the edited bytes.
//!
//! ## Logging in without the login screen
//!
//! [`Login`] decides what the button does with the login screen: show it, or
//! skip it. This crate adds no setting of its own: the account is
//! `WTF\Config.wtf`'s `accountName`, the character is `VALE_CHARACTER` or
//! `lastCharacterIndex`, and the password is the one stand-in this project has
//! (`VALE_PASSWORD`, documented in `vale_config`). The panel changes all
//! three for the session without a restart, because an editor session logs in
//! many times and a client session does not.
//!
//! The only condition is a password. Without one the button opens the login
//! screen, which is also where a wrong password is reported.
//!
//! ## The three readers of an edited tile
//!
//! Three readers load an edited tile, and each must see the edit. The
//! renderer's streamer reads it through `GameAssets`' overlay; the editor's own
//! pick reads the open `AdtFile` directly. The third is the local simulation:
//! `ActiveSession::terrain`, an `vale_assets::MapTerrain` that opens an
//! archive chain of its own so the session thread does not queue behind tile
//! meshing. That chain was opened without the overlay, so a playtest walked on
//! the shipped ground: through a raised hill, and at the height the tile had
//! before the edit. It is now given the overlay through
//! `vale_assets::MapTerrain::open_with`, whose documentation has the rest.

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_client::glue::autologin::AutoLogin;
use vale_client::lua::host::InterfaceAwake;
use vale_client::render::focus::WorldFocus;
use vale_client::render::glue::GlueScenes;
use vale_client::render::terrain::{LoadedTiles, TerrainTile};
use vale_client::render::tuning::WorldTuning;
use vale_client::world::camera::{PointerTaken, RenderTuning};
use vale_client::world::session::{ClientConfig, Credentials, Session};

use bevy::prelude::*;

/// Whether the editor is editing, logging in, or being played.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Playtest {
    /// The free camera, the pointer and the tools are the editor's.
    #[default]
    Editing,
    /// The client's login and character screens are up.
    Entering,
    /// A character is in the world.
    Playing,
}

impl Playtest {
    /// Whether the editor is driving. Every one of the editor's own passes asks
    /// this: the camera, the pick, the brush, the panel. They ask it rather than
    /// `session.active.is_none()`, which is also true on the login screen, where
    /// the keyboard belongs to a password box and the `w` in a name would
    /// otherwise fly the editor's camera north.
    pub fn editing(self) -> bool {
        self == Playtest::Editing
    }

    /// Whether a playtest is running at all, in either of its two stages.
    pub fn playing(self) -> bool {
        !self.editing()
    }
}

/// Whether the editor's panels are drawn over a running playtest.
///
/// When it is false, [`Playtest::Playing`] replaces the whole shell with a
/// one-button bar, so changing anything means leaving the world and coming
/// back. It is a resource separate from the state because [`Playtest`] is a
/// `Copy` enum read by thirty-seven systems that ask [`Playtest::editing`] and
/// must keep getting the same answer: while the shell is open over a playtest
/// the editor is not driving the world, and every one of those early returns
/// stays correct.
///
/// It changes two things. The shell draws instead of the bar, and [`arrange`]
/// gives the editor `PointerTaken` so a drag on a panel does not swing the
/// camera under it. The game's interface stays awake, the login scene stays
/// disabled and the streamer stays on the client's 3x3, because all three are
/// properties of a running playtest rather than of who is looking at it.
///
/// Only the four workspaces are reachable while it is open: Spells, Items,
/// Quests and Tables. The tools that keep the viewport act on the editor's own
/// tiles, which [`stand_the_world_down`] despawned on the way in.
/// `crate::ui`'s shell does not draw the rail, disables the top bar's World
/// part and leaves the viewport rectangle empty, so no tool can act on ground
/// nobody is editing. See `crate::tools::Tool::survives_playtest`.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct ShellOpen(pub bool);

/// Who a playtest logs in as, and whether it skips the login screen.
///
/// Seeded from the install folder and the command line — see [`seed`] — and
/// editable in the panel for the session. Nothing here is written back: the
/// account is already a CVar the interface writes when the game's own login
/// screen remembers it, and the password is deliberately stored nowhere at all.
#[derive(Resource, Debug, Default, Clone)]
pub struct Login {
    /// `realmlist.wtf`'s address, kept so a logon can be built without reading
    /// the config again. Not shown and not editable: which server to talk to is
    /// a property of the install.
    host: String,
    /// `WTF\Config.wtf`'s `accountName`, or `VALE_ACCOUNT`.
    pub account: String,
    /// `VALE_PASSWORD`. Empty is the ordinary case and means the login
    /// screen.
    pub password: String,
    /// `--character`, or `VALE_CHARACTER`. Empty falls back to
    /// `lastCharacterIndex` — see [`AutoLogin`].
    pub character: String,
    /// Whether the button goes straight into the world rather than to the login
    /// screen. Off when there is no password to go with.
    pub straight_in: bool,
}

impl Login {
    /// The name this would log in as, for a button that has room to show it.
    ///
    /// The character when one is named and the account when none is, because
    /// `lastCharacterIndex` is a row and a row is not a name — see
    /// [`AutoLogin`], which is where the fallback is.
    pub fn who(&self) -> String {
        match self.character.trim() {
            "" => self.account.trim().to_string(),
            name => name.to_string(),
        }
    }

    /// Whether a click could actually get in without a screen.
    pub fn can_go_straight_in(&self) -> bool {
        !self.account.trim().is_empty() && !self.password.is_empty()
    }

    /// What to log on with when a click can go straight in. `None` is the login
    /// screen.
    ///
    /// The host is the folder's (`realmlist.wtf`, through
    /// [`Credentials::from_config`]), because it is the one field of the four
    /// this panel does not ask for: which server to talk to is a property of the
    /// install and not of who is playing.
    pub fn credentials(&self) -> Option<Credentials> {
        if !self.straight_in || !self.can_go_straight_in() {
            return None;
        }
        Some(Credentials {
            host: self.host.clone(),
            account: self.account.trim().to_string(),
            password: self.password.clone(),
            realm: None,
        })
    }
}

/// Read who to be out of the install folder and the command line.
///
/// `Startup`, because both sources are fixed for the run — and the panel edits
/// the resource afterwards rather than these.
fn seed(mut login: ResMut<Login>, config: Res<ClientConfig>, args: Res<crate::Args>) {
    let config = &config.0;
    login.host = config.host.clone();
    login.account = config.account.clone();
    login.password = config.password.clone();
    login.character = args
        .character
        .clone()
        .or_else(|| config.character.clone())
        .unwrap_or_default();
    // On when the folder supplies both an account and a password, off
    // otherwise, so the button does what the folder can deliver.
    login.straight_in = login.can_go_straight_in();
}

pub struct PlaytestPlugin;

impl Plugin for PlaytestPlugin {
    fn build(&self, app: &mut App) {
        app
            // Inserted before the first frame rather than left at its default.
            // `HostPlugin` has already `init_resource`d it as awake, and the
            // client's own loaders run in the same schedule as
            // [`arrange`] with nothing ordering the two, so leaving the first
            // frame to `arrange` loads a login screen and then tears it down:
            // a second of work and a visible flicker.
            .insert_resource(InterfaceAwake(false))
            .init_resource::<Playtest>()
            .init_resource::<ShellOpen>()
            .init_resource::<Login>()
            .init_resource::<ViewSettings>()
            .add_systems(Startup, seed)
            .add_systems(
                Update,
                (
                    on_the_command_line,
                    follow_the_session,
                    keep_view_settings,
                    arrange,
                    stand_the_world_down,
                )
                    .chain()
                    // Before everything that reads the state. The editor's
                    // camera, pick and tools all ask [`Playtest::editing`], and a
                    // frame that read it before this ran would drive the free camera
                    // through the first frame of a login. `crate::camera`'s own
                    // ordering note describes the same one-frame camera jump.
                    .before(crate::camera::fly),
            );
    }
}

/// `--playtest <seconds>[,<back>]`, which is the only way either edge is reached
/// without a person — see [`crate::Args::playtest`].
///
/// It presses the same two buttons the panel does, once each and then never
/// again: a scripted run that has come back to editing must not be dragged into
/// a second playtest by a flag that fires at a wall-clock time.
#[allow(clippy::too_many_arguments)]
fn on_the_command_line(
    time: Res<Time>,
    args: Res<crate::Args>,
    assets: Res<GameAssets>,
    mut state: ResMut<Playtest>,
    mut session: Option<ResMut<EditSession>>,
    mut client: ResMut<Session>,
    login: Res<Login>,
    mut auto: ResMut<AutoLogin>,
    mut shell: ResMut<ShellOpen>,
    mut tool: ResMut<crate::tools::Tool>,
    // What a playtest needs to reach the server: the queue its save's writes
    // go on, where the database is, and whether this session keeps its query
    // answers. See [`start`], and `crate::server::Reach`.
    mut reach: crate::server::Reach,
    mut done: Local<u8>,
) {
    // `--shell` is read every frame rather than once at the start. A
    // playtest may also be started from the button or the key while the flag is
    // set, and `arrange` clears the flag on the way back to editing, so the
    // condition is "a playtest is running and the panels are not open" rather
    // than a one-shot.
    if args.shell && state.playing() && !shell.0 {
        shell.0 = true;
        *tool = crate::tools::open_on(*tool);
    }
    let Some((after, back)) = args.playtest else {
        return;
    };
    let now = time.elapsed_secs();
    let Some(session) = session.as_mut() else {
        // No session yet is no project yet, which is a map that has not opened.
        // The clock keeps running and this fires on the frame it can.
        return;
    };
    if *done == 0 && now >= after {
        *done = 1;
        info!("--playtest: {after:.0}s in the editor — starting a playtest");
        start(
            &mut state,
            session,
            &assets,
            &login,
            &mut auto,
            &mut reach.queue,
            &reach.settings,
            &mut reach.caches,
        );
        return;
    }
    // Measured from the same zero as the start, so `--playtest 16,30` is
    // "start at sixteen seconds, stop at thirty" rather than fourteen seconds of
    // playtest. Two absolute times are easier to reason about from a log than a
    // time and a duration.
    if *done == 1 && back.is_some_and(|back| now >= back) {
        *done = 2;
        info!(
            "--playtest: {:.0}s — back to editing",
            back.unwrap_or_default()
        );
        stop(&mut state, &mut client, &mut auto, session);
    }
}

/// Follow the two edges the session decides, and neither of the two it does not.
///
/// It never leaves `Editing`. A playtest is started by a person, so a client
/// that found itself with a session for any other reason must not be able to
/// take the tools away — and equally, a playtest that has come back to the
/// character screen stays a playtest until somebody says otherwise.
fn follow_the_session(session: Res<Session>, mut state: ResMut<Playtest>) {
    if state.editing() {
        return;
    }
    let wanted = if session.active.is_some() {
        Playtest::Playing
    } else {
        Playtest::Entering
    };
    // Guarded on the value, because `ResMut`'s `DerefMut` marks the resource
    // changed whether or not anything moved.
    if *state != wanted {
        *state = wanted;
    }
}

/// Write the switches a playtest implies.
///
/// Every frame rather than on the edge, and guarded on the value rather than on
/// change detection: these are bools that anything else in the app may
/// also write — the debug window has a checkbox for the first — and a state
/// written once at the transition would be a state a checkbox could silently
/// contradict.
fn arrange(
    state: Res<Playtest>,
    args: Res<crate::Args>,
    mut awake: ResMut<InterfaceAwake>,
    mut tuning: ResMut<WorldTuning>,
    mut glue: ResMut<GlueScenes>,
    mut taken: ResMut<PointerTaken>,
    mut reach: ResMut<vale_client::render::terrain::TerrainReach>,
    mut shell: ResMut<ShellOpen>,
) {
    let editing = state.editing();
    // The shell is only ever open over a playtest. Coming back to editing
    // draws it unconditionally, so a flag left set would survive to the next
    // playtest and open it without being asked.
    if editing && shell.0 {
        shell.0 = false;
    }
    // How much ground is streamed: the editor's wide block while editing and
    // the client's own 3x3 while playing. A playtest is the client: the
    // character is in the middle of the block with the fog closing at 500
    // yards, so the extra tiles cost load time and are fogged out. That cost
    // lands on the loading screen, because `glue::loading` holds it
    // until every wanted tile has arrived. See [`crate::REACH`].
    let wanted = match editing {
        true => crate::reach(&args),
        false => vale_client::render::terrain::TerrainReach::default(),
    };
    if *reach != wanted {
        *reach = wanted;
    }
    // Whether the game's interface is loaded at all. The module comment says
    // why this is separate from `WorldTuning::interface`. While editing there
    // is no login screen, so no movement key can be typed into one.
    if awake.0 == editing {
        awake.0 = !editing;
    }
    // Its walk and its paint, a separate switch that is set as well: a host
    // that woke the interface and left this off would have an interface that
    // takes input and draws nothing.
    let wanted = world_baseline(editing).interface;
    if tuning.interface != wanted {
        tuning.interface = wanted;
    }
    // The login screen's own 3D scene. `render::glue::aim_camera` writes the
    // world camera's transform for as long as one is up, so leaving this on
    // while editing would frame every stroke through `UI_MainMenu.m2`'s camera,
    // and leaving it off during a playtest draws the login screen over the
    // editor's viewport instead of its own backdrop.
    if glue.0 == editing {
        glue.0 = !editing;
    }
    // Who owns the world's mouse gestures. The claim is the same one the
    // interface makes through `MouseFocus::over_interface`; `crate::camera`
    // describes what goes wrong when it is incorrect.
    //
    // This is the one switch of the five that follows [`ShellOpen`] as well as
    // the state. While the shell is drawn over a playtest every press lands on a
    // panel, and the game steers with the left button — the same button a
    // slider is dragged with — so without the claim a drag inside the browser
    // swings the camera and locks the pointer under it. The other four stay
    // keyed to the state: the interface, its paint, the login scene and the
    // streamed block are properties of a playtest running, not of whether
    // anybody is looking at a panel.
    let wanted = editing || shell.0;
    if taken.0 != wanted {
        taken.0 = wanted;
    }
}

/// One mode's view settings: everything on the view bar.
#[derive(Clone, Default, PartialEq)]
struct ViewSet {
    world: WorldTuning,
    frame: RenderTuning,
    #[cfg(feature = "diagnostics")]
    overlay: vale_client::render::overlay::DebugOverlay,
}

/// Each mode's own view settings, kept while the other mode is up.
///
/// `None` is a mode that has never been left: the first crossing into a
/// playtest restores the client's own defaults, because that is what a
/// playtest is, and the first crossing back restores whatever the editor had
/// when it left.
#[derive(Resource, Default)]
pub struct ViewSettings {
    editing: Option<ViewSet>,
    playing: Option<ViewSet>,
    /// Which side of the edge the last frame was on; `None` before the first.
    was_editing: Option<bool>,
}

/// The world switches as the editor sets them on entering a state: the
/// client's defaults, with the game's interface off while editing. [`arrange`]
/// sets `interface` from this, and the view bar measures "moved from its
/// default" and `reset` against it, so a switch the editor sets itself does not
/// read as a change somebody made.
pub fn world_baseline(editing: bool) -> WorldTuning {
    WorldTuning {
        interface: !editing,
        ..WorldTuning::default()
    }
}

/// Swap the view settings at each crossing — see the module comment.
///
/// Written back only on a crossing and only when the value moved, for the
/// reason on `crate::ui::viewbar::Bar`: `ResMut`'s `DerefMut` marks the
/// resource changed either way, and `render::tuning::switch` sweeps every
/// batch in the world on that flag.
fn keep_view_settings(
    state: Res<Playtest>,
    mut kept: ResMut<ViewSettings>,
    mut world: ResMut<WorldTuning>,
    mut frame: ResMut<RenderTuning>,
    #[cfg(feature = "diagnostics")] mut overlay: ResMut<
        vale_client::render::overlay::DebugOverlay,
    >,
) {
    let editing = state.editing();
    let Some(was) = kept.was_editing.replace(editing) else {
        return;
    };
    if was == editing {
        return;
    }
    let current = ViewSet {
        world: world.clone(),
        frame: frame.clone(),
        #[cfg(feature = "diagnostics")]
        overlay: overlay.clone(),
    };
    let restore = match was {
        true => {
            kept.editing = Some(current);
            kept.playing.clone().unwrap_or_default()
        }
        false => {
            kept.playing = Some(current);
            kept.editing.clone().unwrap_or_default()
        }
    };
    if *world != restore.world {
        *world = restore.world;
    }
    if *frame != restore.frame {
        *frame = restore.frame;
    }
    #[cfg(feature = "diagnostics")]
    if *overlay != restore.overlay {
        *overlay = restore.overlay;
    }
}

/// Take the editor's own world off the screen for the login.
///
/// The reference client holds no world at its login screen, and this one holds
/// nine tiles: `render::glue` spawns `UI_MainMenu.m2` at the world origin and
/// aims the camera at it, so an editor sitting anywhere near tile (32, 32)
/// would have its own terrain standing inside the login screen's sky. The
/// five-step route described in the module comment produced that picture.
///
/// It also makes the playtest show the edit. A tile forgotten here is a tile
/// the streamer reads again when the character arrives, out of the overlay and
/// therefore with everything in it rebuilt from the edited bytes: the foliage,
/// the tile's bounding sphere, the water, the shading across a chunk boundary.
/// The live patch in [`crate::tools`] keeps a stroke smooth; this makes the
/// ground the character walks on match the file on disk.
///
/// Both halves, or the tile never comes back: `LoadedTiles::reload`'s own note
/// says why, and `forget` is that note over the whole 3x3.
fn stand_the_world_down(
    state: Res<Playtest>,
    mut was_editing: Local<bool>,
    mut focus: ResMut<WorldFocus>,
    mut loaded: ResMut<LoadedTiles>,
    tiles: Query<Entity, With<TerrainTile>>,
    mut commands: Commands,
) {
    let editing = state.editing();
    let started = *was_editing && !editing;
    *was_editing = editing;
    if started {
        for tile in &tiles {
            commands.entity(tile).despawn();
        }
        loaded.forget();
    }
    // Every frame of the login and not only its first, because the focus
    // is one bool away from a streamer that would fill the screen behind the
    // backdrop again. `crate::session::follow_the_camera` stands down for the
    // same reason and says so; this is the value it would have written.
    if *state == Playtest::Entering && focus.present {
        focus.present = false;
    }
}

/// Put every open table where the client reads it, and drop what it already
/// parsed.
///
/// A table edit needs two things before the game reads it. The bytes have to be
/// in the overlay, which a field edit deliberately does not do (see
/// `EditSession::table_edited`, where the arithmetic is), and the tables already
/// parsed have to be forgotten, because `GameAssets` keeps them for the life of
/// the process and nothing asks twice. Without both, spell edits did not show in
/// a playtest.
///
/// It is called at the two moments the world is rebuilt anyway, rather than on
/// every edit: entering a playtest ([`start`]) and saving during one
/// ([`crate::tools::shortcuts`]). That is one 16 MB write and one re-parse per
/// action, instead of both on every frame of a drag.
///
/// It marks rather than rebuilds. `crate::session::forget_what_changed` runs
/// every frame and acts on the two flags set here, and what is standing keeps
/// the handles it has: the next thing to ask reads the edited file. For a
/// playtest that is the next cast, the next entity to come into view, and the
/// next model to be hung — not the aura already worn or the channel already
/// running.
pub fn republish(session: &mut EditSession, assets: &GameAssets) {
    let tables: Vec<String> = session.tables.keys().cloned().collect();
    for name in &tables {
        session.publish_table(name);
    }
    if !tables.is_empty() {
        assets.forget_tables();
        // Also mark the bank that holds an `Arc` of that parse, which
        // `forget_tables` does not reach. See `DisplayCache::forget`, and
        // `session::forget_what_changed`, which acts on this flag. Without
        // it a playtest showed the tables as they were at the first entity of
        // the editor's own session, which for a storyboard is before any edit
        // was made.
        session.tables_republished = true;
    }
    // The same for the files, the other bank keyed by a path
    // and kept for the life of the process: an edited `.m2` in the project
    // would otherwise be read once, at whatever the first session saw, and
    // every playtest after that would draw the old one. See
    // `EditSession::republished`, which `session::forget_changed_models`
    // drains on the next frame.
    session.republished_all = true;
}

/// Start a playtest, from the panel or from the keyboard.
///
/// It saves first, which is the one thing here that touches the disk. The
/// overlay would serve the edit without a save; the save puts the work in the
/// project folder, where a person looks for it afterwards, so the session
/// cannot end with the work in memory alone.
pub fn start(
    state: &mut Playtest,
    session: &mut EditSession,
    assets: &GameAssets,
    login: &Login,
    auto: &mut AutoLogin,
    // The queue whatever this applies goes on.
    queue: &mut crate::server::queue::ServerQueue,
    // Where this machine's server is, for the apply the save does, and for
    // whether the session about to start keeps its query answers.
    server: &crate::server::settings::ServerSettings,
    caches: &mut vale_client::world::session::QueryCaches,
) {
    session.save_all();
    session.save_all_tables();
    // Save the server half as well, for the same reason. See
    // [`crate::server::save`].
    //
    // This save asks the server for no `.reload`. A reload is sent only for a
    // write asked for with the panels open over a playtest, and this write is
    // queued while the state is still `Editing`. A row applied here is live
    // after the server's next restart. See
    // [`crate::server::reload::Reloads::when_there_is_a_session`].
    crate::server::save(session, assets, server, queue);
    republish(session, assets);
    // Whether the session about to start keeps the answers the server gives
    // it. The client writes every query answer to `WDB\` and reads them back
    // at the next login, which for an editor is the wrong behaviour: a
    // `creature_template` row changed a minute ago is answered out of a file
    // written before it was, and no query is ever sent that could replace it.
    // See `vale_client::world::session::QueryCaches`, and
    // [`crate::server::settings::ServerSettings::disable_caching`], which is
    // the Disable caching switch on the Server panel.
    //
    // Written here rather than at startup because it is read when the world is
    // entered, so the switch takes effect on the next playtest rather than on
    // the next launch.
    *caches = vale_client::world::session::QueryCaches(!server.disable_caching);
    match login.credentials() {
        // The account the panel is showing, not the one the folder had,
        // carried on the instruction rather than written to a resource:
        // nothing in the client inserts `Credentials` as a resource, and code
        // that assumed it did has broken twice. See
        // `vale_client::glue::autologin`.
        Some(credentials) => {
            *auto = AutoLogin::log_in_as(login.character.trim()).on_the_account(credentials);
            session.status = match login.character.trim() {
                "" => "playtesting: logging in".to_string(),
                name => format!("playtesting: logging in as {name}"),
            };
        }
        None => {
            auto.stand_down();
            session.status = "playtesting: log in".into();
        }
    }
    // Log which of the two routes it took. The panel shows the same, but a
    // scripted run has no panel: `--playtest` on a
    // machine with no password in the environment has to be distinguishable
    // from one with, and the two differ only in what is on screen.
    info!("playtest: {}", session.status);
    *state = Playtest::Entering;
}

/// Stop a playtest, from whichever of its states it is in.
///
/// [`Session::log_out`] rather than the game's own `Binding::Logout`, which is
/// `CMSG_LOGOUT_REQUEST` and the server's twenty-second delay. The stop returns
/// the tools to the editor rather than logging the character out in the game,
/// and dropping the socket is what the character screen's own Back button does. `render::residency`'s
/// `leave_world` sees the session go and tears the world down, so the editor
/// streams its own ground again from its own camera.
pub fn stop(
    state: &mut Playtest,
    session: &mut Session,
    auto: &mut AutoLogin,
    editing: &mut EditSession,
) {
    // This must run before the session is torn down. A stop
    // during a logon leaves the standing instruction armed, and the handshake
    // that lands afterwards would walk straight back into the world somebody
    // had just left.
    auto.stand_down();
    session.log_out();
    session.cancel_login();
    editing.status = "editing".into();
    *state = Playtest::Editing;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three states answer the one question every editor pass asks.
    #[test]
    fn only_editing_is_editing() {
        assert!(Playtest::Editing.editing());
        assert!(!Playtest::Entering.editing());
        assert!(!Playtest::Playing.editing());
        assert!(Playtest::Entering.playing());
        assert!(Playtest::Playing.playing());
    }

    /// A playtest is never started by the client. The guard is the whole of
    /// [`follow_the_session`]'s first two lines and it is what stops a stray
    /// session — a reconnect, a scripted login, anything — from taking the
    /// tools away from somebody who is painting.
    #[test]
    fn the_session_cannot_start_a_playtest() {
        let mut app = App::new();
        app.init_resource::<Playtest>()
            .init_resource::<Session>()
            .add_systems(Update, follow_the_session);
        app.update();
        assert_eq!(*app.world().resource::<Playtest>(), Playtest::Editing);
    }

    /// Once a playtest is running, a character screen is still a playtest.
    #[test]
    fn a_playtest_with_no_character_is_still_a_playtest() {
        let mut app = App::new();
        app.insert_resource(Playtest::Playing)
            .init_resource::<Session>()
            .add_systems(Update, follow_the_session);
        app.update();
        assert_eq!(*app.world().resource::<Playtest>(), Playtest::Entering);
    }

    /// A playtest can only go straight in when the folder supplies both an
    /// account and a password. An account with no password is the ordinary state of a fresh
    /// install, and it has to reach the login screen rather than a logon that
    /// fails on an empty secret.
    #[test]
    fn straight_in_needs_an_account_and_a_password() {
        let mut login = Login::default();
        assert!(!login.can_go_straight_in());
        login.account = "someone".into();
        assert!(!login.can_go_straight_in(), "no password");
        login.password = "secret".into();
        assert!(login.can_go_straight_in());
        login.account = "   ".into();
        assert!(!login.can_go_straight_in(), "a blank account is no account");
    }

    /// The switch is checked as well as the fields, so unticking it means the
    /// login screen however complete the rest of the form is.
    #[test]
    fn the_login_screen_is_what_an_unticked_box_means() {
        let mut login = Login {
            account: "someone".into(),
            password: "secret".into(),
            straight_in: true,
            ..Login::default()
        };
        assert!(login.credentials().is_some());
        login.straight_in = false;
        assert!(login.credentials().is_none());
        login.straight_in = true;
        login.password.clear();
        assert!(
            login.credentials().is_none(),
            "and so is a missing password"
        );
    }

    /// Each mode keeps its own view settings. Fog and MSAA switched off while
    /// editing are back at the client's own defaults in the playtest, and off
    /// again on the way back; a change made during the playtest belongs to the
    /// playtest and is restored at the next one.
    #[test]
    fn each_mode_keeps_its_own_view_settings() {
        let mut app = App::new();
        app.init_resource::<Playtest>()
            .init_resource::<ViewSettings>()
            .init_resource::<WorldTuning>()
            .init_resource::<RenderTuning>();
        #[cfg(feature = "diagnostics")]
        app.init_resource::<vale_client::render::overlay::DebugOverlay>();
        app.add_systems(Update, keep_view_settings);
        app.update();

        // Editing with two things off.
        app.world_mut().resource_mut::<WorldTuning>().fog = false;
        app.world_mut().resource_mut::<RenderTuning>().msaa = true;
        app.update();
        assert!(!app.world().resource::<WorldTuning>().fog, "still editing");

        // Into a playtest: the defaults.
        app.insert_resource(Playtest::Entering);
        app.update();
        assert!(
            app.world().resource::<WorldTuning>().fog,
            "a playtest has the fog"
        );
        assert_eq!(
            app.world().resource::<RenderTuning>().msaa,
            RenderTuning::default().msaa,
            "…and the client's own MSAA"
        );

        // Something changed while playing, then back to editing.
        app.world_mut().resource_mut::<WorldTuning>().terrain = false;
        app.insert_resource(Playtest::Playing);
        app.update();
        app.insert_resource(Playtest::Editing);
        app.update();
        let world = app.world().resource::<WorldTuning>();
        assert!(!world.fog, "the editor's own fog setting is back");
        assert!(world.terrain, "…and the playtest's change did not follow");
        assert!(
            app.world().resource::<RenderTuning>().msaa,
            "the editor's MSAA is back"
        );

        // The next playtest restores the previous playtest's set.
        app.insert_resource(Playtest::Entering);
        app.update();
        assert!(
            !app.world().resource::<WorldTuning>().terrain,
            "the playtest's own set"
        );
        assert!(app.world().resource::<WorldTuning>().fog);
    }

    /// The switches are the state's, in both directions.
    #[test]
    fn the_switches_follow_the_state() {
        use vale_client::render::terrain::TerrainReach;
        let mut app = App::new();
        app.init_resource::<Playtest>()
            .init_resource::<InterfaceAwake>()
            .init_resource::<WorldTuning>()
            .init_resource::<GlueScenes>()
            .init_resource::<PointerTaken>()
            .init_resource::<TerrainReach>()
            .init_resource::<ShellOpen>()
            .insert_resource(crate::Args::default())
            .add_systems(Update, arrange);
        app.update();
        assert!(!app.world().resource::<InterfaceAwake>().0, "editing");
        assert!(!app.world().resource::<WorldTuning>().interface, "editing");
        assert!(!app.world().resource::<GlueScenes>().0, "editing");
        assert!(app.world().resource::<PointerTaken>().0, "editing");
        assert_eq!(
            app.world().resource::<TerrainReach>().0,
            crate::REACH,
            "editing streams the editor's wide block"
        );

        app.insert_resource(Playtest::Entering);
        app.update();
        assert!(app.world().resource::<InterfaceAwake>().0, "playtesting");
        assert!(
            app.world().resource::<WorldTuning>().interface,
            "playtesting"
        );
        assert!(app.world().resource::<GlueScenes>().0, "playtesting");
        assert!(!app.world().resource::<PointerTaken>().0, "playtesting");
        assert_eq!(
            *app.world().resource::<TerrainReach>(),
            TerrainReach::default(),
            "…and a playtest is the client's own 3x3"
        );

        // And back to editing.
        app.insert_resource(Playtest::Editing);
        app.update();
        assert_eq!(
            app.world().resource::<TerrainReach>().0,
            crate::REACH,
            "back to editing"
        );
    }

    /// The shell over a playtest changes one switch and leaves four alone.
    ///
    /// `PointerTaken` is the editor's while its panels are drawn, because the
    /// game steers with the left button and that is the button a slider is
    /// dragged with. The other four describe a playtest running rather than who
    /// is looking at it, so opening the panels must not put the login scene back
    /// or take the game's interface away.
    #[test]
    fn the_shell_takes_the_pointer_and_nothing_else() {
        use vale_client::render::terrain::TerrainReach;
        let mut app = App::new();
        app.insert_resource(Playtest::Playing)
            .init_resource::<InterfaceAwake>()
            .init_resource::<WorldTuning>()
            .init_resource::<GlueScenes>()
            .init_resource::<PointerTaken>()
            .init_resource::<TerrainReach>()
            .init_resource::<ShellOpen>()
            .insert_resource(crate::Args::default())
            .add_systems(Update, arrange);
        app.update();
        assert!(
            !app.world().resource::<PointerTaken>().0,
            "the bar, not the panels"
        );

        app.world_mut().resource_mut::<ShellOpen>().0 = true;
        app.update();
        assert!(
            app.world().resource::<PointerTaken>().0,
            "a drag lands on a panel"
        );
        assert!(
            app.world().resource::<InterfaceAwake>().0,
            "the game's interface stays up"
        );
        assert!(
            app.world().resource::<WorldTuning>().interface,
            "…and stays painted"
        );
        assert!(
            app.world().resource::<GlueScenes>().0,
            "…and the login scene stays as it was"
        );
        assert_eq!(
            *app.world().resource::<TerrainReach>(),
            TerrainReach::default(),
            "…and the streamer stays on the client's 3x3"
        );
    }

    /// Coming back to editing shuts the shell.
    ///
    /// The panels are drawn unconditionally while editing, so a flag left set
    /// would survive to the next playtest and open it without being asked.
    #[test]
    fn leaving_a_playtest_shuts_the_shell() {
        use vale_client::render::terrain::TerrainReach;
        let mut app = App::new();
        app.insert_resource(Playtest::Playing)
            .init_resource::<InterfaceAwake>()
            .init_resource::<WorldTuning>()
            .init_resource::<GlueScenes>()
            .init_resource::<PointerTaken>()
            .init_resource::<TerrainReach>()
            .insert_resource(ShellOpen(true))
            .insert_resource(crate::Args::default())
            .add_systems(Update, arrange);
        app.update();
        assert!(app.world().resource::<ShellOpen>().0);

        app.insert_resource(Playtest::Editing);
        app.update();
        assert!(!app.world().resource::<ShellOpen>().0);
        assert!(
            app.world().resource::<PointerTaken>().0,
            "the editor is driving again"
        );
    }
}
