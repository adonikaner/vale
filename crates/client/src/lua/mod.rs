//! The interface's own language, running the interface's own code.
//!
//! ```text
//! host.rs      the interpreter: which libraries 1.12 opens, and the one way in
//! dialect.rs   the one Lua 5.0 construct Lua 5.1 does not compile
//! xml.rs       what is loaded into the interpreter: `Interface\FrameXML\`, all
//!              175 files, and `Interface\GlueXML\` before there is a world
//! manifest.rs  the interface API this client implements, derived from the
//!              interpreter; `vale-api` is checked against it (tests only)
//! scoped.rs    the permanent name in front of every per-scope read, so an
//!              addon may keep a function and call it on a later frame
//! audit.rs     the six `--audit` probes, run with no window
//!
//! widgets/     the object model: what a widget is, where it is placed, how it
//!              is drawn
//! api/         the C functions the interface may call: the reads, the writes,
//!              the stubs, the events, the clock, the pointer and the keyboard
//! panels/      the traits behind the reads, one file per panel
//! ```
//!
//! Everything a 1.12 interface does is Lua. `Bindings.xml` states what a key
//! runs, and the body is Lua. `ActionButton.lua` decides whether a button is
//! usable, in Lua. A `.toc` in `Interface\AddOns\` is a list of Lua files. This
//! directory is the interpreter all of that runs in.
//!
//! ## The flow in both directions
//!
//! ```text
//! a key goes down                     game::bindings::dispatch
//!   -> a binding NAME                 Keybindings, the player's half
//!   -> that <Binding>'s Lua body      vale_assets::interface::bindings, the game's half
//!   -> the body calls ToggleSheath()  lua::verbs, a registered Rust closure
//!   -> a Verb on the queue            because a closure cannot hold the World
//!   -> BindingPressed                 drained the same frame, back in game/
//!
//! the client learns something         game::events, named as the game names them
//!   -> lua::events drains it          after the whole of game/
//!   -> every frame that registered     lua::frames, in registration order
//!   -> `this`/`event`/`arg1` set       1.12's convention: the handler takes none
//!   -> the handler asks questions      lua::api, answered from the live world
//! ```
//!
//! ## Writes are queued; reads are answered during the call
//!
//! A registered function cannot take `&mut World`, because it outlives the
//! system call that would lend it one. So a write (a verb) is recorded on a
//! queue, and the system that ran the chunk applies the queue immediately
//! afterwards. The 1.12.1 client has the same boundary: a Lua call reaches C,
//! and C changes the client's state.
//!
//! A read cannot be deferred, because it has to return a value in the middle of
//! an expression. Reads are registered into an `mlua` scope for the length of
//! one call, over a `&dyn Answers` that borrows the world; see [`api`].
//! `UnitHealth("target")` is a live query, not a snapshot, and the trait it
//! goes through makes the read side testable with no `World`.
//!
//! ## Where this differs from the 1.12.1 client
//!
//! Each of these can look as if it works:
//!
//! * The verb set covers part of the 116 names the binding bodies call
//!   (`vale bindings` counts both). A key bound to a name with no verb raises a
//!   Lua error, which is reported once per name and then suppressed. It is not
//!   ignored silently, because a key that does nothing with no message is the
//!   hardest interface fault to diagnose.
//! * A name the interface defines must not be registered as a verb. The loader
//!   runs after the interpreter is built, so `function ActionButtonDown(id)` in
//!   `ActionButton.lua` replaces a registered closure of that name, and the
//!   client's version is never called. That disabled casting for two rounds;
//!   see [`api::verbs`] and [`widgets::button`].
//! * [`widgets::draw`] decides what is visible and in what order, and `crate::ui`
//!   paints it. The painters support fewer blend modes than the game's five;
//!   see `crate::ui::framexml` for where an additive texture is approximated.
//! * [`widgets::layout`] solves the anchor graph, and `GetLeft`, its four
//!   neighbours, `GetWidth` and `GetHeight` answer from it, scaled by the
//!   `uiScale` in force (`crate::ui::scale`).
//! * `OnUpdate` fires and the mouse works, including drags and the wheel; see
//!   [`api::update`] and [`api::mouse`].
//! * `runOnUp` is honoured. Key auto-repeat and the "sticky" mouse-look pair of
//!   1.12 are not modelled.
//! * There is one Lua state, and addons are not isolated from each other. The
//!   1.12.1 client isolates each addon with `setfenv`.
//! * The `io` and `os` libraries are deliberately not opened; see [`host`] for
//!   the library list.

pub mod audit;
pub mod dialect;
pub mod host;
pub mod manifest;
pub mod scoped;
pub mod xml;

pub mod api;
pub mod panels;
pub mod widgets;

use bevy::prelude::*;

/// The interpreter, its verb queue and its event dispatch, as one plugin.
///
/// Registered before [`crate::game::GamePlugins`] reads anything, because the
/// binding dispatch calls into the interpreter, and an interpreter that does
/// not exist yet means a frame at login in which keys do nothing.
pub struct LuaPlugins;

impl Plugin for LuaPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            host::HostPlugin,
            api::events::EventsPlugin,
            api::mouse::MousePlugin,
            api::keyboard::KeyboardPlugin,
            api::update::UpdatePlugin,
            // The one tooltip the interface never fills itself: the one for the
            // unit or object under the mouse in the world, which the 1.12.1
            // client fills from C. See [`widgets::tooltip`].
            widgets::tooltip::TooltipPlugin,
            // The two script kinds only a `<Model>` has, and the cache of the
            // files they load; see [`widgets::model`].
            widgets::model::ModelPlugin,
        ));
    }
}
