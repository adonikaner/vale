//! The WMO tool's panel. The state is [`crate::tools::placements::wmos`].
//!
//! In Place mode the whole panel is the picker's,
//! [`super::place::placing`]. In Select mode:
//!
//! ```text
//! [ Select | Place ]         the mode switch
//! Keys                       Select mode's controls, on hover
//! Handles                    Move, Turn or Off
//! Selection                  the group and its buttons, the Replace Model
//!                            list, the locked WMOs, and the primary's model,
//!                            id and tile
//! Position, Rotation, Sets   the primary's numbers
//! Bounds                     the size of the primary's box, read and not edited
//! Duplicate                  for one WMO
//! ```

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::placements::gizmo::Gizmo;
use crate::tools::placements::place::{Kind, Placing};
use crate::tools::placements::wmos;
use crate::ui::tools::placements::group::{group, locked_placements, Group};
use crate::tools::placements::swap::Swap;
use crate::ui::tools::placements::place::{handles, keys, number_row, placing, replacing};
use crate::ui::widgets::theme::{self, property};

/// Select mode's mouse and keyboard controls, for the Keys button. The last
/// entry is the control a doodad has and a WMO lacks.
const KEYS: [(&str, &str); 18] = [
    ("Left button", "Select a WMO"),
    ("Shift + left button", "Add or remove one"),
    ("Drag on empty ground", "Select a rectangle"),
    ("Drag", "Move"),
    ("Ctrl + drag", "Drop onto the ground"),
    ("Arrows", "Nudge 5 yd"),
    ("Page Up, Page Down", "Raise, lower 5 yd"),
    (", and .", "Turn about Z"),
    ("Shift + any of those keys", "Ten times as far"),
    ("Alt + mouse", "Turn"),
    ("Ctrl + Alt + mouse", "Snap the turn"),
    ("Delete", "Remove"),
    ("Ctrl + D", "Duplicate; a group is copied two yards north"),
    ("Ctrl + C", "Copy"),
    ("Ctrl + V", "Paste at the pointer"),
    ("1, 2", "Select, Place"),
    ("Tab", "Switch between Select and Place"),
    ("Ctrl + wheel", "No scale: MODF has no scale field in 1.12"),
];

