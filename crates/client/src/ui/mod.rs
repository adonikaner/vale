//! Everything drawn over the world, as opposed to in it.
//!
//! ```text
//! framexml.rs  the game's own interface, drawn from the widget tree, and its
//!              glue screens, which are the same painter over the same tree
//! mesh/        the same draw list as batched 2d meshes. This is the default
//!              painter; `VALE_UI_PAINTER=egui` selects the older egui
//!              painter. It also draws the combat text and the loading
//!              screen, so with it egui draws diagnostics only
//! loading.rs   the loading screen, drawn over the interface: three quads and
//!              no widgets, as in 1.12
//! worldtext.rs the numbers that float off a unit that is hit, drawn under
//!              the interface. 1.12 ships no XML for them, so they are not in
//!              the widget tree. The unit name, the other part of 1.12's
//!              interface with no XML, is not here: it is `render::labels`,
//!              because a name is drawn in the world and takes the depth
//!              buffer
//! debug/       the diagnostic surface, behind `F4` and behind the
//!              `diagnostics` feature: a header and seven tabs. They cover
//!              what the frame cost, what is in the world, how it is drawn
//!              and what is drawn over it, what is in it at all, the wire,
//!              the interface, and the client's own log with a Lua command
//!              line, which the backquote key opens
//! report.rs    the channel every other pass writes its own line to, with the
//!              tab the line belongs on
//! cursor.rs    the game's own mouse pointer, with its hotspot
//! scale.rs     the `uiScale` in force, read by the painter, the pointer and
//!              the loader so that the three agree on it
//! boxes.rs     what each grey diagnostic box was supposed to be, and why it
//!              is not, drawn over the box
//! ```
//!
//! `debug/`, `report` and every `report` system in the crate are behind the
//! `diagnostics` feature, so a shipped build does not pay for an instrument.
//! See [`debug`] for what the feature removes and how that is checked.
//!
//! `framexml` draws what [`crate::lua`] loads: the game's art, the game's
//! fonts, the game's own anchors, its bars, its borders and its message
//! lines. The other files are around it.
//!
//! ## The three egui stand-ins that were deleted
//!
//! * `frames.rs` drew a player frame, a target frame, a cast bar, twelve
//!   action buttons and the error text in egui. The real `PlayerFrame`,
//!   `TargetFrame`, `CastingBarFrame`, `ActionButton` and `UIErrorsFrame` are
//!   FrameXML and draw them now. The four things it read (the selection, the
//!   resolved bar, the cooldown clocks and the messages) are in
//!   [`crate::interface`] and remain; see that directory's note on the split.
//! * `chat.rs` was an egui strip with a text field. Its log half went when
//!   `ChatFrame1` started receiving the `CHAT_MSG_*` events. Its text field
//!   was replaced by [`crate::lua::widgets::editbox`] and
//!   [`crate::lua::api::keyboard`]: `ChatFrameEditBox` is a widget in the
//!   tree, it takes the keyboard, and every line typed into it goes out
//!   through `ChatEdit_SendText`, the archive's own function, not through a
//!   slash-command table written here. Whether someone is typing is
//!   [`crate::lua::api::keyboard::KeyboardFocus`], which four systems read.
//! * `glue.rs` drew an account box, a password box and a list of characters.
//!   `Interface\Glues\` holds the art for those screens and
//!   `Interface\GlueXML\` holds the widget tree: 20 `.toc` entries,
//!   `AccountLogin.xml` and `CharacterSelect.xml` among them, in the archives
//!   in the same form as `FrameXML`. The login and character screens are
//!   therefore the game's own files, drawn by [`framexml`], and no code here
//!   knows what a login is. The state under the old screen is unchanged: a
//!   held `Handshake`, and a `Screen` derived from what exists and not stored
//!   beside it. The client half of the two screens is [`crate::glue::glue`].
//!
//! `debug/` stays under any interface. It is the diagnostic surface, which
//! the game has no equivalent of, and every measurement in this project is
//! read from it.

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

/// Every pass in this directory, as one plugin. See
/// [`crate::render::RenderPlugins`] for why the registration is here and not
/// in `lib.rs`.
pub struct UiPlugins;

