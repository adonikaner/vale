//! Everything drawn *over* the world rather than in it.
//!
//! ```text
//! framexml.rs  **the game's own interface**, drawn from the widget tree —
//!              *and its glue*, which is the same painter over the same tree
//! mesh/      …and the same draw list as batched 2d meshes — **the default
//!            painter** since the parity soak; `VALE_UI_PAINTER=egui`
//!            brings the old one back for one more round. It draws the
//!            combat text and the loading screen too, so under it egui is
//!            diagnostics only
//! loading.rs   …and the one thing drawn *over* that: the loading screen, which
//!              is three quads and no widgets because 1.12's is too
//! worldtext.rs …and the one thing drawn *under* it: the numbers that float off
//!              a unit you hit, which is one of the two parts of 1.12's
//!              interface that ship no XML at all and is therefore not in the
//!              tree above. **The other is the unit *name*, and it is not here
//!              at all** — it is `render::labels`, because a name is drawn *in*
//!              the world and takes the depth buffer with everything else there
//! debug/     **the diagnostic surface**, behind `F4` and behind the
//!            `diagnostics` feature: a header and six tabs — what the frame
//!            cost, what is in the world, how it is drawn and what is drawn
//!            over it, what is in it at all, the wire, and the interface
//! report.rs  …and the channel every other pass says its own line down, with
//!            the tab it belongs on
//! cursor.rs  the game's own mouse pointer, hotspot and all
//! scale.rs   …and how big all of it is drawn: the `uiScale` in force, read by
//!            the painter, the pointer and the loader so that the three of them
//!            cannot disagree about it
//! boxes.rs   …and the one thing drawn over a *failure*: what each grey
//!            diagnostic box was supposed to be, and why it is not
//! ```
//!
//! **`debug/` and `report` are behind the `diagnostics` feature** and so is
//! every `report` system in the crate — an instrument that cannot be removed is
//! a cost a shipped build pays for ever. See [`debug`], which says what that
//! means and how it is checked.
//!
//! **`framexml` is the real one and the other four are around it.** It draws
//! what [`crate::lua`] loads — the game's art, the game's fonts, the game's own
//! anchors, its bars, its borders and its message lines.
//!
//! **`frames.rs` is gone** (this round). It drew a player frame, a target frame,
//! a cast bar, twelve action buttons and the error text in egui, and it was
//! always going to be deleted rather than ported: the real `PlayerFrame`,
//! `TargetFrame`, `CastingBarFrame`, `ActionButton` and `UIErrorsFrame` are
//! FrameXML, and they are what draws them now. What it was standing in for is
//! the same four things it read — the selection, the resolved bar, the cooldown
//! clocks and the messages — all of which live in [`crate::interface`] and are not
//! throwaway at all. See that directory's own note on the split.
//!
//! **`chat.rs` is gone too** (this round), and it is the same story one round
//! later. It was an egui strip with a text field in it, and by the end it was
//! *only* the text field — the log half went when `ChatFrame1` started
//! receiving the `CHAT_MSG_*` events. What replaced the last of it is
//! [`crate::lua::widgets::editbox`] and [`crate::lua::api::keyboard`]: `ChatFrameEditBox` is a
//! real widget in the real tree, it takes the keyboard, and every line typed
//! into it goes out through `ChatEdit_SendText` — the archive's own function —
//! rather than through a slash-command table written here. Its "am I typing"
//! flag is [`crate::lua::api::keyboard::KeyboardFocus`], which the same four systems
//! read for the same reason.
//!
//! **`glue.rs` is gone too** (this round), and it is the third and last of the
//! egui stand-ins. It drew an account box, a password box and a list of
//! characters, and the note here used to say it could not be replaced the way
//! the others were because `Interface\Glues\` is "M2 scenes rather than a widget
//! tree at all". **That was wrong about which directory to look in.**
//! `Interface\Glues\` is the *art*; `Interface\GlueXML\` is the tree — 20 `.toc`
//! entries, `AccountLogin.xml` and `CharacterSelect.xml` among them — and it is
//! in the archives in full, in exactly the form `FrameXML` is. So the login and
//! character screens are the game's own files running in the game's own
//! language, drawn by [`framexml`] with no code here that knows what a login is.
//!
//! What was not throwaway underneath it survives unchanged, which is what the
//! old note got right: a held `Handshake` and a `Screen` derived from what
//! exists rather than stored beside it. The client half of the two screens is
//! [`crate::glue::glue`], beside every other piece of state the interface reads.
//!
//! `debug/` itself stays under any interface: it is the *diagnostic* surface,
//! which the game has no equivalent of and which every measurement in this
//! project is read off. It says so in its own first line.

#[cfg(feature = "diagnostics")]
pub mod boxes;
pub mod cursor;
#[cfg(feature = "diagnostics")]
pub mod debug;
pub mod framexml;
pub mod loading;
pub mod mesh;
pub mod scale;
pub mod worldtext;
#[cfg(feature = "diagnostics")]
pub mod report;

use bevy::prelude::*;

/// Every pass in this directory, as one plugin — see
/// [`crate::render::RenderPlugins`] for why the registration lives here rather
/// than in `lib.rs`.
pub struct UiPlugins;

