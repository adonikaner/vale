//! The right-click menu over the viewport: one list of one-click operations
//! per world tool, about what the click landed on. When a click opens a menu
//! and what it is about is `crate::context`.
//!
//! Every item calls the operation its panel button calls, or hands the
//! operation to the system that answers its key (`crate::context::Request`),
//! so a menu item and its button or key cannot behave differently. An item
//! that cannot apply is shown disabled, with the reason on hover.
//!
//! The menu closes when an item is pressed, when Escape is pressed, when a
//! mouse button is pressed outside it, and when the tool changes.

use bevy::prelude::Vec3;
use bevy_egui::egui;

use super::theme;
use super::Editing;
use crate::context::{Open, Request, Target};
use crate::session::EditSession;
use crate::tools::place::Kind;
use crate::tools::Tool;
use vale_client::assets::GameAssets;

/// How wide the menu is, in points.
const WIDTH: f32 = 230.0;

/// Draw the open menu, if there is one, and carry out what is pressed.
pub fn draw(
    ctx: &egui::Context,
    session: &mut EditSession,
    editing: &mut Editing,
    assets: &GameAssets,
    zoom: f32,
    now: f64,
) {
    let Some(open) = editing.menu.open else {
        return;
    };
    let at = egui::pos2(open.at.x / zoom.max(0.01), open.at.y / zoom.max(0.01));
    let mut chosen = false;
    let area = egui::Area::new(egui::Id::new("world-context-menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(at)
        .constrain(true)
        .show(ctx, |ui| {
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(WIDTH);
                ui.set_max_width(WIDTH);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                chosen = items(ui, open, session, editing, assets, now);
            });
        });
    let outside = ctx.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| !area.response.rect.contains(pos))
    });
    let escape = ctx.input(|input| input.key_pressed(egui::Key::Escape));
    if chosen || outside || escape {
        editing.menu.close();
    }
}

/// One item. Answers whether it was pressed.
fn item(ui: &mut egui::Ui, label: &str, enabled: bool, about: &str, why_not: &str) -> bool {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(label)
            .frame(false)
            .min_size(egui::vec2(WIDTH - 12.0, 0.0)),
    );
    let response = match about.is_empty() {
        true => response,
        false => response.on_hover_text(about),
    };
    let response = match why_not.is_empty() {
        true => response,
        false => response.on_disabled_hover_text(why_not),
    };
    response.clicked()
}

/// A section's caption.
fn caption(ui: &mut egui::Ui, text: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(text).size(theme::SMALL).color(theme::INK_FAINT));
}

/// The menu's items for the tool it was opened under. Answers whether one
/// was pressed.
fn items(
    ui: &mut egui::Ui,
    open: Open,
    session: &mut EditSession,
    editing: &mut Editing,
    assets: &GameAssets,
    now: f64,
) -> bool {
    let mut chosen = match (open.tool, open.target) {
        (Tool::Doodads, Target::Doodad { unique_id, locked }) => {
            doodad(ui, session, editing, unique_id, locked)
        }
        (Tool::Wmos, Target::Wmo { unique_id, locked }) => wmo(ui, session, editing, unique_id, locked),
        (Tool::Doodads, Target::Ground { at, .. }) => placements_ground(ui, session, editing, Kind::Doodad, at),
        (Tool::Wmos, Target::Ground { at, .. }) => placements_ground(ui, session, editing, Kind::Wmo, at),
        (Tool::Terrain, Target::Ground { at, .. }) => terrain(ui, session, editing, at),
        (Tool::Chunks, Target::Ground { .. }) => chunks(ui, session, editing),
        (Tool::Textures, Target::Ground { tile, chunk, .. }) => textures(ui, session, editing, tile, chunk),
        (Tool::Areas, Target::Ground { tile, chunk, .. }) => areas(ui, session, editing, tile, chunk),
        (Tool::Holes, Target::Ground { tile, chunk, .. }) => holes(ui, session, tile, chunk),
        (Tool::Grade, Target::Ground { .. }) => grade(ui, session, editing),
        (Tool::Road, Target::Ground { at, .. }) => road(ui, session, editing, at),
        (Tool::Creatures, Target::Creature { guid }) => creature(ui, session, editing, guid, now),
        (Tool::GameObjects, Target::GameObject { guid }) => game_object(ui, session, editing, guid, now),
        (Tool::Creatures | Tool::GameObjects, Target::Ground { at, .. }) => {
            spawns_ground(ui, session, editing, open.tool, at, now)
        }
        _ => false,
    };
    // The doodad and WMO tools' Select and Place switch, whatever was
    // clicked. The creature and game object tools carry theirs in
    // `spawns_ground`, since a spawn's menu is about the spawn.
    if matches!(open.tool, Tool::Doodads | Tool::Wmos) {
        ui.separator();
        chosen |= placement_mode(ui, editing);
    }
    // What every menu ends with: the clicked point, or the clicked spawn's
    // position, and the history.
    let edits = Some(&session.server_edits);
    let at = match open.target {
        Target::Ground { at, .. } => at,
        Target::Creature { guid } => editing.creatures.edited(guid, edits).map(|spawn| spawn.at),
        Target::GameObject { guid } => editing.objects.edited(guid, edits).map(|spawn| spawn.at),
        _ => None,
    };
    chosen |= common(ui, session, assets, at);
    chosen
}

