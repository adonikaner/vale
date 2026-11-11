//! The chunks tool's panel and its pages: copy and paste, stitching, areas,
//! holes, textures, foliage and buried water over a selection of chunks. The state is
//! [`crate::tools::ground::chunks`].

use bevy_egui::egui;

use crate::preview::portraits::Portraits;
use crate::session::EditSession;
use crate::tools::ground::foliage::{Foliage, Target};
use crate::tools::ground::textures::Textures;
use crate::tools::ground::textures;
use crate::tools::placements::place;
use crate::ui::tools::ground::areas::{area_name, area_tree};
use crate::ui::tools::ground::textures::{swatch, tileset_picker};
use crate::ui::widgets::theme::{self, field, property};
use crate::ui::widgets::thumbnails::Thumbnails;

/// How many of a census's rows the chunk panel lists before it says how many
/// more there are.
const CENSUS_ROWS: usize = 6;

/// The chunk panel's pages, in the order the control lists them, in two rows
/// of four and three. Seven in one row at the inspector's width cut "Foliage"
/// short.
const PAGES: [&[(&str, crate::tools::ground::chunks::Page)]; 2] = [
    &[
        ("Paste", crate::tools::ground::chunks::Page::Paste),
        ("Stitch", crate::tools::ground::chunks::Page::Stitch),
        ("Area", crate::tools::ground::chunks::Page::Area),
        ("Holes", crate::tools::ground::chunks::Page::Holes),
    ],
    &[
        ("Paint", crate::tools::ground::chunks::Page::Textures),
        ("Foliage", crate::tools::ground::chunks::Page::Foliage),
        ("Water", crate::tools::ground::chunks::Page::Water),
    ],
];

/// How the selection is made and the tool's keys, for the toolbar's Keys
/// button. The tool has no mode and no switch, so its toolbar over the
/// viewport is that button alone ([`theme::toolbar_keys`]).
pub(crate) const KEYS: [(&str, &str); 8] = [
    ("Left button", "Select a chunk"),
    ("Drag", "Select a block"),
    ("Shift", "Add to the selection"),
    ("Shift + click a selected chunk", "Remove it"),
    ("Ctrl + A", "Select the tile under the pointer"),
    ("Escape", "Deselect"),
    ("Ctrl + C", "Copy"),
    ("Ctrl + V", "Paste at the pointer"),
];

/// What Stitch does, for its button's hover.
const STITCH_ABOUT: &str = "Joins the selection's border to the ground around it. Every vertex on the \
                            border is moved to one height, and the ground within the reach distance \
                            blends toward it on the side that moves.\n\n\
                            One undo entry. Doodads, WMOs and spawns on the moved ground are not moved.";