/// The WMO tool's panel: `MODF` placements.
///
/// The same as the doodad panel without a scale, because 1.12's record does
/// not carry one. Instead it has two sets (which dressing of the model this
/// placement uses, and which name set) and a bounding box that this tool keeps
/// in step with the geometry. The box is shown because it can be wrong.
#[allow(clippy::too_many_arguments)]
pub(in crate::ui::tools) fn wmo(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    selection: &mut wmos::Selection,
    held: &mut wmos::Held,
    gizmo: &mut Gizmo,
    placer: &mut Placing,
    swap: &mut Swap,
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
    now: f64,
) {
    if placing(ui, placer, Kind::Wmo, portraits, favourites) {
        return;
    }
    keys(ui, &KEYS);
    handles(ui, &mut gizmo.handles);

    theme::heading_about(
        ui,
        "Selection",
        "Click a WMO in the world to select it, or drag a rectangle on \
         empty ground. Only visible placements can be selected.",
    );
    let chosen: Vec<(u32, &str)> =
        selection.members().map(|at| (at.unique_id, at.path.as_str())).collect();
    match group(ui, &chosen, "WMOs") {
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
                for at in wmos::all_of(session, &path) {
                    if !selection.holds(at.unique_id) {
                        selection.also.push(at);
                    }
                }
                session.status = format!("{} selected", selection.count());
            }
        }
        Group::Lock => {
            let ids: Vec<u32> = selection.members().map(|at| at.unique_id).collect();
            let n = session.set_placements_locked(Kind::Wmo, ids, true);
            selection.only(None);
            session.status = format!("locked {n} WMOs");
        }
        Group::Replace => match swap.open {
            Some(Kind::Wmo) => swap.close(),
            _ => swap.show(Kind::Wmo),
        },
    }
    if selection.count() > 0 {
        replacing(ui, swap, placer, Kind::Wmo, selection.count(), portraits, favourites);
    }
    if selection.at.is_none() {
        theme::note(ui, "No WMO selected");
    }
    locked_placements(ui, session, Kind::Wmo);
    let grouped = selection.count() > 1;
    let Some(at) = selection.at.as_mut() else {
        return;
    };

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
    theme::heading_about(ui, "Position", "World coordinates: X north, Y west, Z up.");
    let mut world = vale_assets::world::adt::placement_to_world(at.record.position);
    let before = world;
    for (name, axis) in [("X", 0), ("Y", 1), ("Z", 2)] {
        number_row(ui, name, "", egui::DragValue::new(&mut world[axis]).speed(0.1).fixed_decimals(2));
    }
    if world != before {
        // Moved through the tool's own function so the bounding box moves too.
        // The character mover reads the `MODF` box before the `.wmo` has
        // loaded, so a box left behind is wrong there; see `wmos::refit`.
        wmos::set_world_position(&mut at.record, &was, world);
    }

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

    theme::heading_about(ui, "Sets", "Indices into the WMO's doodad sets and name sets.");
    number_row(ui, "Doodad Set", "", egui::DragValue::new(&mut at.record.doodad_set).speed(0.1));
    number_row(ui, "Name Set", "", egui::DragValue::new(&mut at.record.name_set).speed(0.1));

    // Read-only: the box has to agree with the geometry, so it is not typed.
    // `wmos::publish` re-fits it from the drawn geometry when the button comes
    // up.
    theme::heading_about(ui, "Bounds", "Re-fitted from the drawn geometry when a drag ends.");
    let span = [
        at.record.bounds_upper[0] - at.record.bounds_lower[0],
        at.record.bounds_upper[1] - at.record.bounds_lower[1],
        at.record.bounds_upper[2] - at.record.bounds_lower[2],
    ];
    property(ui, "Size", "", |ui| {
        theme::value(
            ui,
            format!(
                "{:.0} x {:.0} x {:.0} yd",
                span[0].abs(),
                span[1].abs(),
                span[2].abs()
            ),
        );
    });

    // Over a group, `Ctrl+D` copies the whole group in place rather than
    // arming the placer with one, and the button is not drawn; the Keys
    // button says so. See `crate::tools::placements::group`.
    if !grouped {
        ui.add_space(6.0);
        if theme::primary(ui, "Duplicate")
            .on_hover_text(
                "Ctrl+D. Puts a copy of this WMO on the cursor at the same turn. \
                 Click to place it.",
            )
            .clicked()
        {
            let turns = vale_assets::world::adt::placement_euler_to_world(was.rotation);
            placer.arm(Kind::Wmo, at.path.clone(), turns[2], 1.0);
        }
    }

    let Some(record) = selection.edited(&was) else {
        return;
    };
    let Some(at) = selection.at.as_ref() else {
        return;
    };
    let (label, kind) = wmos::what_changed(&was, &record);
    let (tile, unique_id) = (at.tile, at.unique_id);
    // Over a group, a move or a turn carries the members on the same entry,
    // under the subject `wmos::nudge` uses for the group. The two sets are the
    // primary's own and are not carried.
    let (label, subject) = match grouped {
        false => (label.to_string(), format!("wmo {unique_id} {kind}")),
        true => (
            format!("{label}s ({})", selection.count()),
            format!("wmo group {unique_id} {kind}"),
        ),
    };
    session.history.begin_gesture(label, subject, now);
    wmos::write_record(session, at);
    session.publish(tile);
    if grouped && (record.position != was.position || record.rotation != was.rotation) {
        wmos::carry_members(session, selection, &was);
        held.members_moved();
    }
    session.history.end();
}

/// The WMO tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, now, .. } = subject;
    wmo(
        ui,
        session,
        &mut editing.wmos,
        &mut editing.wmo_held,
        &mut editing.gizmo,
        &mut editing.placing,
        &mut editing.swap,
        &mut editing.portraits,
        &mut editing.favourites,
        now,
    );
}