/// The point that was clicked, as text, and Undo and Redo.
fn common(ui: &mut egui::Ui, session: &mut EditSession, assets: &GameAssets, at: Option<Vec3>) -> bool {
    let mut chosen = false;
    ui.separator();
    caption(ui, "Point");
    if item(
        ui,
        "Copy position",
        at.is_some(),
        "Copy the clicked point's x, y and z to the clipboard: the ground's, or the clicked          spawn's own position.",
        "The click was not on open ground.",
    ) {
        if let Some(at) = at {
            ui.ctx().copy_text(crate::tools::measure::position_text(at));
        }
        chosen = true;
    }
    if item(
        ui,
        "Copy .go command",
        at.is_some(),
        "Copy a GM command that teleports a character to the clicked point, or to the clicked          spawn, on this map.",
        "The click was not on open ground.",
    ) {
        if let Some(at) = at {
            ui.ctx().copy_text(crate::tools::measure::go_command(at, session.map_id));
        }
        chosen = true;
    }
    ui.separator();
    let undo = session.history.next_undo().map(|c| c.label.clone());
    let redo = session.history.next_redo().map(|c| c.label.clone());
    if item(
        ui,
        &match &undo {
            Some(label) => format!("Undo {label}"),
            None => "Undo".to_string(),
        },
        undo.is_some(),
        "Ctrl+Z.",
        "Nothing to undo.",
    ) {
        session.undo(assets);
        chosen = true;
    }
    if item(
        ui,
        &match &redo {
            Some(label) => format!("Redo {label}"),
            None => "Redo".to_string(),
        },
        redo.is_some(),
        "Ctrl+Y.",
        "Nothing to redo.",
    ) {
        session.redo(assets);
        chosen = true;
    }
    chosen
}

/// The height tool: the vertex selection and its locks, the brush mode, and
/// a flatten to the clicked height.
fn terrain(ui: &mut egui::Ui, session: &mut EditSession, editing: &mut Editing, at: Option<Vec3>) -> bool {
    use crate::tools::terrain::VertexAsk;
    use vale_edit::ops::Mode;
    let terrain = &mut editing.terrain;
    let any = terrain.selected > 0;
    let mut chosen = false;
    let ask = |terrain: &mut crate::tools::terrain::Terrain, what: VertexAsk| {
        terrain.ask = Some(what);
        true
    };

    caption(ui, &format!("Selected vertices: {}", terrain.selected));
    if item(
        ui,
        "Select vertices here",
        at.is_some(),
        "Add the vertices under the footprint at the clicked point to the selection.",
        "The click was not on open ground.",
    ) {
        if let Some(at) = at {
            let (radius, shape) = (terrain.brush.radius, terrain.brush.shape);
            for coord in crate::tools::terrain::tiles_in_range(radius, at) {
                if let Some(tile) = session.tiles.get(&coord) {
                    terrain
                        .vertices
                        .entry(coord)
                        .or_default()
                        .mark(tile, [at.x, at.y], radius, shape, true);
                }
            }
        }
        chosen = true;
    }
    let none = "Select some vertices first.";
    if item(ui, "Level selection", any, "Put every selected vertex at their mean height.", none) {
        chosen = ask(terrain, VertexAsk::Level);
    }
    if item(ui, "Smooth selection", any, "Move each selected vertex part of the way toward its neighbours.", none) {
        chosen = ask(terrain, VertexAsk::Smooth);
    }
    if item(ui, "Lock selection", any, "Add the selected vertices to their tiles' locked vertices.", none) {
        chosen = ask(terrain, VertexAsk::Lock);
    }
    if item(ui, "Unlock selection", any, "Remove the selected vertices from their tiles' locked vertices.", none) {
        chosen = ask(terrain, VertexAsk::Unlock);
    }
    if item(
        ui,
        "Lock tile sides",
        any,
        "Lock the vertices on all four sides of each tile that holds part of the selection.",
        none,
    ) {
        chosen = ask(terrain, VertexAsk::LockSides);
    }
    if item(
        ui,
        "Unlock all",
        terrain.locked > 0,
        "Unlock every vertex of every open tile.",
        "No vertex on the open tiles is locked.",
    ) {
        chosen = ask(terrain, VertexAsk::UnlockAll);
    }
    let inside = match terrain.only_inside {
        true => "Strokes anywhere",
        false => "Strokes only inside the selection",
    };
    if item(ui, inside, any, "Switch \"Only inside the selection\".", none) {
        terrain.only_inside = !terrain.only_inside;
        chosen = true;
    }
    if item(ui, "Clear selection", any, "Select no vertices.", none) {
        chosen = ask(terrain, VertexAsk::Clear);
    }

    ui.separator();
    caption(ui, "Brush");
    let pointer = match terrain.selecting {
        true => "Sculpt",
        false => "Select vertices",
    };
    if item(ui, pointer, true, "What the left button does: sculpt the ground, or select vertices.", "") {
        terrain.selecting = !terrain.selecting;
        chosen = true;
    }
    if item(
        ui,
        "Flatten to this height",
        at.is_some(),
        "Set the brush to Flatten toward the height of the clicked point, typed into the flatten height.",
        "The click was not on open ground.",
    ) {
        if let Some(at) = at {
            terrain.selecting = false;
            terrain.brush.mode = Mode::Flatten { to: at.z };
            terrain.flatten_to_cursor = false;
            terrain.flatten_height = at.z;
        }
        chosen = true;
    }
    for (label, mode) in crate::tools::terrain::MODES {
        let current = std::mem::discriminant(&terrain.brush.mode) == std::mem::discriminant(&mode);
        if item(ui, &format!("Brush: {label}"), !current || terrain.selecting, "", "") {
            terrain.selecting = false;
            terrain.brush.mode = mode;
            chosen = true;
        }
    }
    chosen
}

