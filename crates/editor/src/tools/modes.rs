//! The number row: each digit selects one of the chosen tool's modes.

use bevy::prelude::*;
use super::{Tool, ground, placements};

/// The number row selects the chosen tool's modes.
///
/// Every tool here has a small set of states a person switches between often
/// (raise and lower, select and place), and each switch required moving the
/// pointer to a segmented control. The digits are the one block of keys that
/// nothing in this crate or in the game's own bindings used. Their meaning
/// depends on the tool: `1` is Raise on the terrain brush and Select on the
/// placements, because those are the first entry of each panel's first row.
///
/// `Tab` also works for the placements, because Select and Place is a pair
/// people toggle between rather than choose from.
///
/// It is one system rather than a branch inside each tool so that the whole
/// mapping is visible in one place: if a tool's keys did not follow the
/// layout of its panel, they would have to be learnt separately.
#[allow(clippy::too_many_arguments)]
pub(crate) fn modes(
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<vale_client::lua::api::keyboard::KeyboardFocus>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut terrain: ResMut<ground::terrain::Terrain>,
    mut shading: ResMut<ground::shading::Shading>,
    mut textures: ResMut<ground::textures::Textures>,
    mut placing: ResMut<placements::place::Placing>,
) {
    if !state.editing() || wants.wants_keyboard_input() || typing.active {
        return;
    }
    // `Ctrl` and `Alt` belong to other gestures: `Ctrl` is the
    // undo/redo/save/playtest block and `Alt` turns a placement. `Shift` is
    // free and selects the second row: a brush panel has more choices than
    // there are digits, and a shifted digit is the common convention for that.
    if keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::AltLeft)
        || keys.pressed(KeyCode::AltRight)
    {
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    // Ten, and `0` is the tenth: the row as it is laid out on the keyboard
    // rather than as it is numbered.
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

    match *tool {
        // No modes. A grade is two clicks and a button, so the number row has
        // no modes to pick between. The same is true of the creature, object,
        // item, quest and flight path tools: the number row picks a brush
        // mode, and a row of a table has none.
        Tool::Grade
        | Tool::Road
        | Tool::Creatures
        | Tool::GameObjects
        | Tool::Items
        | Tool::ItemSets
        | Tool::Quests
        | Tool::CreatureData
        | Tool::GameObjectData
        | Tool::CreatureDisplays
        | Tool::ObjectDisplays
        | Tool::ItemDisplays
        | Tool::Assets
        | Tool::Tables
        | Tool::Zones
        | Tool::Triggers
        | Tool::Graveyards
        | Tool::Flightpaths => {}
        Tool::Terrain => {
            // Unshifted digits cover the two long rows; shifted digits cover
            // the short one. 1..5 are the five modes, 6..0 the five falloffs,
            // and `Shift`+1..3 the three shapes: the panel's own rows in the
            // panel's own order, so the panel shows which key does what.
            let Some(at) = digit else { return };
            if shift {
                if let Some(&(_, shape)) = ground::terrain::SHAPES.get(at) {
                    terrain.brush.shape = shape;
                }
            } else if let Some(&(_, mode)) = ground::terrain::MODES.get(at) {
                terrain.brush.mode = mode;
            } else if let Some(&(_, falloff)) = ground::terrain::FALLOFFS.get(at - ground::terrain::MODES.len()) {
                terrain.brush.falloff = falloff;
            }
        }
        Tool::Shading => {
            // Five modes and five falloffs, laid out exactly as the height
            // brush's are and for the same reason: the two are the same three
            // controls over the same vertices, so a key learnt on one is the key
            // on the other.
            let Some(at) = digit else { return };
            if shift {
                if let Some(&(_, shape)) = ground::terrain::SHAPES.get(at) {
                    shading.brush.shape = shape;
                }
            } else if let Some(&(_, mode)) = ground::shading::MODES.get(at) {
                shading.brush.mode = mode;
            } else if let Some(&(_, falloff)) = ground::terrain::FALLOFFS.get(at - ground::shading::MODES.len()) {
                shading.brush.falloff = falloff;
            }
        }
        Tool::Textures => {
            // The texture brush has no modes, so its unshifted digits are the
            // falloffs — the panel's first row of choices — and `Shift` is the
            // shapes, exactly as above.
            let Some(at) = digit else { return };
            match shift {
                true => {
                    if let Some(&(_, shape)) = ground::textures::SHAPES.get(at) {
                        textures.brush.shape = shape;
                    }
                }
                false => {
                    if let Some(&(_, falloff)) = ground::textures::FALLOFFS.get(at) {
                        textures.brush.falloff = falloff;
                    }
                }
            }
        }
        Tool::Doodads | Tool::Wmos => {
            // Two choices and no second row, so a shifted digit means nothing
            // here rather than meaning the unshifted one.
            if shift {
                return;
            }
            let wanted = match digit {
                Some(at) => placements::place::MODES.get(at).map(|&(_, mode)| mode),
                // `Tab` is the toggle rather than a third key, because these two
                // are a pair rather than a list.
                None if keys.just_pressed(KeyCode::Tab) => Some(match placing.mode {
                    placements::place::Mode::Select => placements::place::Mode::Place,
                    placements::place::Mode::Place => placements::place::Mode::Select,
                }),
                None => None,
            };
            if let Some(mode) = wanted {
                placing.mode = mode;
                // Switching to Select puts down whatever was on the cursor,
                // which is what the panel's own switch does — see
                // [`placements::place::aim`], which would do it a frame later anyway.
                if mode == placements::place::Mode::Select {
                    placing.disarm();
                }
            }
        }
        // These tools have no modes. In Spells the digits go to whichever
        // text field has focus: a workspace has text fields, and a number row
        // that meant something here would conflict with every one of them.
        Tool::Areas
        | Tool::Chunks
        | Tool::Water
        | Tool::Select
        | Tool::Measure
        | Tool::Holes
        | Tool::Spells
        | Tool::Lights
        | Tool::Sweep => {}
    }
}
