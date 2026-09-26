//! The Bevy renderer.
//!
//! Runs in-process with `vale-protocol` and `vale-assets`. The world
//! snapshot, the vertex arrays and the pose maths therefore do not cross an IPC
//! boundary, and `M2Skeleton::pose` has one copy rather than two that have to be
//! checked against each other.
//!
//! The parsing crates stay the authority on file formats. This crate only turns
//! what they produce into meshes, materials and entities, and [`render::axes`]
//! is the one place where their coordinate frame becomes Bevy's.
//!
//! ```text
//! render/  file -> picture; nothing in it needs the network
//! world/   what the server's answers mean, keyed by GUID or by position
//! game/    what the player can do: target, attack, cast
//! lua/     the interface's own language, running the interface's own code
//! ui/      drawn over the world: the FrameXML interface and the diagnostics,
//!          see ui/mod.rs
//! sound/   what the world sounds like: music, ambience, footsteps, combat
//!
//! app.rs      builds the app, in the three steps a second host needs
//!             separately from this one's flags
//! assets.rs   the archive chain and the tables read out of it, as resources
//! doctree.rs  the check that every directory's mod.rs still describes what is
//!             in it; the file is a test only
//! ```
//!
//! Each directory's own `mod.rs` says what is in it and what belongs there. The
//! reason: a subject with no directory of its own is appended
//! to the nearest file, which is how `world::entities` reached three and a half
//! thousand lines.

pub mod app;
pub mod assets;
// The test that each directory's own header still describes what is in it.
// All six headers once went stale together. See `doctree.rs`.
mod doctree;
pub mod game;
pub mod lua;
pub mod render;
pub mod sound;
pub mod ui;
pub mod world;

// The subject directories above are the crate's structure. Only the modules
// this file uses are brought into scope. Each pass registers itself through its
// directory's plugin group, so adding one edits `render/mod.rs`, `world/mod.rs`
// or `ui/mod.rs` and never the crate root. See [`render::RenderPlugins`].
use render::axes;
// `draws`, `models` and `particles` are used only by the `--shot` numbers;
// see [`screenshots`], where the dump is behind the `diagnostics` feature.
#[cfg(feature = "diagnostics")]
use render::{draws, models, particles};
use world::{camera, session};

#[cfg(feature = "diagnostics")]
use bevy::camera::visibility::ViewVisibility;
#[cfg(feature = "diagnostics")]
use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
// The GPU backend the window is opened on is chosen in `app::wgpu_settings`,
// the only place in this crate that chooses one; the reasons are its doc.

/// Open a CPU zone for the rest of the enclosing scope. See
/// [`ui::debug::spans`] for the ledger and the slot names.
///
/// A no-op without the `diagnostics` feature: it expands to `()`, so the
/// `Instant` pair and the statics are not compiled, and the `Slot` path in the
/// argument is never expanded. A call site may therefore name a slot in a
/// module that build does not compile.
///
/// Defined in the crate root rather than beside the ledger because
/// `#[macro_export]` only reaches the crate root if the module holding it is
/// compiled, and a `--no-default-features` build does not compile `ui::debug`.
#[macro_export]
macro_rules! zone {
    ($slot:expr) => {{
        #[cfg(feature = "diagnostics")]
        {
            $crate::ui::debug::spans::open($slot)
        }
        #[cfg(not(feature = "diagnostics"))]
        {
        }
    }};
}

/// Build and run the app.
pub fn run() {
    // The folder this was started in is the install; this is the drop-in rule.
    // `realmlist.wtf` names the server, `WTF\Config.wtf` names the account, and
    // `Data\` holds the archives. `VALE_GAMEDATA` overrides the archive
    // folder for a run started elsewhere. The CLI reads the same folder the
    // same way.
    let config = vale_config::Config::load();
    let gamedata_dir = config.gamedata.clone();
    // Cloned here because `config` is moved into the app before its one use.
    let character_env = config.character.clone();

    let args = Args::parse(std::env::args().skip(1));

    // Handled before the `App` is built, because the audit opens no window and
    // needs no server. See [`lua::audit`].
    if args.audit {
        lua::audit::run(
            &gamedata_dir,
            &config.root,
            &lua::audit::Probe {
                script: args.script,
                draw: args.draw,
                spin: args.spin,
                events: args.events,
                typed: args.typed,
                panels: args.panels,
                clicks: args.clicks,
                bindings: args.bindings,
                glue: args.glue,
                party: args.party,
                raid: args.raid,
            },
        );
        return;
    }

    let mut app = App::new();
    let plugins = app::plugins(&app::Host {
        title: "Vale".into(),
        // `--size`, or bevy's own default — see [`WindowScript`].
        size: args.window.open,
    });
    app.add_plugins(plugins);
    app::core(&mut app, gamedata_dir, config);
    // `--character`, or `VALE_CHARACTER`, which gives the same instruction
    // from the environment and is opt-in in the same way as `VALE_ACCOUNT`.
    // See [`game::session::autologin`], which handles both halves of logging in
    // without a screen, and [`vale_config::CHARACTER_ENV`]. A run that names
    // no character leaves the resource at its idle default and starts at the
    // login screen, which is the ordinary start.
    if let Some(character) = args.character.clone().or(character_env) {
        app.insert_resource(game::session::autologin::AutoLogin::log_in_as(character));
    }
    app
        .insert_resource(args.shot)
        .insert_resource(HoverProbe(args.hover))
        .insert_resource(Relogin::new(args.relogin))
        .insert_resource(args.window)
        .insert_resource(lua::host::StartupScript::new(args.script));
    // `--panel` is the only way a scripted run can see the debug window; see
    // [`Args::panel`]. These insertions are behind the `diagnostics` feature,
    // which owns the window, so in a build without it `--panel` is accepted
    // and ignored rather than failing to compile.
    #[cfg(feature = "diagnostics")]
    if let Some(crowd) = args.crowd {
        app.insert_resource(crowd);
    }
    #[cfg(feature = "diagnostics")]
    if let Some(overlay) = args.overlay {
        app.insert_resource(overlay);
    }
    // `--panel` opens the window; `--capture` arms the ring whether or not it
    // is open, since a scripted run may want the packets and no picture.
    #[cfg(feature = "diagnostics")]
    if args.panel.is_some() || args.capture {
        app.insert_resource(ui::debug::SettingsPanel {
            open: args.panel.is_some(),
            tab: args.panel.as_deref().map(ui::debug::Tab::named).unwrap_or_default(),
            capture: args.capture,
        });
    }
    app
        .add_systems(
            Update,
            (
                screenshots,
                // Before the pick, so the frame that requests the logout does
                // not also click a character. The pick reads
                // `Session::selection`, which is `None` while there is a world,
                // so the order only decides whether the second login starts on
                // this frame or the next. It is stated because an ordering that
                // matters is stated rather than inherited.
                relogin_probe.before(game::session::autologin::pick_the_character),
                // Ordered explicitly. Both systems take the window mutably, so
                // Bevy sequences them anyway, but only through that shared
                // access, which holds only while both keep it. The order
                // matters: a scripted `--resize` requests a size, and the snap
                // decides the shape it ends up at.
                (resize_window, hold_the_aspect).chain(),
                // `--hover`, the one input a script cannot otherwise supply;
                // see [`HoverProbe`]. Both systems are behind `run_if`, so a
                // run without `--hover` does not run them.
                (hover_probe, report_the_hover)
                    .chain()
                    .run_if(|probe: Res<HoverProbe>| probe.0.is_some()),
                // Alt+Enter: the other route to full screen besides
                // maximising, and the one the player asks for deliberately.
                // See [`toggle_fullscreen`].
                toggle_fullscreen,
            ),
        );
    // The schedule executors: seven measurements, each with its A/B and its
    // environment switch. See [`app::executors`].
    app::executors(&mut app);
    // Inserted after the plugins, so it replaces the default that
    // `CameraPlugin` installed rather than being replaced by it.
    if let Some(tuning) = args.tuning {
        app.insert_resource(tuning);
    }
    // `--without`: which layers of the world are drawn. This is the scripted
    // form of the F4 window's checkboxes, so a pass can be priced by two runs
    // that differ in one name. See `render::tuning::WorldTuning::without`.
    if let Some(world) = args.world {
        app.insert_resource(world);
    }
    // `--night`: how deep the night is. It adds to the game where `--without`
    // subtracts, and it deviates from the game where `--tune` trades against
    // this machine. See [`render::night`].
    if let Some(night) = args.night {
        app.insert_resource(night);
    }
    // `--hour`: the hour the world is lit at. A scripted shot of the night has
    // no other way to set it. See [`Args::hour`].
    if let Some(half_minutes) = args.hour {
        app.insert_resource(render::sky::WorldClock {
            half_minutes,
            from_server: false,
            override_half_minutes: Some(half_minutes),
        });
    }
    // `--weather`: the weather, for the same reason. See [`Args::weather`].
    if let Some(weather) = args.weather {
        app.insert_resource(weather);
    }
    app.run();
}