/// The chunk tool: every operation of its five pages over the selection, and
/// the selection itself.
fn chunks(ui: &mut egui::Ui, session: &mut EditSession, editing: &mut Editing) -> bool {
    use crate::tools::chunks as tool;
    let selected = editing.chunks.selected.clone();
    let any = !selected.is_empty();
    let none = "No chunk is selected.";
    let mut chosen = false;
    let mut said: Option<String> = None;

    caption(ui, &format!("Selected chunks: {}", selected.len()));
    if item(ui, "Copy", any, "Ctrl+C. Copy the selected chunks.", none) {
        editing.chunks.clip = tool::copy(session, &selected);
        said = Some(format!("copied {} chunks", editing.chunks.clip.chunks.len()));
        chosen = true;
    }
    let clip = !editing.chunks.clip.is_empty();
    let primary = editing.chunks.primary;
    if item(
        ui,
        "Paste here",
        clip && primary.is_some(),
        "Ctrl+V. Paste the copied chunks centred on the clicked chunk, with the Paste page's settings.",
        "No chunks have been copied.",
    ) {
        if let Some(at) = primary {
            said = Some(tool::paste_at(session, &mut editing.chunks, at));
        }
        chosen = true;
    }
    if item(ui, "Turn copy 90°", clip, "Turn the copied block a quarter turn clockwise.", "No chunks have been copied.") {
        editing.chunks.clip = editing.chunks.clip.turned(tool::Turn::Clockwise);
        chosen = true;
    }
    if item(ui, "Mirror copy", clip, "Swap the copied block's east and west.", "No chunks have been copied.") {
        editing.chunks.clip = editing.chunks.clip.turned(tool::Turn::Mirror);
        chosen = true;
    }

    ui.separator();
    caption(ui, "Ground");
    if item(ui, "Stitch", any, "Level and shade the selection's border with the ground beyond it.", none) {
        let n = tool::stitch(session, &selected, editing.chunks.stitch);
        said = Some(format!("the stitch changed {n} chunks"));
        chosen = true;
    }
    if item(ui, "Cut holes", any, "Cut every hole square of the selected chunks.", none) {
        let n = tool::set_holes(session, &selected, true);
        said = Some(format!("holes cut in {n} chunks"));
        chosen = true;
    }
    if item(ui, "Fill holes", any, "Fill every hole of the selected chunks.", none) {
        let n = tool::set_holes(session, &selected, false);
        said = Some(format!("holes filled in {n} chunks"));
        chosen = true;
    }
    if item(ui, "Set impassable", any, "Set the impassable flag on the selected chunks.", none) {
        let n = tool::set_impassable(session, &selected, true);
        said = Some(format!("{n} chunks set impassable"));
        chosen = true;
    }
    if item(ui, "Clear impassable", any, "Clear the impassable flag on the selected chunks.", none) {
        let n = tool::set_impassable(session, &selected, false);
        said = Some(format!("{n} chunks set passable"));
        chosen = true;
    }
    let area = editing.areas.brush.area;
    if item(
        ui,
        &format!("Set area {area}"),
        any && area != 0,
        "Write the Areas brush's area id into the selected chunks.",
        "Select chunks, and choose an area on the Areas tool.",
    ) {
        let n = tool::set_area(session, &selected, area);
        said = Some(format!("area {area} set on {n} chunks"));
        chosen = true;
    }

    ui.separator();
    caption(ui, "Textures");
    let texture = editing.textures.brush.texture.clone();
    let named = !texture.is_empty();
    if item(
        ui,
        "Set base texture",
        any && named,
        "Make the Textures brush's texture the base layer of the selected chunks.",
        "Select chunks, and choose a texture on the Textures tool.",
    ) {
        let n = tool::set_base(session, &selected, &texture);
        said = Some(format!("base texture set on {n} chunks"));
        chosen = true;
    }
    if item(ui, "Clear to base", any, "Remove every layer above the base on the selected chunks.", none) {
        let n = tool::clear_paint(session, &selected);
        said = Some(format!("{n} chunks cleared to their base texture"));
        chosen = true;
    }

    ui.separator();
    caption(ui, "Selection");
    if item(ui, "Select whole tiles", any, "Select every chunk of the tiles the selection touches.", none) {
        let cells = editing.chunks.whole_tiles(session);
        editing.chunks.select(cells);
        chosen = true;
    }
    let area_of_primary = primary.and_then(|cell| {
        let tile = session.tiles.get(&cell.tile())?;
        vale_edit::adt::area::area(tile, cell.chunk_in(tile)?)
    });
    if item(
        ui,
        &match area_of_primary {
            Some(id) => format!("Select all in area {id}"),
            None => "Select all in this area".to_string(),
        },
        area_of_primary.is_some_and(|id| id != 0),
        "Add every open chunk with the clicked chunk's area id to the selection.",
        "The clicked chunk has no area.",
    ) {
        if let Some(id) = area_of_primary {
            let cells = tool::Chunks::in_area(session, id);
            editing.chunks.add(cells);
        }
        chosen = true;
    }
    if item(ui, "Deselect", any, "Escape. Select no chunks.", none) {
        editing.chunks.clear();
        chosen = true;
    }
    if let Some(said) = said {
        session.status = said;
    }
    chosen
}

