//! The WMO tool's panel. The state is [`crate::tools::placements::wmos`].
//!
//! The mode, the handles and the Keys button are on the toolbar over the
//! viewport, [`super::place::toolbar`]. In Place mode the whole panel is the
//! picker's, [`super::place::placing`]. While the building editor is open
//! the panel is its heading ([`super::wmoview::heading`]) and the mode's
//! panel, [`super::wmoareas`]'s, [`super::wmodoodads`]'s or
//! [`super::wmolights`]'s. In Select mode:
//!
//! ```text
//! Selection                  the group and its buttons, the Replace Model
//!                            list, the locked WMOs, and the primary's model,
//!                            id and tile, with Edit Building; with nothing
//!                            selected on a map that is one building, Edit
//!                            for that building
//! Position, Rotation         the primary's numbers
//! Sets                       the primary's doodad set and name set, each a
//!                            field that opens a window to choose one: the
//!                            building's doodad sets by name, and the name
//!                            sets its WMOAreaTable rows use, by the name
//!                            they give the building
//! Bounds                     the size of the primary's box, read and not edited
//! Duplicate                  for one WMO
//! ```

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::placements::place::{Kind, Placing};
use crate::tools::placements::wmos;
use crate::ui::tools::placements::group::{group, locked_placements, Group};
use crate::tools::placements::swap::Swap;
use crate::ui::tools::placements::place::{number_row, placing, replacing};
use crate::ui::widgets::theme::{self, property};
use vale_client::assets::GameAssets;

