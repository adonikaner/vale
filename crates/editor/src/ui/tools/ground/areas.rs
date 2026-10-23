//! The areas tool's panel: the area under the pointer, the area tree, and
//! the impassable flag. The state is [`crate::tools::ground::areas`].

use bevy_egui::egui;

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::placements::place;
use crate::ui::widgets::folders::{breadcrumb, folder_rows, split, split_folder};
use crate::ui::widgets::theme;

/// Whether the server lets a character walk on the chunk under the pointer.
///
/// It is on the areas panel because it is the same gesture on the same unit:
/// one value per chunk, set by a click rather than painted, and invisible in
/// the viewport either way. A rail tile of its own for one checkbox would be
/// hard to find.
///
/// It is a server flag. vmangos reads `MCNK`'s `0x02` when it builds `mmaps`;
/// this client does not read it. Marking a hill changes where a character may
/// go once the server's maps are rebuilt from the edited files, and never
/// changes anything on screen. The panel states this, because otherwise the
/// checkbox appears to do nothing.
fn impassability(ui: &mut egui::Ui, session: &mut EditSession, cursor: &Cursor) {
    use crate::tools::ground::areas;

    theme::heading(ui, "Walkable");
    let Some((coord, chunk)) = cursor.tile.zip(cursor.chunk) else {
        theme::note(ui, "pointer is not over a chunk");
        ui.add_space(6.0);
        return;
    };
    let marked = session
        .tiles
        .get(&coord)
        .is_some_and(|tile| vale_edit::adt::impass::impassable(tile, chunk));

    theme::row(ui, "chunk", |ui| {
        ui.label(theme::number(format!("{},{} · {chunk}", coord.0, coord.1)));
    });
    let mut wanted = marked;
    if ui
        .checkbox(&mut wanted, "impassable")
        .on_hover_text(
            "MCNK flag 0x02. vmangos reads it when it builds mmaps, so it decides where a \
             character may walk once the server's maps are rebuilt. This client does not \
             read it, so the viewport does not change.",
        )
        .changed()
    {
        areas::set_impassable(session, coord, chunk, wanted);
    }
    if let Some(count) = areas::impassable_count(session, coord) {
        theme::note(ui, format!("{count} of 256 chunks marked on this tile"));
    }
    theme::note(
        ui,
        "No shipped 1.12 tile sets this bit, measured over 1,024 chunks.",
    );
    ui.add_space(6.0);
}

