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
                chosen = items(ui, open, session, editing, assets);
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
        _ => false,
    };
    // What every menu ends with: the clicked point, and the history.
    let at = match open.target {
        Target::Ground { at, .. } => at,
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
        "Copy the clicked point's x, y and z to the clipboard.",
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
        "Copy a GM command that teleports a character to the clicked point on this map.",
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
