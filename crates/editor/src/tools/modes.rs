//! The number row: each digit selects one of the chosen tool's modes.
//!
//! Every tool here has a small set of states a person switches between often
//! (raise and lower, select and place), and each switch required moving the
//! pointer to a segmented control. The digits are the one block of keys that
//! nothing in this crate or in the game's own bindings used. Their meaning
//! depends on the tool: `1` is Raise on the terrain brush and Select on the
//! placements, because those are the first entry of each panel's first row.
//!
//! This file says when a press reaches a tool ([`NumberRow`]). Each tool with
//! modes maps it onto its own panel in a `modes` system its plugin registers:
//! `ground::terrain`, `ground::shading`, `ground::textures`, and
//! `placements::place` for the doodad and WMO tools. A tool with no modes
//! registers none. In a workspace the digits go to whichever text field has
//! focus: a workspace has text fields, and a number row that meant something
//! there would conflict with every one of them.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use super::Tool;

/// One frame's press on the number row, for the tool it reached.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Press {
    /// Which key of the row went down this frame, as its place on the row:
    /// `1` is 0 and `0` is 9, the row as it is laid out on the keyboard
    /// rather than as it is numbered.
    pub(crate) digit: Option<usize>,
    /// Whether `Shift` is held, which selects a panel's second row.
    pub(crate) shift: bool,
    /// Whether `Tab` went down this frame. The placements use it to toggle
    /// between Select and Place.
    pub(crate) tab: bool,
}

/// What decides whether the number row reaches a tool this frame.
#[derive(SystemParam)]
pub(crate) struct NumberRow<'w> {
    tool: Res<'w, Tool>,
    state: Res<'w, crate::playtest::Playtest>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    typing: Res<'w, vale_client::lua::api::keyboard::KeyboardFocus>,
    wants: Res<'w, bevy_egui::input::EguiWantsInput>,
}

impl NumberRow<'_> {
    /// This frame's press, when the chosen tool is one of `tools`, or `None`.
    ///
    /// `None` during a playtest and while a text field has the keyboard, in
    /// egui or in the game's own frames. `None` too while `Ctrl` or `Alt` is
    /// held: those belong to other gestures, `Ctrl` to the
    /// undo/redo/save/playtest block and `Alt` to turning a placement.
    /// `Shift` is free and selects the second row: a brush panel has more
    /// choices than there are digits, and a shifted digit is the common
    /// convention for that.
    pub(crate) fn read(&self, tools: &[Tool]) -> Option<Press> {
        if !tools.contains(&*self.tool) {
            return None;
        }
        if !self.state.editing() || self.wants.wants_keyboard_input() || self.typing.active {
            return None;
        }
        let keys = &self.keys;
        if keys.pressed(KeyCode::ControlLeft)
            || keys.pressed(KeyCode::ControlRight)
            || keys.pressed(KeyCode::AltLeft)
            || keys.pressed(KeyCode::AltRight)
        {
            return None;
        }
        let digit = [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
            KeyCode::Digit6,
            KeyCode::Digit7,
            KeyCode::Digit8,
            KeyCode::Digit9,
            KeyCode::Digit0,
        ]
        .iter()
        .position(|&key| keys.just_pressed(key));
        Some(Press {
            digit,
            shift: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
            tab: keys.just_pressed(KeyCode::Tab),
        })
    }
}