/// `hh:mm`, or a bare hour, as the half-minutes past midnight `WorldClock` is
/// in. See [`Args::hour`].
///
/// A value that does not parse returns `None` and logs a warning, so the run
/// does not silently photograph whatever hour the server is at. `--without`
/// and `--night` follow the same rule for the same reason.
fn parse_hour(text: &str) -> Option<u32> {
    let read = || {
        let (hours, minutes) = match text.trim().split_once(':') {
            Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
            None => (text.trim().parse::<u32>().ok()?, 0),
        };
        (hours <= 23 && minutes <= 59).then_some((hours * 60 + minutes) * 2)
    };
    let parsed = read();
    if parsed.is_none() {
        warn!("--hour {text}: expected hh:mm, or a bare hour in 0..23");
    }
    parsed
}

/// The command line: who to log in as, and whether this run is a scripted look
/// at the screen rather than someone sitting in front of it.
///
/// ```text
/// vale-client <Character>
/// vale-client <Character> --shot <path> [--after <seconds>] [--view <distance,pitch°,yaw°>]
///                            [--tune none|msaa,novsync,shadows]
///                            [--without doodads,buildings,entities,water,particles,interface,...]
///                            [--night off|0..1] [--hour <hh:mm>] [--weather <kind>[,<grade>]]
///                            [--script <lua>] [--overlay wireframe,collision,…]
///                            [--panel <tab>] [--relogin <seconds>[,<gap>]]
/// ```
///
/// These are flags rather than environment variables so that a check runs in
/// one line. The environment-variable form needed three `$env:` assignments,
/// did not end, and had to be killed. The `--shot` form logs in, frames the
/// view, writes the PNG and quits.
struct Args {
    /// Enters the world without waiting to be clicked, mirroring `vale live
    /// <Character>`. `None` leaves the app sitting at the empty world.
    character: Option<String>,
    shot: Screenshots,
    /// `--tune`: the render settings to start with, `None` for the defaults.
    tuning: Option<camera::RenderTuning>,
    /// `--without`: the world layers to leave out; see
    /// [`render::tuning::WorldTuning::without`]. The counterpart of `--tune`:
    /// it prices a pass where `--tune` prices a setting.
    world: Option<render::tuning::WorldTuning>,
    /// `--night <off|0..1>`: how deep the deep night is. `None` keeps the
    /// default, which is on.
    ///
    /// `--tune` trades CPU against GPU against image quality on this machine,
    /// and `--without` removes a layer so a pass can be priced. This flag adds
    /// something the game does not have, which is why it is separate from both.
    /// It exists so the option can be compared with a shot of the same framing
    /// under `--night off`, which is the comparison that shows whether it is too
    /// dark.
    ///
    /// See [`render::night`], which owns it and says why the default is on:
    /// the grade is exactly the identity in daylight, so an unflagged run at
    /// noon renders the same client every earlier measurement was taken against.
    night: Option<render::night::NightTuning>,
    /// `--hour <hh:mm>`: sets the world's clock by hand for the whole run.
    ///
    /// The scripted counterpart of the debug window's hour slider, which exists
    /// for the same reason: the world's clock runs at the server's rate, so
    /// waiting for dusk takes up to twenty-four minutes. A script cannot reach
    /// the slider any more than it can press `F4` or move the mouse. Before this
    /// flag, no `--shot` could photograph the night.
    ///
    /// `hh:mm`, or a bare hour. It sets `WorldClock::override_half_minutes`,
    /// the same latch the slider sets, so the server's clock is still read
    /// underneath and the HUD says the hour was set by hand.
    hour: Option<u32>,
    /// `--weather <kind>[,<grade>]`: a weather packet the server never sent,
    /// applied for the whole run, such as `snow` or `rain,0.4`. The counterpart
    /// of `--hour`: a scripted shot cannot wait for a zone to start snowing, and
    /// a GM command needs an account the run may not have. See
    /// [`render::weather::ForcedWeather`].
    weather: Option<render::weather::ForcedWeather>,
    /// `--script`: one Lua chunk, run once the interface has settled.
    ///
    /// An interface path that cannot be driven from a script cannot be checked
    /// without a person at the keyboard.
    /// `--script 'ActionButtonDown(1); ActionButtonUp(1)'` confirms that the
    /// casting chain reaches the server, and it is how the fix for a
    /// casting-chain regression was confirmed.
    script: Option<String>,
    /// `--overlay <names>`: the visualisations to turn on; see
    /// [`render::overlay::DebugOverlay::with`]. The counterpart of `--without`:
    /// it adds rather than subtracts (a wireframe, the bounding boxes, the solid
    /// triangles underfoot). Off unless named.
    ///
    /// Behind the feature that owns the overlays, like `--panel`, so a build
    /// without it ignores the flag rather than failing to compile.
    #[cfg(feature = "diagnostics")]
    overlay: Option<render::overlay::DebugOverlay>,
    /// `--panel <tab>`: open the debug window at that tab on the first frame.
    ///
    /// The debug window is the one thing on screen a script could not
    /// otherwise reach. A `--shot` checks the world by framing it and the
    /// interface through `--script`, but the window is behind `F4`, and a
    /// scripted run cannot press a key any more than it can move the mouse (see
    /// [`HoverProbe`] for the pointer). Without this flag a rebuilt panel could
    /// only be checked by a person, which is the cost to design against.
    ///
    /// The value is a tab name as the tab strip spells it: `frame`, `scene`,
    /// `render`, `world`, `net`, `interface`. A name that matches nothing logs a
    /// warning and opens the default tab, rather than silently photographing
    /// the wrong one.
    panel: Option<String>,
    /// `--capture`: arm the recent-packet ring as soon as there is a session.
    ///
    /// The ring is armed by a checkbox, and a scripted run cannot tick one any
    /// more than it can press `F4`; `--panel` and `--overlay` exist for the same
    /// reason. Without this flag the one instrument that shows what a packet
    /// contained could only be used by a person at the keyboard, which is the
    /// cost to design against.
    ///
    /// Arming it costs a lock and a bounded copy per packet (see
    /// [`vale_protocol::socket::world::Capture`]), so it is off unless asked
    /// for, like every other instrument here.
    #[cfg(feature = "diagnostics")]
    capture: bool,
    /// `--audit`: load the interface, print what broke, and do not open a
    /// window. See [`lua::audit`]. It is the run-time half of
    /// `vale framexml`, and the only check that can say which missing name
    /// breaks which panel.
    audit: bool,
    /// `--draw`, with `--audit`: after the load (and after `--script`, when one
    /// is given), dump every visible object with its solved rectangle and its
    /// paint. It answers "what is this quad on the screen" without a window.
    draw: bool,
    /// `--spin <frames>`, with `--audit`: run that many simulated frames of the
    /// interface's per-frame work and print the timing shape. It is the
    /// headless check for a frame rate that oscillates; see [`lua::audit`].
    spin: usize,
    /// `--party <n>`, with `--audit`: how many people the double is grouped
    /// with, 0..4. The party size is the one dimension of the harness's world
    /// that a report blamed, so it is a subtraction: two runs that differ only
    /// in this flag price the party frames, as `--without` prices a render
    /// pass. Defaults to 2, which every earlier round measured against.
    party: Option<usize>,
    /// `--raid <n>`, with `--audit`: how many people are in the double's raid,
    /// the double included, 0..40. Defaults to 0, a party. This number decides
    /// which of its two screens the raid panel draws, and the interface hides
    /// every party frame while it is above zero, so a harness that was always
    /// in a raid would stop checking the party.
    raid: Option<usize>,
    /// `--events`, with `--audit`: fire every event this client can raise at
    /// the frames that registered for it, and report what the handlers broke
    /// on. The load exercises only `OnLoad`, one of the game's 36 script kinds;
    /// this exercises `OnEvent`, the one that runs during play.
    events: bool,
    /// `--type <line>`, with `--audit`: open the chat line, type that into it
    /// and press Enter, then say what would have gone on the wire. This checks
    /// the path a person drives without a person; see
    /// [`lua::audit::type_a_line`].
    typed: Option<String>,
    /// `--panels`, with `--audit`: open every panel `UIPanelWindows` names and
    /// report what its `OnShow` broke on. The fourth script kind an instrument
    /// reaches, and the one in which 1.12 fills a panel; see
    /// [`lua::audit::open_every_panel`].
    panels: bool,
    /// `--clicks`, with `--audit`: press every visible button on every one of
    /// those panels, re-collecting after each round so the tabs are followed,
    /// and report what the `OnClick` bodies broke on. The fifth script kind an
    /// instrument reaches, and the one a player uses most; see
    /// [`lua::audit::click_everything`].
    clicks: bool,
    /// `--bindings`, with `--audit`: run the `<Binding>` body of every command
    /// the game's own default key table binds, on both edges where the
    /// declaration wants one, and report what they broke on. The sixth script
    /// kind an instrument reaches, and the only one a keystroke runs; see
    /// [`lua::audit::press_every_binding`].
    bindings: bool,
    /// `--glue`, with `--audit`: load `Interface\GlueXML\` instead of
    /// `Interface\FrameXML\` — the login screen and character select — and put
    /// the login screen up. Composes with every flag above it: `--glue --clicks`
    /// presses every button on the screen a player sees first.
    glue: bool,
    /// `--size <w>x<h>` and `--resize <w>x<h>`: the window to open at, and the
    /// window to become part way through the run.
    ///
    /// The same reasoning as `--tune` and `--without`, for an input that could
    /// not be varied from a script at all. Every aspect-ratio report is about a
    /// window somebody resized. Before these flags, only a person at the
    /// keyboard could see such a fault: a scripted shot always ran at Bevy's
    /// default 1280x720, the one shape at which nothing goes wrong.
    window: WindowScript,
    /// `--hover <x>,<y>`: plant the pointer on the window and report what it
    /// landed on, in logical window pixels from the top left.
    ///
    /// The one input a scripted run could not otherwise supply. Everything
    /// else this client does can be driven from `--script`, but the mouse pick
    /// reads `Window::cursor_position` and Lua cannot move the mouse, so
    /// hovering had no check that did not involve a person. The substitute was
    /// a PowerShell loop moving the OS cursor across the window while the
    /// client logged what it saw. Two game-object hover bugs were found that
    /// way, and each would have been a one-line run with this flag. See
    /// [`hover_probe`].
    hover: Option<Vec2>,
    /// `--relogin <seconds>`: leave the world for character select and come
    /// straight back in, once, after this long in the world.
    ///
    /// No other flag reaches the second login. `--script` runs one chunk
    /// against the interface, and the interface does not survive the logout:
    /// the Lua host is rebuilt for `Interface\GlueXML\`, taking any frame a
    /// chunk created with it, so a run that logs out ends at the character
    /// screen. Every other check in this file therefore covers only the first
    /// login. The map, the tiles, the global building and the caches are all
    /// re-derived on the second, and the teardown, the host rebuild and the
    /// cache purge were written from reasoning rather than tested by a run.
    ///
    /// The logout is the game's own: a `Binding::Logout` on the queue the key
    /// press writes to, so the server's delay and `SMSG_LOGOUT_COMPLETE` are
    /// both real. Give `--after` room for the time in the world, this delay,
    /// the server's logout delay, the wait below, and the second login.
    ///
    /// `--relogin <seconds>,<gap>` waits `gap` seconds at the character
    /// screen. [`render::residency`]'s sweep runs every five seconds and treats
    /// the logout as the deadline, so a second login taken immediately would
    /// reuse caches that a real player's client would have purged. The default
    /// gap allows at least one purge.
    relogin: Option<(f32, f32)>,
    /// `--crowd <players>[,<mobs>]`: fill the frame with entities the server
    /// never sent, to price the entity pass, which had no subtraction.
    ///
    /// `--without` does this for a render layer and `--party` for the
    /// interface; neither reaches the entity population. The only other way to
    /// get forty players into one frame is forty people, so the entity pass was
    /// the one cost in this renderer that could not be measured from a script.
    /// See [`world::crowd`].
    #[cfg(feature = "diagnostics")]
    crowd: Option<world::crowd::Crowd>,
}

impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Args {
        let mut parsed = Args {
            character: None,
            // 25 seconds is about what the login and the first nine tiles take,
            // so the default is a shot of a world that has finished arriving.
            shot: Screenshots { after: 25.0, ..default() },
            tuning: None,
            world: None,
            night: None,
            hour: None,
            weather: None,
            script: None,
            #[cfg(feature = "diagnostics")]
            overlay: None,
            panel: None,
            #[cfg(feature = "diagnostics")]
            capture: false,
            audit: false,
            draw: false,
            spin: 0,
            party: None,
            raid: None,
            events: false,
            typed: None,
            panels: false,
            clicks: false,
            bindings: false,
            glue: false,
            window: WindowScript::default(),
            hover: None,
            relogin: None,
            #[cfg(feature = "diagnostics")]
            crowd: None,
        };
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            // A flag's value is the next argument, and a missing one is left as
            // the default rather than being an error — this is a developer tool,
            // and a mistyped `--after` should still give a screenshot.
            let mut value = || args.next();
            match arg.as_str() {
                "--shot" => parsed.shot.path = value(),
                "--after" => parsed.shot.after = value().and_then(|s| s.parse().ok()).unwrap_or(25.0),
                "--view" => {
                    parsed.shot.view = value().and_then(|s| {
                        let n: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                        <[f32; 3]>::try_from(n.as_slice()).ok()
                    })
                }
                "--tune" => parsed.tuning = value().map(|s| tune(&s)),
                // Takes its value like every other flag here, so it always
                // takes one: a bare `--night` would swallow the character name.
                // `--night on` keeps the default, which is what `on` parses to.
                "--night" => {
                    parsed.night = Some(render::night::NightTuning::parse(value().as_deref()))
                }
                "--hour" => parsed.hour = value().as_deref().and_then(parse_hour),
                "--weather" => {
                    parsed.weather = value().as_deref().and_then(|text| {
                        let parsed = render::weather::ForcedWeather::parse(text);
                        if parsed.is_none() {
                            warn!("--weather {text}: expected rain|snow|sand|fine, with an optional ,0..1 grade");
                        }
                        parsed
                    })
                }
                "--without" => {
                    parsed.world = value().map(|s| render::tuning::WorldTuning::without(&s))
                }
                "--hover" => {
                    parsed.hover = value().and_then(|s| {
                        let n: Vec<f32> =
                            s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                        <[f32; 2]>::try_from(n.as_slice()).ok().map(Vec2::from)
                    })
                }
                "--script" => parsed.script = value(),
                "--relogin" => {
                    let spelt = value().unwrap_or_default();
                    let mut halves = spelt.split(',').map(str::trim);
                    let after = halves.next().and_then(|s| s.parse().ok()).unwrap_or(20.0);
                    let gap = halves.next().and_then(|s| s.parse().ok()).unwrap_or(12.0);
                    parsed.relogin = Some((after, gap));
                }
                #[cfg(feature = "diagnostics")]
                "--crowd" => parsed.crowd = value().map(|s| world::crowd::Crowd::parse(&s)),
                #[cfg(feature = "diagnostics")]
                "--overlay" => {
                    parsed.overlay = value().map(|s| render::overlay::DebugOverlay::with(&s))
                }
                "--panel" => parsed.panel = value(),
                #[cfg(feature = "diagnostics")]
                "--capture" => parsed.capture = true,
                "--audit" => parsed.audit = true,
                "--draw" => parsed.draw = true,
                "--spin" => parsed.spin = value().and_then(|s| s.parse().ok()).unwrap_or(0),
                "--party" => parsed.party = value().and_then(|s| s.parse().ok()),
                "--raid" => parsed.raid = value().and_then(|s| s.parse().ok()),
                "--events" => parsed.events = true,
                "--panels" => parsed.panels = true,
                "--clicks" => parsed.clicks = true,
                "--bindings" => parsed.bindings = true,
                "--glue" => parsed.glue = true,
                "--type" => parsed.typed = value(),
                "--size" => parsed.window.open = value().as_deref().and_then(parse_size),
                "--resize" => parsed.window.then = value().as_deref().and_then(parse_size),
                _ if arg.starts_with("--") => warn!("unknown argument {arg}"),
                _ => parsed.character = Some(arg),
            }
        }
        // Derived from `--after` rather than given, so the run has one clock. A
        // resize with its own time would be a second value to keep in step with
        // the shutter, and the only relationship between them that matters is
        // that the resize comes first.
        parsed.window.at = parsed.shot.after * RESIZE_FRACTION;
        parsed
    }
}

