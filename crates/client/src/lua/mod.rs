//! **The interface's own language, running the interface's own code.**
//!
//! ```text
//! host.rs      the interpreter: which libraries 1.12 opens, and the one way in
//! dialect.rs   …and the one thing 5.0 said that 5.1 will not compile
//! xml.rs       …and what is loaded into it: `Interface\FrameXML\`, all 175
//!              files of it, and `Interface\GlueXML\` before there is a world
//! manifest.rs  …and what all of that comes to, which `vale-api` is held to
//! scoped.rs    …and the permanent name in front of every per-scope read, so
//!              that an addon may keep one and call it on a later frame
//! audit.rs     …and what broke: the six probes, run headless with no window
//!
//! widgets/     the object model: what a widget is, where it lands, how it draws
//! api/         the C functions it may call — the reads, the writes, the stubs,
//!              the events, the clock, the pointer and the keyboard
//! panels/      …and the subject traits behind the reads, one file per panel
//! ```
//!
//! Everything a 1.12 interface does is Lua. `Bindings.xml` says what a key runs
//! and the body is Lua; `ActionButton.lua` decides whether a button is usable and
//! that is Lua; a `.toc` in `Interface\AddOns\` is a list of Lua files. So the
//! whole of the remaining interface work sits on one question — is there an
//! interpreter — and this directory is the answer to it.
//!
//! ## The shape, both ways round
//!
//! ```text
//! a key goes down                     game::bindings::dispatch
//!   -> a binding NAME                 Keybindings, the player's half
//!   -> that <Binding>'s Lua body      vale_assets::interface::bindings, the game's half
//!   -> the body calls ToggleSheath()  lua::verbs, a registered Rust closure
//!   -> a Verb on the queue            …because a closure cannot hold the World
//!   -> BindingPressed                 drained the same frame, back in game/
//!
//! the client learns something         game::events, nine of the game's own names
//!   -> lua::events drains it          after the whole of game/
//!   -> every frame that registered     lua::frames, in registration order
//!   -> `this`/`event`/`arg1` set       1.12's convention: the handler takes none
//!   -> the handler asks questions      lua::api, answered from the live world
//! ```
//!
//! ## Writes record; reads answer
//!
//! A registered function cannot take `&mut World` — it outlives the system call
//! that would lend it one — so a **verb records** rather than acts, and the system
//! that ran the chunk drains the record immediately afterwards. That is also how
//! the real thing works at the boundary this models: a Lua call reaches C, and C
//! touches the client's state.
//!
//! A **read cannot be deferred**, because it has to produce a value in the middle
//! of an expression. Those are registered into an `mlua` *scope* for the length of
//! one call, over a `&dyn Answers` that borrows the world — see [`api`], which is
//! where the whole of that argument is. `UnitHealth("target")` is a live query,
//! not a snapshot, and the trait it goes through is what makes the read side
//! testable with no `World` at all.
//!
//! ## Where this is not the real thing yet
//!
//! Stated, because each of these looks like it works:
//!
//! * **the verb set is a fraction of the 116** the binding bodies call
//!   (`vale bindings` counts both). A key bound to a name whose verb is
//!   missing raises a Lua error, which is reported once per name and then
//!   suppressed — not silently ignored, because a key that does nothing with no
//!   explanation is the single most confusing failure an interface can have.
//! * **…and a name the *interface* defines is not a verb at all.** Registering
//!   one is a bug rather than a shortfall: the loader runs after the host is
//!   built, so `function ActionButtonDown(id)` in `ActionButton.lua` overwrites
//!   the closure and the client's copy is dead code from the first login. That
//!   is what took casting out for two rounds; see [`verbs`] and [`button`].
//! * **it draws now, and what it draws through has one blend mode.** [`draw`]
//!   says what is visible and in what order, and [`crate::ui::framexml`] paints
//!   it through egui — which cannot do the game's `ADD`, so an additive texture
//!   is approximated. See that module, where the deviation is written out. The
//!   `ui/frames.rs` stand-in is still up beside it.
//! * **anchors resolve now** — [`layout`] solves the graph and `GetLeft` and its
//!   four neighbours answer off it, as do `GetWidth` and `GetHeight`. What is
//!   still not modelled is 1.12's **UI scale**: the rectangles are in screen
//!   pixels where the game's are in UI units, which agree exactly at a scale of
//!   1.0 and differ by a constant factor otherwise.
//! * **`OnUpdate` fires and the mouse works** — [`update`] and [`mouse`],
//!   drag included. What is still not modelled about the pointer is named in
//!   that module: the wheel, which wants the clipping this client does not do.
//! * **`runOnUp` is honoured, and nothing else about the key is.** 1.12 also has
//!   auto-repeat and a "sticky" mouse-look pair, neither of which is modelled.
//! * **there is one Lua state and it is not sandboxed per addon.** `setfenv`
//!   per addon is how the real client isolates them, and there are no addons.
//! * **`io` and `os` are deliberately absent** — see [`host`], where the library
//!   list is the interesting part.

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
/// binding dispatch calls into the host and a host that does not exist yet
/// means a frame of dead keys at login.
pub struct LuaPlugins;

impl Plugin for LuaPlugins {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            host::HostPlugin,
            api::events::EventsPlugin,
            api::mouse::MousePlugin,
            api::keyboard::KeyboardPlugin,
            api::update::UpdatePlugin,
            // …and the one plate the interface never asks for itself: the world
            // mouseover's, which 5875 fills from C. See [`tooltip`].
            widgets::tooltip::TooltipPlugin,
            // …and the two script kinds only a `<Model>` has, plus the cache of
            // the files they hold — see [`model`].
            widgets::model::ModelPlugin,
        ));
    }
}