/// `n` with `noun`, pluralised by adding an s.
fn counted(n: usize, noun: &str) -> String {
    match n {
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// The chunk tool: what is selected, and the operations on it, one page at a
/// time. See [`crate::tools::ground::chunks`].
///
/// The selection's count and the buttons that grow or drop it are above the
/// pages, since every page acts on the selection. The pages are the
/// operations grouped by what they write: a paste, a stitch, the area id with
/// the impassable flag, the holes, the textures, the foliage, the water. One page is open at a time,
/// so the panel is about a screen tall whichever is open.
///
/// The area and the texture an operation writes are the area brush's and the
/// texture brush's chosen values, picked with the pickers those two panels
/// draw. A second chosen area or texture kept by this tool would be a second
/// answer to which one is chosen.
///
/// How to select and the tool's keys are on the toolbar's Keys button. A
/// page has no heading of its own name: the lit segment names it.
///
/// The line about the chunk under the pointer is last, as in the measuring
/// panel: it changes as the pointer moves on and off the world, and above the
/// buttons it would move the buttons under the pointer.
pub(in crate::ui::tools) fn chunks(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::ground::chunks::Chunks,
    areas: &mut crate::tools::ground::areas::Areas,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    assets: &vale_client::assets::GameAssets,
    foliage: &mut Foliage,
    portraits: &mut Portraits,
) {
    use crate::tools::ground::chunks as tool;

    let census = chunks.census(session).clone();
    let edited = areas.table.clone();
    let table = edited
        .as_deref()
        .or_else(|| tables.and_then(|tables| tables.areas()));
    let any = census.open > 0;

    theme::heading(ui, "Selection");
    property(ui, "Selected", "", |ui| {
        theme::value(
            ui,
            match census.open + census.closed {
                0 => "None".to_string(),
                _ => format!("{} in {}", counted(census.open, "chunk"), counted(census.tiles, "tile")),
            },
        );
    });
    if census.closed > 0 {
        theme::note(
            ui,
            format!(
                "{} more in tiles that are not open; operations skip them",
                census.closed
            ),
        );
    }
    ui.columns(2, |columns| {
        let button = theme::full_button(&columns[0], "Whole Tiles");
        if columns[0]
            .add_enabled(any, button)
            .on_hover_text("Select every chunk of each tile the selection touches.")
            .clicked()
        {
            let cells = chunks.whole_tiles(session);
            chunks.select(cells);
        }
        let button = theme::full_button(&columns[1], "Deselect");
        if columns[1]
            .add_enabled(census.open + census.closed > 0, button)
            .on_hover_text("Escape. Clear the selection.")
            .clicked()
        {
            chunks.clear();
        }
    });
    // On a line of its own and cut with an ellipsis: an area's name can be
    // wider than half the panel.
    if let Some(area) = chunks.primary.and_then(|cell| tool::area_of(session, cell)) {
        let button = theme::full_button(ui, &format!("Add All in {}", area_name(table, area))).truncate();
        if ui
            .add(button)
            .on_hover_text(
                "Add every chunk of the open tiles that has the area of the chunk \
                 last clicked.",
            )
            .clicked()
        {
            chunks.add(tool::Chunks::in_area(session, area));
        }
    }

    ui.add_space(8.0);
    for row in &PAGES {
        theme::segmented(ui, &mut chunks.page, row, |a, b| a == b);
    }
    ui.add_space(6.0);
    match chunks.page {
        tool::Page::Paste => chunks_paste(ui, chunks, session, any),
        tool::Page::Stitch => chunks_stitch(ui, chunks, session, &census),
        tool::Page::Area => chunks_area(ui, chunks, areas, session, tables, &census),
        tool::Page::Holes => chunks_holes(ui, chunks, session, &census),
        tool::Page::Textures => chunks_textures(ui, chunks, textures, thumbnails, session, &census),
        tool::Page::Foliage => chunks_foliage(ui, &chunks.selected, thumbnails, session, assets, foliage, portraits, &census),
        tool::Page::Water => chunks_water(ui, chunks, session, &census),
    }

    ui.add_space(10.0);
    theme::note(
        ui,
        match chunks.at {
            Some(cell) => {
                let (tile, within) = (cell.tile(), cell.within());
                let area = tool::area_of(session, cell)
                    .map(|area| format!(" \u{b7} {}", area_name(table, area)))
                    .unwrap_or_default();
                format!(
                    "Under the pointer: tile {},{} \u{b7} chunk {},{}{area}",
                    tile.0, tile.1, within.0, within.1
                )
            }
            None => "Pointer is not over an open tile".to_string(),
        },
    );
}

/// The Paste page: copy, what a paste writes, and at what height.
fn chunks_paste(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::ground::chunks::Chunks,
    session: &mut EditSession,
    any: bool,
) {
    use crate::tools::ground::chunks as tool;

    property(
        ui,
        "Copied",
        "Ctrl + V pastes centred on the chunk under the pointer. Hold Ctrl to preview.",
        |ui| {
            theme::value(
                ui,
                match chunks.clip.is_empty() {
                    true => "None".to_string(),
                    false => format!(
                        "{}, {} by {}",
                        counted(chunks.clip.chunks.len(), "chunk"),
                        chunks.clip.size.0,
                        chunks.clip.size.1
                    ),
                },
            );
        },
    );
    // Copy, and the copied block turned before it is pasted. Each press of
    // Turn or Mirror turns what is held, so two mirrors or four quarter turns
    // are the block as copied. The padding is reduced for the three: a third
    // of the inspector at its narrowest is 68 points, and the middle label
    // at the usual padding is about that wide.
    ui.scope(|ui| {
        ui.spacing_mut().button_padding.x = 4.0;
        paste_buttons(ui, chunks, session, any);
    });

    theme::heading(ui, "Paste Includes");
    // A part switched off is left as the ground under the paste has it. Two
    // columns of three, so the six boxes line up.
    let parts = &mut chunks.parts;
    ui.columns(2, |columns| {
        for (at, (label, on, about)) in [
            ("Heights", &mut parts.heights, "MCVT. The normals are recomputed."),
            ("Textures", &mut parts.textures, "The layers and their blend maps."),
            ("Shading", &mut parts.shading, "MCCV, the colour painted onto the vertices."),
            ("Holes", &mut parts.holes, "The hole mask."),
            ("Water", &mut parts.water, "MCLQ, at the level it was copied at."),
            ("Area", &mut parts.area, "The area id and the impassable flag."),
        ]
        .into_iter()
        .enumerate()
        {
            columns[at / 3].checkbox(on, label).on_hover_text(about);
        }
    });

    theme::heading(ui, "Height");
    property(
        ui,
        "Level",
        "Absolute: heights are pasted as they were copied. Relative: heights are moved to the \
         level of the ground they replace; water is not moved.",
        |ui| {
            theme::choice(
                ui,
                "chunks-paste-level",
                &mut chunks.level,
                &[
                    ("Absolute", tool::Level::Absolute),
                    ("Relative", tool::Level::Relative),
                ],
                |a, b| a == b,
            );
        },
    );
    ui.checkbox(&mut chunks.stitch_pasted, "Stitch each paste to the ground around it")
        .on_hover_text(
            "After a paste that writes heights, the Stitch page's settings are applied \
             to the pasted block, in the paste's own undo entry.",
        );
}

/// The Paste page's row of three equal buttons: Copy, Turn 90 degrees and
/// Mirror.
fn paste_buttons(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::ground::chunks::Chunks,
    session: &mut EditSession,
    any: bool,
) {
    use crate::tools::ground::chunks as tool;

    ui.columns(3, |columns| {
        let button = theme::full_button(&columns[0], "Copy");
        if columns[0]
            .add_enabled(any, button)
            .on_hover_text(
                "Ctrl+C. Copy the selected chunks: heights, textures, shading, holes, \
                 water, area and the impassable flag.",
            )
            .on_disabled_hover_text("Select chunks first.")
            .clicked()
        {
            chunks.clip = tool::copy(session, &chunks.selected);
            session.status = format!("Copied {}", counted(chunks.clip.chunks.len(), "chunk"));
        }
        let held = !chunks.clip.is_empty();
        for (column, (label, how, about)) in columns[1..].iter_mut().zip([
            (
                "Turn 90\u{b0}",
                tool::Turn::Clockwise,
                "Turn the copied block a quarter turn clockwise, seen from above: heights, \
                 blends, shading, holes and water together. Four presses restore the \
                 original orientation.",
            ),
            (
                "Mirror",
                tool::Turn::Mirror,
                "Swap the copied block's east and west. A second press swaps them back.",
            ),
        ]) {
            let button = theme::full_button(column, label);
            if column
                .add_enabled(held, button)
                .on_hover_text(about)
                .on_disabled_hover_text("Copy chunks first.")
                .clicked()
            {
                chunks.clip = chunks.clip.turned(how);
                session.status = format!(
                    "The copied block is now {} by {}",
                    chunks.clip.size.0, chunks.clip.size.1
                );
            }
        }
    });
}

/// The Stitch page: what the border meets, which side moves, how far the
/// blend reaches, and the button. What a stitch does is on the button's
/// hover, [`STITCH_ABOUT`].
fn chunks_stitch(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::ground::chunks::Chunks,
    session: &mut EditSession,
    census: &crate::tools::ground::chunks::Census,
) {
    use crate::tools::ground::chunks as tool;

    // One value and its step were one line, which is wider than a row, so
    // the step has a row of its own while the border meets open ground.
    property(ui, "Border", "", |ui| {
        theme::value(
            ui,
            match census.border.sides {
                0 => "No open ground".to_string(),
                n => counted(n, "side"),
            },
        )
        .on_hover_text("The sides of selected chunks that meet an unselected chunk in an open tile.");
    });
    if census.border.sides > 0 {
        property(ui, "Tallest Step", "", |ui| {
            ui.label(egui::RichText::new(format!("{:.1} y", census.border.step)).color(
                match census.border.step > 0.5 {
                    true => theme::WARN,
                    false => theme::INK,
                },
            ))
            .on_hover_text("The largest difference between the two heights stored for one vertex on the border.");
        });
    }
    property(
        ui,
        "Moving Side",
        "Selection: the selected chunks move to meet the ground around them. Ground: the ground \
         around the selection moves to meet it. Halfway: both move halfway.",
        |ui| {
            theme::choice(
                ui,
                "chunks-stitch-yields",
                &mut chunks.stitch.yields,
                &[
                    ("Selection", tool::Yields::Selection),
                    ("Ground", tool::Yields::Ground),
                    ("Halfway", tool::Yields::Both),
                ],
                |a, b| a == b,
            );
        },
    );
    property(ui, "Reach", "", |ui| {
        field(
            ui,
            egui::DragValue::new(&mut chunks.stitch.reach)
                .range(0.0..=tool::FURTHEST)
                .fixed_decimals(0)
                .suffix(" yd"),
        )
        .on_hover_text(
            "How far from the border the blend reaches, on each side that moves. \
             Zero moves the border's vertices and nothing else. A chunk is 33 yards.",
        );
    });

    ui.add_space(6.0);
    let ready = census.open > 0 && census.border.sides > 0;
    ui.add_enabled_ui(ready, |ui| {
        if theme::primary(ui, &format!("Stitch {}", counted(census.open, "Chunk")))
            .on_hover_text(STITCH_ABOUT)
            .on_disabled_hover_text(STITCH_ABOUT)
            .clicked()
        {
            let moved = tool::stitch(session, &chunks.selected, chunks.stitch);
            session.status = match moved {
                0 => "Nothing moved: the border already meets the ground".to_string(),
                n => format!("Stitched: {n} chunks moved"),
            };
        }
    });
    if !ready {
        theme::note(ui, "Select chunks whose border meets open ground.");
    }
}

/// The Area page: the area id written to the selection, and the impassable
/// flag beside it, which nothing on screen reads either.
fn chunks_area(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::ground::chunks::Chunks,
    areas: &mut crate::tools::ground::areas::Areas,
    session: &mut EditSession,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    census: &crate::tools::ground::chunks::Census,
) {
    use crate::tools::ground::chunks as tool;

    let edited = areas.table.clone();
    let table = edited
        .as_deref()
        .or_else(|| tables.and_then(|tables| tables.areas()));
    let any = census.open > 0;
    // The button is laid out first, from the row's right end, so the name
    // has what is left and is cut with an ellipsis when it is longer.
    property(ui, "Area", "", |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(any, egui::Button::new("Set"))
                .on_hover_text(
                    "Write this area id to every selected chunk, as one undo entry. Nothing \
                     on screen is drawn from an area id; a playtest reads it for its zone text.",
                )
                .on_disabled_hover_text("Select chunks first.")
                .clicked()
            {
                let changed = tool::set_area(session, &chunks.selected, areas.brush.area);
                session.status = format!("Area {} on {}", areas.brush.area, counted(changed, "chunk"));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                theme::value(ui, area_name(table, areas.brush.area));
            });
        });
    });
    // The areas the selection is made of. `Use` takes one as the area to
    // write, which is this panel's form of the area tool's Space.
    for (id, count) in census.areas.iter().take(CENSUS_ROWS) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(area_name(table, *id))
                    .size(theme::SMALL)
                    .color(theme::INK_DIM),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(*id != areas.brush.area, egui::Button::new("Use").small())
                    .on_hover_text("Put this area on the brush, as the area Set writes.")
                    .clicked()
                {
                    areas.brush.area = *id;
                }
                ui.label(
                    egui::RichText::new(count.to_string())
                        .size(theme::SMALL)
                        .color(theme::INK_FAINT),
                );
            });
        });
    }
    if census.areas.len() > CENSUS_ROWS {
        theme::note(
            ui,
            format!("{} more areas", census.areas.len() - CENSUS_ROWS),
        );
    }
    // Closed at first, which `theme::fold` cannot be, so this stays egui's
    // own folding header.
    egui::CollapsingHeader::new(egui::RichText::new("Areas").color(theme::INK_DIM))
        .id_salt("chunks-area-picker")
        .show(ui, |ui| match table {
            Some(table) => {
                // The tree opens on the map being edited, as the area tool's
                // does.
                if areas.folder.is_empty() {
                    if let Some(map) = tables
                        .map(|tables| tables.map_name(session.map_id))
                        .filter(|name| !name.is_empty())
                    {
                        areas.folder = format!("{}{}", map.to_ascii_lowercase(), place::SEP);
                    }
                }
                if let Some(picked) = area_tree(ui, areas, table, tables) {
                    areas.brush.area = picked;
                }
            }
            None => {
                theme::note(ui, "No AreaTable.dbc: the archives did not open");
                property(ui, "ID", "", |ui| {
                    field(ui, egui::DragValue::new(&mut areas.brush.area).speed(1.0));
                });
            }
        });

    theme::heading(ui, "Walkable");
    property(ui, "Impassable", "", |ui| {
        theme::value(ui, format!("{} of {}", census.impassable, counted(census.open, "chunk")));
    });
    ui.columns(2, |columns| {
        for (column, (label, on)) in columns.iter_mut().zip([("Impassable", true), ("Passable", false)]) {
            let button = theme::full_button(column, label);
            if column
                .add_enabled(any, button)
                .on_hover_text(
                    "MCNK flag 0x02 on every selected chunk. vmangos reads it when it \
                     builds mmaps. This client does not read it, so the viewport does \
                     not change.",
                )
                .clicked()
            {
                let changed = tool::set_impassable(session, &chunks.selected, on);
                session.status = format!("Marked {} on {}", label.to_lowercase(), counted(changed, "chunk"));
            }
        }
    });
}