/// `--tune`'s value: the settings that are on, by name, comma-separated.
/// `none` (or anything else that names nothing) leaves them all off.
///
/// F5, F9 and F10 toggle the same three settings at the keyboard, but a
/// scripted shot has no keyboard, and a setting that cannot be scripted cannot
/// have its A/B logged beside the numbers. This lets two `--shot` runs differ
/// in one setting at one framing, the same subtraction that attributed the
/// alpha-map seam grid and the anisotropy lattice.
fn tune(list: &str) -> camera::RenderTuning {
    let on = |name: &str| list.split(',').any(|p| p.trim().eq_ignore_ascii_case(name));
    camera::RenderTuning {
        msaa: on("msaa"),
        // Spelled as the off switch because vsync is the one setting whose
        // default is on, and leaving it out of a list like `msaa,shadows` must
        // not silently unlock the frame rate. An unlocked frame is a different
        // measurement and has to be asked for by name.
        vsync: !on("novsync"),
        sun_shadows: on("shadows"),
    }
}

/// Saving the frame to a PNG, on a key or on a timer.
///
/// When the output is a picture, the picture decides whether it is right. The
/// terrain once rendered inside out while every measurement passed (the
/// draw-group count cross-checked against the CLI, the triangle counts, no
/// errors), and one screenshot showed the fault.
///
/// The game's `SCREENSHOT` binding (`PRINTSCREEN` by default) saves a frame on
/// demand for someone at the keyboard. `--shot <path>` takes the same picture
/// from a script and ends the process once the file is on disk. A capture whose
/// caller has to decide when to kill the app hangs a terminal, and that cost
/// would be paid on every check.
#[derive(Resource, Default)]
struct Screenshots {
    /// Where the scripted one goes, if one was asked for.
    path: Option<String>,
    after: f32,
    taken: bool,
    /// Serial number for the ones taken by hand.
    manual: u32,
    /// `distance,pitch°,yaw°` for the scripted one, so a shot can be framed the
    /// same way twice. Without it a screenshot check compares two different
    /// views of a world that has moved on. The ground artefacts in particular
    /// only show at a shallow enough pitch.
    view: Option<[f32; 3]>,
}

/// The window size, set from a script: `--size <w>x<h>` and `--resize <w>x<h>`.
///
/// Every aspect-ratio report this project has had is about a window somebody
/// resized, and a scripted shot always ran at Bevy's default 1280x720. The size
/// alone is not enough. An interface that latches something at load and a
/// picture that is fitted once are both correct until the window changes under
/// them, so a run also has to change size part way through to reach that fault.
///
/// `--resize` fires at [`RESIZE_FRACTION`] of `--after`, so a `--shot` frames
/// the window after it has changed shape, with time left for anything watching
/// to react.
#[derive(Resource, Default, Clone, Copy)]
struct WindowScript {
    /// `--size`: what to open at. `None` is Bevy's own default.
    open: Option<(u32, u32)>,
    /// `--resize`: what to become part way through.
    then: Option<(u32, u32)>,
    /// When that happens, in seconds — derived from `--after`, not given.
    at: f32,
    done: bool,
}

/// Where in a `--shot` run the scripted resize lands. Early enough that the
/// picture is of a settled window rather than of one mid-change.
const RESIZE_FRACTION: f32 = 0.5;

/// Apply `--resize` once, when its moment comes.
fn resize_window(
    time: Res<Time>,
    mut script: ResMut<WindowScript>,
    mut windows: Query<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>,
) {
    let Some((width, height)) = script.then else {
        return;
    };
    if script.done || time.elapsed_secs() < script.at {
        return;
    }
    script.done = true;
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    info!(
        "--resize: {}x{} -> {width}x{height}",
        window.width(),
        window.height()
    );
    window.resolution.set(width as f32, height as f32);
}

/// Holds the window to the interface's aspect, so a drag on its edge changes
/// its size but never its shape.
///
/// `lua::widgets::layout::Viewport` already fits the interface into any
/// window, which keeps it correct at any aspect, but correct there means
/// letterboxed, and a window dragged to 21:9 spends most of its area on bars.
/// Winit applies whatever size the drag asks for, and this system sets it back,
/// rounded to [`crate::lua::widgets::layout::ASPECT`].
///
/// The side kept is the side that moved. A drag on the right edge changes the
/// width and sets the height; a drag on the bottom edge does the reverse; a
/// corner drag follows whichever moved further. Deriving the size one fixed way
/// round would leave half the window's edges unresponsive to the mouse, which
/// is a worse fault than the one being fixed.
///
/// Windowed mode only, and not while the window fills its screen. A fullscreen
/// or borderless surface has the monitor's shape, and those are the cases
/// [`crate::lua::widgets::layout::Viewport`]'s bars are for, so the bars stay.
///
/// The fills-the-screen condition exists because the mode test does not cover
/// a maximised window. Winit reports a maximised window as
/// [`WindowMode::Windowed`]: maximising is a state, not a mode, and Bevy's
/// `Window` has `set_maximized` but no way to read the state back. Without the
/// condition the snap ran on a maximised window like any other resize. A
/// maximised window on a 1920x1080 monitor is 1920x1032 with the taskbar
/// showing, which is not 16:9, so on the first frame after the maximise button
/// was pressed the snap shrank it to 1835x1032 and un-maximised it. That was
/// the reported fault "fullscreen no longer works": the window went full screen
/// and came back out every time, with nothing logged.
///
/// [`fills_the_screen`] is the test. It compares against the monitor the window
/// is on rather than the primary; otherwise, on a two-monitor desk, the fix
/// would apply on one screen and not the other.
fn hold_the_aspect(
    mut windows: Query<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>,
    monitors: Query<(&bevy::window::Monitor, Option<&bevy::window::PrimaryMonitor>)>,
    mut was: Local<Option<(f32, f32)>>,
) {
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if window.mode != bevy::window::WindowMode::Windowed {
        return;
    }
    let (width, height) = (window.width(), window.height());
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    // Physical against physical. `Window::width` is logical and a monitor's
    // size is physical, so on a 125% display a direct comparison reports a
    // maximised 1920-wide window as 1536 and the test never passes. The pick
    // ray carries a note about the same logical-versus-physical mismatch.
    let physical = (
        window.resolution.physical_width() as f32,
        window.resolution.physical_height() as f32,
    );
    if let Some(monitor) = monitor_of(&window, &monitors) {
        if fills_the_screen(physical, monitor) {
            // The last accepted size is still recorded, so a drag on an edge
            // after leaving the maximised state still knows which edge moved.
            *was = Some((width, height));
            return;
        }
    }
    match snapped_to_aspect((width, height), *was) {
        Some((snapped_width, snapped_height)) => {
            window.resolution.set(snapped_width, snapped_height);
            *was = Some((snapped_width, snapped_height));
            // Every write here reconfigures the swapchain, so a snap that fires
            // repeatedly is a per-frame acquire stall. Snaps are counted and
            // logged on the first and every 128th rather than on each one,
            // because the failure this detects fires sixty times a second.
            SNAPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let n = SNAPS.load(std::sync::atomic::Ordering::Relaxed);
            if n == 1 || n % 128 == 0 {
                info!(
                    "aspect lock: snap #{n} — {width:.0}x{height:.0} -> \
                     {snapped_width:.0}x{snapped_height:.0}"
                );
            }
        }
        None => *was = Some((width, height)),
    }
}

