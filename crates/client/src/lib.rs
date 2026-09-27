//! The game client: the Bevy renderer, the interface runtime and the game
//! state they share.
//!
//! It runs in the same process as `vale-protocol` and `vale-assets`. The world
//! snapshot, the vertex arrays and the pose maths therefore do not cross a
//! process boundary, and `M2Skeleton::pose` exists once rather than in two
//! copies that have to be checked against each other.
//!
//! The parsing crates remain the authority on file formats. This crate turns
//! what they produce into meshes, materials and entities, and [`render::axes`]
//! is the one place where their coordinate frame becomes Bevy's.
//!
//! ```text
//! render/      file -> picture; nothing in it needs the network
//! world/       what the server's answers mean, keyed by GUID or by position
//! game/        what the player can do: target, attack, cast
//! lua/         the interface's own language, running the interface's own code
//! ui/          drawn over the world: the FrameXML interface and the
//!              diagnostics; see ui/mod.rs
//! sound/       what the world sounds like: music, ambience, footsteps, combat
//!
//! app.rs       builds the app, in the three steps another host needs separately
//!              from this binary's flags
//! assets.rs    the archive chain and the tables read from it, as resources
//! args.rs      the command line: the character, and the flags of a scripted run
//! window.rs    the window: its size, the `--resize` script, the aspect lock and
//!              Alt+Enter
//! scripted.rs  the instruments a scripted run uses: `--shot`, `--hover`,
//!              `--relogin`
//! doctree.rs   the check that every directory's mod.rs still describes what is
//!              in it; tests only
//! ```
//!
//! Each directory's `mod.rs` says what is in it and what belongs there. A
//! subject with no directory of its own gets appended to the nearest file,
//! which is how `world::entities` once reached about 3,500 lines.

pub mod app;
mod args;
pub mod assets;
// The test that each directory's own header still describes what is in it.
// All six headers once went stale together. See `doctree.rs`.
mod doctree;
pub mod game;
pub mod lua;
pub mod render;
mod scripted;
pub mod sound;
pub mod ui;
mod window;
pub mod world;

// The subject directories above are the crate's structure. Each pass registers
// itself through its directory's plugin group, so adding one edits
// `render/mod.rs`, `world/mod.rs` or `ui/mod.rs` and never the crate root. See
// [`render::RenderPlugins`].
//
// `axes` and `session` are imported here because other modules use them as
// `crate::axes` and `crate::session`.
use render::axes;
use world::session;
use bevy::prelude::*;

use args::Args;
// Re-exported because the pick, the debug window and other hosts use it as
// `crate::HoverProbe` and `vale_client::HoverProbe`.
pub use scripted::HoverProbe;
use scripted::Relogin;
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
        // `--size`, or bevy's own default — see [`window::WindowScript`].
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
    // [`args::Args::panel`]. These insertions are behind the `diagnostics` feature,
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
                scripted::screenshots,
                // Before the pick, so the frame that requests the logout does
                // not also click a character. The pick reads
                // `Session::selection`, which is `None` while there is a world,
                // so the order only decides whether the second login starts on
                // this frame or the next. It is stated because an ordering that
                // matters is stated rather than inherited.
                scripted::relogin_probe.before(game::session::autologin::pick_the_character),
                // Ordered explicitly. Both systems take the window mutably, so
                // Bevy sequences them anyway, but only through that shared
                // access, which holds only while both keep it. The order
                // matters: a scripted `--resize` requests a size, and the snap
                // decides the shape it ends up at.
                (window::resize_window, window::hold_the_aspect).chain(),
                // `--hover`, the one input a script cannot otherwise supply;
                // see [`HoverProbe`]. Both systems are behind `run_if`, so a
                // run without `--hover` does not run them.
                (scripted::hover_probe, scripted::report_the_hover)
                    .chain()
                    .run_if(|probe: Res<HoverProbe>| probe.0.is_some()),
                // Alt+Enter: the other route to full screen besides
                // maximising, and the one the player asks for deliberately.
                // See [`window::toggle_fullscreen`].
                window::toggle_fullscreen,
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
    // no other way to set it. See [`args::Args::hour`].
    if let Some(half_minutes) = args.hour {
        app.insert_resource(render::sky::WorldClock {
            half_minutes,
            from_server: false,
            override_half_minutes: Some(half_minutes),
        });
    }
    // `--weather`: the weather, for the same reason. See [`args::Args::weather`].
    if let Some(weather) = args.weather {
        app.insert_resource(weather);
    }
    app.run();
}