/// A doodad that was right-clicked: the selection it is in, or its lock.
fn doodad(ui: &mut egui::Ui, session: &mut EditSession, editing: &mut Editing, unique_id: u32, locked: bool) -> bool {
    let mut chosen = false;
    if locked {
        caption(ui, &format!("Doodad {unique_id}, locked"));
        if item(ui, "Unlock", true, "Let this doodad be selected, moved and deleted again.", "") {
            session.set_placements_locked(Kind::Doodad, [unique_id], false);
            session.status = format!("doodad {unique_id} unlocked");
            chosen = true;
        }
        return chosen;
    }
    let selection = &mut editing.selection;
    let count = selection.count();
    let path = selection.at.as_ref().map(|at| at.path.clone()).unwrap_or_default();
    caption(
        ui,
        &match count {
            1 => format!("Doodad {unique_id}"),
            n => format!("{n} doodads selected"),
        },
    );
    if item(ui, "Lock", true, "Lock the selected doodads: no tool selects, moves or deletes them until they are unlocked.", "") {
        let ids: Vec<u32> = selection.members().map(|m| m.unique_id).collect();
        let n = session.set_placements_locked(Kind::Doodad, ids, true);
        selection.only(None);
        session.status = format!("locked {n} doodads");
        chosen = true;
    }
    if item(ui, "Copy", true, "Ctrl+C.", "") {
        editing.menu.request = Some(Request::Copy);
        chosen = true;
    }
    if count == 1 {
        if item(ui, "Duplicate", true, "Ctrl+D. Put a copy of this doodad on the pointer to place.", "") {
            if let Some(at) = selection.at.as_ref() {
                let turns = vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
                let scale = f32::from(at.record.scale) / 1024.0;
                editing.placing.arm(Kind::Doodad, at.path.clone(), turns[2], scale);
            }
            chosen = true;
        }
    } else if item(ui, "Duplicate", true, "Ctrl+D. Copy the whole group two yards north.", "") {
        editing.menu.request = Some(Request::Duplicate);
        chosen = true;
    }
    if item(ui, "Delete", true, "Delete.", "") {
        editing.menu.request = Some(Request::Delete);
        chosen = true;
    }
    ui.separator();
    if item(ui, "Align to slope", true, "Set each selected doodad's rotation about x and y to the slope under its origin.", "") {
        let n = crate::tools::doodads::lean_selection(session, selection, &mut editing.doodad_held, false);
        session.status = format!("{n} doodads aligned to the slope");
        chosen = true;
    }
    if item(ui, "Stand upright", true, "Set each selected doodad's rotation about x and y to zero.", "") {
        let n = crate::tools::doodads::lean_selection(session, selection, &mut editing.doodad_held, true);
        session.status = format!("{n} doodads stood upright");
        chosen = true;
    }
    if item(ui, "Select all of this model", !path.is_empty(), "Add every placement of this model on the open tiles.", "") {
        for found in crate::tools::doodads::all_of(session, &path) {
            if !selection.holds(found.unique_id) {
                selection.also.push(found);
            }
        }
        chosen = true;
    }
    if item(ui, "Place this model", !path.is_empty(), "Arm the placer with this doodad's model.", "") {
        editing.placing.arm(Kind::Doodad, path, 0.0, 1.0);
        chosen = true;
    }
    chosen
}

/// A WMO that was right-clicked: the selection it is in, or its lock.
fn wmo(ui: &mut egui::Ui, session: &mut EditSession, editing: &mut Editing, unique_id: u32, locked: bool) -> bool {
    let mut chosen = false;
    if locked {
        caption(ui, &format!("WMO {unique_id}, locked"));
        if item(ui, "Unlock", true, "Let this WMO be selected, moved and deleted again.", "") {
            session.set_placements_locked(Kind::Wmo, [unique_id], false);
            session.status = format!("WMO {unique_id} unlocked");
            chosen = true;
        }
        return chosen;
    }
    let selection = &mut editing.wmos;
    let count = selection.count();
    let path = selection.at.as_ref().map(|at| at.path.clone()).unwrap_or_default();
    caption(
        ui,
        &match count {
            1 => format!("WMO {unique_id}"),
            n => format!("{n} WMOs selected"),
        },
    );
    if item(ui, "Lock", true, "Lock the selected WMOs: no tool selects, moves or deletes them until they are unlocked.", "") {
        let ids: Vec<u32> = selection.members().map(|m| m.unique_id).collect();
        let n = session.set_placements_locked(Kind::Wmo, ids, true);
        selection.only(None);
        session.status = format!("locked {n} WMOs");
        chosen = true;
    }
    if item(ui, "Copy", true, "Ctrl+C.", "") {
        editing.menu.request = Some(Request::Copy);
        chosen = true;
    }
    if count == 1 {
        if item(ui, "Duplicate", true, "Ctrl+D. Put a copy of this WMO on the pointer to place.", "") {
            if let Some(at) = selection.at.as_ref() {
                let turns = vale_assets::world::adt::placement_euler_to_world(at.record.rotation);
                editing.placing.arm(Kind::Wmo, at.path.clone(), turns[2], 1.0);
            }
            chosen = true;
        }
    } else if item(ui, "Duplicate", true, "Ctrl+D. Copy the whole group two yards north.", "") {
        editing.menu.request = Some(Request::Duplicate);
        chosen = true;
    }
    if item(ui, "Delete", true, "Delete.", "") {
        editing.menu.request = Some(Request::Delete);
        chosen = true;
    }
    ui.separator();
    if item(ui, "Select all of this model", !path.is_empty(), "Add every placement of this WMO on the open tiles.", "") {
        for found in crate::tools::wmos::all_of(session, &path) {
            if !selection.holds(found.unique_id) {
                selection.also.push(found);
            }
        }
        chosen = true;
    }
    if item(ui, "Place this model", !path.is_empty(), "Arm the placer with this WMO.", "") {
        editing.placing.arm(Kind::Wmo, path, 0.0, 1.0);
        chosen = true;
    }
    chosen
}

/// The doodad and WMO tools' Select and Place switch, which the two tools
/// share, as the panel's switch and the number keys set it.
fn placement_mode(ui: &mut egui::Ui, editing: &mut Editing) -> bool {
    use crate::tools::place::Mode;
    let (label, to) = match editing.placing.mode {
        Mode::Select => ("Switch to Place", Mode::Place),
        Mode::Place => ("Switch to Select", Mode::Select),
    };
    if !item(ui, label, true, "The panel's Select and Place switch. 1 and 2 choose them; Tab toggles.", "") {
        return false;
    }
    editing.placing.mode = to;
    // Switching to Select drops whatever was on the cursor, as the panel's
    // switch does.
    if to == Mode::Select {
        editing.placing.disarm();
    }
    true
}

