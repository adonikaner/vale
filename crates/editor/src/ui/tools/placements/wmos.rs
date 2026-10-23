//! The WMO tool's panel. The state is [`crate::tools::placements::wmos`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::placements::gizmo::{Gizmo, HANDLES};
use crate::tools::placements::place::{Kind, Placing};
use crate::tools::placements::wmos;
use crate::ui::tools::placements::group::{group, locked_placements, Group};
use crate::ui::tools::placements::place::placing;
use crate::ui::widgets::theme;

/// The building tool: `MODF` placements.
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
    portraits: &mut crate::preview::portraits::Portraits,
    favourites: &mut crate::prefs::favourites::Favourites,
    now: f64,
) {
    if placing(ui, placer, Kind::Wmo, portraits, favourites) {
        return;
    }
    ui.add_space(4.0);
    theme::heading(ui, "Handles");
    theme::segmented(ui, &mut gizmo.handles, &HANDLES, |a, b| a == b);

    ui.add_space(4.0);
    theme::heading(ui, "Selection");
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
    }
    locked_placements(ui, session, Kind::Wmo);
    let grouped = selection.count() > 1;
    let Some(at) = selection.at.as_mut() else {
        theme::note(
            ui,
            "Click a building in the world to select it, or drag a rectangle on \
             empty ground. Only visible placements can be selected.",
        );
        return;
    };

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
    let mut world = vale_assets::world::adt::placement_to_world(at.record.position);
    let before = world;
    for (label, axis) in [("x", 0), ("y", 1), ("z", 2)] {
        theme::row(ui, label, |ui| {
            ui.add(
                egui::DragValue::new(&mut world[axis])
                    .speed(0.1)
                    .fixed_decimals(2),
            );
        });
    }
    if world != before {
        // Moved through the tool's own function so the bounding box moves too.
        // The character mover reads the `MODF` box before the `.wmo` has
        // loaded, so a box left behind is wrong there; see `wmos::refit`.
        wmos::set_world_position(&mut at.record, &was, world);
    }
    theme::note(ui, "x north · y west · z up");

    ui.add_space(4.0);
    theme::heading(ui, "Rotation");
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

    ui.add_space(4.0);
    theme::heading(ui, "Sets");
    theme::row(ui, "doodad set", |ui| {
        ui.add(egui::DragValue::new(&mut at.record.doodad_set).speed(0.1));
    });
    theme::row(ui, "name set", |ui| {
        ui.add(egui::DragValue::new(&mut at.record.name_set).speed(0.1));
    });
    theme::note(ui, "indices into the WMO's doodad sets and name sets");

    ui.add_space(4.0);
    theme::heading(ui, "Bounds");
    // Read-only: the box has to agree with the geometry, so it is not typed.
    // `wmos::publish` re-fits it from the drawn geometry when the button comes
    // up.
    let span = [
        at.record.bounds_upper[0] - at.record.bounds_lower[0],
        at.record.bounds_upper[1] - at.record.bounds_lower[1],
        at.record.bounds_upper[2] - at.record.bounds_lower[2],
    ];
    theme::note(
        ui,
        format!(
            "{:.0} x {:.0} x {:.0} yd",
            span[0].abs(),
            span[1].abs(),
            span[2].abs()
        ),
    );
    theme::note(ui, "re-fitted from the drawn geometry when a drag ends");

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "drag to move · ctrl-drag drops it onto the ground");
    theme::note(ui, "arrows nudge 5 yd · shift nudges ten times as far");
    theme::note(ui, ", and . turn about z · alt + mouse turns it");
    theme::note(ui, "delete removes it · ctrl+d duplicates");
    theme::note(ui, "shift + click adds or removes one");
    theme::note(ui, "drag on empty ground selects a rectangle");
    theme::note(ui, "ctrl+c copies · ctrl+v pastes at the pointer");
    theme::note(ui, "tab, or 1 and 2, switch select and place");
    theme::note(ui, "no scale: MODF has no scale field in 1.12");

    ui.add_space(6.0);
    // Over a group, `Ctrl+D` copies the whole group in place rather than
    // arming the placer with one — see `crate::tools::placements::group`.
    if grouped {
        theme::note(ui, "ctrl+d copies the whole group two yards north");
    } else if theme::primary(ui, "Duplicate")
        .on_hover_text(
            "Ctrl+D. Puts a copy of this building on the cursor at the same turn. \
         Click to place it.",
        )
        .clicked()
    {
        let turns = vale_assets::world::adt::placement_euler_to_world(was.rotation);
        placer.arm(Kind::Wmo, at.path.clone(), turns[2], 1.0);
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
        &mut editing.portraits,
        &mut editing.favourites,
        now,
    );
}
