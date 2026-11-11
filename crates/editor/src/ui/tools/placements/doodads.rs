//! The doodad tool's panel. The state is [`crate::tools::placements::doodads`].
//!
//! In Place and Cluster mode the whole panel is the picker's,
//! [`super::place::placing`]. In Select mode:
//!
//! ```text
//! [ Select | Place | Cluster ]   the mode switch
//! Keys                           Select mode's controls, on hover
//! Handles                        Move, Turn or Off
//! Selection                      the group and its buttons, the Replace Model
//!                                list, the locked doodads, and the primary's
//!                                model, id and tile
//! Position, Rotation, Scale      the primary's numbers
//! Duplicate                      for one doodad
//! ```

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::placements::doodads::Selection;
use crate::tools::placements::gizmo::Gizmo;
use crate::tools::placements::place::{Kind, Placing};
use crate::ui::tools::placements::group::{group, locked_placements, Group};
use crate::tools::placements::swap::Swap;
use crate::ui::tools::placements::place::{handles, keys, number_row, placing, replacing};
use crate::ui::widgets::theme::{self, property};

/// Select mode's mouse and keyboard controls, for the Keys button.
const KEYS: [(&str, &str); 18] = [
    ("Left button", "Select a doodad"),
    ("Shift + left button", "Add or remove one"),
    ("Drag on empty ground", "Select a rectangle"),
    ("Drag", "Move"),
    ("Ctrl + drag", "Drop onto the ground"),
    ("Arrows", "Nudge"),
    ("Page Up, Page Down", "Raise, lower"),
    (", and .", "Turn about Z"),
    ("Shift + any of those keys", "Ten times as far"),
    ("Alt + mouse", "Turn"),
    ("Ctrl + Alt + mouse", "Snap the turn"),
    ("Ctrl + wheel", "Scale"),
    ("Delete", "Remove"),
    ("Ctrl + D", "Duplicate; a group is copied two yards north"),
    ("Ctrl + C", "Copy"),
    ("Ctrl + V", "Paste at the pointer"),
    ("1, 2, 3", "Select, Place, Cluster"),
    ("Tab", "Switch between Select and placing"),
];