/// The doodad or WMO tool, clicked on ground: paste, the selection, and the
/// map's locks.
fn placements_ground(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    editing: &mut Editing,
    kind: Kind,
    at: Option<Vec3>,
) -> bool {
    let mut chosen = false;
    let (noun, copied, count) = match kind {
        Kind::Doodad => ("doodads", !editing.clipboard.doodads.is_empty(), editing.selection.count()),
        Kind::Wmo => ("WMOs", !editing.clipboard.buildings.is_empty(), editing.wmos.count()),
    };
    caption(ui, &format!("{count} {noun} selected"));
    if item(
        ui,
        "Paste here",
        copied && at.is_some(),
        &format!("Ctrl+V. Paste the copied {noun} at the clicked point."),
        &format!("Copy some {noun} first, and click on open ground."),
    ) {
        if let Some(at) = at {
            editing.menu.request = Some(Request::Paste(at));
        }
        chosen = true;
    }
    if item(ui, "Deselect", count > 0, "Escape.", "Nothing is selected.") {
        match kind {
            Kind::Doodad => editing.selection.only(None),
            Kind::Wmo => editing.wmos.only(None),
        }
        chosen = true;
    }
    let locked = session.placement_locks().of(kind).len();
    if item(
        ui,
        &format!("Unlock all {noun} ({locked})"),
        locked > 0,
        &format!("Unlock every locked {} on this map.", noun.trim_end_matches('s')),
        &format!("No {} on this map is locked.", noun.trim_end_matches('s')),
    ) {
        let ids: Vec<u32> = session.placement_locks().of(kind).iter().copied().collect();
        let n = session.set_placements_locked(kind, ids, false);
        session.status = format!("unlocked {n} {noun}");
        chosen = true;
    }
    chosen
}

/// The texture tool, about the clicked chunk: its layers as brush textures,
/// pinning it, and its base.
fn textures(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    editing: &mut Editing,
    tile: Option<(u32, u32)>,
    chunk: Option<usize>,
) -> bool {
    let mut chosen = false;
    let (Some(coord), Some(chunk)) = (tile, chunk) else {
        caption(ui, "The click was not on an open chunk.");
        return false;
    };
    caption(ui, &format!("Chunk {chunk} of tile {}, {}", coord.0, coord.1));
    let layers = crate::tools::textures::layers_of(session, coord, chunk).unwrap_or_default();
    for layer in &layers {
        let name = layer.texture.rsplit(['\\', '/']).next().unwrap_or(&layer.texture);
        if item(ui, &format!("Use {name}"), true, &format!("Put {} on the brush.", layer.texture), "") {
            editing.textures.brush.texture = layer.texture.clone();
            chosen = true;
        }
    }
    ui.separator();
    let pinned = editing.textures.pinned == Some((coord, chunk));
    if item(
        ui,
        match pinned {
            true => "Unpin chunk",
            false => "Pin chunk",
        },
        true,
        "Space. Keep this chunk's layers in the panel while the pointer moves.",
        "",
    ) {
        editing.textures.pinned = match pinned {
            true => None,
            false => Some((coord, chunk)),
        };
        chosen = true;
    }
    if item(ui, "Clear chunk to base", layers.len() > 1, "Remove every layer above this chunk's base.", "The chunk has only its base layer.") {
        crate::tools::textures::clear_paint(session, coord, chunk);
        chosen = true;
    }
    let texture = editing.textures.brush.texture.clone();
    if item(
        ui,
        "Set brush texture as chunk base",
        !texture.is_empty(),
        "Make the brush's texture this chunk's base layer.",
        "Choose a texture first.",
    ) {
        crate::tools::textures::set_base(session, coord, chunk, &texture);
        chosen = true;
    }
    if item(
        ui,
        "Set brush texture as tile base",
        !texture.is_empty(),
        "Make the brush's texture the base layer of every chunk of this tile.",
        "Choose a texture first.",
    ) {
        let n = crate::tools::textures::set_tile_base(session, coord, &texture);
        session.status = format!("base texture set on {n} chunks of tile {}, {}", coord.0, coord.1);
        chosen = true;
    }
    chosen
}

/// The area tool, about the clicked chunk: its area on the brush, its row,
/// and its impassable flag.
fn areas(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    editing: &mut Editing,
    tile: Option<(u32, u32)>,
    chunk: Option<usize>,
) -> bool {
    use crate::tools::areas::Ask;
    let mut chosen = false;
    let (Some(coord), Some(chunk)) = (tile, chunk) else {
        caption(ui, "The click was not on an open chunk.");
        return false;
    };
    let tile_file = session.tiles.get(&coord);
    let area = tile_file.and_then(|t| vale_edit::adt::area::area(t, chunk)).unwrap_or(0);
    let impassable = tile_file.is_some_and(|t| vale_edit::adt::impass::impassable(t, chunk));
    caption(ui, &format!("Chunk {chunk} of tile {}, {}: area {area}", coord.0, coord.1));
    if item(ui, "Use this area", area != 0, "Space. Put this chunk's area on the brush.", "The chunk has no area.") {
        editing.areas.brush.area = area;
        chosen = true;
    }
    if item(ui, "Edit this area", area != 0, "Open this chunk's AreaTable row in the Zones workspace.", "The chunk has no area.") {
        editing.areas.ask = Some(Ask::Edit(area));
        chosen = true;
    }
    if item(
        ui,
        match impassable {
            true => "Clear impassable",
            false => "Set impassable",
        },
        true,
        "Switch this chunk's impassable flag.",
        "",
    ) {
        crate::tools::areas::set_impassable(session, coord, chunk, !impassable);
        chosen = true;
    }
    chosen
}

/// The hole tool, about the clicked chunk.
fn holes(ui: &mut egui::Ui, session: &mut EditSession, tile: Option<(u32, u32)>, chunk: Option<usize>) -> bool {
    use crate::tools::chunks::Cell;
    let mut chosen = false;
    let (Some(coord), Some(chunk)) = (tile, chunk) else {
        caption(ui, "The click was not on an open chunk.");
        return false;
    };
    caption(ui, &format!("Chunk {chunk} of tile {}, {}", coord.0, coord.1));
    let cells: std::collections::BTreeSet<Cell> =
        [Cell::of(coord, ((chunk % 16) as u32, (chunk / 16) as u32))].into_iter().collect();
    if item(ui, "Cut the whole chunk", true, "Cut every hole square of this chunk.", "") {
        crate::tools::chunks::set_holes(session, &cells, true);
        chosen = true;
    }
    if item(ui, "Fill the whole chunk", true, "Fill every hole of this chunk.", "") {
        crate::tools::chunks::set_holes(session, &cells, false);
        chosen = true;
    }
    chosen
}