/// The zone tool: where the pointer is, what it would paint, and the area tree.
///
/// This panel carries more of its tool than any other panel does. An area id
/// changes nothing on screen, so everything a person needs in order to decide
/// (what the chunk is now, which zone that belongs to, what the tile is made
/// of, what a press would write) has to be stated in words. See
/// [`crate::tools::ground::areas`].
pub(in crate::ui::tools) fn zones(
    ui: &mut egui::Ui,
    areas: &mut crate::tools::ground::areas::Areas,
    session: &mut EditSession,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    cursor: &Cursor,
    asks: &mut crate::ui::shell::asks::ShellAsks,
) {
    impassability(ui, session, cursor);
    // The session's own copy of the table while it has one open, so an area
    // made or renamed in the Zones workspace is listed here before a save.
    let edited = areas.table.clone();
    let table = edited
        .as_deref()
        .or_else(|| tables.and_then(|tables| tables.areas()));
    // The tree opens on the map being edited. Its top level is `AreaTable`'s
    // thirty-six maps, most of them instances that a continent edit does not
    // need; opening there would add a click to every use. `all` on the
    // breadcrumb goes up to them.
    if areas.folder.is_empty() {
        if let Some(map) = tables
            .map(|tables| tables.map_name(session.map_id))
            .filter(|name| !name.is_empty())
        {
            areas.folder = format!("{}{}", map.to_ascii_lowercase(), place::SEP);
        }
    }
    // An area's own name, and the zone it is part of. `0` is a valid value
    // (a chunk that belongs to no area) and the shipped tiles contain it, so
    // it is named rather than shown as an error.
    let name_of = |id: u32| area_name(table, id);

    theme::heading(ui, "Under the pointer");
    match areas.at {
        Some((tile, chunk)) => {
            theme::row(ui, "tile", |ui| {
                ui.label(theme::number(format!("{},{}", tile.0, tile.1)));
            });
            theme::row(ui, "chunk", |ui| {
                ui.label(theme::number(chunk.to_string()));
            });
            let here = areas.under.unwrap_or(0);
            ui.label(egui::RichText::new(name_of(here)).color(theme::INK));
            // The zone the area sits in, which is the name the interface
            // shows: `GetZoneText` is the parent and `GetSubZoneText` is the
            // row itself, so a subzone shows both.
            if let Some(zone) = table.and_then(|table| table.zone_of(here)) {
                if zone.id != here {
                    theme::note(ui, format!("in {}", zone.name));
                }
            }
        }
        None => theme::note(ui, "pointer is not over an open tile"),
    }

    ui.add_space(4.0);
    theme::heading(ui, "Brush area");
    ui.label(egui::RichText::new(name_of(areas.brush.area)).color(theme::INK));
    theme::note(ui, "space takes the area under the pointer");
    // What the area on the brush is, and the two ways to make one. The rows
    // are AreaTable's, which the Zones workspace edits; the shell carries
    // each request out after the panel is drawn. See `tools::ground::areas::Ask`.
    {
        use crate::tools::ground::areas::Ask;
        let on_brush = table.and_then(|table| table.get(areas.brush.area));
        let zone = table
            .and_then(|table| table.zone_of(areas.brush.area))
            .map(|zone| zone.name.clone());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(on_brush.is_some(), egui::Button::new("Edit\u{2026}").small())
                .on_hover_text(
                    "Open the brush's area in the Zones workspace to edit its name, parent \
                     zone, music, ambience, flags and exploration.",
                )
                .on_disabled_hover_text("AreaTable has no row for the area on the brush.")
                .clicked()
            {
                asks.push(crate::ui::shell::asks::ShellAsk::Area(Ask::Edit(areas.brush.area)));
            }
            if ui
                .add_enabled(on_brush.is_some(), egui::Button::new("+ Sub-area").small())
                .on_hover_text(match &zone {
                    Some(zone) => format!(
                        "Create a sub-area of {zone} with that zone's flags, ambience and \
                         music, and put it on the brush. Name it with Edit\u{2026}."
                    ),
                    None => "Create a sub-area of the zone on the brush.".to_string(),
                })
                .on_disabled_hover_text("Put a zone or one of its areas on the brush first.")
                .clicked()
            {
                asks.push(crate::ui::shell::asks::ShellAsk::Area(Ask::NewSubArea(areas.brush.area)));
            }
            if ui
                .small_button("+ Zone")
                .on_hover_text(
                    "Create a zone on this map and put it on the brush. Name it with Edit\u{2026}.",
                )
                .clicked()
            {
                asks.push(crate::ui::shell::asks::ShellAsk::Area(Ask::NewZone));
            }
        });
    }
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut areas.brush.radius)
                .speed(0.5)
                .range(crate::tools::ground::areas::RADIUS)
                .suffix(" yd"),
        );
    });
    theme::note(
        ui,
        format!("{} chunk(s) under the brush", areas.covered.len()),
    );

    ui.add_space(4.0);
    theme::heading(ui, "Area");
    if let Some(table) = table {
        if let Some(picked) = area_tree(ui, areas, table, tables) {
            areas.brush.area = picked;
        }
    } else {
        theme::note(ui, "no AreaTable.dbc: the archives did not open");
        theme::row(ui, "id", |ui| {
            ui.add(egui::DragValue::new(&mut areas.brush.area).speed(1.0));
        });
    }

    ui.add_space(4.0);
    theme::heading(ui, "Tile");
    // The areas the tile is made of. The viewport cannot show this, because
    // an area's extent is invisible.
    match areas.at.and_then(|(tile, _)| session.tiles.get(&tile)) {
        Some(tile) => {
            let census = vale_edit::adt::area::census(tile);
            theme::note(ui, format!("{} area(s) across 256 chunks", census.len()));
            for (id, count) in census.iter().take(8) {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(name_of(*id))
                            .size(theme::SMALL)
                            .color(theme::INK_DIM),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(count.to_string())
                                .size(theme::SMALL)
                                .color(theme::INK_FAINT),
                        );
                    });
                });
            }
        }
        None => theme::note(ui, "no tile open under the pointer"),
    }

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button paints · ctrl + wheel resizes");
    theme::note(ui, "space takes the area under the pointer");
    theme::note(
        ui,
        "area ids are per chunk (33 yd); the file has no finer unit",
    );
    theme::note(ui, "a playtest shows this area as the zone text");
}