/// Select mode's mouse and keyboard controls, for the toolbar's Keys button.
/// The last entry is the control a doodad has and a WMO lacks.
pub(super) const KEYS: [(&str, &str); 18] = [
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

/// What Edit Building does, for its hover text.
const EDIT_BUILDING: &str = "Opens the building editor: the building alone in the viewport, its areas, \
                             and the doodads and doodad sets its file carries.";

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
    assets: &GameAssets,
    selection: &mut wmos::Selection,
    held: &mut wmos::Held,
    placer: &mut Placing,
    swap: &mut Swap,
    view: &mut crate::tools::placements::wmoview::WmoView,
    portraits: &mut crate::preview::portraits::Portraits,
    drawer: &mut crate::ui::shell::drawer::Drawer,
    now: f64,
) {
    if placing(ui, placer, Kind::Wmo, portraits, drawer) {
        return;
    }

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
        replacing(ui, swap, Kind::Wmo, selection.count(), portraits, drawer);
    }
    if selection.at.is_none() {
        theme::note(ui, "No WMO selected");
        // A map that is one building places it from its WDT, and no tile
        // lists it, so it cannot be selected; it is opened from here.
        if let Some(path) = view.global.clone() {
            let leaf = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
            if theme::primary(ui, &format!("Edit {leaf}")).on_hover_text(EDIT_BUILDING).clicked() {
                view.asked = Some(u32::MAX);
            }
        }
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
    if !grouped && theme::primary(ui, "Edit Building").on_hover_text(EDIT_BUILDING).clicked() {
        view.asked = Some(at.unique_id);
    }

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

    sets(ui, session, assets, &at.path, &mut at.record.doodad_set, &mut at.record.name_set);

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

/// One row of a list of sets: the set's number, its name and what it holds.
struct SetRow {
    id: u16,
    title: String,
    sub: String,
}

/// The placement's doodad set and name set, each a field that shows the
/// set's name and number and opens a window to choose another.
///
/// The doodad sets are the building root file's `MODS` sets. A name set has
/// no list in the file: it is the second key of the building's
/// `WMOAreaTable` rows, so the name sets listed are the ones the table has
/// rows for, named by what those rows call the building. Set 0 and the
/// placement's own set are listed even when no row names them. When the
/// building's file cannot be read, both are number fields.
fn sets(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &GameAssets,
    path: &str,
    doodad_set: &mut u16,
    name_set: &mut u16,
) {
    let root = session
        .open_building(assets, path)
        .ok()
        .and_then(|()| session.building(path))
        .map(|open| (open.building.root.doodad_sets(), open.building.root.wmo_id()));
    let Some((doodad_sets, wmo_id)) = root else {
        theme::heading_about(ui, "Sets", "The building's file could not be read, so its sets are shown as numbers.");
        number_row(ui, "Doodad Set", "", egui::DragValue::new(doodad_set).speed(0.1));
        number_row(ui, "Name Set", "", egui::DragValue::new(name_set).speed(0.1));
        return;
    };

    theme::heading_about(ui, "Sets", "Which of the building's doodads this placement draws, and which rows name its places.");
    let mut rows: Vec<SetRow> = doodad_sets
        .iter()
        .enumerate()
        .filter_map(|(id, set)| {
            let id = u16::try_from(id).ok()?;
            let title = match set.name.is_empty() {
                true => format!("Set {id}"),
                false => set.name.clone(),
            };
            let sub = match set.count {
                1 => "1 doodad".to_string(),
                n => format!("{n} doodads"),
            };
            Some(SetRow { id, title, sub })
        })
        .collect();
    if !rows.iter().any(|row| row.id == *doodad_set) {
        rows.push(SetRow { id: *doodad_set, title: format!("Set {doodad_set}"), sub: "Not in the building's file".to_string() });
    }
    set_field(
        ui,
        "Doodad Set",
        "The set of the building's doodads that this placement draws. Click to choose another.",
        "Choose a doodad set",
        "wmo-doodad-set",
        &rows,
        doodad_set,
    );

    let has_table = session.open_table(assets, crate::tools::placements::wmoareas::TABLE);
    session.open_table(assets, "AreaTable");
    let named = match (has_table, wmo_id, session.table(crate::tools::placements::wmoareas::TABLE)) {
        (true, Some(wmo_id), Some(table)) => {
            crate::tools::placements::wmoareas::name_sets(table, wmo_id, session.table("AreaTable"))
        }
        _ => Vec::new(),
    };
    let mut rows: Vec<SetRow> = named
        .into_iter()
        .map(|set| SetRow {
            id: set.id,
            title: match set.name.is_empty() {
                true => format!("Name set {}", set.id),
                false => set.name,
            },
            sub: match set.rows {
                1 => "1 area row".to_string(),
                n => format!("{n} area rows"),
            },
        })
        .collect();
    for id in [0, *name_set] {
        if !rows.iter().any(|row| row.id == id) {
            rows.push(SetRow { id, title: format!("Name set {id}"), sub: "No area rows".to_string() });
        }
    }
    rows.sort_by_key(|row| row.id);
    set_field(
        ui,
        "Name Set",
        "Which WMOAreaTable rows name the places in this placement. Each set is shown by the name its rows give \
         the building. Click to choose another.",
        "Choose a name set",
        "wmo-name-set",
        &rows,
        name_set,
    );
}

/// A property row whose field shows the chosen set's name, with its number
/// at the right. A click on the field opens a window that lists every set,
/// searched by name or number; a click on a set chooses it and closes the
/// window. `key` keeps the window's place and its search.
fn set_field(ui: &mut egui::Ui, name: &str, about: &str, title: &str, key: &str, rows: &[SetRow], chosen: &mut u16) {
    let open_id = egui::Id::new((key, "open"));
    let search_id = egui::Id::new((key, "search"));
    let shown = rows.iter().find(|row| row.id == *chosen).map_or_else(|| format!("Set {chosen}"), |row| row.title.clone());
    let field = property(ui, name, about, |ui| {
        let size = egui::vec2(theme::field_width(ui), ui.spacing().interact_size.y);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let stroke = match response.hovered() {
            true => egui::Stroke::new(1.0, theme::ACCENT),
            false => egui::Stroke::new(1.0, theme::LINE),
        };
        ui.painter().rect(rect, egui::CornerRadius::same(3), theme::SUNK, stroke, egui::StrokeKind::Inside);
        let inner = rect.shrink2(egui::vec2(6.0, 0.0));
        let number = ui.painter().layout_no_wrap(chosen.to_string(), egui::FontId::monospace(theme::SMALL), theme::INK_FAINT);
        let number_at = egui::pos2(inner.right() - number.size().x, inner.center().y - number.size().y / 2.0);
        let room = (inner.width() - number.size().x - 8.0).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(shown, egui::FontId::proportional(theme::BODY), theme::INK);
        job.wrap = egui::text::TextWrapping::truncate_at_width(room);
        let text = ui.painter().layout_job(job);
        ui.painter().galley(egui::pos2(inner.left(), inner.center().y - text.size().y / 2.0), text, theme::INK);
        ui.painter().galley(number_at, number, theme::INK_FAINT);
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    });
    let mut open = ui.data(|data| data.get_temp::<bool>(open_id)).unwrap_or(false);
    if field.clicked() {
        open = true;
    }
    if !open {
        return;
    }
    let mut search = ui.data(|data| data.get_temp::<String>(search_id)).unwrap_or_default();
    let mut keep_open = true;
    let mut picked = None;
    theme::window(title, key)
        .open(&mut keep_open)
        .default_size([320.0, 360.0])
        .default_pos(field.rect.left_bottom())
        .show(ui.ctx(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut search).hint_text("Name or number").desired_width(f32::INFINITY));
            let query = search.trim().to_lowercase();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for row in rows {
                    let trailing = row.id.to_string();
                    if !query.is_empty() && !row.title.to_lowercase().contains(&query) && !trailing.contains(&query) {
                        continue;
                    }
                    let shape = theme::list_row(
                        ui,
                        theme::ListRow { title: &row.title, sub: &row.sub, trailing: &trailing, tint: theme::INK, picture: false },
                        row.id == *chosen,
                    );
                    if shape.response.clicked() {
                        picked = Some(row.id);
                    }
                }
            });
        });
    if let Some(id) = picked {
        *chosen = id;
    }
    let open = keep_open && picked.is_none();
    ui.data_mut(|data| {
        data.insert_temp(open_id, open);
        data.insert_temp(search_id, search);
    });
}

/// The WMO tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { tool, session, assets, now, .. } = subject;
    if editing.wmo_view.open.is_some() {
        super::wmoview::heading(ui, &editing.wmo_view);
        match editing.placing.switch() {
            crate::tools::placements::place::Switch::Lights => super::wmolights::panel(
                ui,
                session,
                assets,
                &mut editing.wmo_spawns,
                &mut editing.wmo_lamps,
                &mut editing.wmo_view,
                now,
            ),
            crate::tools::placements::place::Switch::Doodads => super::wmodoodads::panel(
                ui,
                session,
                assets,
                &mut editing.wmo_spawns,
                &mut editing.wmo_view,
                &mut editing.drawer,
                now,
            ),
            _ => {
                // The mode's state is taken out for the time the workspace
                // holds the rest of the shell.
                let mut areas = std::mem::take(&mut *editing.wmo_areas);
                super::wmoareas::panel(ui, crate::ui::tables::Workspace::new(tool, session, editing, assets, now, false, false), &mut areas);
                *editing.wmo_areas = areas;
            }
        }
        return;
    }
    wmo(
        ui,
        session,
        assets,
        &mut editing.wmos,
        &mut editing.wmo_held,
        &mut editing.placing,
        &mut editing.swap,
        &mut editing.wmo_view,
        &mut editing.portraits,
        &mut editing.drawer,
        now,
    );
}