/// The grade tool: apply or clear the grade between its two points.
fn grade(ui: &mut egui::Ui, session: &mut EditSession, editing: &mut Editing) -> bool {
    let mut chosen = false;
    let ready = editing.grading.grade().and_then(|g| g.slope()).is_some();
    if item(ui, "Apply grade", ready, "Make the grade between the two points.", "Pick two different points first.") {
        let n = crate::tools::grade::apply(session, &editing.grading);
        session.status = format!("grade written to {n} chunks");
        chosen = true;
    }
    if item(ui, "Clear points", true, "Forget both points.", "") {
        editing.grading.clear();
        chosen = true;
    }
    chosen
}

fn road(ui: &mut egui::Ui, session: &mut EditSession, editing: &mut Editing, at: Option<Vec3>) -> bool {
    let mut chosen = false;
    let road = &mut editing.road;
    let ready = road.is_ready();
    if item(ui, "Apply road", ready, "Enter. Grade the ground along the road and paint it.", "Place at least two points first.") {
        crate::tools::road::apply_and_report(session, road);
        chosen = true;
    }
    ui.separator();
    caption(ui, "Points");
    let near = at.and_then(|at| road.point_near(at.x, at.y));
    if let Some(at) = at {
        if item(ui, "Add point here", true, "Add a point at the end of the road.", "") {
            road.push([at.x, at.y], at.z);
            chosen = true;
        }
        if item(
            ui,
            "Insert point here",
            road.points.len() >= 2,
            "Put a point into the segment nearest the click.",
            "Place at least two points first.",
        ) {
            road.insert([at.x, at.y], at.z);
            chosen = true;
        }
    }
    if item(ui, "Remove this point", near.is_some(), "Shift+click. Remove the point under the click.", "The click was not on a point.") {
        if let Some(i) = near {
            road.remove(i);
        }
        chosen = true;
    }
    if item(ui, "Remove last point", !road.points.is_empty(), "Backspace.", "There are no points.") {
        road.points.pop();
        chosen = true;
    }
    if item(ui, "Clear points", !road.points.is_empty(), "Remove every point.", "There are no points.") {
        road.clear();
        chosen = true;
    }
    chosen
}

/// Whether a spawn's `npc_flags` carries the bit vmangos names `name`.
fn has_npc_flag(npc_flags: u32, name: &str) -> bool {
    vale_mangos::creature::NPC_FLAGS
        .iter()
        .find(|flag| flag.name == name)
        .is_some_and(|flag| npc_flags & flag.bit != 0)
}

/// The selected spawns of the tool the menu is for, primary first, as the
/// project's edits leave them.
fn spawn_selection(tool: Tool, editing: &Editing, session: &EditSession) -> Vec<crate::tools::spawn::Standing> {
    let edits = Some(&session.server_edits);
    let mut all: Vec<_> = crate::tools::spawn::chosen(tool, &editing.creatures, &editing.objects, edits)
        .into_iter()
        .collect();
    all.extend(crate::tools::spawn::members(tool, &editing.creatures, &editing.objects, edits));
    all
}

/// Stand each spawn on the terrain under it: its `position_z` becomes the
/// height of the open tile's ground at its x and y, as one undo entry. A
/// spawn over no open tile is left. Answers how many moved.
fn drop_to_terrain(session: &mut EditSession, spawns: &[crate::tools::spawn::Standing], noun: &str, now: f64) -> usize {
    let Some(first) = spawns.first() else { return 0 };
    let label = format!("Drop {} {noun} to the terrain", spawns.len());
    let subject = format!("{noun} {} drop", first.guid);
    session.as_one(&label, &subject, |session| {
        let mut moved = 0;
        for spawn in spawns {
            let tile = vale_assets::tile_for_position(spawn.at.x, spawn.at.y);
            let ground = session
                .tiles
                .get(&tile)
                .and_then(|open| vale_edit::adt::heights::height_at(open, spawn.at.x, spawn.at.y));
            if let Some(z) = ground {
                spawn.move_to(session, Vec3::new(spawn.at.x, spawn.at.y, z), now);
                moved += 1;
            }
        }
        moved
    })
}