/// The Holes page: all sixteen squares of every selected chunk, cut or put
/// back.
fn chunks_holes(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::ground::chunks::Chunks,
    session: &mut EditSession,
    census: &crate::tools::ground::chunks::Census,
) {
    use crate::tools::ground::chunks as tool;

    let any = census.open > 0;
    property(ui, "Squares Cut", "To cut or fill one square at a time, use the Holes tool.", |ui| {
        theme::value(ui, format!("{} of {}", census.cut, census.open * 16));
    });
    ui.columns(2, |columns| {
        for (column, (label, cut)) in columns.iter_mut().zip([("Cut", true), ("Fill", false)]) {
            let button = theme::full_button(column, label);
            if column
                .add_enabled(any, button)
                .on_hover_text(match cut {
                    true => "Cut all sixteen squares of every selected chunk.",
                    false => "Fill all sixteen squares of every selected chunk.",
                })
                .clicked()
            {
                let changed = tool::set_holes(session, &chunks.selected, cut);
                session.status = match cut {
                    true => format!("Cut holes in {}", counted(changed, "chunk")),
                    false => format!("Filled holes in {}", counted(changed, "chunk")),
                };
            }
        }
    });
}

/// The Water page: the liquid cells the ground covers entirely, and the
/// button that removes them.
fn chunks_water(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::ground::chunks::Chunks,
    session: &mut EditSession,
    census: &crate::tools::ground::chunks::Census,
) {
    use crate::tools::ground::chunks as tool;

    property(
        ui,
        "Liquid Cells",
        "A liquid cell is buried when the ground is above its surface at all four corners and \
         at the centre, so no part of it can be seen. A cell at a shore that the ground covers \
         only in part is kept, and so is a cell under a hole.",
        |ui| {
            ui.label(
                egui::RichText::new(format!("{} wet, {} buried", census.wet, census.buried)).color(
                    match census.buried > 0 {
                        true => theme::WARN,
                        false => theme::INK,
                    },
                ),
            )
            .on_hover_text(
                "Cells of MCLQ's 8x8 grid in the selected chunks, counted once for each \
                 liquid on the cell.",
            );
        },
    );
    ui.add_space(6.0);
    let about = "One undo entry. A chunk left with no wet cell loses its liquid.";
    ui.add_enabled_ui(census.buried > 0, |ui| {
        if theme::primary(ui, &format!("Remove {}", counted(census.buried, "Buried Cell")))
            .on_hover_text(about)
            .on_disabled_hover_text(about)
            .clicked()
        {
            let (dried, changed) = tool::drain_buried(session, &chunks.selected);
            session.status = format!("Removed {} from {}", counted(dried, "buried liquid cell"), counted(changed, "chunk"));
        }
    });
}