/// The doodad tool's panel: the placements of M2 models.
#[allow(clippy::too_many_arguments)]
pub(in crate::ui::tools) fn doodad(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    selection: &mut Selection,
    held: &mut crate::tools::placements::doodads::Held,
    gizmo: &mut Gizmo,
    placer: &mut Placing,
    swap: &mut Swap,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
    now: f64,
) {
    if placing(ui, placer, Kind::Doodad, portraits, favourites) {
        return;
    }
    keys(ui, &KEYS);
    handles(ui, &mut gizmo.handles);

    theme::heading_about(
        ui,
        "Selection",
        "Click a doodad in the world to select it, or drag a rectangle on \
         empty ground. Selection tests the model's triangles rather than \
         its bounding box, and only visible placements can be selected.",
    );
    let chosen: Vec<(u32, &str)> =
        selection.members().map(|at| (at.unique_id, at.path.as_str())).collect();
    match group(ui, &chosen, "doodads") {
        Group::Nothing => {}
        Group::Primary(id) => selection.promote(id),
        Group::Drop(id) => selection.also.retain(|at| at.unique_id != id),
        Group::Only => {
            let primary = selection.at.take();
            selection.only(primary);
        }
        Group::Clear => selection.only(None),
        Group::AllOfModel => {
            if let Some(path) = selection.at.as_ref().map(|at| at.path.clone()) {
                for at in crate::tools::placements::doodads::all_of(session, &path) {
                    if !selection.holds(at.unique_id) {
                        selection.also.push(at);
                    }
                }
                session.status = format!("{} selected", selection.count());
            }
        }
        Group::Lock => {
            let ids: Vec<u32> = selection.members().map(|at| at.unique_id).collect();
            let n = session.set_placements_locked(Kind::Doodad, ids, true);
            selection.only(None);
            session.status = format!("locked {n} doodads");
        }
        Group::Replace => match swap.open {
            Some(Kind::Doodad) => swap.close(),
            _ => swap.show(Kind::Doodad),
        },
    }
    if selection.count() > 0 {
        replacing(ui, swap, placer, Kind::Doodad, selection.count(), portraits, favourites);
    }
    if selection.at.is_none() {
        theme::note(ui, "No doodad selected");
    }
    locked_placements(ui, session, Kind::Doodad);
    let grouped = selection.count() > 1;
    let Selection { at, align, .. } = &mut *selection;
    let Some(at) = at.as_mut() else {
        return;
    };

    // The file name rather than the whole path: the path is about a hundred
    // characters of directory, and only the last part identifies the model.
    // The whole path is on the hover.
    let leaf = at
        .path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(&at.path)
        .to_string();
    property(ui, "Model", "", |ui| {
        theme::value(ui, leaf).on_hover_text(at.path.clone());
    });
    property(ui, "ID", "", |ui| {
        theme::value(ui, at.unique_id.to_string());
    });
    property(ui, "Tile", "", |ui| {
        theme::value(ui, format!("{}, {}", at.tile.0, at.tile.1));
    });

    let was = at.record;
    // World coordinates, not the file's. `MDDF` measures from the map corner
    // in its own axis order, which no other tool reports. The status line,
    // `.gps` in the game and every other check in this project use world
    // coordinates. `vale_assets::world::adt` does the conversion both ways.
    theme::heading_about(ui, "Position", "World coordinates: X north, Y west, Z up.");
    let mut world = vale_assets::world::adt::placement_to_world(at.record.position);
    for (name, axis) in [("X", 0), ("Y", 1), ("Z", 2)] {
        number_row(ui, name, "", egui::DragValue::new(&mut world[axis]).speed(0.1).fixed_decimals(2));
    }
    at.record.position = vale_assets::world::adt::placement_from_world(world);

    // The angles are converted too. The file's three angles are in a different
    // order and with different signs from the world's, so showing them raw
    // would label as "x" a field that turns the model about z.
    // `placement_euler_to_world` is the single definition of the mapping; see
    // the module comment.
    theme::heading_about(ui, "Rotation", "Right-handed, about the world's own axes.");
    let mut turns = vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
    for (name, axis) in [("About X", 0), ("About Y", 1), ("About Z", 2)] {
        number_row(
            ui,
            name,
            "",
            egui::DragValue::new(&mut turns[axis]).speed(1.0).fixed_decimals(1).suffix("\u{b0}"),
        );
    }
    at.record.rotation = vale_assets::world::adt::placement_euler_from_world(turns);
    // Slope alignment: the two leans are rewritten from the ground's normal
    // under the origin and the turn is kept; see `doodads::lean_onto_ground`.
    // A button applies it to every selected placement, each to the ground
    // under its own origin, and a switch applies it to every later ctrl-drag.
    let mut each: Option<Lean> = None;
    ui.columns(2, |columns| {
        let button = theme::full_button(&columns[0], "Align to Slope");
        if columns[0]
            .add(button)
            .on_hover_text(
                "Set the rotation about x and y so the model stands perpendicular \
                 to the slope under its origin. The rotation about z is kept. In a \
                 group, each member is aligned to the slope under its own origin.",
            )
            .clicked()
        {
            each = Some(Lean::Ground);
            if !crate::tools::placements::doodads::lean_onto_ground(session, at) {
                session.status = "the doodad's origin is not over open ground".to_string();
            }
        }
        let button = theme::full_button(&columns[1], "Stand Upright");
        if columns[1]
            .add(button)
            .on_hover_text("Set the rotation about x and y to zero, keeping the rotation about z. In a group, applies to every member.")
            .clicked()
        {
            each = Some(Lean::Upright);
            crate::tools::placements::doodads::stand_upright(at);
        }
    });
    ui.checkbox(align, "Ctrl + drag also aligns to slope")
        .on_hover_text(
            "A Ctrl + drag that drops the doodad onto the ground also aligns \
             it to the slope there.",
        );

    theme::heading(ui, "Scale");
    // The file stores unit scale as 1024. The panel shows a multiplier and
    // writes the file's number.
    let mut scale = f32::from(at.record.scale) / 1024.0;
    if number_row(
        ui,
        "Scale",
        "",
        egui::DragValue::new(&mut scale)
            .speed(0.01)
            .range(crate::tools::placements::doodads::SCALE_RANGE)
            .fixed_decimals(3)
            .suffix("x"),
    )
    .changed()
    {
        // Clamped only to what the field can hold; see `doodads::SCALE`,
        // which is `MDDF`'s own `u16`.
        at.record.scale = (scale * 1024.0).round().clamp(1.0, f32::from(u16::MAX)) as u16;
    }

    // Over a group, Ctrl + D copies the whole group in place and the button
    // is not drawn; the Keys button says so.
    if !grouped {
        ui.add_space(6.0);
        if theme::primary(ui, "Duplicate")
            .on_hover_text(
                "Ctrl+D. Puts a copy of this placement on the cursor at the same turn \
                 and size. Click to place it.",
            )
            .clicked()
        {
            let turns = vale_assets::world::adt::placement_euler_to_world(was.rotation);
            let scale = f32::from(was.scale) / 1024.0;
            placer.arm(Kind::Doodad, at.path.clone(), turns[2], scale);
        }
    }

    // A change made in the fields above becomes one history entry, recorded
    // here rather than in the tool: the tool owns the pointer and this panel
    // owns the numbers, and the two meet at [`Selection::edited`].
    //
    // One entry per drag of a field, not one per frame. A `DragValue` held for
    // a second produces sixty changes, and without `begin_gesture` each would
    // need its own undo. `vale_edit::undo::History::begin_gesture` continues the
    // last entry while the change is to the same thing.
    //
    // Over a group the members go on the same entry: a field carries them by
    // the primary's change, as the keys and the gizmo do, and the two slope
    // buttons lean or straighten each member on its own. The subject is the
    // group's, the one `doodads::nudge` uses.
    let edited = selection.edited(&was).map(|(_, record)| record);
    if edited.is_none() && !(grouped && each.is_some()) {
        return;
    }
    let record = edited.unwrap_or(was);
    let Some(at) = selection.at.as_ref() else {
        return;
    };
    let (tile, unique_id) = (at.tile, at.unique_id);
    let (label, kind) = match each {
        Some(_) => ("Lean doodad", "rotation"),
        None => crate::tools::placements::doodads::what_changed(&was, &record),
    };
    let (label, subject) = match grouped {
        false => (label.to_string(), format!("doodad {unique_id} {kind}")),
        true => (
            format!("{label}s ({})", selection.count()),
            format!("doodad group {unique_id} {kind}"),
        ),
    };
    session.history.begin_gesture(label, subject, now);
    // Written through the tool's own writer, which reads the change's `before`
    // from the tile rather than taking this panel's `was`. This panel holds a
    // cached copy of the record, which is a frame behind the file whenever an
    // undo lands; see `vale_edit::ops::Edit::move_doodad` for the invariant.
    crate::tools::placements::doodads::write_record(session, at);
    session.publish(tile);
    if grouped {
        match each {
            Some(lean) => {
                for member in selection.also.iter_mut() {
                    match lean {
                        Lean::Ground => {
                            crate::tools::placements::doodads::lean_onto_ground(session, member);
                        }
                        Lean::Upright => crate::tools::placements::doodads::stand_upright(member),
                    }
                    crate::tools::placements::doodads::write_record(session, member);
                }
            }
            None => crate::tools::placements::doodads::carry_members(session, selection, &was),
        }
        held.members_moved();
    }
    session.history.end();
}

/// Which of the doodad panel's two slope buttons was pressed.
#[derive(Debug, Clone, Copy)]
enum Lean {
    Ground,
    Upright,
}

/// The doodad tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, now, .. } = subject;
    doodad(
        ui,
        session,
        &mut editing.selection,
        &mut editing.doodad_held,
        &mut editing.gizmo,
        &mut editing.placing,
        &mut editing.swap,
        &mut editing.portraits,
        &mut editing.favourites,
        now,
    );
}