/// A creature spawn that was right-clicked: the windows about it, what its
/// keys do, and the selection.
fn creature(ui: &mut egui::Ui, session: &mut EditSession, editing: &mut Editing, guid: u64, now: f64) -> bool {
    use crate::tools::creatures::Creatures;
    use crate::tools::quests::Holder;
    use crate::tools::services::Kind as Service;
    let mut chosen = false;
    let Some(spawn) = editing.creatures.edited(guid, Some(&session.server_edits)) else {
        caption(ui, &format!("Creature {guid}, not on this map"));
        return false;
    };
    let count = 1 + editing.creatures.also.len();
    caption(
        ui,
        &match count {
            1 => format!("{} \u{b7} guid {guid}", spawn.label()),
            n => format!("{n} creatures selected"),
        },
    );
    if item(ui, "Edit creature\u{2026}", true, "Open creature_template's columns in a separate window.", "") {
        editing.creatures.template_window = true;
        chosen = true;
    }
    if item(
        ui,
        "Open in workspace",
        true,
        "Open this creature_template row in the Creatures workspace on the top bar.",
        "",
    ) {
        editing.creatures.open_listed(spawn.entry);
        editing.creatures.switch_to = Some(Tool::CreatureData);
        chosen = true;
    }
    if item(
        ui,
        "Waypoints\u{2026}",
        count == 1,
        "Open this spawn's path from creature_movement, drawn on the ground.",
        "A path belongs to one spawn; select one.",
    ) {
        editing.waypoints.showing = true;
        editing.waypoints.open_for(spawn.guid, spawn.entry, spawn.movement_type);
        chosen = true;
    }
    if item(ui, "Quests\u{2026}", true, "The quests this creature starts and ends.", "") {
        editing.quests.window_for = Some((Holder::Creature, spawn.entry, spawn.label()));
        chosen = true;
    }
    if item(ui, "Loot\u{2026}", true, "The creature's drop, pickpocket and skinning loot.", "") {
        if !editing.loot.open {
            editing.loot.toggle();
        }
        chosen = true;
    }
    if item(ui, "Events\u{2026}", true, "The creature's creature_ai_events.", "") {
        editing.behaviour.events_open = true;
        chosen = true;
    }
    if has_npc_flag(spawn.npc_flags, "GOSSIP")
        && item(ui, "Gossip\u{2026}", true, "The gossip menu the template's gossip_menu_id names.", "")
    {
        editing.gossip.open = true;
        editing.gossip.menu = None;
        editing.gossip.trail.clear();
        chosen = true;
    }
    if has_npc_flag(spawn.npc_flags, "VENDOR")
        && item(ui, "Vendor\u{2026}", true, "What the creature sells: npc_vendor and its vendor_id's list.", "")
    {
        if !editing.services.is_open(Service::Vendor) {
            editing.services.toggle(Service::Vendor);
        }
        chosen = true;
    }
    if has_npc_flag(spawn.npc_flags, "TRAINER")
        && item(ui, "Trainer\u{2026}", true, "What the creature teaches: npc_trainer and its trainer_id's list.", "")
    {
        if !editing.services.is_open(Service::Trainer) {
            editing.services.toggle(Service::Trainer);
        }
        chosen = true;
    }

    ui.separator();
    if item(
        ui,
        "Duplicate",
        count == 1,
        "Ctrl+D. Copy this spawn's row to a new guid two yards north.",
        "Ctrl+D duplicates one spawn at a time.",
    ) {
        editing.menu.request = Some(Request::Duplicate);
        chosen = true;
    }
    if spawn.is_removed() {
        if item(ui, "Cancel removal", true, "Keep the row. Column edits this project made to it remain.", "") {
            for standing in spawn_selection(Tool::Creatures, editing, session) {
                Creatures::keep(session, standing.guid, now);
            }
            session.status = format!("removal of {count} creature(s) cancelled");
            chosen = true;
        }
    } else {
        let label = match (count, spawn.is_new()) {
            (1, true) => "Discard",
            _ => "Remove",
        };
        if item(
            ui,
            label,
            true,
            "Delete. Mark the spawn for removal; Apply deletes the row, its path, its addon and \
             its game-event rows. A spawn this project created is discarded.",
            "",
        ) {
            editing.menu.request = Some(Request::Delete);
            chosen = true;
        }
    }
    if item(
        ui,
        "Drop to terrain",
        true,
        "Set position_z to the terrain height under each selected spawn. The terrain only: a \
         creature on a bridge or a building's floor is dropped through it.",
        "",
    ) {
        let spawns = spawn_selection(Tool::Creatures, editing, session);
        let n = drop_to_terrain(session, &spawns, "creature", now);
        session.status = format!("{n} creature(s) dropped to the terrain");
        chosen = true;
    }

    ui.separator();
    if item(
        ui,
        "Select all of this creature",
        true,
        "Add every spawn of this creature_template entry on this map to the selection.",
        "",
    ) {
        let creatures = &mut *editing.creatures;
        let more: Vec<u64> = (0..creatures.spawn_count())
            .filter_map(|index| creatures.base_spawn(index))
            .filter(|other| other.entry == spawn.entry && !creatures.holds(other.guid))
            .map(|other| other.guid)
            .collect();
        session.status = format!("{} more spawn(s) of {} selected", more.len(), spawn.label());
        creatures.also.extend(more);
        chosen = true;
    }
    let known = crate::tools::creatures::known_from(&spawn);
    if item(
        ui,
        "Place this creature",
        known.is_some(),
        "Switch to Place with this creature on the pointer.",
        "The creature's template has not been read.",
    ) {
        editing.creatures.mode = crate::tools::place::Mode::Place;
        editing.creatures.new_spawn.chosen = known;
        chosen = true;
    }
    if item(ui, "Copy guid", true, "Copy the spawn's guid to the clipboard.", "") {
        ui.ctx().copy_text(guid.to_string());
        chosen = true;
    }
    chosen
}