impl bevy::app::Plugin for UiPlugins {
    fn build(&self, app: &mut bevy::app::App) {
        // **egui itself is not diagnostic and is registered here.** It used to
        // be added by the HUD's plugin, which meant the game's own interface
        // and the glue screens — both of which paint through it — depended on
        // the debug window being compiled in. That was invisible until the
        // `diagnostics` feature existed and then it was a link error.
        app.add_plugins((
            bevy_egui::EguiPlugin::default(),
            // **Before the four passes that read it**, though not by an
            // ordering: it runs in `PreUpdate` so that everything drawn in one
            // frame is drawn at one scale. See [`scale`].
            scale::InterfaceScalePlugin,
            // First, so that the game's own interface is the thing the
            // diagnostics are drawn *over* rather than under.
            framexml::FrameXmlPlugin,
            // …and the numbers that float off a unit you hit, in a layer of
            // their own beside them.
            worldtext::WorldTextPlugin,
            cursor::CursorPlugin,
            // …and the one pass that is over *all* of it, diagnostics
            // included — see [`loading`], which says why that is the point
            // rather than an oversight.
            loading::LoadingScreenPlugin,
        ));
        // The mesh painter — the default; conditional so an
        // `VALE_UI_PAINTER=egui` run carries no second painter in its
        // schedules. The egui emissions' own skips check the same
        // [`mesh::active`].
        if mesh::active() {
            app.add_plugins(mesh::UiMeshPlugin);
        }
        // **Before both input polls**, so a key or a wheel that egui took is
        // refused in the frame it happened rather than the next one.
        // `EguiWantsInput` is written by `bevy_egui` in `PreUpdate`, so there
        // is a fresh answer to read in `Update`.
        app.add_systems(
            Update,
            claim_input
                .before(crate::lua::api::mouse::poll)
                .before(crate::lua::api::keyboard::poll),
        );
        #[cfg(feature = "diagnostics")]
        app.init_resource::<report::HudReport>().add_plugins((
            debug::DebugPanelPlugin,
            // **Diagnostic, and it reads a diagnostic switch.** It is behind
            // the feature rather than beside `cursor` because it holds
            // `render::overlay::DebugOverlay`, which is gated — so an
            // ungated registration is not merely a cost in a shipped build,
            // it does not compile at all. `--no-default-features` is the
            // check, and it is the reason this line moved.
            boxes::ErrorBoxPlugin,
        ));
    }
}

/// **Tell the world when egui has the keyboard or the pointer**, so that typing
/// into a panel does not also walk the character and scrolling one does not also
/// zoom the camera.
///
/// ## bevy_egui does not consume Bevy's input
///
/// egui takes a keystroke for its own text field and `ButtonInput<KeyCode>` sees
/// it too; egui takes a wheel event over its own window and
/// `AccumulatedMouseScroll` sees it too. So every one of this client's input
/// readers is, by default, deaf to the fact that a panel is drawn over the top
/// of the game. The two symptoms are the ones the debug window was reported for:
/// typing "we ran away" into its console holds W, E, A and D and sends the
/// character sprinting off, and dragging a slider pulls the camera in and out
/// under it.
///
/// ## Two flags, and neither of them is a new rule
///
/// The client already has a complete answer to "the interface has the input,
/// stand down": [`crate::lua::api::keyboard::KeyboardFocus`] and
/// [`crate::lua::api::mouse::MouseFocus`], read by seven systems between them —
/// the binding table, targeting, the action bar, the cursor, `send_input`,
/// `arm_look` and `orbit`. Both of those are rewritten every frame off the
/// *widget tree*, which knows nothing about an egui panel, so this writes two
/// separate flags that the two polls OR in:
/// [`crate::lua::api::keyboard::ExternalKeyboard`] and
/// [`crate::lua::api::mouse::ExternalPointer`]. No reader changes.
///
/// ## `wants_any_pointer_input`, not `is_pointer_over_area`
///
/// The wider of the two: it is true while the pointer is over the panel, while a
/// widget is being dragged (which is how a slider grabbed inside a panel and
/// pulled outside it stays the panel's), and while a popup is open. The narrow
/// one would hand the camera back mid-drag the moment the pointer crossed the
/// panel's edge, which is the same "asked per frame" mistake
/// `crate::world::camera::arm_look` documents for the game's own frames.
///
/// ## It is registered here rather than with the debug window
///
/// It was the debug window's, behind the `diagnostics` feature, because that
/// window was the only egui drawn over a live world. It is not any more: a host
/// that builds this app and draws panels of its own — the world editor's shell
/// during a playtest — makes the same claim on the same grounds, and under
/// `--no-default-features` there was nothing to make it. `EguiPlugin` is
/// registered unconditionally above, so the resource this reads always exists.
fn claim_input(
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut keyboard: ResMut<crate::lua::api::keyboard::ExternalKeyboard>,
    mut pointer: ResMut<crate::lua::api::mouse::ExternalPointer>,
) {
    // Written only on a change: both are plain flags with no `is_changed`
    // reader today, and leaving it that way is one line rather than a rule
    // somebody has to remember later.
    let (k, p) = (wants.wants_any_keyboard_input(), wants.wants_any_pointer_input());
    if keyboard.0 != k {
        keyboard.0 = k;
    }
    if pointer.0 != p {
        pointer.0 = p;
    }
}
