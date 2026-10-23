//! The chunks tool's panel and its pages: copy and paste, stitching, areas,
//! holes and textures over a selection of chunks. The state is
//! [`crate::tools::ground::chunks`].

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::ground::textures::Textures;
use crate::tools::ground::textures;
use crate::tools::placements::place;
use crate::ui::tools::ground::areas::{area_name, area_tree};
use crate::ui::tools::ground::textures::{swatch, tileset_picker};
use crate::ui::widgets::theme;
use crate::ui::widgets::thumbnails::Thumbnails;

/// How many of a census's rows the chunk panel lists before it says how many
/// more there are.
const CENSUS_ROWS: usize = 6;

/// The chunk panel's pages, in the order the control lists them.
const PAGES: [(&str, crate::tools::ground::chunks::Page); 5] = [
    ("Paste", crate::tools::ground::chunks::Page::Paste),
    ("Stitch", crate::tools::ground::chunks::Page::Stitch),
    ("Area", crate::tools::ground::chunks::Page::Area),
    ("Holes", crate::tools::ground::chunks::Page::Holes),
    ("Paint", crate::tools::ground::chunks::Page::Textures),
];

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
/// the impassable flag, the holes, the textures. One page is open at a time,
/// so the panel is about a screen tall whichever is open.
///
/// The area and the texture an operation writes are the area brush's and the
/// texture brush's chosen values, picked with the pickers those two panels
/// draw. A second chosen area or texture kept by this tool would be a second
/// answer to which one is chosen.
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
) {
    use crate::tools::ground::chunks as tool;

    let census = chunks.census(session).clone();
    let edited = areas.table.clone();
    let table = edited
        .as_deref()
        .or_else(|| tables.and_then(|tables| tables.areas()));
    let any = census.open > 0;

    theme::heading(ui, "Selection");
    match census.open + census.closed {
        0 => theme::note(
            ui,
            "nothing selected: click a chunk, or drag a block on the ground",
        ),
        _ => {
            ui.label(
                egui::RichText::new(format!(
                    "{} in {}",
                    counted(census.open, "chunk"),
                    counted(census.tiles, "tile")
                ))
                .color(theme::INK),
            );
            if census.closed > 0 {
                theme::note(
                    ui,
                    format!(
                        "{} more in tiles that are not open; operations skip them",
                        census.closed
                    ),
                );
            }
        }
    }
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(any, egui::Button::new("Whole tiles"))
            .on_hover_text("Select every chunk of each tile the selection touches.")
            .clicked()
        {
            let cells = chunks.whole_tiles(session);
            chunks.select(cells);
        }
        if let Some(area) = chunks.primary.and_then(|cell| tool::area_of(session, cell)) {
            if ui
                .button(format!("All in {}", area_name(table, area)))
                .on_hover_text(
                    "Add every chunk of the open tiles that has the area of the chunk \
                     last clicked.",
                )
                .clicked()
            {
                chunks.add(tool::Chunks::in_area(session, area));
            }
        }
        if ui
            .add_enabled(census.open + census.closed > 0, egui::Button::new("Deselect"))
            .on_hover_text("Escape. Clear the selection.")
            .clicked()
        {
            chunks.clear();
        }
    });

    ui.add_space(8.0);
    theme::segmented(ui, &mut chunks.page, &PAGES, |a, b| a == b);
    ui.add_space(6.0);
    match chunks.page {
        tool::Page::Paste => chunks_paste(ui, chunks, session, any),
        tool::Page::Stitch => chunks_stitch(ui, chunks, session, &census),
        tool::Page::Area => chunks_area(ui, chunks, areas, session, tables, &census),
        tool::Page::Holes => chunks_holes(ui, chunks, session, &census),
        tool::Page::Textures => chunks_textures(ui, chunks, textures, thumbnails, session, &census),
    }

    ui.add_space(10.0);
    egui::CollapsingHeader::new(
        egui::RichText::new("Keys")
            .size(theme::SMALL)
            .color(theme::INK_FAINT),
    )
    .id_salt("chunks-keys")
    .show(ui, |ui| {
        theme::note(ui, "click selects a chunk · drag selects a block");
        theme::note(ui, "shift adds · shift + click on a selected chunk removes it");
        theme::note(ui, "ctrl + a selects the tile under the pointer · escape deselects");
        theme::note(ui, "ctrl + c copies · ctrl + v pastes at the pointer");
    });
    ui.add_space(2.0);
    theme::note(
        ui,
        match chunks.at {
            Some(cell) => {
                let (tile, within) = (cell.tile(), cell.within());
                let area = tool::area_of(session, cell)
                    .map(|area| format!(" · {}", area_name(table, area)))
                    .unwrap_or_default();
                format!(
                    "under the pointer: tile {},{} · chunk {},{}{area}",
                    tile.0, tile.1, within.0, within.1
                )
            }
            None => "pointer is not over an open tile".to_string(),
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

    ui.horizontal(|ui| {
        if ui
            .add_enabled(any, egui::Button::new("Copy"))
            .on_hover_text(
                "Ctrl+C. Copy the selected chunks: heights, textures, shading, holes, \
                 water, area and the impassable flag.",
            )
            .on_disabled_hover_text("Select chunks first.")
            .clicked()
        {
            chunks.clip = tool::copy(session, &chunks.selected);
            session.status = format!("copied {} chunks", chunks.clip.chunks.len());
        }
        ui.label(
            egui::RichText::new(match chunks.clip.is_empty() {
                true => "nothing copied".to_string(),
                false => format!(
                    "{} copied, {} by {}",
                    counted(chunks.clip.chunks.len(), "chunk"),
                    chunks.clip.size.0,
                    chunks.clip.size.1
                ),
            })
            .size(theme::SMALL)
            .color(theme::INK_DIM),
        );
    });
    // The copied block, turned before it is pasted. Each press turns what
    // is held, so two mirrors or four quarter turns are the block as copied.
    ui.horizontal(|ui| {
        let held = !chunks.clip.is_empty();
        for (label, how, about) in [
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
        ] {
            if ui
                .add_enabled(held, egui::Button::new(label).small())
                .on_hover_text(about)
                .on_disabled_hover_text("Copy chunks first.")
                .clicked()
            {
                chunks.clip = chunks.clip.turned(how);
                session.status = format!(
                    "the copied block is now {} by {}",
                    chunks.clip.size.0, chunks.clip.size.1
                );
            }
        }
    });
    theme::note(
        ui,
        "ctrl + v pastes centred on the chunk under the pointer · hold ctrl to preview",
    );

    ui.add_space(6.0);
    theme::heading(ui, "Paste includes");
    // A part switched off is left as the ground under the paste has it.
    ui.horizontal_wrapped(|ui| {
        let parts = &mut chunks.parts;
        for (label, on, about) in [
            ("heights", &mut parts.heights, "MCVT. The normals are recomputed."),
            ("textures", &mut parts.textures, "The layers and their blend maps."),
            ("shading", &mut parts.shading, "MCCV, the colour painted onto the vertices."),
            ("holes", &mut parts.holes, "The hole mask."),
            ("water", &mut parts.water, "MCLQ, at the level it was copied at."),
            ("area", &mut parts.area, "The area id and the impassable flag."),
        ] {
            ui.checkbox(on, label).on_hover_text(about);
        }
    });

    ui.add_space(6.0);
    theme::heading(ui, "Height");
    theme::segmented(
        ui,
        &mut chunks.level,
        &[
            ("Absolute", tool::Level::Absolute),
            ("Relative", tool::Level::Relative),
        ],
        |a, b| a == b,
    );
    theme::note(
        ui,
        match chunks.level {
            tool::Level::Absolute => "heights are pasted as they were copied",
            tool::Level::Relative => {
                "heights are moved to the level of the ground they replace; water is not moved"
            }
        },
    );
    ui.add_space(4.0);
    ui.checkbox(&mut chunks.stitch_pasted, "stitch each paste to the ground around it")
        .on_hover_text(
            "After a paste that writes heights, the Stitch page's settings are applied \
             to the pasted block, in the paste's own undo entry.",
        );
}

/// The Stitch page: what the border meets, which side moves, how far the
/// blend reaches, and the button.
fn chunks_stitch(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::ground::chunks::Chunks,
    session: &mut EditSession,
    census: &crate::tools::ground::chunks::Census,
) {
    use crate::tools::ground::chunks as tool;

    theme::note(
        ui,
        "Joins the selection's border to the ground around it. Every vertex on the \
         border is moved to one height, and the ground within the reach distance \
         blends toward it on the side that moves.",
    );
    ui.add_space(4.0);
    theme::row(ui, "border", |ui| {
        ui.label(
            egui::RichText::new(match census.border.sides {
                0 => "meets no open ground".to_string(),
                n => format!(
                    "{}; the tallest step is {:.1} y",
                    counted(n, "side"),
                    census.border.step
                ),
            })
            .color(match census.border.step > 0.5 {
                true => theme::WARN,
                false => theme::INK,
            }),
        )
        .on_hover_text(
            "The sides of selected chunks that meet an unselected chunk in an open \
             tile, and the largest difference between the two heights stored for one \
             vertex on them.",
        );
    });

    ui.add_space(4.0);
    theme::segmented(
        ui,
        &mut chunks.stitch.yields,
        &[
            ("Selection", tool::Yields::Selection),
            ("Ground", tool::Yields::Ground),
            ("Halfway", tool::Yields::Both),
        ],
        |a, b| a == b,
    );
    theme::note(
        ui,
        match chunks.stitch.yields {
            tool::Yields::Selection => "the selected chunks move to meet the ground around them",
            tool::Yields::Ground => "the ground around the selection moves to meet it",
            tool::Yields::Both => "both move halfway",
        },
    );
    theme::row(ui, "reach", |ui| {
        ui.add(
            egui::Slider::new(&mut chunks.stitch.reach, 0.0..=tool::FURTHEST)
                .fixed_decimals(0)
                .suffix(" y"),
        )
        .on_hover_text(
            "How far from the border the blend reaches, on each side that moves. \
             Zero moves the border's vertices and nothing else. A chunk is 33 yards.",
        );
    });

    ui.add_space(6.0);
    let ready = census.open > 0 && census.border.sides > 0;
    ui.add_enabled_ui(ready, |ui| {
        if theme::primary(ui, &format!("Stitch {}", counted(census.open, "chunk"))).clicked() {
            let moved = tool::stitch(session, &chunks.selected, chunks.stitch);
            session.status = match moved {
                0 => "nothing moved: the border already meets the ground".to_string(),
                n => format!("stitched: {n} chunks moved"),
            };
        }
    });
    if !ready {
        theme::note(ui, "select chunks whose border meets open ground");
    }
    theme::note(
        ui,
        "One undo entry. Doodads, WMOs and spawns on the moved ground are not moved.",
    );
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
    ui.horizontal(|ui| {
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
            session.status = format!("area {} on {changed} chunk(s)", areas.brush.area);
        }
        ui.label(egui::RichText::new(area_name(table, areas.brush.area)).color(theme::INK));
    });
    // The areas the selection is made of. `use` takes one as the area to
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
                    .add_enabled(*id != areas.brush.area, egui::Button::new("use").small())
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
            format!("and {} more areas", census.areas.len() - CENSUS_ROWS),
        );
    }
    egui::CollapsingHeader::new(egui::RichText::new("Choose an area").color(theme::INK_DIM))
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
                theme::note(ui, "no AreaTable.dbc: the archives did not open");
                theme::row(ui, "id", |ui| {
                    ui.add(egui::DragValue::new(&mut areas.brush.area).speed(1.0));
                });
            }
        });

    ui.add_space(6.0);
    theme::heading(ui, "Walkable");
    ui.horizontal(|ui| {
        for (label, on) in [("Impassable", true), ("Passable", false)] {
            if ui
                .add_enabled(any, egui::Button::new(label))
                .on_hover_text(
                    "MCNK flag 0x02 on every selected chunk. vmangos reads it when it \
                     builds mmaps. This client does not read it, so the viewport does \
                     not change.",
                )
                .clicked()
            {
                let changed = tool::set_impassable(session, &chunks.selected, on);
                session.status = format!("marked {} on {changed} chunk(s)", label.to_lowercase());
            }
        }
        theme::note(
            ui,
            format!("{} of {} impassable", census.impassable, census.open),
        );
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
    ui.horizontal(|ui| {
        for (label, cut) in [("Cut", true), ("Fill", false)] {
            if ui
                .add_enabled(any, egui::Button::new(label))
                .on_hover_text(match cut {
                    true => "Cut all sixteen squares of every selected chunk.",
                    false => "Fill all sixteen squares of every selected chunk.",
                })
                .clicked()
            {
                let changed = tool::set_holes(session, &chunks.selected, cut);
                session.status = match cut {
                    true => format!("cut holes in {changed} chunk(s)"),
                    false => format!("filled holes in {changed} chunk(s)"),
                };
            }
        }
        theme::note(
            ui,
            format!("{} of {} squares cut", census.cut, census.open * 16),
        );
    });
    theme::note(
        ui,
        "To cut or fill one square at a time, use the Holes tool.",
    );
}

/// The Paint page: the chosen texture as a base, the layers cleared, and the
/// textures the selection carries with a swap and a remove each.
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
        None => theme::note(ui, "no texture chosen; pick one below"),
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(any && chosen.is_some(), egui::Button::new("Base"))
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
                    format!("base texture of {changed} chunk(s) set to {}", textures::leaf(path));
            }
        }
        if ui
            .add_enabled(any, egui::Button::new("Clear to base"))
            .on_hover_text("Remove every layer except the base from every selected chunk.")
            .clicked()
        {
            let changed = tool::clear_paint(session, &chunks.selected);
            session.status = format!("{changed} chunk(s) cleared to their base");
        }
    });

    ui.add_space(6.0);
    theme::heading(ui, "Selection textures");
    // The textures the selection carries. `swap` and `×` act on every chunk
    // that carries the row's texture, which is the chunk list's two buttons
    // over a selection.
    let mut swap: Option<String> = None;
    let mut drop: Option<String> = None;
    if census.textures.is_empty() {
        theme::note(ui, "nothing selected");
    }
    for (path, carried, based) in census.textures.iter().take(CENSUS_ROWS) {
        ui.horizontal(|ui| {
            swatch(ui, thumbnails, path);
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new(textures::leaf(path))
                        .size(theme::SMALL)
                        .color(theme::INK),
                )
                .on_hover_text(path.clone());
                theme::note(
                    ui,
                    match based {
                        0 => format!("in {carried} chunk(s)"),
                        _ => format!("in {carried} chunk(s), the base in {based}"),
                    },
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(carried > based, egui::Button::new("×").small())
                    .on_hover_text(
                        "Remove this texture's layer from every selected chunk \
                         where it is not the base.",
                    )
                    .on_disabled_hover_text(
                        "This texture is only a base here. A base can be replaced with Base, not removed.",
                    )
                    .clicked()
                {
                    drop = Some(path.clone());
                }
                let differs = chosen
                    .as_deref()
                    .is_some_and(|chosen| !chosen.eq_ignore_ascii_case(path));
                if ui
                    .add_enabled(differs, egui::Button::new("swap").small())
                    .on_hover_text(
                        "Replace this texture with the chosen texture on every \
                         selected chunk that carries it. Blend maps are kept.",
                    )
                    .on_disabled_hover_text("Choose another texture below.")
                    .clicked()
                {
                    swap = Some(path.clone());
                }
            });
        });
    }
    if census.textures.len() > CENSUS_ROWS {
        theme::note(
            ui,
            format!("and {} more textures", census.textures.len() - CENSUS_ROWS),
        );
    }
    if census.full > 0 {
        theme::note(
            ui,
            format!(
                "{} chunk(s) carry four textures, the maximum per chunk.",
                census.full
            ),
        );
    }
    if let (Some(from), Some(to)) = (&swap, &chosen) {
        let changed = tool::swap_texture(session, &chunks.selected, from, to);
        session.status = format!(
            "replaced {} with {} on {changed} chunk(s)",
            textures::leaf(from),
            textures::leaf(to)
        );
    }
    if let Some(path) = &drop {
        let changed = tool::remove_texture(session, &chunks.selected, path);
        session.status = format!("removed {} from {changed} chunk(s)", textures::leaf(path));
    }
    ui.add_space(4.0);
    egui::CollapsingHeader::new(egui::RichText::new("Choose a texture").color(theme::INK_DIM))
        .id_salt("chunks-texture-picker")
        .default_open(chosen.is_none())
        .show(ui, |ui| {
            tileset_picker(ui, textures, thumbnails, session);
        });
}

/// The chunk tool's inspector panel.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    let crate::ui::tools::Subject { session, tables, .. } = subject;
    chunks(
        ui,
        &mut editing.chunks,
        &mut editing.areas,
        &mut editing.textures,
        &mut editing.thumbnails,
        session,
        tables,
    );
}