/// A game object spawn that was right-clicked, on [`creature`]'s terms.
fn game_object(ui: &mut egui::Ui, session: &mut EditSession, editing: &mut Editing, guid: u64, now: f64) -> bool {
    use crate::tools::gameobjects::GameObjects;
    use crate::tools::quests::Holder;
    let mut chosen = false;
    let Some(spawn) = editing.objects.edited(guid, Some(&session.server_edits)) else {
        caption(ui, &format!("Game object {guid}, not on this map"));
        return false;
    };
    let count = 1 + editing.objects.also.len();
    caption(
        ui,
        &match count {
            1 => format!("{} \u{b7} guid {guid}", spawn.label()),
            n => format!("{n} game objects selected"),
        },
    );
    if item(ui, "Edit object\u{2026}", true, "Open gameobject_template's columns in a separate window.", "") {
        editing.objects.template_window = true;
        chosen = true;
    }
    if item(
        ui,
        "Open in workspace",
        true,
        "Open this gameobject_template row in the Game Objects workspace on the top bar.",
        "",
    ) {
        editing.objects.open_listed(spawn.entry);
        editing.objects.switch_to = Some(Tool::GameObjectData);
        chosen = true;
    }
    if item(ui, "Quests\u{2026}", true, "The quests this object starts and ends.", "") {
        editing.quests.window_for = Some((Holder::Object, spawn.entry, spawn.label()));
        chosen = true;
    }
    if item(ui, "Loot\u{2026}", true, "The loot of a chest, vein, herb or fishing hole.", "") {
        if !editing.loot.open {
            editing.loot.toggle();
        }
        chosen = true;
    }

    ui.separator();
    if item(
        ui,
        "Duplicate",
        count == 1,
        "Ctrl+D. Copy this spawn's row to a new guid two yards north.",
        "Ctrl+D duplicates one spawn at a time.",
    ) {
        editing.menu.request = Some(Request::Duplicate);
        chosen = true;
    }
    if spawn.is_removed() {
        if item(ui, "Cancel removal", true, "Keep the row. Column edits this project made to it remain.", "") {
            for standing in spawn_selection(Tool::GameObjects, editing, session) {
                GameObjects::keep(session, standing.guid, now);
            }
            session.status = format!("removal of {count} game object(s) cancelled");
            chosen = true;
        }
    } else {
        let label = match (count, spawn.is_new()) {
            (1, true) => "Discard",
            _ => "Remove",
        };
        if item(
            ui,
            label,
            true,
            "Delete. Mark the spawn for removal; Apply deletes the row. A spawn this project \
             created is discarded.",
            "",
        ) {
            editing.menu.request = Some(Request::Delete);
            chosen = true;
        }
    }
    if item(
        ui,
        "Drop to terrain",
        true,
        "Set position_z to the terrain height under each selected spawn. The terrain only: an \
         object on a table, a wall or a building's floor is dropped through it.",
        "",
    ) {
        let spawns = spawn_selection(Tool::GameObjects, editing, session);
        let n = drop_to_terrain(session, &spawns, "gameobject", now);
        session.status = format!("{n} game object(s) dropped to the terrain");
        chosen = true;
    }

    ui.separator();
    if item(
        ui,
        "Select all of this object",
        true,
        "Add every spawn of this gameobject_template entry on this map to the selection.",
        "",
    ) {
        let objects = &mut *editing.objects;
        let more: Vec<u64> = (0..objects.spawn_count())
            .filter_map(|index| objects.base_spawn(index))
            .filter(|other| other.entry == spawn.entry && !objects.holds(other.guid))
            .map(|other| other.guid)
            .collect();
        session.status = format!("{} more spawn(s) of {} selected", more.len(), spawn.label());
        objects.also.extend(more);
        chosen = true;
    }
    if item(
        ui,
        "Place this object",
        spawn.known.is_some(),
        "Switch to Place with this object on the pointer.",
        "The spawn has no gameobject_template row.",
    ) {
        editing.objects.mode = crate::tools::place::Mode::Place;
        editing.objects.new_spawn.chosen = spawn.known.clone();
        chosen = true;
    }
    if item(ui, "Copy guid", true, "Copy the spawn's guid to the clipboard.", "") {
        ui.ctx().copy_text(guid.to_string());
        chosen = true;
    }
    chosen
}

/// The creature or game object tool, clicked on ground: place the chosen
/// row there, switch between Select and Place, and clear the selection.
fn spawns_ground(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    editing: &mut Editing,
    tool: Tool,
    at: Option<Vec3>,
    now: f64,
) -> bool {
    use crate::tools::place::Mode;
    let mut chosen = false;
    let (noun, count, mode, chosen_label) = match tool {
        Tool::Creatures => (
            "creatures",
            editing.creatures.selected.map_or(0, |_| 1 + editing.creatures.also.len()),
            editing.creatures.mode,
            editing.creatures.new_spawn.chosen.as_ref().map(|known| known.label()),
        ),
        _ => (
            "game objects",
            editing.objects.selected.map_or(0, |_| 1 + editing.objects.also.len()),
            editing.objects.mode,
            editing.objects.new_spawn.chosen.as_ref().map(|known| known.label()),
        ),
    };
    caption(ui, &format!("{count} {noun} selected"));
    let place_label = match &chosen_label {
        Some(label) => format!("Place {label} here"),
        None => "Place here".to_string(),
    };
    if item(
        ui,
        &place_label,
        chosen_label.is_some() && at.is_some(),
        "Create a spawn of the chosen row at the clicked point, facing as the one on the pointer.",
        "Choose a row in Place, and click on open ground.",
    ) {
        if let Some(at) = at {
            let made = match tool {
                Tool::Creatures => editing.creatures.new_spawn.chosen.clone().map(|known| {
                    let facing = editing.creatures.ghost_facing.unwrap_or(0.0);
                    editing.creatures.create(session, &known, at, facing, now)
                }),
                _ => editing.objects.new_spawn.chosen.clone().map(|known| {
                    let facing = editing.objects.ghost_facing.unwrap_or(0.0);
                    editing.objects.create(session, &known, at, facing, now)
                }),
            };
            if let Some(guid) = made {
                session.status = format!("placed as guid {guid}");
            }
        }
        chosen = true;
    }
    let (switch, to) = match mode {
        Mode::Select => ("Switch to Place", Mode::Place),
        Mode::Place => ("Switch to Select", Mode::Select),
    };
    if item(ui, switch, true, "The panel's Select and Place switch.", "") {
        match tool {
            Tool::Creatures => {
                editing.creatures.mode = to;
                if to == Mode::Select {
                    editing.creatures.new_spawn.chosen = None;
                } else {
                    // Only one mode may take the click; see the creature panel.
                    editing.waypoints.adding = false;
                }
            }
            _ => {
                editing.objects.mode = to;
                if to == Mode::Select {
                    editing.objects.new_spawn.chosen = None;
                }
            }
        }
        chosen = true;
    }
    if item(ui, "Deselect", count > 0, "Clear the selection.", "Nothing is selected.") {
        match tool {
            Tool::Creatures => editing.creatures.select_only(None),
            _ => editing.objects.select(None),
        }
        chosen = true;
    }
    chosen
}