/// The Paint page: the chosen texture as a base, the layers cleared, and the
/// textures the selection carries with a move to the base, a swap and a
/// remove each.
fn chunks_textures(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::ground::chunks::Chunks,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    census: &crate::tools::ground::chunks::Census,
) {
    use crate::tools::ground::chunks as tool;

    let any = census.open > 0;
    let chosen = match textures.brush.texture.is_empty() {
        true => None,
        false => Some(textures.brush.texture.clone()),
    };
    match &chosen {
        Some(path) => {
            ui.horizontal(|ui| {
                swatch(ui, thumbnails, path);
                ui.add(
                    egui::Label::new(egui::RichText::new(textures::leaf(path)).color(theme::INK))
                        .truncate(),
                )
                .on_hover_text(path.clone());
            });
        }
        None => theme::note(ui, "No texture chosen."),
    }
    ui.columns(2, |columns| {
        let button = theme::full_button(&columns[0], "Set Base");
        if columns[0]
            .add_enabled(any && chosen.is_some(), button)
            .on_hover_text(
                "Make the chosen texture the base of every selected chunk, under \
                 whatever each already carries. Every blend map is kept. One undo entry.",
            )
            .on_disabled_hover_text("Select chunks, and choose a texture below.")
            .clicked()
        {
            if let Some(path) = &chosen {
                let changed = tool::set_base(session, &chunks.selected, path);
                session.status =
                    format!("Base texture of {} set to {}", counted(changed, "chunk"), textures::leaf(path));
            }
        }
        let button = theme::full_button(&columns[1], "Clear to Base");
        if columns[1]
            .add_enabled(any, button)
            .on_hover_text("Remove every layer except the base from every selected chunk.")
            .clicked()
        {
            let changed = tool::clear_paint(session, &chunks.selected);
            session.status = format!("{} cleared to the base", counted(changed, "chunk"));
        }
    });

    theme::heading(ui, "Selection Textures");
    // The textures the selection carries, one card each. To Base, Swap and
    // Remove act on every selected chunk that carries the card's texture.
    // The three buttons are a line of their own: beside the name they were
    // wider than the panel.
    let mut swap: Option<String> = None;
    let mut drop: Option<String> = None;
    let mut rebase: Option<String> = None;
    if census.textures.is_empty() {
        theme::note(ui, "Nothing selected");
    }
    for (path, carried, based) in census.textures.iter().take(CENSUS_ROWS) {
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(theme::SUNK)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(6, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    swatch(ui, thumbnails, path);
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(textures::leaf(path)).size(theme::SMALL).color(theme::INK),
                            )
                            .truncate(),
                        )
                        .on_hover_text(path.clone());
                        theme::note(
                            ui,
                            match based {
                                0 => format!("In {}", counted(*carried, "chunk")),
                                _ => format!("In {}, the base in {based}", counted(*carried, "chunk")),
                            },
                        );
                    });
                });
                // A third of the card at the inspector's narrowest is 64
                // points, which "Remove" at the usual padding does not fit.
                ui.spacing_mut().button_padding.x = 4.0;
                ui.columns(3, |columns| {
                    let button = theme::full_button(&columns[0], "To Base");
                    if columns[0]
                        .add_enabled(*based < census.open, button)
                        .on_hover_text(
                            "Move this texture to the base of every selected chunk and \
                             recompute the blend maps so the ground looks the same. A chunk \
                             without it gains it under its base. One undo entry.",
                        )
                        .on_disabled_hover_text("This texture is already the base of every selected chunk.")
                        .clicked()
                    {
                        rebase = Some(path.clone());
                    }
                    let differs = chosen
                        .as_deref()
                        .is_some_and(|chosen| !chosen.eq_ignore_ascii_case(path));
                    let button = theme::full_button(&columns[1], "Swap");
                    if columns[1]
                        .add_enabled(differs, button)
                        .on_hover_text(
                            "Replace this texture with the chosen texture on every \
                             selected chunk that carries it. Blend maps are kept.",
                        )
                        .on_disabled_hover_text("Choose another texture below.")
                        .clicked()
                    {
                        swap = Some(path.clone());
                    }
                    let button = theme::full_button(&columns[2], "Remove");
                    if columns[2]
                        .add_enabled(carried > based, button)
                        .on_hover_text(
                            "Remove this texture's layer from every selected chunk \
                             where it is not the base.",
                        )
                        .on_disabled_hover_text(
                            "This texture is only a base here. A base can be replaced with Set Base, not removed.",
                        )
                        .clicked()
                    {
                        drop = Some(path.clone());
                    }
                });
            });
    }
    if census.textures.len() > CENSUS_ROWS {
        theme::note(
            ui,
            format!("{} more textures", census.textures.len() - CENSUS_ROWS),
        );
    }
    if census.full > 0 {
        theme::note(
            ui,
            match census.full {
                1 => "1 chunk carries four textures, the maximum per chunk.".to_string(),
                n => format!("{n} chunks carry four textures, the maximum per chunk."),
            },
        );
    }
    if let (Some(from), Some(to)) = (&swap, &chosen) {
        let changed = tool::swap_texture(session, &chunks.selected, from, to);
        session.status = format!(
            "Replaced {} with {} on {}",
            textures::leaf(from),
            textures::leaf(to),
            counted(changed, "chunk")
        );
    }
    if let Some(path) = &drop {
        let changed = tool::remove_texture(session, &chunks.selected, path);
        session.status = format!("Removed {} from {}", textures::leaf(path), counted(changed, "chunk"));
    }
    if let Some(path) = &rebase {
        let (changed, full) = tool::move_to_base(session, &chunks.selected, path);
        session.status = format!("Moved {} to the base of {}", textures::leaf(path), counted(changed, "chunk"));
        if full > 0 {
            session.status += &format!("; {} skipped: four other textures", counted(full, "chunk"));
        }
    }
    ui.add_space(4.0);
    // Open at first only while no texture is chosen, which `theme::fold`
    // cannot be, so this stays egui's own folding header.
    egui::CollapsingHeader::new(egui::RichText::new("Tilesets").color(theme::INK_DIM))
        .id_salt("chunks-texture-picker")
        .default_open(chosen.is_none())
        .show(ui, |ui| {
            tileset_picker(ui, textures, thumbnails, session);
        });
}