/// How many times the aspect lock has rewritten the window this session — see
/// the note at the write.
static SNAPS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The size of the monitor this window is on, in physical pixels, or `None`
/// when the windowing backend has reported no monitors.
///
/// The window's position decides it. The primary monitor is the fallback for
/// the frame before a position exists ([`WindowPosition::Automatic`], which a
/// window is created with).
fn monitor_of(
    window: &bevy::window::Window,
    monitors: &Query<(&bevy::window::Monitor, Option<&bevy::window::PrimaryMonitor>)>,
) -> Option<(f32, f32)> {
    let size = |monitor: &bevy::window::Monitor| {
        (monitor.physical_width as f32, monitor.physical_height as f32)
    };
    if let bevy::window::WindowPosition::At(at) = window.position {
        for (monitor, _) in monitors {
            let origin = monitor.physical_position;
            let (width, height) = (monitor.physical_width as i32, monitor.physical_height as i32);
            if at.x >= origin.x
                && at.y >= origin.y
                && at.x < origin.x + width
                && at.y < origin.y + height
            {
                return Some(size(monitor));
            }
        }
    }
    monitors
        .iter()
        .find(|(_, primary)| primary.is_some())
        .or_else(|| monitors.iter().next())
        .map(|(monitor, _)| size(monitor))
}

/// `--hover <x>,<y>`, or nothing.
///
/// Always inserted, so the one reader in the pick can consult it
/// unconditionally — see [`HoverProbe::instead_of`].
#[derive(Resource, Clone, Copy)]
pub struct HoverProbe(pub Option<Vec2>);

impl HoverProbe {
    /// The planted position, or the real one.
    ///
    /// `crate::game::combat::target::hover` reads the pointer through this, and
    /// nothing else does. In a real session the option is `None` (`--hover` was
    /// not passed), so this returns the real position. It exists, rather than
    /// the probe only moving the mouse, because of focus: `set_cursor_position`
    /// is a request to the window manager, and a window without focus is
    /// silently refused. No scripted run has focus, because the terminal has
    /// it. Measured: the planted pointer landed for the two seconds after the
    /// window was created and never again, so the shot always showed a pointer
    /// resting on nothing.
    ///
    /// The probe still asks for the real pointer to be moved as well, so that
    /// when the window is focused the interface's own `Pointer`, which reads the
    /// window directly, agrees with the pick.
    pub fn instead_of(&self, real: Option<Vec2>) -> Option<Vec2> {
        self.0.or(real)
    }
}

/// Holds the real pointer at the `--hover` position, so a scripted run can
/// hover.
///
/// `Window::set_cursor_position` moves the real pointer. When the window has
/// focus, the ray is the one a person's hand would produce, and every system
/// downstream (the cursor bitmap, the highlight, the tooltip, the click) sees
/// an ordinary hover. An unfocused window's request is refused, so the pick
/// also reads the planted position through [`HoverProbe::instead_of`].
///
/// Every frame, not once. The window is not focused when a run starts under a
/// script, and a single write before the surface exists is silently dropped.
/// Rewriting the same position costs one message a frame, and it also holds
/// the pointer still against anything else that moves it.
fn hover_probe(
    probe: Res<HoverProbe>,
    mut windows: Query<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>,
) {
    let (Some(at), Ok(mut window)) = (probe.0, windows.single_mut()) else {
        return;
    };
    window.set_cursor_position(Some(at));
}

/// Logs what the `--hover` pointer landed on, once per change.
///
/// The two resources are printed together because the informative case is the
/// one where both are empty: a pointer on a unit fills the first, a pointer on
/// a door fills the second, and a pointer on something that should be one of
/// those but fills neither is a bug. Two game-object faults were read off this
/// line: one as `usable=false` on everything except a chest, the other as a
/// unit hover cleared by a signpost.
fn report_the_hover(
    hovered: Res<crate::game::combat::target::Hovered>,
    object: Res<crate::game::npc::object::HoveredObject>,
    mut last: Local<Option<(Option<u64>, Option<u64>)>>,
) {
    let now = (hovered.guid, object.guid);
    if last.replace(now) == Some(now) {
        return;
    }
    info!(
        "--hover: unit={:?} cursor={:?} | object={:?} {:?} usable={} floating={} \
         highlight={} cursor={:?} cast={:?}",
        hovered.guid,
        hovered.cursor,
        object.guid,
        object.name,
        object.usable,
        object.hover.floating,
        object.hover.highlight,
        object.cursor,
        object.cast,
    );
}