impl bevy::app::Plugin for UiPlugins {
    fn build(&self, app: &mut bevy::app::App) {
        // egui itself is not diagnostic and is registered here. The HUD's
        // plugin once added it, so the game's own interface and the glue
        // screens, which can paint through it, depended on the debug window
        // being compiled in. Without the `diagnostics` feature that was a
        // link error.
        app.add_plugins((
            bevy_egui::EguiPlugin::default(),
            // Runs in `PreUpdate`, so everything drawn in one frame is drawn
            // at one scale. The four passes that read it need no ordering
            // against it. See [`scale`].
            scale::InterfaceScalePlugin,
            // First, so that the diagnostics are drawn over the game's own
            // interface and not under it.
            framexml::FrameXmlPlugin,
            // The numbers that float off a unit that is hit, in a layer of
            // their own.
            worldtext::WorldTextPlugin,
            cursor::CursorPlugin,
            // The one pass drawn over everything else, diagnostics included;
            // [`loading`] gives the reason.
            loading::LoadingScreenPlugin,
        ));
        // The mesh painter, which is the default. Conditional, so a
        // `VALE_UI_PAINTER=egui` run carries no second painter in its
        // schedules. The egui emissions' own skips check the same
        // [`mesh::active`].
        if mesh::active() {
            app.add_plugins(mesh::UiMeshPlugin);
        }
        // Before both input polls, so a key or a wheel event that egui took
        // is refused in the frame it happened and not the next one.
        // `bevy_egui` writes `EguiWantsInput` in `PreUpdate`, so `Update`
        // reads a current value.
        app.add_systems(
            Update,
            claim_input
                .before(crate::lua::api::mouse::poll)
                .before(crate::lua::api::keyboard::poll),
        );
        #[cfg(feature = "diagnostics")]
        app.init_resource::<report::HudReport>().add_plugins((
            debug::DebugPanelPlugin,
            // Diagnostic, and it reads a diagnostic switch. It is behind the
            // feature and not beside `cursor` because it holds
            // `render::overlay::DebugOverlay`, which is gated, so an ungated
            // registration does not compile. `--no-default-features` is the
            // check.
            boxes::ErrorBoxPlugin,
        ));
    }
}

/// Tell the world when egui has the keyboard or the pointer, so that typing
/// into a panel does not also walk the character and scrolling one does not
/// also zoom the camera.
///
/// ## bevy_egui does not consume Bevy's input
///
/// egui takes a keystroke for its own text field and `ButtonInput<KeyCode>`
/// still sees it; egui takes a wheel event over its own window and
/// `AccumulatedMouseScroll` still sees it. Every input reader in this client
/// therefore acts on input an egui panel has already used unless it is told
/// not to. Two cases were reported for the debug window: typing "we ran away"
/// into its console held W, E, A and D and moved the character, and dragging
/// a slider zoomed the camera.
///
/// ## The two flags reuse an existing rule
///
/// The client already has the rule "the interface has the input, do not
/// act": [`crate::lua::api::keyboard::KeyboardFocus`] and
/// [`crate::lua::api::mouse::MouseFocus`], read by seven systems between them:
/// the binding table, targeting, the action bar, the cursor, `send_input`,
/// `arm_look` and `orbit`. Both are rewritten every frame from the widget
/// tree, which knows nothing about an egui panel. This system writes two
/// separate flags that the two polls OR in:
/// [`crate::lua::api::keyboard::ExternalKeyboard`] and
/// [`crate::lua::api::mouse::ExternalPointer`]. No reader changes.
///
/// ## `wants_any_pointer_input`, not `is_pointer_over_area`
///
/// `wants_any_pointer_input` is the wider of the two. It is true while the
/// pointer is over the panel, while a widget is being dragged (so a slider
/// grabbed inside a panel and pulled outside it stays the panel's), and while
/// a popup is open. The narrow one would return the camera mid-drag when the
/// pointer crossed the panel's edge. `crate::world::camera::arm_look`
/// documents the same per-frame mistake for the game's own frames.
///
/// ## It is registered here, not with the debug window
///
/// It was the debug window's, behind the `diagnostics` feature, when that
/// window was the only egui drawn over a live world. A host that builds this
/// app and draws panels of its own, such as the world editor's shell during a
/// playtest, makes the same claim, and under `--no-default-features` nothing
/// made it. `EguiPlugin` is registered unconditionally above, so the resource
/// this reads always exists.
fn claim_input(
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut keyboard: ResMut<crate::lua::api::keyboard::ExternalKeyboard>,
    mut pointer: ResMut<crate::lua::api::mouse::ExternalPointer>,
) {
    // Written only on a change. Both are plain flags with no `is_changed`
    // reader today; writing conditionally keeps that safe if one is added.
    let (k, p) = (wants.wants_any_keyboard_input(), wants.wants_any_pointer_input());
    if keyboard.0 != k {
        keyboard.0 = k;
    }
    if pointer.0 != p {
        pointer.0 = p;
    }
}