/// The Foliage page: each texture in the selection with what its layers
/// grow, as a Foliage field that opens the Foliage window for every layer of
/// that texture in the selection, and the two commands over every layer.
#[allow(clippy::too_many_arguments)]
fn chunks_foliage(
    ui: &mut egui::Ui,
    selected: &std::collections::BTreeSet<crate::tools::ground::chunks::Cell>,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    assets: &vale_client::assets::GameAssets,
    foliage: &mut Foliage,
    portraits: &mut Portraits,
    census: &crate::tools::ground::chunks::Census,
) {
    if census.open == 0 {
        theme::note(ui, "Select chunks to see and change what their textures grow.");
        return;
    }
    let catalogue = foliage.catalogue(session, assets);
    let mut open: Option<(Option<String>, u32)> = None;
    // `census.foliage` is by texture, so each texture's pairs are one run.
    let mut at = 0;
    while at < census.foliage.len() {
        let texture = census.foliage[at].0.clone();
        let pairs: Vec<(u32, usize)> = census.foliage[at..]
            .iter()
            .take_while(|(name, ..)| *name == texture)
            .map(|&(_, id, layers)| (id, layers))
            .collect();
        at += pairs.len();
        let ids: Vec<u32> = pairs.iter().map(|&(id, _)| id).collect();
        let layers: usize = pairs.iter().map(|&(_, layers)| layers).sum();
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(theme::SUNK)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(6, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    swatch(ui, thumbnails, &texture);
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(textures::leaf(&texture)).size(theme::SMALL).color(theme::INK))
                                .truncate(),
                        )
                        .on_hover_text(texture.clone());
                        theme::note(ui, counted(layers, "layer"));
                    });
                });
                if super::foliage::field(ui, catalogue.as_deref(), portraits, &ids, true, "") {
                    let current = match ids.as_slice() {
                        [one] => *one,
                        _ => 0,
                    };
                    open = Some((Some(texture.clone()), current));
                }
                if ids.len() > 1 {
                    for &(id, layers) in &pairs {
                        // The row id first: two rows can name the same
                        // models with another density or footstep sound.
                        let name = catalogue.as_ref().map_or_else(String::new, |catalogue| catalogue.describe(id));
                        theme::note(ui, format!("Row {id}: {name} \u{b7} {}", counted(layers, "layer")));
                    }
                }
            });
    }
    ui.add_space(6.0);
    // At their own widths and not two equal columns: half the narrowest
    // panel is narrower than "Set All Layers...".
    ui.horizontal(|ui| {
        if ui
            .button("Set All Layers\u{2026}")
            .on_hover_text("Choose one set for every layer of every selected chunk, whatever its texture.")
            .clicked()
        {
            open = Some((None, 0));
        }
        if ui
            .button("Clear Foliage")
            .on_hover_text("Make every layer of the selected chunks grow nothing. One undo entry.")
            .clicked()
        {
            let changed = crate::tools::ground::chunks::set_foliage(session, selected, None, 0);
            session.status = format!("Foliage cleared on {}", counted(changed, "chunk"));
        }
    });
    if let Some((texture, current)) = open {
        let shown = texture.clone().unwrap_or_default();
        foliage.open(Target::Selection { texture }, &shown, current, session);
    }
}

/// The chunk tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, tables, assets, .. } = subject;
    chunks(
        ui,
        &mut editing.chunks,
        &mut editing.areas,
        &mut editing.textures,
        &mut editing.thumbnails,
        session,
        tables,
        assets,
        &mut editing.foliage,
        &mut editing.portraits,
    );
}
