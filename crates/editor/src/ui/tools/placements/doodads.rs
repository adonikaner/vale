//! The doodad tool's panel. The state is [`crate::tools::placements::doodads`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::placements::doodads::Selection;
use crate::tools::placements::gizmo::{Gizmo, HANDLES, Handles};
use crate::tools::placements::place::{Kind, Placing};
use crate::ui::tools::placements::group::{group, locked_placements, Group};
use crate::ui::tools::placements::place::placing;
use crate::ui::widgets::theme;

/// The doodad tool: what is standing on the ground.
#[allow(clippy::too_many_arguments)]
pub(in crate::ui::tools) fn doodad(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    selection: &mut Selection,
    held: &mut crate::tools::placements::doodads::Held,
    gizmo: &mut Gizmo,
    placer: &mut Placing,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
    now: f64,
) {
    if placing(ui, placer, Kind::Doodad, portraits, favourites) {
        return;
    }
    ui.add_space(4.0);
    theme::heading(ui, "Handles");
    theme::segmented(ui, &mut gizmo.handles, &HANDLES, |a, b| a == b);
    theme::note(
        ui,
        match gizmo.handles {
            Handles::Move => "drag an arrow to move along one axis",
            Handles::Turn => "drag a ring to turn about one axis",
            Handles::Off => "no handles: click selects, drag moves along the ground",
        },
    );

    ui.add_space(4.0);
    theme::heading(ui, "Selection");
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
    }
    locked_placements(ui, session, Kind::Doodad);
    let grouped = selection.count() > 1;
    let Selection { at, align, .. } = &mut *selection;
    let Some(at) = at.as_mut() else {
        theme::note(
            ui,
            "Click a doodad in the world to select it, or drag a rectangle on \
             empty ground. Selection tests the model's triangles rather than \
             its bounding box, and only visible placements can be selected.",
        );
        theme::note(ui, "tab, or 1 and 2, switch select and place");
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
    ui.label(egui::RichText::new(leaf).color(theme::INK))
        .on_hover_text(at.path.clone());
    theme::note(
        ui,
        format!("id {} · tile {},{}", at.unique_id, at.tile.0, at.tile.1),
    );

    let was = at.record;
    ui.add_space(4.0);
    theme::heading(ui, "Position");
    // World coordinates, not the file's. `MDDF` measures from the map corner
    // in its own axis order, which no other tool reports. The status line,
    // `.gps` in the game and every other check in this project use world
    // coordinates. `vale_assets::world::adt` does the conversion both ways.
    let mut world = vale_assets::world::adt::placement_to_world(at.record.position);
    for (label, axis) in [("x", 0), ("y", 1), ("z", 2)] {
        theme::row(ui, label, |ui| {
            ui.add(
                egui::DragValue::new(&mut world[axis])
                    .speed(0.1)
                    .fixed_decimals(2),
            );
        });
    }
    at.record.position = vale_assets::world::adt::placement_from_world(world);
    theme::note(ui, "x north · y west · z up");

    ui.add_space(4.0);
    theme::heading(ui, "Rotation");
    // The angles are converted too. The file's three angles are in a different
    // order and with different signs from the world's, so showing them raw
    // would label as "x" a field that turns the model about z.
    // `placement_euler_to_world` is the single definition of the mapping; see
    // the module comment.
    let mut turns = vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
    for (label, axis) in [("about x", 0), ("about y", 1), ("about z", 2)] {
        theme::row(ui, label, |ui| {
            ui.add(
                egui::DragValue::new(&mut turns[axis])
                    .speed(1.0)
                    .fixed_decimals(1)
                    .suffix("°"),
            );
        });
    }
    at.record.rotation = vale_assets::world::adt::placement_euler_from_world(turns);
    theme::note(ui, "right-handed, about the world's own axes");
    // Slope alignment: the two leans are rewritten from the ground's normal
    // under the origin and the turn is kept; see `doodads::lean_onto_ground`.
    // A button applies it to every selected placement, each to the ground
    // under its own origin, and a switch applies it to every later ctrl-drag.
    let mut each: Option<Lean> = None;
    ui.horizontal(|ui| {
        if ui
            .small_button("Align to slope")
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
        if ui
            .small_button("Stand upright")
            .on_hover_text("Set the rotation about x and y to zero, keeping the rotation about z. In a group, applies to every member.")
            .clicked()
        {
            each = Some(Lean::Upright);
            crate::tools::placements::doodads::stand_upright(at);
        }
    });
    ui.checkbox(align, "ctrl-drag also aligns to slope")
        .on_hover_text(
            "A ctrl-drag that drops the doodad onto the ground also aligns \
             it to the slope there.",
        );

    ui.add_space(4.0);
    theme::heading(ui, "Scale");
    // The file stores unit scale as 1024. The panel shows a multiplier and
    // writes the file's number.
    let mut scale = f32::from(at.record.scale) / 1024.0;
    theme::row(ui, "scale", |ui| {
        if ui
            .add(
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
    });

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "drag to move · ctrl-drag drops it onto the ground");
    theme::note(
        ui,
        "arrows nudge · page up/down raises and lowers · shift moves ten times as far",
    );
    theme::note(ui, ", and . turn about z · alt + mouse turns it");
    theme::note(ui, "ctrl + wheel scales · ctrl with alt snaps the turn");
    theme::note(ui, "delete removes it · ctrl+d duplicates");
    theme::note(ui, "shift + click adds or removes one");
    theme::note(ui, "drag on empty ground selects a rectangle");
    theme::note(ui, "ctrl+c copies · ctrl+v pastes at the pointer");
    theme::note(ui, "tab, or 1 and 2, switch select and place");

    ui.add_space(6.0);
    if grouped {
        theme::note(ui, "ctrl+d copies the whole group two yards north");
    } else if theme::primary(ui, "Duplicate")
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