/// Alt+Enter switches the window to borderless full screen and back. It is the
/// game's own gesture; before this system the client had no way to go full
/// screen at all.
///
/// [`hold_the_aspect`]'s fills-the-screen condition makes a maximised window
/// behave; this is the other route, and both exist because they are the two
/// ways a person asks for the same thing. The gesture is an interpretation, not
/// a transcription: the toggle is a C-side handler rather than a row in
/// `Bindings.xml` (the archives' file has no entry for it). It is the gesture
/// every Windows game of the era shipped, and the one 5875's own manual names.
///
/// Borderless rather than exclusive. [`WindowMode::Fullscreen`] takes over the
/// display mode and restores it on exit, which on a modern compositor means a
/// mode switch, a black screen and a stutter, for no gain to this client.
/// Borderless gives the same picture without those, and it is what the
/// reference's "windowed (fullscreen)" option is today.
///
/// Alt+Enter is not a binding and does not collide with one. The key table is
/// keyed by the whole spelling, so a press with Alt held looks up `ALT-ENTER`
/// and finds nothing; `OPENCHAT` is on bare `ENTER` and does not open. 1.12
/// declares no binding for the window mode. This is the window manager's
/// convention rather than the game's, which is why it is the one keystroke in
/// this client still read raw.
fn toggle_fullscreen(
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<crate::lua::api::keyboard::KeyboardFocus>,
    mut windows: Query<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>,
) {
    use bevy::window::{MonitorSelection, WindowMode};
    // Not while a text field has the keyboard. Alt+Enter inside the chat line
    // is a keystroke the interface may want, and a window changing shape
    // mid-sentence is worse than a missed shortcut.
    if typing.active {
        return;
    }
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let entered = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    if !alt || !entered {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    window.mode = match window.mode {
        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        // Anything else — borderless, or an exclusive mode somebody set — comes
        // back to a window. There is no third state to cycle through.
        _ => WindowMode::Windowed,
    };
    info!("alt+enter: window mode is now {:?}", window.mode);
}

/// How much of its monitor a window has to cover before [`hold_the_aspect`]
/// stops arguing with it, on each axis independently.
///
/// The slack allows for the taskbar only. A maximised window is the monitor's
/// work area: the whole screen less what the shell reserves. Windows' default
/// taskbar is 40 physical pixels on a 1080p screen (3.7%), and the widest
/// ordinary setting is about 8%. Ten per cent covers all of those and is still
/// far from any shape a person would drag a window to on purpose: on a
/// 1920x1080 monitor it means 1728x972, which is nearly the whole screen.
const FILLS_THE_SCREEN: f32 = 0.90;

/// Whether this window is as big as its screen: maximised, or borderless full
/// screen by a route that still reports itself as windowed.
///
/// Both axes are checked. A window dragged to the full width of the monitor and
/// left short is the 21:9 letterbox the snap exists to prevent, so a width-only
/// test would disable the snap in the case it is for.
///
/// Pure, so the rule can be tested without maximising a window.
fn fills_the_screen(window: (f32, f32), monitor: (f32, f32)) -> bool {
    if monitor.0 <= 0.0 || monitor.1 <= 0.0 {
        return false;
    }
    window.0 >= monitor.0 * FILLS_THE_SCREEN && window.1 >= monitor.1 * FILLS_THE_SCREEN
}

/// The size this window should become, or `None` if it already has the right
/// shape. Pure, so the rule can be tested without a drag.
///
/// `was` is the last size accepted; it only decides which edge the mouse is on.
/// `None` (the first frame, and a stated `--size`) keeps the width.
fn snapped_to_aspect(now: (f32, f32), was: Option<(f32, f32)>) -> Option<(f32, f32)> {
    let aspect = crate::lua::widgets::layout::ASPECT as f32;
    let (width, height) = now;
    // Already on the ratio, to the pixel the height would be rounded to.
    // Compared against the derived height rather than against a ratio with a
    // tolerance, because winit deals in whole pixels, and a tolerance wide
    // enough for the rounding would also let a wrong shape through.
    let derived = (width / aspect).round();
    if (height - derived).abs() < 1.0 {
        return None;
    }
    let (moved_w, moved_h) =
        was.map_or((1.0, 0.0), |(w, h)| ((width - w).abs(), (height - h).abs()));
    Some(if moved_w >= moved_h {
        (width, derived)
    } else {
        ((height * aspect).round(), height)
    })
}

/// `<w>x<h>`, or `None` for anything else — a mistyped size opens the default
/// window rather than a 0x0 one.
fn parse_size(value: &str) -> Option<(u32, u32)> {
    let (w, h) = value.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// How long the smoothed diagnostics get to describe the scripted framing
/// before they are logged beside it.
///
/// The frame time and the pass spans are moving averages over a couple of
/// seconds of frames. The scripted view is applied 25 seconds in, so a dump
/// taken on the next frame would report the load-in view the shot replaces:
/// the PNG would show one framing and the numbers another. This is the same
/// problem as the one-frame-early framing comment in [`screenshots`], one level
/// up: the picture and the measurement have to be of the same view.
const SETTLE_SECS: f32 = 5.0;

#[allow(clippy::too_many_arguments)]
fn screenshots(
    mut commands: Commands,
    mut state: ResMut<Screenshots>,
    mut rig: ResMut<camera::CameraRig>,
    time: Res<Time>,
    mut pressed: MessageReader<crate::game::bindings::BindingPressed>,
    mut quit: MessageWriter<AppExit>,
    // The measurement half, behind the `diagnostics` feature: `DrawCalls` is
    // registered by `DrawCallPlugin` and `frame_verdict` lives in
    // `ui::debug::frame`, and neither exists without it. In that build a
    // `--shot` still takes its picture and omits the numbers beside it (see
    // `ui::debug`).
    #[cfg(feature = "diagnostics")] diagnostics: Res<DiagnosticsStore>,
    #[cfg(feature = "diagnostics")] draws: Res<draws::DrawCalls>,
    #[cfg(feature = "diagnostics")] materials: Res<models::MaterialPool>,
    // The unit names, which used to be one draw per name; on a crowd that was
    // a seventh of the frame. See `render::labels` and the note in the
    // rendering facts.
    #[cfg(feature = "diagnostics")] labels: Res<crate::render::labels::Labels>,
    #[cfg(feature = "diagnostics")] meshes: Query<&ViewVisibility, With<Mesh3d>>,
    // The emitters, counted like the meshes, for the reason given beside the
    // HUD's copy of this count: drawn emitters are this renderer's only
    // per-frame cost paid per object rather than per mesh, and a scripted A/B
    // that cannot see them cannot subtract them.
    // (An emitter carries a `Mesh3d` too, so it is in the mesh counts above as
    // well; this is the subset whose mesh is rewritten every frame.)
    #[cfg(feature = "diagnostics")] emitters: Query<&ViewVisibility, With<particles::Emitter>>,
    // The merged emitters: an additive emitter is drawn through its material's
    // field and its own entity is never visible, so without this the count
    // above is short by exactly the merged ones.
    #[cfg(feature = "diagnostics")] fields: Res<particles::ParticleFields>,
    // The CPU span ledger, the other half of the CPU/GPU verdict printed above.
    // `verdict` says when a frame is CPU-bound; only this says where the time
    // went. See [`ui::debug::spans`].
    #[cfg(feature = "diagnostics")] mut spans: ResMut<ui::debug::spans::Spans>,
    // The skinned meshes, the population Bevy spends its own per-frame time on,
    // which no other count in this client reports. Every visible skinned mesh's
    // joints are read and written into the skin buffer every frame, so the
    // number of joints on screen drives `extract_skins`. See
    // `render::models::loader::RawDraw::bones`.
    #[cfg(feature = "diagnostics")]
    skins: Query<(&bevy::mesh::skinning::SkinnedMesh, &ViewVisibility)>,
    // What `--capture` caught, so the packet capture is readable from a log
    // and not only from a tab a scripted run cannot scroll. See
    // [`ui::debug::net`].
    #[cfg(feature = "diagnostics")] status: Res<world::session::WorldStatus>,
) {
    use bevy::render::view::screenshot::{save_to_disk, Screenshot};

    // The game's own `SCREENSHOT` binding, `PRINTSCREEN` in the shipped
    // defaults. This used to read `just_pressed(KeyCode::F12)`, but F12 is
    // `TOGGLEBACKPACK`, so the key opened the bag as well as saving a PNG.
    if pressed
        .read()
        .any(|crate::game::bindings::BindingPressed(b)| {
            matches!(b, crate::game::bindings::Binding::Screenshot)
        })
    {
        state.manual += 1;
        let path = format!("screenshot-{}.png", state.manual);
        info!("screenshot -> {path}");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }

    let Some(path) = state.path.clone() else {
        return;
    };
    // The PNG is written by a task rather than by this system, so the file
    // existing on disk is the only reliable signal that the shot was saved, and
    // it is what the caller waits for. Quitting when the observer fires would
    // race the write.
    if state.taken {
        if std::path::Path::new(&path).exists() {
            quit.write(AppExit::Success);
        }
        return;
    }
    if time.elapsed_secs() < state.after {
        return;
    }
    // Frame the view one frame early. This system and the camera's `place` are
    // both in `Update` and unordered, so a rig written now might not reach the
    // `Transform` until the next frame, and the screenshot would show the view
    // this was meant to replace. That would defeat a check whose purpose is to
    // compare two framings.
    if let Some([distance, pitch, yaw]) = state.view.take() {
        rig.distance = distance;
        rig.pitch = pitch.to_radians();
        rig.yaw = yaw.to_radians();
        // Then hold this framing until the moving averages describe it rather
        // than the load-in view it replaced. See [`SETTLE_SECS`].
        state.after = time.elapsed_secs() + SETTLE_SECS;
        // Restart the measurement here, so it covers the whole settle window
        // rather than the last half second; see
        // [`ui::debug::spans::Spans::restart`]. Everything before this point is
        // the load, and a load hitch in the sample can move the median by
        // several milliseconds.
        #[cfg(feature = "diagnostics")]
        spans.restart();
        return;
    }
    state.taken = true;
    info!("screenshot -> {path}");
    // A file left from a previous run would satisfy the exists-check above as
    // soon as `taken` is set, quitting the app before the new PNG is written,
    // and the numbers would be logged beside a stale picture. A scripted
    // shot's picture and numbers must describe the same frame.
    let _ = std::fs::remove_file(&path);

    #[cfg(feature = "diagnostics")]
    {
        // The HUD's verdict and counts, logged to stdout, because this is the
        // one code path a script reaches: the spans live in a collapsed egui
        // section a PNG does not show, and a measurement that cannot be captured
        // in a log has to be taken again by hand each time. The numbers come
        // from the same source as the HUD's (`debug::frame::frame_verdict`).
        let verdict = ui::debug::frame::frame_verdict(&diagnostics);
        let (mut total_meshes, mut visible_meshes) = (0usize, 0usize);
        for visible in &meshes {
            total_meshes += 1;
            visible_meshes += usize::from(visible.get());
        }
        let fps = if verdict.frame_ms > 0.0 { 1000.0 / verdict.frame_ms } else { 0.0 };
        info!(
            "shot: {fps:.0} fps — frame {:.1} ms, gpu passes {:.1} ms",
            verdict.frame_ms, verdict.gpu_ms
        );
        info!(
            "shot: {} draw calls — {} opaque, {} masked, {} blended  ({})",
            draws.total(),
            draws.opaque(),
            draws.alpha_mask(),
            draws.transparent(),
            draws.mode(),
        );
        let (mut total_emitters, mut drawn_emitters) = (0usize, 0usize);
        for visible in &emitters {
            total_emitters += 1;
            drawn_emitters += usize::from(visible.get());
        }
        info!(
            "shot: {visible_meshes} of {total_meshes} meshes drawn, {} materials",
            materials.distinct()
        );
        info!(
            "shot: {} names up in {} draw(s) — see `render::labels`",
            labels.count(),
            labels.draws()
        );
        info!(
            "shot: {} of {total_emitters} emitters drawn, {} merged into {} draws",
            drawn_emitters + fields.merged,
            fields.merged,
            fields.drawn
        );
        for (pass, ms) in &verdict.passes {
            info!("shot: {ms:6.2} ms  {pass}");
        }
        let (mut skinned, mut skinned_drawn, mut joints, mut joints_drawn) = (0, 0, 0, 0);
        for (skin, visible) in &skins {
            skinned += 1;
            joints += skin.joints.len();
            if visible.get() {
                skinned_drawn += 1;
                joints_drawn += skin.joints.len();
            }
        }
        info!(
            "shot: {skinned_drawn} of {skinned} skinned meshes drawn, \
             {joints_drawn} joints extracted a frame (of {joints})"
        );
        let (frames, median, p95) = spans.distribution();
        info!(
            "shot: over {frames} frames — median {median:.2} ms ({:.0} fps), p95 {p95:.2} ms",
            if median > 0.0 { 1000.0 / median } else { 0.0 },
        );
        for (name, ms, calls) in spans.measured() {
            info!("shot: {ms:6.3} ms  {name}  ({calls:.1} runs/frame)");
        }
        // The longest frames, each with its own breakdown. The list above is a
        // mean, and a cost paid once (a tile arriving, a building becoming
        // assets) is averaged down until it looks negligible. See
        // [`ui::debug::spans::WorstFrame`].
        if !spans.worst.is_empty() {
            info!("shot: the longest frames since the view was framed, and what each spent:");
        }
        for frame in &spans.worst {
            let own: Vec<String> = frame
                .top(6)
                .into_iter()
                .filter(|(_, ms)| *ms >= 0.05)
                .map(|(name, ms)| format!("{name} {ms:.1}"))
                .collect();
            let phases: Vec<String> = frame
                .phases()
                .into_iter()
                .filter(|(_, ms)| *ms >= 0.5)
                .map(|(name, ms)| format!("{} {ms:.1}", name.trim_start_matches("phase ")))
                .collect();
            info!(
                "shot: {:7.1}s  {:6.1} ms   systems: {}   phases: {}",
                frame.at_secs,
                frame.ms,
                if own.is_empty() { "-".to_string() } else { own.join(", ") },
                phases.join(", "),
            );
        }
        // What `--capture` caught, logged because a scripted run cannot scroll
        // the tab. The tab is where a packet body is read byte by byte; this
        // shows whether the ring filled, which is the part a log can carry.
        // Nothing is logged when the capture was never armed.
        let capture = &status.capture;
        if capture.seen > 0 {
            let span = capture.packets.last().map_or(0, |p| p.at_ms);
            info!(
                "shot: capture — {} packet(s) kept of {} seen over {:.1}s{}",
                capture.packets.len(),
                capture.seen,
                f64::from(span) / 1000.0,
                match capture.armed {
                    true => "",
                    false => ", disarmed",
                }
            );
            // The newest few, which is what a person reads first on the tab.
            for packet in capture.packets.iter().rev().take(8) {
                info!(
                    "shot: capture   {:>7.3}s  {}  {}  {} B",
                    f64::from(packet.at_ms) / 1000.0,
                    if packet.inbound { "recv" } else { "sent" },
                    packet.name,
                    packet.length,
                );
            }
        }
    }

    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}

/// `--relogin <seconds>`, and whether it has fired.
///
/// A resource rather than a `Local` inside the system because the flag has to
/// reach it from the command line, and `None` is the ordinary run — see
/// [`Args::relogin`].
#[derive(Resource)]
struct Relogin {
    /// How long in the world before leaving it, and how long at the character
    /// screen before coming back. `None` for an ordinary run.
    timings: Option<(f32, f32)>,
    stage: ReloginStage,
    /// When the logout was asked for, which is what the gap is measured from.
    left_at: Option<f32>,
}

/// How far through [`relogin_probe`]'s one round trip this run is.
#[derive(Clone, Copy)]
enum ReloginStage {
    /// In the world, waiting out the first of the two timings.
    Playing,
    /// The logout has been asked for; waiting for the second.
    Camping,
    /// The second login has been asked for, and this probe is finished.
    Done,
}

impl Relogin {
    fn new(timings: Option<(f32, f32)>) -> Self {
        Relogin { timings, stage: ReloginStage::Playing, left_at: None }
    }
}

/// Leaves the world and comes back, once, for a scripted run.
///
/// It exists to check the second login, where the world is built from state
/// that a first login found empty: `LoadedTiles`, `GlobalBuilding`, the caches,
/// `WorldStatus`. Nothing else in this file reaches it; see [`Args::relogin`]
/// for why `--script` cannot.
///
/// It leaves by the game's own route rather than by calling
/// [`session::Session::log_out_to_characters`] directly. The direct call
/// reclaims the socket without telling the server, so the character is still in
/// the world when `CMSG_CHAR_ENUM` arrives and the login that follows is not
/// the one a player makes. `Binding::Logout` means `CMSG_LOGOUT_REQUEST`, the
/// server's own delay, and `SMSG_LOGOUT_COMPLETE`.
///
/// Coming back re-arms [`game::session::autologin::AutoLogin`]: it already
/// waits for a character list and clicks the named row, which is the code path
/// the first login took.
fn relogin_probe(
    time: Res<Time>,
    mut probe: ResMut<Relogin>,
    mut auto: ResMut<game::session::autologin::AutoLogin>,
    session: Res<session::Session>,
    status: Res<session::WorldStatus>,
    mut pressed: MessageWriter<game::bindings::BindingPressed>,
    mut in_world_since: Local<Option<f32>>,
) {
    let Some((after, gap)) = probe.timings else {
        return;
    };
    let now = time.elapsed_secs();
    match probe.stage {
        ReloginStage::Done => {}
        ReloginStage::Playing => {
            if session.active.is_none() || !status.in_world {
                return;
            }
            let since = *in_world_since.get_or_insert(now);
            if now - since < after {
                return;
            }
            info!("--relogin: {:.0}s in the world — Logout()", now - since);
            pressed.write(game::bindings::BindingPressed(game::bindings::Binding::Logout));
            probe.stage = ReloginStage::Camping;
            probe.left_at = Some(now);
        }
        ReloginStage::Camping => {
            // The gap starts only once the world is gone. Otherwise a server
            // that takes its twenty seconds would spend the whole wait in the
            // world, and the second login would follow the logout immediately,
            // which is the case the gap exists to avoid.
            if session.active.is_some() {
                probe.left_at = Some(now);
                return;
            }
            if now - probe.left_at.unwrap_or(now) < gap {
                return;
            }
            info!("--relogin: {gap:.0}s at the character screen — entering the world again");
            // The pick waits for a character list and clicks the named row,
            // which is the same path the first login took — and the socket is
            // already held, so this arms that half alone.
            auto.arm_at_the_character_screen();
            probe.stage = ReloginStage::Done;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Args {
        Args::parse(line.split_whitespace().map(str::to_string))
    }

    /// The character is the argument that is not a flag, wherever it sits, and a
    /// scripted shot gets a default `--after` rather than firing on frame zero —
    /// which would photograph an empty world and look like a rendering fault.
    #[test]
    fn the_command_line_says_who_to_be_and_what_to_capture() {
        let plain = parse("Alden");
        assert_eq!(plain.character.as_deref(), Some("Alden"));
        assert!(plain.shot.path.is_none(), "no shot unless one is asked for");

        let scripted = parse("--shot look.png --view 3,10,0 Alden");
        assert_eq!(scripted.character.as_deref(), Some("Alden"));
        assert_eq!(scripted.shot.path.as_deref(), Some("look.png"));
        assert_eq!(scripted.shot.view, Some([3.0, 10.0, 0.0]));
        assert_eq!(scripted.shot.after, 25.0, "the world has to arrive first");

        assert_eq!(parse("x --after 8").shot.after, 8.0);
        // A flag whose value is missing must not eat the character or panic.
        assert_eq!(parse("Alden --view").shot.view, None);
    }

    /// `--night` and `--hour`, which exist for the same reason as `--without`:
    /// a view that cannot be set up from a script is not compared twice.
    /// `--night` always takes its value, since a bare one would swallow the
    /// character, and `--hour` is the only way a scripted shot can show the
    /// night.
    #[test]
    fn the_night_and_the_hour_can_be_set_from_the_line() {
        let plain = parse("Bram");
        assert!(plain.night.is_none() && plain.hour.is_none());

        let dark = parse("Bram --night off --hour 00:00");
        assert_eq!(dark.character.as_deref(), Some("Bram"));
        assert!(!dark.night.expect("--night was given").enabled);
        assert_eq!(dark.hour, Some(0));

        let half = parse("--night 0.5 --hour 21:30 Bram");
        assert_eq!(half.character.as_deref(), Some("Bram"));
        assert_eq!(half.night.expect("--night was given").strength, 0.5);
        assert_eq!(half.hour, Some((21 * 60 + 30) * 2));
        // A bare hour is midnight-relative like the panel's own slider.
        assert_eq!(parse_hour("18"), Some(18 * 120));
        // …and the weather beside it, which eats its value the same way.
        let snowing = parse("Bram --weather snow,0.8 --hour 12:00");
        assert_eq!(snowing.character.as_deref(), Some("Bram"));
        let forced = snowing.weather.expect("--weather was given").0;
        assert_eq!(forced.kind, vale_protocol::play::weather::WeatherKind::Snow);
        assert_eq!(forced.grade, 0.8);
        assert!(parse("Bram --weather hail").weather.is_none());
        for bad in ["", "24:00", "12:60", "noon", "12:", ":30", "-1"] {
            assert_eq!(parse_hour(bad), None, "{bad:?}");
        }
    }

    /// `--size` and `--resize` are the only way to vary the input every
    /// aspect-ratio report is about; see [`WindowScript`]. A mistyped size opens
    /// the default window rather than a 0x0 one, like every other flag here.
    #[test]
    fn the_window_can_be_given_a_size_and_a_change_of_size() {
        let plain = parse("Alden");
        assert_eq!((plain.window.open, plain.window.then), (None, None));

        let sized = parse("--size 1920x1080 --resize 1280X720 --after 20 G");
        assert_eq!(sized.window.open, Some((1920, 1080)));
        // Capital `X` too: this is a developer flag and both are typed.
        assert_eq!(sized.window.then, Some((1280, 720)));
        // …and the resize lands inside the run rather than beside it, so a
        // `--shot` frames a window that has already changed shape.
        assert_eq!(sized.window.at, 20.0 * RESIZE_FRACTION);
        assert!(sized.window.at < sized.shot.after);

        for bad in ["", "1920", "1920x", "x720", "0x720", "1920x0", "axb", "-8x6"] {
            assert_eq!(parse_size(bad), None, "{bad:?}");
        }
    }

    /// `--tune` names the settings that are on; everything unnamed is off,
    /// and `none` is the all-off spelling. No flag at all keeps the defaults —
    /// which are not all-off, so `--tune` and its absence must stay distinct.
    #[test]
    fn tune_names_the_settings_that_are_on() {
        assert!(parse("Alden").tuning.is_none(), "no flag keeps the defaults");

        let none = parse("G --tune none").tuning.unwrap();
        assert!(!none.msaa && !none.sun_shadows);
        assert!(none.vsync, "vsync is off only by name, never by omission");

        let some = parse("G --tune msaa,shadows").tuning.unwrap();
        assert!(some.msaa && some.sun_shadows);
        assert!(some.vsync);

        assert!(!parse("G --tune novsync").tuning.unwrap().vsync);
    }

    /// A drag can change the window's size but not its shape, and the side
    /// kept is the side the mouse is on.
    ///
    /// This pins the report "I can still resize the window however". The
    /// earlier half of the fix made it easy to miss: the interface was locked
    /// to 16:9 and fitted into any window, so a dragged window was letterboxed
    /// rather than distorted, and every count in the client reported success
    /// while the window could still change shape.
    #[test]
    fn a_resize_may_change_the_size_and_not_the_shape() {
        // A window already on the ratio is left alone at any size.
        assert_eq!(snapped_to_aspect((1920.0, 1080.0), None), None);
        assert_eq!(snapped_to_aspect((1280.0, 720.0), Some((1920.0, 1080.0))), None);

        // Dragging the right edge out: the width is what moved, so the height
        // follows it.
        assert_eq!(
            snapped_to_aspect((2560.0, 1080.0), Some((1920.0, 1080.0))),
            Some((2560.0, 1440.0))
        );
        // …and the bottom edge: the height moved, so the width follows.
        assert_eq!(
            snapped_to_aspect((1920.0, 600.0), Some((1920.0, 1080.0))),
            Some((1067.0, 600.0))
        );
        // A corner takes whichever moved further — here the height, by 480
        // against the width's 320.
        assert_eq!(
            snapped_to_aspect((1600.0, 600.0), Some((1920.0, 1080.0))),
            Some((1067.0, 600.0))
        );

        // It settles in one step, which keeps it from fighting the window
        // manager: feeding a correction back in asks for no further change.
        let corrected = snapped_to_aspect((2560.0, 1080.0), Some((1920.0, 1080.0))).unwrap();
        assert_eq!(snapped_to_aspect(corrected, Some(corrected)), None);

        // With nothing to compare against — the first frame, and a stated
        // `--size` — the width is kept.
        assert_eq!(snapped_to_aspect((1920.0, 600.0), None), Some((1920.0, 1080.0)));
    }

    /// A maximised window is left alone; this is the fullscreen fix.
    ///
    /// The first assertion is the one that matters. 1920x1032 is what Windows
    /// gives a maximised window on a 1080p screen with the default taskbar. It
    /// is not 16:9, and the snap would answer `Some((1835, 1032))` for it,
    /// which un-maximises the window on the frame after the button was pressed.
    /// Winit reports it as `WindowMode::Windowed` and Bevy cannot read the
    /// maximised state back, so the size against the monitor is the only test
    /// available.
    #[test]
    fn a_window_that_fills_its_screen_is_not_snapped_back() {
        let monitor = (1920.0, 1080.0);
        assert!(
            fills_the_screen((1920.0, 1032.0), monitor),
            "maximised with a horizontal taskbar"
        );
        assert!(
            fills_the_screen((1848.0, 1080.0), monitor),
            "…and with a vertical one"
        );
        assert!(fills_the_screen(monitor, monitor), "borderless, exactly");

        // …and the shapes the snap still has to correct, which a width-only
        // test would wrongly exempt.
        assert!(
            !fills_the_screen((1920.0, 600.0), monitor),
            "full width and short is the letterbox the snap exists for"
        );
        assert!(!fills_the_screen((1280.0, 720.0), monitor), "an ordinary window");
        assert!(
            !fills_the_screen((1920.0, 1032.0), (3840.0, 2160.0)),
            "the same window on a bigger screen is not maximised on it"
        );
        // A monitor the backend has not measured yet cannot make anything
        // fullscreen — the snap keeps working rather than switching itself off.
        assert!(!fills_the_screen((1920.0, 1080.0), (0.0, 0.0)));
    }
}