/// `AreaTable`'s own hierarchy as a tree. Returns the row a click chose.
///
/// The table has 1,081 rows and is already a tree: a zone has no parent, a
/// subzone names the zone it is in, and every row names a map. The picker is
/// the same browser the model and tileset pickers use, over paths built from
/// those three: `Azeroth / Elwynn Forest / Goldshire`. The shape comes from
/// the table; only the separator is this editor's.
///
/// Search runs flat across every row, as in the other two pickers: a search
/// limited to the open folder would find nothing for a zone on another
/// continent.
pub(super) fn area_tree(
    ui: &mut egui::Ui,
    areas: &mut crate::tools::ground::areas::Areas,
    table: &vale_assets::tables::area::Areas,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
) -> Option<u32> {
    // The paths, and the id each one leads to. They are rebuilt every frame
    // over about a thousand rows: two string joins per row, far less than the
    // panel's own layout costs.
    let mut paths: Vec<String> = Vec::new();
    let mut ids: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for area in table.iter() {
        let map = tables
            .map(|tables| tables.map_name(area.map))
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string())
            .unwrap_or_else(|| format!("map {}", area.map));
        let mut path = format!("{map}{}", place::SEP);
        // One level of parent, which is all the shipped table has: a row is
        // either a zone or a subzone of one.
        if let Some(zone) = table.zone_of(area.id).filter(|zone| zone.id != area.id) {
            path.push_str(&zone.name);
            path.push(place::SEP);
        }
        path.push_str(&area.name);
        // The id is kept beside the path rather than parsed back out of it,
        // because two rows can share a name: `Deathknell` is in two maps.
        let path = path.to_ascii_lowercase();
        let path = match ids.contains_key(&path) {
            true => format!("{path} #{}", area.id),
            false => path,
        };
        ids.insert(path.clone(), area.id);
        paths.push(path);
    }
    paths.sort();

    ui.add(
        egui::TextEdit::singleline(&mut areas.search)
            .hint_text("search the areas")
            .desired_width(f32::INFINITY),
    );
    let needle = areas.search.to_ascii_lowercase();
    let chosen = paths
        .iter()
        .find(|path| ids.get(*path) == Some(&areas.brush.area))
        .cloned()
        .unwrap_or_default();

    if !needle.is_empty() {
        let matches: Vec<String> = paths
            .iter()
            .filter(|path| path.contains(&needle))
            .cloned()
            .collect();
        theme::note(ui, format!("{} of {} match", matches.len(), paths.len()));
        return text_rows(ui, "areas", &matches, &chosen, true)
            .and_then(|path| ids.get(&path).copied());
    }

    let here = areas.folder.clone();
    // `all` names the level above the maps; see [`breadcrumb`].
    let mut go = breadcrumb(ui, &here, "all");
    let (folders, files) = split_folder(&paths, &here);
    theme::note(
        ui,
        match folders.len() {
            0 => format!("{} areas", files.len()),
            n => format!("{n} folders · {} areas", files.len()),
        },
    );
    let row = ui.spacing().interact_size.y + 6.0;
    let mut picked: Option<String> = None;
    egui::ScrollArea::vertical()
        .id_salt("area-tree")
        .max_height(260.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            go = folder_rows(ui, &here, &folders, row).or(go.take());
            picked = text_rows(ui, "area-here", &files, &chosen, false);
        });
    if let Some(folder) = go {
        areas.folder = folder;
    }
    picked.and_then(|path| ids.get(&path).copied())
}

/// An area's name with its id. `0` is a chunk that belongs to no area, which
/// the shipped tiles contain, so it is named. An id `AreaTable` has no row
/// for is shown as the number.
pub(super) fn area_name(table: Option<&vale_assets::tables::area::Areas>, id: u32) -> String {
    match (id, table.and_then(|table| table.get(id))) {
        (0, _) => "no area (0)".to_string(),
        (id, Some(area)) => format!("{} ({id})", area.name),
        (id, None) => format!("area {id}"),
    }
}

/// A run of named rows with no picture, used by the area tree. Returns the one
/// a click chose. The folder is drawn under the name only in search results;
/// inside one folder every row would repeat it.
fn text_rows(
    ui: &mut egui::Ui,
    salt: &str,
    paths: &[String],
    chosen: &str,
    scroller: bool,
) -> Option<String> {
    let row = ui.spacing().interact_size.y + 6.0;
    let mut picked: Option<String> = None;
    let mut draw = |ui: &mut egui::Ui, range: std::ops::Range<usize>| {
        for path in &paths[range] {
            let (folder, file) = split(path);
            let on = chosen.eq_ignore_ascii_case(path);
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), row - 2.0),
                egui::Sense::click(),
            );
            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                if on || response.hovered() {
                    painter.rect_filled(
                        rect,
                        egui::CornerRadius::same(3),
                        match on {
                            true => theme::ACCENT_SUNK,
                            false => theme::RAISED,
                        },
                    );
                }
                let text = painter.with_clip_rect(rect);
                let colour = match on {
                    true => theme::INK,
                    false => theme::INK_DIM,
                };
                match scroller {
                    true => {
                        text.text(
                            egui::pos2(rect.left() + 5.0, rect.center().y - 7.0),
                            egui::Align2::LEFT_CENTER,
                            file,
                            egui::FontId::proportional(12.0),
                            colour,
                        );
                        text.text(
                            egui::pos2(rect.left() + 5.0, rect.center().y + 7.0),
                            egui::Align2::LEFT_CENTER,
                            folder,
                            egui::FontId::proportional(theme::SMALL),
                            theme::INK_FAINT,
                        );
                    }
                    false => {
                        text.text(
                            egui::pos2(rect.left() + 5.0, rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            file,
                            egui::FontId::proportional(12.0),
                            colour,
                        );
                    }
                }
            }
            if response.on_hover_text(path.clone()).clicked() {
                picked = Some(path.clone());
            }
        }
    };
    match scroller {
        true => {
            egui::ScrollArea::vertical()
                .id_salt(salt)
                .max_height(300.0)
                .auto_shrink([false, true])
                .show_rows(ui, row, paths.len(), |ui, range| draw(ui, range));
        }
        false => draw(ui, 0..paths.len()),
    }
    picked
}

/// The area tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, tables, cursor, .. } = subject;
    zones(ui, &mut editing.areas, session, tables, cursor, &mut editing.asks);
}
