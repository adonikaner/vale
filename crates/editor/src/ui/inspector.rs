//! The panel beside the viewport, which shows the chosen tool's settings.
//!
//! ## One panel for every tool
//!
//! Every subject on the rail draws into the same rectangle, and one `match`
//! here decides what goes in it. A person learns where the settings are once,
//! and a new subject takes the same position instead of choosing one.
//!
//! A subject that is not built yet (a greyed row on the rail) has nothing to
//! draw, and says so in this one place.
//!
//! ## Axis names
//!
//! The panel labels coordinates `x`, `y` and `z`, meaning the world's x, y and
//! z: north, west and up, the same three the status line reads and `.gps`
//! prints in the game. It does not label them "north", "west" and "up": a
//! compass word names an axis only until something is turned, after which
//! "west" is a direction and `y` is still an axis.
//!
//! The conversion belongs to `vale_assets`, both ways and for both kinds: a
//! position through `placement_to_world` and the three angles through
//! `placement_euler_to_world`, which states that the file's second angle is
//! the turn about the world's z. See [`crate::tools::doodads`], where this is
//! written down in full.
//!
//! ## Why the undo history is on this panel and not on the bar
//!
//! Undo applies to the edit, not to the session; [`super::topbar`] holds the
//! session controls. Undo is also the control a person reaches for without
//! looking, so it sits at the top of the panel they are already looking at.

use bevy_egui::egui;

use super::thumbnails::{Thumbnails, SIDE};
use super::{theme, Editing};
use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::doodads::Selection;
use crate::tools::gizmo::{Gizmo, Handles, HANDLES};
use crate::tools::place::{self, Kind, Placing};
use crate::tools::terrain::{self, Terrain, FALLOFFS, MODES, SHAPES};
use crate::tools::textures::{self, Textures};
use crate::tools::wmos;
use crate::tools::Tool;
use vale_edit::ops::Mode;

/// What the panel is about this frame, as one argument.
///
/// Bundled for the same reason as [`Editing`]: the shell was at Bevy's
/// sixteen-parameter limit and this function took eight. `Editing` is the
/// state the tools hold across frames; this struct is what is true in the
/// current frame.
pub struct Subject<'a> {
    pub tool: Tool,
    pub session: &'a mut EditSession,
    /// The DBCs, for the panel that turns a row id into a name. `None` on a
    /// machine with no archives, where the zone tool shows bare numbers
    /// instead of refusing to open.
    pub tables: Option<&'a vale_assets::tables::dbc::DisplayTables>,
    /// The archives, for the panel that opens a table on demand: following a
    /// reference reads a DBC nothing has loaded yet. Without this field the
    /// button that follows a reference cannot open that table.
    pub assets: &'a vale_client::assets::GameAssets,
    /// Where the pointer is. The texture tool uses it to say what the chunk
    /// under the pointer carries; no other panel reads it yet.
    pub cursor: &'a Cursor,
    /// The frame clock used to fold a gesture into one history entry; see
    /// `vale_edit::undo::History::begin_gesture`.
    pub now: f64,
    /// Where this machine's server is, for the panels whose subject is a row
    /// in vmangos' database rather than a file.
    pub server: &'a crate::server::settings::ServerSettings,
    /// The bar's Server… popover, which holds every server operation. The
    /// same panels link to it; see [`super::sync`].
    pub server_panel: &'a mut super::popover::Popover,
    /// The ground guides, for the one a tool's panel repeats: the texture
    /// brush shows the full chunks. See [`crate::tools::guides`].
    pub guides: &'a mut crate::tools::guides::Guides,
}

/// Draw the panel for whichever tool is chosen.
pub fn draw(ui: &mut egui::Ui, subject: Subject<'_>, editing: &mut Editing<'_>) {
    let Subject {
        tool,
        session,
        tables,
        assets,
        cursor,
        now,
        server,
        server_panel,
        guides,
    } = subject;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(tool.name())
                .strong()
                .size(13.0)
                .color(theme::INK),
        );
    });
    ui.add_space(2.0);
    if tool.edits() {
        history(ui, session, assets);
    }
    ui.add_space(4.0);

    let showing_hour = editing.clock.half_minutes;
    match tool {
        Tool::Select => {
            theme::heading(ui, "Pointer");
            theme::note(
                ui,
                "Select reports the position under the pointer and changes \
                 nothing. Choose a tool on the rail to edit, or Measure to inspect \
                 a point and the distance between two points.",
            );
        }
        Tool::Measure => measure(ui, &mut editing.measuring, session, cursor, tables),
        Tool::Terrain => brush(ui, &mut editing.terrain),
        Tool::Grade => grading(ui, &mut editing.grading, session),
        Tool::Road => road(
            ui,
            &mut editing.road,
            &mut editing.textures,
            &mut editing.thumbnails,
            session,
        ),
        Tool::Shading => shading(ui, &mut editing.shading),
        Tool::Holes => holes(ui, &editing.holes, session),
        Tool::Areas => zones(ui, &mut editing.areas, session, tables, cursor),
        Tool::Chunks => chunks(
            ui,
            &mut editing.chunks,
            &mut editing.areas,
            &mut editing.textures,
            &mut editing.thumbnails,
            session,
            tables,
        ),
        Tool::Water => water(ui, &mut editing.water),
        Tool::Sweep => sweep(ui, &mut editing.sweep, session, assets),
        Tool::Textures => paint(
            ui,
            &mut editing.textures,
            &mut editing.thumbnails,
            session,
            cursor,
            assets,
            guides,
            now,
        ),
        Tool::Doodads => doodad(
            ui,
            session,
            &mut editing.selection,
            &mut editing.doodad_held,
            &mut editing.gizmo,
            &mut editing.placing,
            &mut editing.portraits,
            &mut editing.favourites,
            now,
        ),
        Tool::Wmos => wmo(
            ui,
            session,
            &mut editing.wmos,
            &mut editing.wmo_held,
            &mut editing.gizmo,
            &mut editing.placing,
            &mut editing.portraits,
            &mut editing.favourites,
            now,
        ),
        // A light's whole form is drawn here, because a light is picked in the
        // viewport and the inspector is where this shell shows the selection,
        // as it does a doodad's position and turn. The list of all 374 lights
        // is a dialog behind a button, for finding one that is not in view.
        Tool::Lights => super::data::light_inspector(
            ui,
            super::data::Workspace {
                tool,
                session,
                browser: &mut editing.browser,
                assets,
                thumbnails: &mut editing.thumbnails,
                portraits: &mut editing.portraits,
                favourites: &mut editing.favourites,
                now,
                wireframe: None,
                particles: None,
                lights: Some(&mut editing.lights),
                quests: None,
                hour: showing_hour,
            },
        ),
        // The flight paths are client tables whose rows are places, picked in
        // the viewport like a light, so their form is drawn here too.
        Tool::Flightpaths => super::flightpaths::draw(
            ui,
            super::flightpaths::Subject {
                session,
                flights: &mut editing.flightpaths,
                assets,
                now,
                server_panel,
            },
        ),
        // Area triggers and safe places are client tables whose rows are
        // places, picked in the viewport like a flight node, with server rows
        // beside them.
        Tool::Triggers => super::triggers::draw(
            ui,
            super::triggers::Subject {
                session,
                triggers: &mut editing.triggers,
                assets,
                now,
                server_panel,
                behaviour: &mut editing.behaviour,
            },
        ),
        Tool::Graveyards => super::graveyards::draw(
            ui,
            super::graveyards::Subject {
                session,
                graveyards: &mut editing.graveyards,
                assets,
                now,
                server_panel,
            },
        ),
        // The server's creatures are picked in the viewport like a light, so
        // their form is drawn here for the same reason. It shows two rows: the
        // spawn that was clicked and the template behind it. See
        // `super::creatures`.
        Tool::Creatures => super::creatures::draw(
            ui,
            super::creatures::Subject {
                session,
                creatures: &mut editing.creatures,
                waypoints: &mut editing.waypoints,
                quests: &mut editing.quests,
                loot: &mut editing.loot,
                behaviour: &mut editing.behaviour,
                services: &mut editing.services,
                gossip: &mut editing.gossip,
                server,
                server_panel,
                assets,
                portraits: &mut editing.portraits,
                thumbnails: &mut editing.thumbnails,
                now,
                gizmo: Some(&mut editing.gizmo),
            },
        ),
        // The server's game objects, handled the same way as creatures. See
        // `super::gameobjects`.
        Tool::GameObjects => super::gameobjects::draw(
            ui,
            super::gameobjects::Subject {
                session,
                objects: &mut editing.objects,
                quests: &mut editing.quests,
                loot: &mut editing.loot,
                server,
                server_panel,
                assets,
                portraits: &mut editing.portraits,
                thumbnails: &mut editing.thumbnails,
                now,
                gizmo: Some(&mut editing.gizmo),
            },
        ),
        // The item workspace's third part: what the open item looks like,
        // which is this panel's view of the selection. `super::items` says why
        // it is not a column of its own.
        Tool::Items => super::items::inspector(
            ui,
            super::items::Looking {
                session,
                items: &mut editing.items,
                assets,
                thumbnails: &mut editing.thumbnails,
                portraits: &mut editing.portraits,
            },
        ),
        // The quest workspace's third part, following the shell's rule that the
        // inspector shows the selection: the open quest as the log would show
        // it, who gives and takes it, and its chain. `super::quests` says why
        // it is not a column of its own.
        Tool::Quests => super::quests::inspector(
            ui,
            super::quests::Workspace {
                session,
                quests: &mut editing.quests,
                items: &mut editing.items,
                assets,
                thumbnails: &mut editing.thumbnails,
                now,
            },
        ),
        // A table's inspector shows what the open row points at. The form
        // itself is in the middle, where the viewport would be; see
        // `super::data`. This follows the shell's rule that the inspector
        // shows the selection.
        Tool::Spells | Tool::ItemSets | Tool::Tables | Tool::Zones => table(
            ui,
            super::data::Workspace {
                tool,
                session,
                browser: &mut editing.browser,
                assets,
                thumbnails: &mut editing.thumbnails,
                portraits: &mut editing.portraits,
                favourites: &mut editing.favourites,
                now,
                wireframe: None,
                particles: None,
                lights: None,
                quests: None,
                hour: showing_hour,
            },
        ),
    }
}

/// The water tool: what is under the pointer, and the level the brush writes.
///
/// The level is the main setting because a wrong value is invisible: a
/// surface written a yard below the ground is a pool nobody can see. The only
/// way to tell before committing is to compare the level with the ground under
/// it, so the panel prints both and the difference between them.
fn water(ui: &mut egui::Ui, water: &mut crate::tools::water::Water) {
    use crate::tools::water;

    theme::heading(ui, "Under the pointer");
    match water.ground {
        Some(ground) => {
            theme::row(ui, "ground", |ui| {
                ui.label(theme::number(format!("{ground:.2}")));
            });
            match water.surface {
                Some((kind, level)) => {
                    theme::row(ui, "surface", |ui| {
                        ui.label(theme::number(format!("{level:.2}")));
                    });
                    theme::note(
                        ui,
                        format!("{:?} · {:.2} yd deep", kind, (level - ground).max(0.0)),
                    );
                    if let Some(flags) = water.flags {
                        theme::note(ui, water::flag_words(flags));
                    }
                }
                None => theme::note(ui, "no water"),
            }
        }
        None => theme::note(ui, "pointer is not over an open tile"),
    }

    ui.add_space(4.0);
    theme::heading(ui, "Surface");
    theme::segmented(ui, &mut water.sloped, &[("Flat", false), ("Sloped", true)], |a, b| a == b);
    match water.sloped {
        false => {
            theme::row(ui, "height", |ui| {
                ui.add(
                    egui::DragValue::new(&mut water.brush.level)
                        .speed(0.1)
                        .range(water::LEVEL)
                        .fixed_decimals(2),
                );
            });
        }
        true => slope_rows(ui, water),
    }
    // The surface relative to the ground under the pointer. A negative
    // difference is printed as "under the ground" in the warning colour,
    // because that is the mistake this panel is meant to catch.
    let here = water
        .point
        .map(|point| water.brush.level_at(point.x, point.y))
        .unwrap_or(water.brush.level);
    if let (Some(ground), false) = (water.ground, water.sloped && water.brush.slope.is_none()) {
        let over = here - ground;
        ui.label(
            egui::RichText::new(match over >= 0.0 {
                true => format!("{over:.2} yd above the ground at the pointer"),
                false => format!("{:.2} yd below the ground at the pointer", -over),
            })
            .size(theme::SMALL)
            .color(match over >= 0.0 {
                true => theme::INK_DIM,
                false => theme::BAD,
            }),
        );
    }
    match water.sloped {
        false => {
            theme::note(ui, "space takes the level from the water under the pointer");
            theme::note(ui, "ctrl + space takes it from the ground");
        }
        true => {
            theme::note(ui, "space sets the start, shift + space the end");
            theme::note(ui, "height from the water at the pointer, or with ctrl the ground");
            theme::note(ui, "flat beyond either end");
        }
    }

    ui.add_space(4.0);
    theme::heading(ui, "Liquid");
    theme::segmented(ui, &mut water.brush.kind, &water::KINDS, |a, b| a == b);
    // The cell's two flags. A stroke writes them on every cell it wets; with
    // alt held, it writes them on the wet cells under the brush without
    // changing their level. The census is at
    // `vale_edit::adt::liquid::FISHABLE`.
    for (bit, label, hint) in [
        (
            vale_edit::adt::liquid::FISHABLE,
            "fishable",
            "Set on nearly every sea and lake cell in the shipped maps.",
        ),
        (
            vale_edit::adt::liquid::FATIGUE,
            "deep water",
            "Fatigue flag: a swimmer in these cells takes fatigue damage. The \
             shipped maps set it on open sea only. vmangos reads this bit and \
             no other.",
        ),
    ] {
        let mut on = water.brush.cell_flags & bit != 0;
        if ui.checkbox(&mut on, label).on_hover_text(hint).changed() {
            water.brush.cell_flags = match on {
                true => water.brush.cell_flags | bit,
                false => water.brush.cell_flags & !bit,
            };
        }
    }

    ui.add_space(4.0);
    theme::heading(ui, "Brush");
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut water.brush.radius)
                .speed(0.25)
                .range(water::RADIUS)
                .suffix(" yd"),
        );
    });
    theme::note(ui, "a cell is 4.17 yd, the same grid as the ground");
    if water.wet != 0 {
        theme::note(
            ui,
            match water.wet > 0 {
                true => format!("{} cell(s) flooded this session", water.wet),
                false => format!("{} cell(s) drained this session", -water.wet),
            },
        );
    }

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button floods · shift + left drains");
    theme::note(
        ui,
        "alt + left writes the two flags to wet cells, level unchanged",
    );
    theme::note(ui, "ctrl + left updates depth from the ground below");
    theme::note(ui, "ctrl + wheel resizes");
    theme::note(ui, "playtest movement reads the same MCLQ data");
}

/// The two ends of a sloped surface: each one's height, or that it is not
/// set, and how far the surface falls between them.
fn slope_rows(ui: &mut egui::Ui, water: &mut crate::tools::water::Water) {
    for (at, label) in [(0, "start"), (1, "end")] {
        theme::row(ui, label, |ui| match &mut water.ends[at] {
            Some(end) => {
                ui.add(
                    egui::DragValue::new(&mut end[2])
                        .speed(0.1)
                        .range(crate::tools::water::LEVEL)
                        .fixed_decimals(2),
                );
                if ui.small_button("Clear").clicked() {
                    water.ends[at] = None;
                }
            }
            None => {
                ui.label(egui::RichText::new("not set").color(theme::INK_FAINT));
            }
        });
    }
    if let [Some(start), Some(end)] = water.ends {
        let run = ((end[0] - start[0]).powi(2) + (end[1] - start[1]).powi(2)).sqrt();
        let fall = start[2] - end[2];
        theme::note(
            ui,
            match fall >= 0.0 {
                true => format!("falls {fall:.1} yd over {run:.0} yd"),
                false => format!("rises {:.1} yd over {run:.0} yd", -fall),
            },
        );
    }
}

/// What `MCLY`'s animation bits come to, in words: which way the texture
/// crawls and how long it takes to cross itself.
///
/// The direction is printed as a compass point rather than the raw 0..7,
/// because the direction is what is being chosen. Step 0 is north (up the
/// texture) and each step is 45° clockwise. The seconds are the period of one
/// full width at `layer_flags::ANIMATION_BASE_RATE`; that rate is the client's,
/// not the file's.
fn crawls((turn, rate, on): (u32, u32, bool)) -> String {
    if !on {
        return "not animated".to_string();
    }
    const COMPASS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let speed =
        vale_assets::world::adt::layer_flags::ANIMATION_BASE_RATE * (1u32 << rate.min(7)) as f32;
    format!(
        "{} every {:.1}s",
        COMPASS[(turn & 7) as usize],
        1.0 / speed.max(1e-6)
    )
}

/// What a `GroundEffectTexture` row plants, in words: the models by their file
/// names and how many a cell gets, or that it plants nothing.
fn grows(assets: &vale_client::assets::GameAssets, effect_id: u32) -> String {
    if effect_id == 0 {
        return "no ground effect".to_string();
    }
    let effects = assets.ground_effects();
    let Some(effect) = effects.effect(effect_id) else {
        return format!("GroundEffectTexture {effect_id} not found");
    };
    let mut names: Vec<String> = effect.models
        [..usize::from(effect.choices).min(effect.models.len())]
        .iter()
        .map(|&model| {
            let path = effects.model(model);
            path.rsplit(['\\', '/'])
                .next()
                .unwrap_or(path)
                .trim_end_matches(".m2")
                .trim_end_matches(".M2")
                .to_string()
        })
        .collect();
    names.dedup();
    format!("{} · {} per cell", names.join(", "), effect.density)
}

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
    use crate::tools::areas;

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
/// [`crate::tools::areas`].
fn zones(
    ui: &mut egui::Ui,
    areas: &mut crate::tools::areas::Areas,
    session: &mut EditSession,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    cursor: &Cursor,
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
    // each request out after the panel is drawn. See `tools::areas::Ask`.
    {
        use crate::tools::areas::Ask;
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
                areas.ask = Some(Ask::Edit(areas.brush.area));
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
                areas.ask = Some(Ask::NewSubArea(areas.brush.area));
            }
            if ui
                .small_button("+ Zone")
                .on_hover_text(
                    "Create a zone on this map and put it on the brush. Name it with Edit\u{2026}.",
                )
                .clicked()
            {
                areas.ask = Some(Ask::NewZone);
            }
        });
    }
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut areas.brush.radius)
                .speed(0.5)
                .range(crate::tools::areas::RADIUS)
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
fn area_tree(
    ui: &mut egui::Ui,
    areas: &mut crate::tools::areas::Areas,
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

/// The hole tool, which has no settings at all.
///
/// A hole is one of sixteen fixed squares per chunk: there is no radius,
/// strength, falloff or shape to choose. The panel shows what the pointer is
/// over and what the tile already has, which the square outline in the
/// viewport does not show.
fn holes(ui: &mut egui::Ui, holes: &crate::tools::holes::Holes, session: &EditSession) {
    theme::heading(ui, "Square");
    match holes.at {
        Some(at) => {
            theme::row(ui, "tile", |ui| {
                ui.label(theme::number(format!("{},{}", at.tile.0, at.tile.1)));
            });
            theme::row(ui, "chunk", |ui| {
                ui.label(theme::number(at.chunk.to_string()));
            });
            theme::row(ui, "square", |ui| {
                // Row and column of the 4x4 grid are printed beside the bit,
                // because "row 2, column 1" can be matched to the square
                // outlined on screen and "square 9" cannot.
                ui.label(theme::number(format!(
                    "{} · row {} col {}",
                    at.bit,
                    at.bit / 4,
                    at.bit % 4
                )));
            });
            ui.label(
                egui::RichText::new(match holes.already {
                    true => "already cut — shift-click fills it",
                    false => "solid — click cuts it",
                })
                .size(theme::SMALL)
                .color(match holes.already {
                    true => theme::WARN,
                    false => theme::INK_DIM,
                }),
            );
        }
        None => theme::note(ui, "pointer is not over an open tile"),
    }

    ui.add_space(4.0);
    theme::heading(ui, "Tile");
    // What the map already has, not only what this session did. A tile of
    // Azeroth ships with holes wherever a cave mouth or a doorway is, and the
    // count tells an untouched tile from an edited one.
    match holes.at.and_then(|at| session.tiles.get(&at.tile)) {
        Some(tile) => theme::row(ui, "cut", |ui| {
            ui.label(theme::number(format!(
                "{} of {}",
                vale_edit::adt::holes::cut_in(tile),
                tile.chunks.len() * 16
            )));
        }),
        None => theme::note(ui, "no tile open under the pointer"),
    }
    if holes.cut != 0 {
        theme::note(
            ui,
            match holes.cut > 0 {
                true => format!("{} cut this session", holes.cut),
                false => format!("{} filled this session", -holes.cut),
            },
        );
    }

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button cuts · drag cuts every square it crosses");
    theme::note(ui, "shift + left button fills");
    theme::note(
        ui,
        "16 squares per chunk; the file has no finer unit",
    );
}

/// An area's name with its id. `0` is a chunk that belongs to no area, which
/// the shipped tiles contain, so it is named. An id `AreaTable` has no row
/// for is shown as the number.
fn area_name(table: Option<&vale_assets::tables::area::Areas>, id: u32) -> String {
    match (id, table.and_then(|table| table.get(id))) {
        (0, _) => "no area (0)".to_string(),
        (id, Some(area)) => format!("{} ({id})", area.name),
        (id, None) => format!("area {id}"),
    }
}

/// How many of a census's rows the chunk panel lists before it says how many
/// more there are.
const CENSUS_ROWS: usize = 6;

/// The chunk panel's pages, in the order the control lists them.
const PAGES: [(&str, crate::tools::chunks::Page); 5] = [
    ("Paste", crate::tools::chunks::Page::Paste),
    ("Stitch", crate::tools::chunks::Page::Stitch),
    ("Area", crate::tools::chunks::Page::Area),
    ("Holes", crate::tools::chunks::Page::Holes),
    ("Paint", crate::tools::chunks::Page::Textures),
];

/// `n` with `noun`, pluralised by adding an s.
fn counted(n: usize, noun: &str) -> String {
    match n {
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// The chunk tool: what is selected, and the operations on it, one page at a
/// time. See [`crate::tools::chunks`].
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
fn chunks(
    ui: &mut egui::Ui,
    chunks: &mut crate::tools::chunks::Chunks,
    areas: &mut crate::tools::areas::Areas,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
) {
    use crate::tools::chunks as tool;

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
    chunks: &mut crate::tools::chunks::Chunks,
    session: &mut EditSession,
    any: bool,
) {
    use crate::tools::chunks as tool;

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
    chunks: &mut crate::tools::chunks::Chunks,
    session: &mut EditSession,
    census: &crate::tools::chunks::Census,
) {
    use crate::tools::chunks as tool;

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
    chunks: &mut crate::tools::chunks::Chunks,
    areas: &mut crate::tools::areas::Areas,
    session: &mut EditSession,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    census: &crate::tools::chunks::Census,
) {
    use crate::tools::chunks as tool;

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
    chunks: &mut crate::tools::chunks::Chunks,
    session: &mut EditSession,
    census: &crate::tools::chunks::Census,
) {
    use crate::tools::chunks as tool;

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
    chunks: &mut crate::tools::chunks::Chunks,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    census: &crate::tools::chunks::Census,
) {
    use crate::tools::chunks as tool;

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

/// The Select/Place switch, and the model picker shown in Place mode.
///
/// Shared by the two placement tools, because choosing a model is the same
/// action whichever list it goes into; see [`crate::tools::place`] for the rest
/// of the reasoning. Returns `true` while the tool is in Place mode, which
/// tells the caller to draw the placement rows instead of the selection.
fn placing(
    ui: &mut egui::Ui,
    placing: &mut Placing,
    kind: Kind,
    portraits: &mut crate::portraits::Portraits,
    favourites: &mut crate::favourites::Favourites,
) -> bool {
    let mut mode = placing.mode;
    theme::segmented(ui, &mut mode, &place::MODES, |a, b| a == b);
    if mode != placing.mode {
        placing.mode = mode;
        // Switching to Select drops whatever was on the cursor. `aim` also
        // does this; doing it here as well makes the panel correct on the
        // frame of the switch rather than the frame after.
        if mode == place::Mode::Select {
            placing.disarm();
        }
    }
    if placing.mode != place::Mode::Place {
        return false;
    }

    ui.add_space(4.0);
    theme::heading(ui, "Model");
    match placing.path.clone() {
        Some(path) => {
            ui.label(egui::RichText::new(place::leaf(&path)).color(theme::INK))
                .on_hover_text(&path);
            // A button that opens the chosen model's folder, because the next
            // model is most likely to be in the same folder.
            let folder = Placing::folder_of(&path);
            if placing.folder(kind) != folder
                && ui
                    .small_button(egui::RichText::new("show folder").color(theme::INK_DIM))
                    .clicked()
            {
                placing.open_folder(kind, &folder);
            }
        }
        None => theme::note(ui, "no model chosen; pick one below"),
    }
    // A model the archives will not open is reported here. Without this the
    // cursor stays empty and a click writes a record for a file that is not
    // there. [`crate::tools::place`] describes the `.mdx` case that made this
    // necessary.
    if let Some(bad) = placing.trouble.clone() {
        ui.label(
            egui::RichText::new(format!("{} failed to load", place::leaf(&bad)))
                .color(theme::BAD)
                .size(theme::SMALL),
        )
        .on_hover_text(bad);
    }

    // A large preview of the chosen model, under its name; see
    // `crate::portraits`. A row's small picture identifies a model; this one
    // is large enough to judge it, and a drag turns it.
    //
    // It sits under the chosen model's name, not under the list. The list is
    // three hundred points tall in a panel of six hundred, so a pane under it
    // would start below the fold and need scrolling to. The browser's dialog
    // places its pane under its list by the same rule, because that is where
    // its chosen model is named.
    if let Some(path) = placing.path.clone() {
        ui.add_space(4.0);
        preview_pane(ui, portraits, &path);
    }

    ui.add_space(4.0);
    ui.add(
        egui::TextEdit::singleline(&mut placing.search)
            .hint_text("search the models")
            .desired_width(f32::INFINITY),
    );

    let chosen = placing.path.clone().unwrap_or_default();
    if let Some(picked) = browse(ui, placing, kind, &chosen, portraits, favourites) {
        favourites.used(list_of(kind), &picked);
        let (turn, scale) = (placing.turn, placing.scale);
        placing.arm(kind, picked, turn, scale);
    }

    ui.add_space(4.0);
    theme::heading(ui, "Placement");
    theme::row(ui, "turn", |ui| {
        ui.add(
            egui::DragValue::new(&mut placing.turn)
                .speed(1.0)
                .fixed_decimals(1)
                .suffix("°"),
        );
    });
    if kind == Kind::Doodad {
        theme::row(ui, "scale", |ui| {
            ui.add(
                egui::DragValue::new(&mut placing.scale)
                    .speed(0.01)
                    .range(crate::tools::doodads::SCALE_RANGE)
                    .fixed_decimals(3)
                    .suffix("x"),
            );
        });
    }

    // Slope alignment: a drop is leaned onto the ground under the cursor, and
    // the ghost shows the lean. Doodads only. See `Placing::align`.
    if kind == Kind::Doodad {
        ui.add_space(4.0);
        theme::heading(ui, "Ground");
        ui.checkbox(&mut placing.align, "align each placement to the slope")
            .on_hover_text(
                "Tilts each placed model to the ground normal under the \
                 cursor, keeping the current turn, instead of using the \
                 scatter tilt. The preview shows the result before the click.",
            );
    }

    // Scatter: every drop rolls the next one's turn, size and lean inside
    // these bounds. See `crate::tools::place`'s module comment.
    ui.add_space(4.0);
    theme::heading(ui, "Scatter");
    ui.checkbox(&mut placing.scatter.on, "randomise after each placement")
        .on_hover_text(
            "After each placement, picks a random turn, scale and tilt for \
             the next one within the ranges below. The preview shows what \
             the next click writes.",
        );
    if placing.scatter.on {
        theme::row(ui, "turn", |ui| {
            ui.checkbox(&mut placing.scatter.turn, "any angle");
        });
        if kind == Kind::Doodad {
            theme::row(ui, "scale", |ui| {
                ui.add(
                    egui::DragValue::new(&mut placing.scatter.size.0)
                        .speed(0.01)
                        .range(crate::tools::doodads::SCALE_RANGE)
                        .fixed_decimals(2)
                        .suffix("x"),
                );
                ui.label(egui::RichText::new("to").color(theme::INK_FAINT));
                ui.add(
                    egui::DragValue::new(&mut placing.scatter.size.1)
                        .speed(0.01)
                        .range(crate::tools::doodads::SCALE_RANGE)
                        .fixed_decimals(2)
                        .suffix("x"),
                );
            });
            theme::row(ui, "tilt", |ui| {
                ui.add(
                    egui::DragValue::new(&mut placing.scatter.tilt_max)
                        .speed(0.5)
                        .range(0.0..=90.0)
                        .fixed_decimals(1)
                        .suffix("°"),
                )
                .on_hover_text(
                    "Maximum tilt from vertical, in any direction. Zero keeps the model upright.",
                );
            });
        }
        if ui
            .small_button("Randomise now")
            .on_hover_text("Pick new random values for the next placement without placing anything.")
            .clicked()
        {
            placing.roll();
        }
    }

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button places · the model stays on the cursor");
    theme::note(ui, ", and . turn · shift turns three times as far");
    if kind == Kind::Doodad {
        theme::note(ui, "ctrl + wheel scales");
    }
    theme::note(ui, "escape clears the model from the cursor");
    theme::note(ui, "alt + mouse turns the model · ctrl snaps to 15°");
    theme::note(ui, "tab, or 1 and 2, switch select and place");
    true
}

/// The catalogue as folders rather than one list of several thousand names.
/// Returns the model a click chose.
///
/// A flat list would put 5,816 models and 816 buildings in one scroller, where
/// every model has to be found by typing, which works only when the name is
/// already known. The archives' own directories are the game's
/// grouping (`vale catalogue` prints them, seventeen at the top level), so
/// the folders are taken from them rather than invented.
///
/// Search still runs flat across everything. A search limited to the open
/// folder would find nothing for a model one directory over.
///
/// The starred and recent lists come before the folders, collapsible, and
/// take no space when empty. Scattering a copse uses the same three models
/// repeatedly, and without these lists each one needs a search; see
/// [`crate::favourites`]. Every row shows the model's picture (see
/// [`crate::portraits`]), because a file name does not show what the model
/// looks like.
fn browse(
    ui: &mut egui::Ui,
    placing: &mut Placing,
    kind: Kind,
    chosen: &str,
    portraits: &mut crate::portraits::Portraits,
    favourites: &mut crate::favourites::Favourites,
) -> Option<String> {
    let needle = placing.search.to_ascii_lowercase();
    let catalogue = placing.catalogue(kind);
    if !catalogue.iter().any(|path| !path.is_empty()) {
        theme::note(ui, "reading the model list…");
        return None;
    }
    let total = catalogue.len();
    let list = list_of(kind);

    // Searching: every match, wherever it is, with its folder under it.
    if !needle.is_empty() {
        let matches: Vec<String> = catalogue
            .iter()
            .filter(|path| !path.is_empty() && path.to_ascii_lowercase().contains(&needle))
            .cloned()
            .collect();
        theme::note(ui, format!("{} of {total} match", matches.len()));
        return rows(
            ui, "search", &matches, chosen, true, portraits, favourites, list,
        );
    }

    let mut picked: Option<String> = None;
    for (name, paths) in [
        ("Starred", favourites.starred(list)),
        ("Recent", favourites.recent(list)),
    ] {
        if paths.is_empty() {
            continue;
        }
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("{name} ({})", paths.len()))
                .size(theme::SMALL)
                .color(theme::INK_DIM),
        )
        .id_salt(("kept", name, kind as u8))
        .default_open(name == "Starred")
        .show(ui, |ui| {
            picked = rows(ui, name, &paths, chosen, false, portraits, favourites, list)
                .or(picked.take());
        });
    }

    // Browsing: the way back up, then the folders, then what is in this one.
    let here = placing.folder(kind).to_string();
    let mut go = breadcrumb(ui, &here, "");
    let (folders, files) = split_folder(catalogue, &here);
    theme::note(
        ui,
        match folders.len() {
            0 => format!("{} models", files.len()),
            n => format!("{n} folders · {} models", files.len()),
        },
    );

    let row = ui.spacing().interact_size.y + 6.0;
    egui::ScrollArea::vertical()
        .id_salt("models")
        .max_height(300.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            go = folder_rows(ui, &here, &folders, row).or(go.take());
            picked = rows(
                ui, "here", &files, chosen, false, portraits, favourites, list,
            )
            .or(picked.take());
        });
    if let Some(folder) = go {
        placing.open_folder(kind, &folder);
    }
    picked
}

/// The large preview of one model, with its heading and a button that resets
/// it to the resting view.
///
/// Shared by the placement pickers and the spell workspace's model browser,
/// because the pane is the same under both. `crate::portraits::paint_large`
/// does the drawing.
pub(super) fn preview_pane(
    ui: &mut egui::Ui,
    portraits: &mut crate::portraits::Portraits,
    path: &str,
) {
    preview_pane_at(ui, portraits, path, PREVIEW_MAX);
}

/// [`preview_pane`] with a size cap chosen by the caller.
///
/// [`PREVIEW_MAX`] is the inspector's cap, set by the list under the pane; see
/// its own note. A panel with nothing under the pane has no such limit and
/// passes the space it has. The item workspace's appearance column holds one
/// picture and its details and is resizable, so the picture grows with the
/// column.
pub(super) fn preview_pane_at(
    ui: &mut egui::Ui,
    portraits: &mut crate::portraits::Portraits,
    path: &str,
    largest: f32,
) {
    ui.horizontal(|ui| {
        theme::heading(ui, "Preview");
        if portraits.large_turned() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("reset")
                    .on_hover_text("Reset the preview to the camera angle the list rows use.")
                    .clicked()
                {
                    portraits.reset_large_turn();
                }
            });
        }
    });
    // Square, and never wider than the panel it is in: the camera renders at
    // one aspect, so a box of another shape would stretch the model.
    let side = ui.available_width().min(largest);
    ui.vertical_centered(|ui| {
        crate::portraits::paint_large(ui, portraits, path, side);
    });
}

/// The largest the preview pane is drawn, in points.
///
/// 170: four times a row's picture and about a third of the inspector's
/// height. At 220 the list under the pane shows a single row. The pane is for
/// judging the model already chosen, and taking the list's space makes choosing
/// the next one harder. A wide panel does not enlarge it, for the same reason.
pub(super) const PREVIEW_MAX: f32 = 170.0;

/// Which of the kept lists a placement kind's picker reads and writes.
fn list_of(kind: Kind) -> crate::favourites::Kind {
    match kind {
        Kind::Doodad => crate::favourites::Kind::Doodad,
        Kind::Wmo => crate::favourites::Kind::Wmo,
    }
}
/// A breadcrumb: the open folder's path as a row of segments. Returns the
/// folder a click asked for.
///
/// The open folder is a label and the folders above it are buttons, so one
/// click reaches any folder on the path.
///
/// `root` names the empty prefix, for a tree whose top level is not one
/// folder. The model and tileset pickers do not need it: everything they hold
/// is under `world\` or `tileset\`, so the root is a real segment, and passing
/// `""` leaves the row unchanged. The area tree needs it: its top level is
/// thirty-six maps, and without a name for the level above them the empty
/// segment draws as an empty button and a map cannot be left once opened.
fn breadcrumb(ui: &mut egui::Ui, here: &str, root: &str) -> Option<String> {
    let mut go: Option<String> = None;
    ui.horizontal_wrapped(|ui| {
        let mut first = true;
        let mut crumb = |ui: &mut egui::Ui, label: &str, walked: Option<String>| {
            if !first {
                ui.label(egui::RichText::new("/").color(theme::INK_FAINT).size(theme::SMALL));
            }
            first = false;
            let text = egui::RichText::new(label).size(theme::SMALL).color(match walked {
                None => theme::INK,
                Some(_) => theme::ACCENT,
            });
            match walked {
                // The open folder.
                None => {
                    ui.label(text);
                }
                Some(to) => {
                    if ui.small_button(text).clicked() {
                        go = Some(to);
                    }
                }
            }
        };

        if !root.is_empty() {
            crumb(ui, root, (!here.is_empty()).then(String::new));
        }
        if here.is_empty() {
            return;
        }
        let mut walked = String::new();
        for segment in here.trim_end_matches(place::SEP).split(place::SEP) {
            walked.push_str(segment);
            walked.push(place::SEP);
            let last = walked.len() == here.len();
            crumb(ui, segment, (!last).then(|| walked.clone()));
        }
    });
    go
}

/// The folders directly under the open folder, with how many entries each
/// holds. Returns the one a click opened.
///
/// The arrow is a painted triangle, not a glyph. egui's bundled fonts have no
/// `▸`, and a missing glyph draws as a hollow box beside every folder.
fn folder_rows(
    ui: &mut egui::Ui,
    here: &str,
    folders: &[(String, usize)],
    row: f32,
) -> Option<String> {
    let mut go: Option<String> = None;
    for (name, count) in folders {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), row - 2.0),
            egui::Sense::click(),
        );
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if response.hovered() {
                painter.rect_filled(rect, egui::CornerRadius::same(3), theme::RAISED);
            }
            let text = painter.with_clip_rect(rect);
            let tip = egui::pos2(rect.left() + 12.0, rect.center().y);
            text.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(tip.x - 3.0, tip.y - 4.0),
                    egui::pos2(tip.x + 3.0, tip.y),
                    egui::pos2(tip.x - 3.0, tip.y + 4.0),
                ],
                theme::INK_DIM,
                egui::Stroke::NONE,
            ));
            text.text(
                egui::pos2(rect.left() + 20.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                egui::FontId::proportional(12.0),
                theme::INK,
            );
            text.text(
                egui::pos2(rect.right() - 6.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                count.to_string(),
                egui::FontId::proportional(theme::SMALL),
                theme::INK_FAINT,
            );
        }
        if response.clicked() {
            go = Some(format!("{here}{name}{}", place::SEP));
        }
    }
    go
}

/// The folders directly under a prefix with how much is in each, and the files
/// that are directly in it.
///
/// One pass over the catalogue each frame rather than a tree built once and
/// kept. The catalogue is a few thousand strings and the panel is drawn sixty
/// times a second, so the cost is one prefix comparison per string; a kept
/// tree would be a second copy of the listing that has to be kept in step.
fn split_folder(catalogue: &[String], here: &str) -> (Vec<(String, usize)>, Vec<String>) {
    let mut folders: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut files: Vec<String> = Vec::new();
    for path in catalogue {
        let Some(rest) = path.strip_prefix(here) else {
            continue;
        };
        match rest.find(place::SEP) {
            Some(at) => *folders.entry(rest[..at].to_string()).or_default() += 1,
            None if !rest.is_empty() => files.push(path.clone()),
            None => {}
        }
    }
    (folders.into_iter().collect(), files)
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

/// A run of model rows. Returns the one a click chose.
///
/// Uses `show_rows`, as the tileset list does: the archives name thousands of
/// models and only the dozen on screen need building. Each row here requests a
/// rendered picture, so only the rows on screen are rendered. The row is the
/// workspace model browser's (`data::model_row`): the picture, the file name
/// over its folder, and a star.
#[allow(clippy::too_many_arguments)]
fn rows(
    ui: &mut egui::Ui,
    salt: &str,
    paths: &[String],
    chosen: &str,
    scroller: bool,
    portraits: &mut crate::portraits::Portraits,
    favourites: &mut crate::favourites::Favourites,
    list: crate::favourites::Kind,
) -> Option<String> {
    let mut picked: Option<String> = None;
    let mut draw = |ui: &mut egui::Ui, range: std::ops::Range<usize>| {
        for path in &paths[range] {
            let on = chosen.eq_ignore_ascii_case(path);
            let hit = super::data::model_row(ui, portraits, favourites, list, path, on);
            if hit.starred {
                favourites.toggle(list, path);
            }
            if hit.clicked || hit.double {
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
                .show_rows(ui, super::data::MODEL_ROW, paths.len(), |ui, range| {
                    draw(ui, range)
                });
        }
        // Already inside the browser's own scroller, which also holds the
        // folders above these rows; a nested scroller would draw two bars.
        false => draw(ui, 0..paths.len()),
    }
    picked
}
/// The building tool: `MODF` placements.
///
/// The same as the doodad panel without a scale, because 1.12's record does
/// not carry one. Instead it has two sets (which dressing of the model this
/// placement uses, and which name set) and a bounding box that this tool keeps
/// in step with the geometry. The box is shown because it can be wrong.
#[allow(clippy::too_many_arguments)]
fn wmo(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    selection: &mut wmos::Selection,
    held: &mut wmos::Held,
    gizmo: &mut Gizmo,
    placer: &mut Placing,
    portraits: &mut crate::portraits::Portraits,
    favourites: &mut crate::favourites::Favourites,
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
    // arming the placer with one — see `crate::tools::group`.
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

/// A client table: what the open row points at, and whether the table is
/// saved.
///
/// The form is in the middle of the shell rather than here; [`super::data`]
/// says why. The inspector shows what is true of the selection, which for a
/// row is its references, plus the one button that acts on the file rather
/// than the row.
fn table(ui: &mut egui::Ui, mut work: super::data::Workspace<'_>) {
    let name = work.browser.table.clone();
    // The Tables workspace on its list of tables: nothing is selected.
    if name.is_empty() {
        theme::heading(ui, "Table");
        theme::note(ui, "Choose a table to see its file and the open row's references.");
        return;
    }
    let session = &mut *work.session;
    theme::heading(ui, "Table");
    theme::row(ui, "file", |ui| {
        ui.label(theme::number(format!("{name}.dbc")));
    });
    if let Some(open) = session.table(&name) {
        let rows = open.record_count();
        let fields = open.field_count();
        theme::row(ui, "rows", |ui| {
            ui.label(theme::number(format!("{rows} x {fields}")));
        });
    }
    let unsaved = session.unsaved_tables.contains(&name);
    theme::row(ui, "status", |ui| match unsaved {
        true => {
            ui.label(egui::RichText::new("unsaved").color(theme::WARN));
        }
        false => {
            ui.label(egui::RichText::new("saved").color(theme::INK_DIM));
        }
    });
    if unsaved && theme::primary(ui, "Save table").clicked() {
        session.save_table(&name);
    }
    ui.add_space(4.0);

    super::data::references(ui, &mut work);
}

/// What undo would take back, and what redo would put in.
fn history(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    assets: &vale_client::assets::GameAssets,
) {
    let undo = session.history.next_undo().map(|c| c.label.clone());
    let redo = session.history.next_redo().map(|c| c.label.clone());
    ui.horizontal(|ui| {
        let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if ui
            .add_enabled(
                undo.is_some(),
                egui::Button::new("Undo").min_size(egui::vec2(half, 22.0)),
            )
            .on_hover_text(match &undo {
                Some(label) => format!("Ctrl+Z — {label}"),
                None => "Ctrl+Z".to_string(),
            })
            .clicked()
        {
            session.undo(assets);
        }
        if ui
            .add_enabled(
                redo.is_some(),
                egui::Button::new("Redo").min_size(egui::vec2(half, 22.0)),
            )
            .on_hover_text(match &redo {
                Some(label) => format!("Ctrl+Y — {label}"),
                None => "Ctrl+Y".to_string(),
            })
            .clicked()
        {
            session.redo(assets);
        }
    });
    let depth = session.history.depth_done();
    theme::note(
        ui,
        match (&undo, depth) {
            (Some(label), 1) => format!("{label} — 1 step to undo"),
            (Some(label), n) => format!("{label} — {n} steps to undo"),
            (None, _) => "nothing to undo".to_string(),
        },
    );
}

/// The measuring tool: the span between the two kept points, the points,
/// and what is under the pointer. See [`crate::tools::measure`].
///
/// The live pointer rows are last because their height changes as the
/// pointer moves on and off the world. Placed above the other rows, they
/// would move everything below them each time the height changed.
fn measure(
    ui: &mut egui::Ui,
    measuring: &mut crate::tools::measure::Measuring,
    session: &mut EditSession,
    cursor: &Cursor,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
) {
    use crate::tools::measure;

    theme::note(
        ui,
        "Click a first point, then a second to measure between them. A third \
         click starts over; Escape clears. Changes nothing.",
    );

    ui.add_space(4.0);
    theme::heading(ui, "Between points");
    match measuring.span() {
        Some(span) => {
            theme::row(ui, "distance", |ui| {
                ui.label(theme::number(format!("{:.2} yd", span.distance)));
            });
            theme::row(ui, "horizontal", |ui| {
                ui.label(theme::number(format!("{:.2} yd", span.horizontal)))
                    .on_hover_text("The distance in the x-y plane, ignoring height.");
            });
            theme::row(ui, "rise", |ui| {
                ui.label(theme::number(format!("{:+.2} yd", span.rise)));
            });
            theme::row(ui, "slope", |ui| {
                ui.label(theme::number(format!("{:+.1}°", span.slope)).color(slope_colour(span.slope.abs())))
                    .on_hover_text(
                        "The angle of the straight line between the points, not of the \
                         ground between them. A character cannot climb ground steeper \
                         than about 50 degrees.",
                    );
            });
            theme::row(ui, "facing", |ui| {
                ui.label(theme::number(format!(
                    "{:.4}  ({:.1}°)",
                    span.facing,
                    span.facing.to_degrees()
                )))
                .on_hover_text(
                    "The direction from the first point to the second, in radians from \
                     north (+x) toward west (+y). It is the value a creature or game \
                     object row's orientation column needs for a spawn at the first \
                     point to face the second.",
                );
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(format!("{:.4}", span.facing));
                }
            });
        }
        None => theme::note(ui, "pick two points to measure between them"),
    }
    if measuring.first.is_some() {
        ui.add_space(2.0);
        if ui.button("Clear points").clicked() {
            measuring.clear();
        }
    }

    // The second point's tile and area are left out when they are the first
    // point's, which is the usual case for two points a few yards apart.
    let points = [
        ("First point", measuring.first, None),
        ("Second point", measuring.second, measuring.first),
    ];
    for (name, probe, beside) in points {
        ui.add_space(6.0);
        theme::heading(ui, name);
        match probe {
            Some(probe) => probe_rows(ui, &probe, session, tables, true, beside.as_ref()),
            None => theme::note(ui, "not picked"),
        }
    }

    ui.add_space(6.0);
    theme::heading(ui, "Under the pointer");
    match cursor.surface {
        Some(at) => {
            let probe = measure::probe(session, at, cursor.ground);
            probe_rows(ui, &probe, session, tables, false, None);
        }
        None => theme::note(ui, "pointer is not over an open tile"),
    }
}

/// The colour a slope in degrees is drawn in: warning past 35, bad past the
/// climbing limit.
fn slope_colour(degrees: f32) -> egui::Color32 {
    match degrees {
        d if d > crate::tools::measure::CLIMB_LIMIT => theme::BAD,
        d if d > 35.0 => theme::WARN,
        _ => theme::INK,
    }
}

/// The rows describing one point. `kept` adds the two Copy buttons, which a
/// point under a moving pointer cannot be clicked for. With `beside`, the tile
/// and area rows are left out where they are the same as that point's.
fn probe_rows(
    ui: &mut egui::Ui,
    probe: &crate::tools::measure::Probe,
    session: &EditSession,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    kept: bool,
    beside: Option<&crate::tools::measure::Probe>,
) {
    use crate::tools::measure::{self, On};

    let at = probe.at;
    theme::row(ui, "position", |ui| {
        ui.label(theme::number(format!("{:.2} {:.2} {:.2}", at.x, at.y, at.z)));
    });
    if kept {
        theme::row(ui, "", |ui| {
            if ui
                .small_button("Copy position")
                .on_hover_text("x y z, as three numbers separated by spaces.")
                .clicked()
            {
                ui.ctx().copy_text(measure::position_text(at));
            }
            if ui
                .small_button("Copy .go command")
                .on_hover_text(
                    ".go xyz x y z map: vmangos' GM command to teleport to this point, \
                     for the playtest's chat line.",
                )
                .clicked()
            {
                ui.ctx().copy_text(measure::go_command(at, session.map_id));
            }
        });
    }
    theme::row(ui, "surface", |ui| {
        let text = match (probe.on, probe.ground) {
            (On::Ground, _) => "ground".to_string(),
            (On::Model, Some(ground)) => format!("model, {:.2} yd above the ground", at.z - ground),
            (On::Model, None) => "model, no ground below".to_string(),
        };
        ui.label(egui::RichText::new(text).color(theme::INK));
    });
    theme::row(ui, "ground", |ui| {
        let text = match (probe.ground, probe.hole, probe.chunk) {
            (Some(z), _, _) => format!("{z:.2} z"),
            (None, true, _) => "a hole in the ground".to_string(),
            (None, false, Some(_)) => "none".to_string(),
            (None, false, None) => "no open tile".to_string(),
        };
        ui.label(theme::number(text));
    });
    if beside.is_none_or(|other| other.tile != probe.tile) {
        theme::row(ui, "tile", |ui| {
            ui.label(theme::number(format!(
                "{}, {}  {}_{}_{}",
                probe.tile.0, probe.tile.1, session.map, probe.tile.0, probe.tile.1
            )));
        });
    }
    if let Some(chunk) = probe.chunk {
        theme::row(ui, "chunk", |ui| {
            ui.label(theme::number(format!("{chunk}  ({}, {})", chunk % 16, chunk / 16)))
                .on_hover_text("The MCNK index, then its column and row in the tile's 16x16 grid.");
        });
    }
    if let Some(id) = probe.area.filter(|&id| beside.is_none_or(|other| other.area != Some(id))) {
        let table = tables.and_then(|tables| tables.areas());
        let zone = table.and_then(|table| table.zone_of(id)).filter(|zone| zone.id != id);
        let text = match (id, table.and_then(|table| table.get(id)), zone) {
            (0, _, _) => "none (0)".to_string(),
            (id, Some(area), Some(zone)) => format!("{} ({id}), in {}", area.name, zone.name),
            (id, Some(area), None) => format!("{} ({id})", area.name),
            (id, None, _) => format!("area {id}"),
        };
        theme::row(ui, "area", |ui| {
            ui.label(egui::RichText::new(text).color(theme::INK));
        });
    }
    if let Some(slope) = probe.slope {
        theme::row(ui, "slope", |ui| {
            ui.label(theme::number(format!("{slope:.1}°")).color(slope_colour(slope)))
                .on_hover_text(
                    "The terrain's slope at this point. A character cannot climb ground \
                     steeper than about 50 degrees, and the server's navmesh marks such \
                     ground as a steep slope.",
                );
        });
    }
    if let Some((kind, level, flags)) = probe.water {
        let flags = crate::tools::water::flag_words(flags);
        let name = measure::liquid_name(kind);
        let text = match probe.ground {
            Some(z) => format!("{name} at {level:.2} z, {:.2} yd deep; {flags}", level - z),
            None => format!("{name} at {level:.2} z; {flags}"),
        };
        theme::row(ui, "water", |ui| {
            ui.label(egui::RichText::new(text).color(theme::INK));
        });
    }
    if probe.impassable {
        theme::row(ui, "flags", |ui| {
            ui.label(egui::RichText::new("impassable chunk").color(theme::WARN))
                .on_hover_text("MCNK flag 0x02. vmangos reads it when it builds mmaps.");
        });
    }
}

/// The grading tool: two points, a width, and the ramp between them.
///
/// The panel is mostly a report: which ends are picked, how long the grade is
/// and how steep. The slope decides whether the ramp is walkable, and two
/// clicks in the viewport do not show it.
fn grading(
    ui: &mut egui::Ui,
    grading: &mut crate::tools::grade::Grading,
    session: &mut EditSession,
) {
    use crate::tools::grade;

    theme::heading(ui, "Grade");
    theme::note(
        ui,
        "Click a start, then an end; a third click starts over. Each end takes \
         the ground height at the click, and either height can be edited.",
    );

    ui.add_space(4.0);
    for (name, end) in [
        ("start", grading.from.as_mut()),
        ("end", grading.to.as_mut()),
    ] {
        theme::row(ui, name, |ui| match end {
            Some(end) => {
                ui.add(
                    egui::DragValue::new(&mut end.height)
                        .speed(0.25)
                        .suffix(" z")
                        .fixed_decimals(1),
                )
                .on_hover_text(
                    "The height of this end of the ramp, taken from the ground at the \
                     click. Edit it to grade to a height that has no ground at it, \
                     such as a terrace or a bridge deck.",
                );
                ui.label(theme::number(format!(
                    "{:>8.1} {:>8.1}",
                    end.at[0], end.at[1]
                )));
            }
            None => theme::note(ui, "not picked"),
        });
    }

    // The length and slope between the two ends, which the viewport does not
    // show.
    ui.add_space(2.0);
    match grading.grade().and_then(|g| g.slope().map(|s| (g, s))) {
        Some((grade, slope)) => {
            theme::row(ui, "length", |ui| {
                ui.label(theme::number(format!("{:.1} y", grade.length())));
            });
            theme::row(ui, "slope", |ui| {
                let degrees = slope.atan().to_degrees();
                // 1.12's limit is about 50 degrees; above it a character
                // slides rather than climbs. The colour marks slopes near and
                // over that limit.
                let colour = match degrees.abs() {
                    d if d > 50.0 => theme::BAD,
                    d if d > 35.0 => theme::WARN,
                    _ => theme::INK,
                };
                ui.label(
                    theme::number(format!(
                        "{:.1}:1  ({degrees:.0}°)",
                        1.0 / slope.abs().max(1e-3)
                    ))
                    .color(colour),
                );
            });
        }
        None => theme::note(ui, "pick both ends to see the length and the slope"),
    }

    ui.add_space(6.0);
    theme::heading(ui, "Shape");
    theme::row(ui, "width", |ui| {
        ui.add(
            egui::Slider::new(&mut grading.width, 2.0..=200.0)
                .fixed_decimals(0)
                .suffix(" y"),
        )
        .on_hover_text("How wide the flat part of the ramp is, from its centre line to a side.");
    });
    theme::row(ui, "falloff", |ui| {
        ui.add(
            egui::Slider::new(&mut grading.falloff, 0.0..=200.0)
                .fixed_decimals(0)
                .suffix(" y"),
        )
        .on_hover_text(
            "How far past the width the ramp blends back into the surrounding ground. \
             Zero leaves a vertical face along both sides.",
        );
    });
    theme::row(ui, "strength", |ui| {
        ui.add(egui::Slider::new(&mut grading.strength, 0.05..=1.0).fixed_decimals(2))
            .on_hover_text(
                "How far toward the ramp the ground moves. Applied once per press, not \
                 accumulated over a stroke.",
            );
    });

    objects_follow(ui, &mut grading.objects_follow);

    ui.add_space(6.0);
    let ready = grading.grade().and_then(|g| g.slope()).is_some();
    if theme::primary(ui, "Apply grade").clicked() && ready {
        let moved = grade::apply(session, grading);
        grading.said = match moved {
            0 => "nothing moved: the ground already has that shape".to_string(),
            n => format!("{n} chunks graded"),
        };
        session.status = grading.said.clone();
    }
    if !ready {
        theme::note(ui, "pick two ends at different points");
    }
    ui.add_space(2.0);
    if ui.button("Clear ends").clicked() {
        grading.clear();
    }
    if !grading.said.is_empty() {
        ui.add_space(4.0);
        theme::note(ui, grading.said.clone());
    }
}

/// The road tool: the points, the shape the road gives the ground, the two
/// textures it paints, and the tileset list that chooses them.
fn road(
    ui: &mut egui::Ui,
    road: &mut crate::tools::road::RoadTool,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
) {
    use crate::tools::road::{self, Slot};

    theme::heading(ui, "Road");
    theme::note(
        ui,
        "Click the ground to add a point at the end of the road. Drag a point to \
         move it; Shift+click a point to remove it. Backspace removes the last \
         point and Enter applies. To change a road, undo it, adjust it and apply \
         again.",
    );

    // The line: its length, its ends and its steepest part.
    ui.add_space(4.0);
    let path = road::path(session, road);
    theme::row(ui, "points", |ui| {
        ui.label(theme::number(road.points.len().to_string()));
    });
    match &path {
        Some(path) => {
            theme::row(ui, "length", |ui| {
                ui.label(theme::number(format!("{:.1} y", path.length())));
            });
            if let Some((first, last)) = path.ends() {
                theme::row(ui, "ends", |ui| {
                    ui.label(theme::number(format!("{first:.1} z to {last:.1} z")));
                });
            }
            theme::row(ui, "steepest", |ui| {
                let degrees = path.steepest(8.0).atan().to_degrees();
                // As on the Grade panel: above about 50 degrees a character
                // slides rather than climbs.
                let colour = match degrees {
                    d if d > 50.0 => theme::BAD,
                    d if d > 35.0 => theme::WARN,
                    _ => theme::INK,
                };
                ui.label(theme::number(format!("{degrees:.0}°")).color(colour))
                    .on_hover_text("The steepest slope along the centre line, measured over runs of 8 yards.");
            });
        }
        None => theme::note(ui, "place at least two points"),
    }

    ui.add_space(6.0);
    theme::heading(ui, "Line");
    ui.checkbox(&mut road.curved, "Curve through the points")
        .on_hover_text("Off: straight segments between the points.");
    theme::segmented(
        ui,
        &mut road.follow_ground,
        &[("Height from ground", true), ("Height from points", false)],
        |a, b| a == b,
    );
    match road.follow_ground {
        true => {
            theme::row(ui, "smoothing", |ui| {
                ui.add(
                    egui::Slider::new(&mut road.smoothing, 0.0..=200.0)
                        .fixed_decimals(0)
                        .suffix(" y"),
                )
                .on_hover_text(
                    "The road follows the ground under its centre line, averaged over \
                     this many yards. Zero follows every bump.",
                );
            });
        }
        false => {
            theme::note(ui, "each point's height, joined along the line");
            let mut remove = None;
            egui::ScrollArea::vertical()
                .id_salt("road-points")
                .max_height(140.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    for (i, point) in road.points.iter_mut().enumerate() {
                        theme::row(ui, &format!("point {}", i + 1), |ui| {
                            ui.add(
                                egui::DragValue::new(&mut point.height)
                                    .speed(0.25)
                                    .suffix(" z")
                                    .fixed_decimals(1),
                            );
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                });
            if let Some(i) = remove {
                road.remove(i);
            }
        }
    }

    ui.add_space(6.0);
    theme::heading(ui, "Ground");
    ui.checkbox(&mut road.shape_ground, "Shape the ground")
        .on_hover_text("Off: the apply paints the textures and leaves the heights unchanged.");
    if road.shape_ground {
        theme::row(ui, "width", |ui| {
            ui.add(egui::Slider::new(&mut road.width, 0.5..=60.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("From the centre line to the edge of the part moved fully to the road's height.");
        });
        theme::row(ui, "shoulder", |ui| {
            ui.add(egui::Slider::new(&mut road.shoulder, 0.0..=100.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text(
                    "How far past the width the road blends back into the ground. Zero \
                     leaves a vertical face along both sides.",
                );
        });
        theme::row(ui, "strength", |ui| {
            ui.add(egui::Slider::new(&mut road.strength, 0.05..=1.0).fixed_decimals(2))
                .on_hover_text("How far toward the road's height the ground moves.");
        });
        theme::row(ui, "crown", |ui| {
            ui.add(egui::Slider::new(&mut road.crown, -2.0..=2.0).fixed_decimals(2).suffix(" y"))
                .on_hover_text(
                    "How far the centre stands above the edges. Negative makes a dished \
                     road.",
                );
        });
        theme::row(ui, "offset", |ui| {
            ui.add(egui::Slider::new(&mut road.offset, -10.0..=10.0).fixed_decimals(2).suffix(" y"))
                .on_hover_text(
                    "Added to the road's height everywhere. Negative sinks the road into \
                     the ground; positive raises it as a causeway.",
                );
        });
        ui.label(egui::RichText::new("shoulder falloff").size(theme::SMALL).color(theme::INK_DIM));
        theme::segmented(ui, &mut road.falloff, &FALLOFFS, |a, b| a == b);
        objects_follow(ui, &mut road.objects_follow);
    }

    ui.add_space(6.0);
    theme::heading(ui, "Surface texture");
    ui.checkbox(&mut road.paint_surface, "Paint a surface")
        .on_hover_text("Paint a texture down the middle of the road.");
    if road.paint_surface {
        road_texture(ui, thumbnails, &road.surface.texture, &mut road.surface.effect_id);
        theme::row(ui, "width", |ui| {
            ui.add(egui::Slider::new(&mut road.surface.width, 0.5..=60.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("From the centre line to where the texture starts to fade.");
        });
        theme::row(ui, "softness", |ui| {
            ui.add(egui::Slider::new(&mut road.surface.softness, 0.0..=20.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("How far past its width the texture fades out.");
        });
        theme::row(ui, "opacity", |ui| {
            ui.add(egui::Slider::new(&mut road.surface.opacity, 0.05..=1.0).fixed_decimals(2))
                .on_hover_text("How visible the texture is where it is solid.");
        });
        theme::row(ui, "wear", |ui| {
            ui.add(egui::Slider::new(&mut road.surface.wear, 0.0..=0.9).fixed_decimals(2))
                .on_hover_text(
                    "How much of the surface is left unpainted in patches, so the ground \
                     under it shows through. Zero is solid.",
                );
        });
    }

    ui.add_space(6.0);
    theme::heading(ui, "Verge texture");
    ui.checkbox(&mut road.paint_verge, "Paint a verge")
        .on_hover_text(
            "Paint a second texture in a band along both sides of the surface, starting \
             under the surface's fade.",
        );
    if road.paint_verge {
        road_texture(ui, thumbnails, &road.verge.texture, &mut road.verge.effect_id);
        theme::row(ui, "width", |ui| {
            ui.add(egui::Slider::new(&mut road.verge.width, 0.5..=40.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("How far the band reaches past the surface's width.");
        });
        theme::row(ui, "softness", |ui| {
            ui.add(egui::Slider::new(&mut road.verge.softness, 0.0..=20.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("How far past its width the band's outer edge fades out.");
        });
        theme::row(ui, "opacity", |ui| {
            ui.add(egui::Slider::new(&mut road.verge.opacity, 0.05..=1.0).fixed_decimals(2))
                .on_hover_text("How visible the texture is where it is solid.");
        });
    }

    if road.paint_surface || road.paint_verge {
        ui.add_space(6.0);
        theme::heading(ui, "Edges");
        theme::row(ui, "ragged", |ui| {
            ui.add(egui::Slider::new(&mut road.ragged, 0.0..=5.0).fixed_decimals(1).suffix(" y"))
                .on_hover_text("How far the painted edges wander in and out of a smooth line. Zero is smooth.");
        });
        theme::row(ui, "grain", |ui| {
            ui.add(egui::Slider::new(&mut road.grain, 1.0..=40.0).fixed_decimals(0).suffix(" y"))
                .on_hover_text("The size, in yards, of one wander of the edges and of one patch of wear.");
        });
        ui.checkbox(&mut road.reuse_hidden, "Reuse a hidden layer").on_hover_text(
            "When a chunk already has four textures, give the road's texture the layer \
             that shows least, if it shows almost nothing. Off: such a chunk is not painted.",
        );
    }

    ui.add_space(6.0);
    if theme::primary(ui, "Apply road").clicked() {
        road::apply_and_report(session, road);
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!road.points.is_empty(), egui::Button::new("Remove last point"))
            .clicked()
        {
            road.points.pop();
        }
        if ui
            .add_enabled(!road.points.is_empty(), egui::Button::new("Clear points"))
            .clicked()
        {
            road.clear();
        }
    });
    if !road.said.is_empty() {
        ui.add_space(4.0);
        theme::note(ui, road.said.clone());
    }

    // The tileset list, which sets whichever of the two textures is chosen
    // above it.
    if road.paint_surface || road.paint_verge {
        ui.add_space(6.0);
        theme::heading(ui, "Tilesets");
        theme::segmented(
            ui,
            &mut road.slot,
            &[("Set surface", Slot::Surface), ("Set verge", Slot::Verge)],
            |a, b| a == b,
        );
        if ui
            .add_enabled(
                !textures.brush.texture.is_empty(),
                egui::Button::new("Use the Textures tool's texture"),
            )
            .on_hover_text("Set the chosen texture to the one on the Textures tool's brush.")
            .clicked()
        {
            road.choose(textures.brush.texture.clone(), textures.brush.effect_id);
        }
        let chosen = match road.slot {
            Slot::Surface => road.surface.texture.clone(),
            Slot::Verge => road.verge.texture.clone(),
        };
        if let Some(Some(path)) = tileset_browser(ui, textures, thumbnails, &chosen) {
            // A road texture grows nothing unless asked: grass down the
            // middle of a road is rarely what was meant. The ground effect is
            // set beside the texture.
            road.choose(path, 0);
        }
    }
}

/// One of the road's textures: its picture and name, and its ground effect.
fn road_texture(ui: &mut egui::Ui, thumbnails: &mut Thumbnails, path: &str, effect_id: &mut u32) {
    if path.is_empty() {
        theme::note(ui, "no texture chosen; choose one in the tileset list below");
        return;
    }
    ui.horizontal(|ui| {
        swatch(ui, thumbnails, path);
        ui.add(egui::Label::new(egui::RichText::new(textures::leaf(path)).color(theme::INK)).truncate())
            .on_hover_text(path.to_string());
    });
    theme::row(ui, "ground effect", |ui| {
        ui.add(egui::DragValue::new(effect_id).speed(1.0).range(0..=u32::MAX))
            .on_hover_text(
                "MCLY effectId for a layer the road adds: the GroundEffectTexture row \
                 whose doodads grow on it. 0 grows none.",
            );
    });
}

/// The height brush.
fn brush(ui: &mut egui::Ui, terrain: &mut Terrain) {
    // What the left button does: move the ground, or choose vertices. It is
    // above the brush because it decides which of the two panels follows.
    theme::heading(ui, "Pointer");
    theme::segmented(
        ui,
        &mut terrain.selecting,
        &[("Sculpt", false), ("Select vertices", true)],
        |a, b| a == b,
    );
    ui.add_space(4.0);
    if terrain.selecting {
        vertex_panel(ui, terrain);
        return;
    }

    theme::heading(ui, "Brush");
    theme::segmented(ui, &mut terrain.brush.mode, &MODES, |a, b| {
        // A flatten's target is set when the stroke begins, so the variant held
        // here carries a stale height and cannot be compared whole.
        std::mem::discriminant(a) == std::mem::discriminant(b)
    });

    ui.add_space(4.0);
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut terrain.brush.radius)
                .speed(0.5)
                .range(terrain::RADIUS)
                .suffix(" yd"),
        );
    });
    // The limits are the tool's own constants, which document the reason for
    // each. A range written as a literal in the panel would have no stated
    // reason.
    let (label, range, speed) = match terrain.brush.mode {
        Mode::Raise | Mode::Lower => ("strength", terrain::STRENGTH, 0.25),
        _ => ("strength", terrain::RATE, 0.05),
    };
    theme::row(ui, label, |ui| {
        ui.add(
            egui::DragValue::new(&mut terrain.brush.strength)
                .speed(speed)
                .range(range)
                .suffix(match terrain.brush.mode {
                    Mode::Raise | Mode::Lower => " yd/s",
                    _ => "/s",
                }),
        );
    });
    // The fraction of the radius that applies full strength: the inner ring
    // the preview draws. See `vale_edit::ops::Falloff::over`.
    theme::row(ui, "core", |ui| {
        ui.add(
            egui::DragValue::new(&mut terrain.brush.core)
                .speed(0.01)
                .range(crate::tools::CORE)
                .fixed_decimals(2),
        );
    });
    // Shown for Noise only, the one mode that reads it. A field shown but
    // ignored in four modes of five would suggest that the panel's fields do
    // not all apply.
    if matches!(terrain.brush.mode, Mode::Noise) {
        theme::row(ui, "scale", |ui| {
            ui.add(
                egui::DragValue::new(&mut terrain.brush.scale)
                    .speed(0.5)
                    .range(terrain::SCALE)
                    .suffix(" yd"),
            );
        });
        theme::note(
            ui,
            "noise feature size; under 4 yd is finer than the vertex grid",
        );
    }

    ui.add_space(4.0);
    theme::heading(ui, "Falloff");
    theme::segmented(ui, &mut terrain.brush.falloff, &FALLOFFS, |a, b| a == b);

    ui.add_space(4.0);
    theme::heading(ui, "Shape");
    theme::segmented(ui, &mut terrain.brush.shape, &SHAPES, |a, b| a == b);
    theme::note(ui, "the square shape is aligned to the chunk grid");

    if matches!(terrain.brush.mode, Mode::Flatten { .. }) {
        flatten_panel(ui, terrain);
    }

    ui.add_space(4.0);
    theme::heading(ui, "Objects");
    objects_follow(ui, &mut terrain.objects_follow);

    // The selection, while there is one, and the locks, while there are any.
    // What is done with either is on the Select vertices half.
    if terrain.selected > 0 || terrain.locked > 0 {
        ui.add_space(4.0);
        theme::heading(ui, "Vertices");
        if terrain.selected > 0 {
            only_inside(ui, terrain);
        }
        if terrain.locked > 0 {
            theme::note(
                ui,
                format!("{} locked on the open tiles; strokes leave them in place", terrain.locked),
            );
        }
    }

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button paints");
    theme::note(ui, "ctrl + wheel resizes · shift + wheel the strength");
    theme::note(ui, "alt + wheel the core");
    theme::note(ui, "1 - 5 pick the mode · 6 - 0 the falloff");
    theme::note(ui, "shift + 1 · 2 · 3 pick the shape");
}

/// What a flatten goes to: which height, which way it may move the ground,
/// and whether the target is level or a tilted plane.
fn flatten_panel(ui: &mut egui::Ui, terrain: &mut Terrain) {
    use vale_edit::ops::Only;
    ui.add_space(4.0);
    theme::heading(ui, "Flatten to");
    ui.checkbox(&mut terrain.flatten_to_cursor, "height under the pointer")
        .on_hover_text("Sampled where the stroke starts. Off: the height set below.");
    if !terrain.flatten_to_cursor {
        theme::row(ui, "height", |ui| {
            ui.add(
                egui::DragValue::new(&mut terrain.flatten_height)
                    .speed(0.25)
                    .fixed_decimals(1)
                    .suffix(" yd"),
            );
        });
    }
    theme::segmented(
        ui,
        &mut terrain.brush.only,
        &[("Both", Only::Both), ("Fill", Only::Fill), ("Cut", Only::Cut)],
        |a, b| a == b,
    );
    theme::note(
        ui,
        match terrain.brush.only {
            Only::Both => "raises low ground and lowers high ground",
            Only::Fill => "only raises low ground",
            Only::Cut => "only lowers high ground",
        },
    );
    tilt_rows(ui, &mut terrain.tilt_angle, &mut terrain.tilt_toward);
    if terrain.tilt_angle > 0.0 {
        theme::note(ui, "the slope pivots where the stroke starts");
    }
}

/// A tilt's two numbers: how steep, and which way is up. Answers whether
/// either changed.
fn tilt_rows(ui: &mut egui::Ui, angle: &mut f32, toward: &mut f32) -> bool {
    let mut changed = false;
    theme::row(ui, "tilt", |ui| {
        changed |= ui.add(
            egui::DragValue::new(angle)
                .speed(0.25)
                .range(0.0..=60.0)
                .fixed_decimals(0)
                .suffix("\u{b0}"),
        )
        .on_hover_text("Slope in degrees. 0 is level.")
        .changed();
        if *angle == 0.0 {
            theme::note(ui, "level");
        }
    });
    // The bearing only matters once there is a slope.
    if *angle > 0.0 {
        theme::row(ui, "uphill", |ui| {
            changed |= ui.add(
                egui::DragValue::new(&mut *toward)
                    .speed(1.0)
                    .range(0.0..=359.0)
                    .fixed_decimals(0)
                    .suffix("\u{b0}"),
            )
            .on_hover_text("Compass bearing of the uphill side. 0 is north, 90 east.")
            .changed();
            theme::note(ui, compass(*toward));
        });
    }
    changed
}

/// The nearest of the eight compass points to a bearing in degrees.
fn compass(bearing: f32) -> &'static str {
    const POINTS: [&str; 8] = ["north", "north-east", "east", "south-east", "south", "south-west", "west", "north-west"];
    POINTS[((bearing.rem_euclid(360.0) / 45.0).round() as usize) % 8]
}

/// Whether brush strokes move only the selected vertices.
fn only_inside(ui: &mut egui::Ui, terrain: &mut Terrain) {
    ui.checkbox(&mut terrain.only_inside, "Only inside the selection").on_hover_text(
        "On: a brush stroke moves the selected vertices and nothing else, and the \
         selection is drawn green. Off: strokes move any vertex under the brush that \
         is not locked. To keep vertices from moving, lock them.",
    );
}

/// The locked vertices: how many the open tiles hold, and the four ways to
/// change them.
fn vertex_locks(ui: &mut egui::Ui, terrain: &mut Terrain) {
    use crate::tools::terrain::VertexAsk;
    let any = terrain.selected > 0;
    theme::note(
        ui,
        match terrain.locked {
            0 => "none on the open tiles".to_string(),
            n => format!("{n} on the open tiles, drawn red"),
        },
    );
    ui.horizontal_wrapped(|ui| {
        for (label, ask, about, enabled) in [
            (
                "Lock selection",
                VertexAsk::Lock,
                "Add the selected vertices to their tiles' locked vertices.",
                any,
            ),
            (
                "Unlock selection",
                VertexAsk::Unlock,
                "Remove the selected vertices from their tiles' locked vertices.",
                any,
            ),
            (
                "Lock tile sides",
                VertexAsk::LockSides,
                "Lock the vertices on all four sides of each tile that holds part of the \
                 selection. A tile's side is shared with the tile beside it, so this keeps \
                 the ground at the seam level with a tile that is not being edited.",
                any,
            ),
            (
                "Unlock all",
                VertexAsk::UnlockAll,
                "Unlock every vertex of every open tile.",
                terrain.locked > 0,
            ),
        ] {
            let disabled_why = match ask {
                VertexAsk::UnlockAll => "No vertex on the open tiles is locked.",
                _ => "Select some vertices first.",
            };
            if ui
                .add_enabled(enabled, egui::Button::new(label))
                .on_hover_text(about)
                .on_disabled_hover_text(disabled_why)
                .clicked()
            {
                terrain.ask = Some(ask);
            }
        }
    });
    theme::note(
        ui,
        "no height operation moves a locked vertex: brush strokes, Grade, Stitch, \
         chunk paste, height import, and Level, Smooth, Tilt and height here. Locks \
         are saved in the project at once and are not on the undo history.",
    );
}

/// The terrain tool's panel while the left button selects vertices: the
/// footprint a press selects, and what is done with the selection.
fn vertex_panel(ui: &mut egui::Ui, terrain: &mut Terrain) {
    use crate::tools::terrain::VertexAsk;
    theme::heading(ui, "Footprint");
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut terrain.brush.radius)
                .speed(0.5)
                .range(terrain::RADIUS)
                .suffix(" yd"),
        );
    });
    theme::segmented(ui, &mut terrain.brush.shape, &SHAPES, |a, b| a == b);

    ui.add_space(4.0);
    theme::heading(ui, "Selection");
    let any = terrain.selected > 0;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(match terrain.selected {
                0 => "nothing selected".to_string(),
                n => format!("{n} selected"),
            })
            .color(match any {
                true => theme::INK,
                false => theme::INK_DIM,
            }),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(any, egui::Button::new("Clear").small())
                .clicked()
            {
                terrain.ask = Some(VertexAsk::Clear);
            }
        });
    });
    // Dragging the mean height moves every selected vertex by the same
    // amount, so the shape they make is kept.
    theme::row(ui, "height", |ui| {
        let mut mean = terrain.selected_mean;
        let moved = ui
            .add_enabled(
                any,
                egui::DragValue::new(&mut mean)
                    .speed(0.1)
                    .fixed_decimals(1)
                    .suffix(" yd"),
            )
            .on_hover_text("Mean height. Drag or type to move the selection up or down.")
            .on_disabled_hover_text("Select some vertices first.");
        if moved.changed() && mean != terrain.selected_mean {
            terrain.ask = Some(VertexAsk::MoveTo(mean));
        }
    });
    ui.horizontal(|ui| {
        for (label, ask, about) in [
            ("Level", VertexAsk::Level, "Flatten the selection at its mean height."),
            (
                "Smooth",
                VertexAsk::Smooth,
                "Smooth the selection. Click again to smooth more.",
            ),
        ] {
            if ui
                .add_enabled(any, egui::Button::new(label))
                .on_hover_text(about)
                .on_disabled_hover_text("Select some vertices first.")
                .clicked()
            {
                terrain.ask = Some(ask);
            }
        }
    });

    ui.add_space(4.0);
    theme::heading(ui, "Tilt");
    // The selection follows the sliders, pivoting on its centre.
    let [angle, toward] = &mut terrain.vertex_tilt;
    if tilt_rows(ui, angle, toward) && any {
        terrain.ask = Some(VertexAsk::Tilt);
    }
    ui.checkbox(&mut terrain.tilt_flat, "flatten onto the slope")
        .on_hover_text("Off: the selection tilts and keeps its relief. On: it becomes a flat plane at the tilt.");
    theme::note(
        ui,
        match any {
            true => "changing these tilts the selection",
            false => "select some vertices to tilt them",
        },
    );

    ui.add_space(4.0);
    theme::heading(ui, "Locked vertices");
    vertex_locks(ui, terrain);

    ui.add_space(4.0);
    theme::heading(ui, "Brush");
    only_inside(ui, terrain);
    objects_follow(ui, &mut terrain.objects_follow);

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button selects \u{b7} with shift, deselects");
    theme::note(ui, "ctrl + wheel resizes the footprint");
    theme::note(ui, "shift + 1 \u{b7} 2 \u{b7} 3 pick the shape");
}

/// The switch the height brush and the grade share: whether what stands on
/// the ground is carried when the ground moves. One function, so the two
/// panels word it alike.
fn objects_follow(ui: &mut egui::Ui, on: &mut bool) {
    ui.checkbox(on, "Doodads and WMOs follow the ground").on_hover_text(
        "When the ground moves, every doodad and WMO on it moves up or down by the \
         same amount, in the same undo entry. Off: placements keep their stored \
         height and the ground moves through them. Creature and game object spawns \
         are rows in the world database and are not moved.",
    );
}

/// The shading brush: `MCCV`, the light painted onto the ground's vertices.
///
/// Colour and brightness are separate controls because `MCCV`'s midpoint is
/// 1.0 rather than 0.5. White at brightness one is the value that leaves a
/// vertex unchanged, and every other value departs from it. A single 0..1
/// swatch could not express "keep the hue and halve the light", which is the
/// most common shading edit.
///
/// See `crate::tools::shading` for the deviation this brush is part of.
fn shading(ui: &mut egui::Ui, shading: &mut crate::tools::shading::Shading) {
    use crate::tools::shading;

    theme::heading(ui, "Brush");
    theme::segmented(ui, &mut shading.brush.mode, &shading::MODES, |a, b| a == b);

    ui.add_space(4.0);
    theme::heading(ui, "Colour");
    // Only `Paint` reads the colour. The other four modes aim at values they
    // decide themselves (white, black, the mean, neutral), so a picker beside
    // them would do nothing. The height brush hides its scale row for the
    // same reason.
    match shading.brush.mode {
        vale_edit::ops::Shade::Paint => {
            theme::row(ui, "tint", |ui| {
                ui.color_edit_button_rgb(&mut shading.picked);
            });
        }
        _ => theme::note(ui, "this mode sets its own colour"),
    }
    theme::row(ui, "brightness", |ui| {
        ui.add(
            egui::DragValue::new(&mut shading.brightness)
                .speed(0.01)
                .range(0.0..=vale_edit::ops::SHADE_MAX)
                .fixed_decimals(2)
                .suffix("x"),
        );
    });
    // The combined value in the file's own units, which is what the region
    // stores and what is compared between two chunks.
    let aim = shading.target();
    theme::note(
        ui,
        format!(
            "target {:.2}, {:.2}, {:.2}; 1.00 leaves a vertex unchanged",
            aim[0], aim[1], aim[2]
        ),
    );
    if ui.button("Reset to neutral").clicked() {
        shading.picked = [1.0, 1.0, 1.0];
        shading.brightness = 1.0;
    }

    // The same three headings as the height brush, in the same order. With one
    // heading, Shape, over the radius, strength, core, falloff row and shape
    // row, the two unlabelled segmented controls at the bottom read as part of
    // the rows above them.
    ui.add_space(4.0);
    theme::heading(ui, "Size");
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut shading.brush.radius)
                .speed(0.5)
                .range(shading::RADIUS)
                .suffix(" yd"),
        );
    });
    theme::row(ui, "strength", |ui| {
        ui.add(
            egui::DragValue::new(&mut shading.brush.strength)
                .speed(0.05)
                .range(shading::RATE)
                .suffix("/s"),
        );
    });
    theme::row(ui, "core", |ui| {
        ui.add(
            egui::DragValue::new(&mut shading.brush.core)
                .speed(0.01)
                .range(crate::tools::CORE)
                .fixed_decimals(2),
        );
    });

    ui.add_space(4.0);
    theme::heading(ui, "Falloff");
    theme::segmented(ui, &mut shading.brush.falloff, &FALLOFFS, |a, b| a == b);

    ui.add_space(4.0);
    theme::heading(ui, "Shape");
    theme::segmented(ui, &mut shading.brush.shape, &SHAPES, |a, b| a == b);

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button paints");
    theme::note(ui, "ctrl + wheel resizes · shift + wheel the strength");
    theme::note(ui, "alt + wheel the core");
    theme::note(ui, "1 - 5 pick the mode · 6 - 0 the falloff");
    theme::note(ui, "shift + 1 · 2 · 3 pick the shape");

    ui.add_space(6.0);
    theme::heading(ui, "About");
    theme::note(
        ui,
        "MCCV is a per-vertex colour multiplier; the reference client does not \
         read it. A chunk painted back to neutral drops its MCCV data, so an \
         undone tile is written back byte for byte.",
    );
}

/// The texture brush.
#[allow(clippy::too_many_arguments)]
fn paint(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &mut EditSession,
    cursor: &Cursor,
    assets: &vale_client::assets::GameAssets,
    guides: &mut crate::tools::guides::Guides,
    now: f64,
) {
    tileset_chosen(ui, textures, thumbnails, session, assets);
    ui.add_space(4.0);
    if !tileset_picker(ui, textures, thumbnails, session) {
        return;
    }

    ui.add_space(4.0);
    theme::heading(ui, "Brush");
    // Paint or Erase first, because it decides what every number under it
    // means. Shift held is the other one for the length of a stroke.
    theme::segmented(
        ui,
        &mut textures.brush.erase,
        &[("Paint", false), ("Erase", true)],
        |a, b| a == b,
    );
    ui.add_space(2.0);
    theme::row(ui, "radius", |ui| {
        ui.add(
            egui::DragValue::new(&mut textures.brush.radius)
                .speed(0.25)
                .range(textures::RADIUS)
                .suffix(" yd"),
        );
    });
    theme::row(ui, "strength", |ui| {
        ui.add(
            egui::DragValue::new(&mut textures.brush.strength)
                .speed(0.05)
                .range(textures::RATE)
                .suffix("/s"),
        );
    });
    // Opacity is where a held stroke stops, which an eraser does not have:
    // it always goes to nothing.
    let erasing = textures.brush.erase;
    theme::row(ui, "opacity", |ui| {
        let mut percent = textures.brush.opacity * 100.0;
        let response = ui
            .add_enabled(
                !erasing,
                egui::DragValue::new(&mut percent)
                    .speed(0.5)
                    .range(1.0..=100.0)
                    .fixed_decimals(0)
                    .suffix("%"),
            )
            .on_hover_text(
                "The texture's opacity at the end of a held stroke. At 100% the \
                 stroke converges on the texture alone; at 60%, on the texture at 60% \
                 over what is beneath it, from either side.",
            )
            .on_disabled_hover_text("Erase always reduces the texture to 0%.");
        if response.changed() {
            textures.brush.opacity = percent / 100.0;
        }
    });

    theme::row(ui, "core", |ui| {
        ui.add(
            egui::DragValue::new(&mut textures.brush.core)
                .speed(0.01)
                .range(crate::tools::CORE)
                .fixed_decimals(2),
        )
        .on_hover_text(
            "The share of the radius painted at full strength, drawn as the inner              ring. Outside it the paint fades along the falloff to nothing at the              outer ring, and a held stroke goes no further than that fade.",
        );
    });

    ui.add_space(4.0);
    theme::heading(ui, "Falloff");
    theme::segmented(
        ui,
        &mut textures.brush.falloff,
        &textures::FALLOFFS,
        |a, b| a == b,
    );

    ui.add_space(4.0);
    theme::heading(ui, "Shape");
    theme::segmented(ui, &mut textures.brush.shape, &textures::SHAPES, |a, b| {
        a == b
    });

    // Spray: a stroke in patches. One number while it is solid, and the
    // patch size only once it is not.
    ui.add_space(4.0);
    theme::heading(ui, "Spray");
    theme::row(ui, "density", |ui| {
        let mut percent = textures.brush.density * 100.0;
        if ui
            .add(
                egui::DragValue::new(&mut percent)
                    .speed(0.5)
                    .range(5.0..=100.0)
                    .fixed_decimals(0)
                    .suffix("%"),
            )
            .on_hover_text(
                "The share of the ground under the brush that is painted. At 100% the \
                 stroke is solid. Below 100% it paints in patches that stay fixed while \
                 the button is held, to break up the edge between two textures.",
            )
            .changed()
        {
            textures.brush.density = percent / 100.0;
        }
        if textures.brush.density >= 1.0 {
            theme::note(ui, "solid");
        }
    });
    if textures.brush.density < 1.0 {
        theme::row(ui, "grain", |ui| {
            ui.add(
                egui::DragValue::new(&mut textures.brush.grain)
                    .speed(0.05)
                    .range(0.5..=20.0)
                    .fixed_decimals(1)
                    .suffix(" yd"),
            )
            .on_hover_text("How many yards a patch is across. A blend map's texel is half a yard.");
        });
    }

    // The three switches about the limit of four textures to a chunk, which
    // is what stops a stroke on about half the shipped ground.
    ui.add_space(4.0);
    theme::heading(ui, "Four-texture limit");
    ui.checkbox(&mut textures.brush.existing_only, "Paint existing layers only")
        .on_hover_text(
            "Skip chunks that do not carry the texture. A stroke then changes only \
             the blend of existing layers and never adds a layer.",
        );
    ui.add_enabled(
        !textures.brush.existing_only && !textures.brush.erase,
        egui::Checkbox::new(&mut textures.brush.reuse_hidden, "Reuse a hidden layer"),
    )
    .on_hover_text(
        "When a chunk already has four textures, replace the least visible layer \
         if it covers under 2% of the chunk. Otherwise the stroke is refused on \
         that chunk, and the Chunk list below shows its four textures.",
    )
    .on_disabled_hover_text("Only a stroke that may add a texture can reuse a layer.");
    ui.checkbox(&mut guides.layers, "Show full chunks").on_hover_text(
        "Tint red the chunks that carry four textures. The same switch as Full chunks \
         in the view bar's Guides menu.",
    );

    ui.add_space(6.0);
    chunk_layers(ui, thumbnails, textures, session, cursor, assets, now);

    ui.add_space(6.0);
    theme::heading(ui, "Keys");
    theme::note(ui, "left button paints · with shift, erases");
    theme::note(ui, "ctrl + wheel resizes · shift + wheel the strength");
    theme::note(ui, "alt + wheel the core");
    theme::note(ui, "space pins the chunk under the pointer");
    theme::note(ui, "1 - 5 pick the falloff · shift + 1 · 2 · 3 the shape");
}

/// The texture brush's chosen texture, and what a new layer of it grows.
fn tileset_chosen(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &EditSession,
    assets: &vale_client::assets::GameAssets,
) {
    theme::heading(ui, "Texture");
    match textures.brush.texture.is_empty() {
        true => theme::note(ui, "no texture chosen; pick one below"),
        false => {
            let chosen = textures.brush.texture.clone();
            ui.horizontal(|ui| {
                swatch(ui, thumbnails, &chosen);
                // Truncated. `leaf` is a folder and a file name, and many of
                // the 621 are wider than the panel. A label in a horizontal
                // layout extends rather than wrapping, so an untruncated name
                // runs off the right-hand edge. The whole path is on the hover.
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(textures::leaf(&chosen)).color(theme::INK),
                    )
                    .truncate(),
                )
                .on_hover_text(chosen.clone());
            });
            // The foliage a new layer of this texture grows: `MCLY`'s
            // `effectId`. It is set, when the texture is chosen, from what the
            // open ground pairs this texture with, and can be edited here. See
            // `PaintBrush::effect_id`.
            theme::row_about(
                ui,
                "foliage",
                "What a new layer of this texture grows: MCLY effectId, a \
                 GroundEffectTexture row. Set, when the texture is chosen, to what \
                 the open tiles grow on it most.",
                |ui| {
                    let picked = ground_effect(
                        ui,
                        "brush",
                        textures.brush.effect_id,
                        true,
                        session,
                        &chosen,
                        assets,
                    );
                    if let Some(id) = picked {
                        textures.brush.effect_id = id;
                    }
                },
            );
        }
    }
}

/// The tileset picker: the search box and the list by folder. A click sets
/// the texture brush's chosen texture.
///
/// Shared by the texture brush's panel and the chunk selection's. Both act
/// with the brush's chosen texture, so there is one picker and one chosen
/// texture. Returns `false` while the tileset list is still being read, and
/// draws nothing below the search box then.
fn tileset_picker(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    session: &EditSession,
) -> bool {
    let chosen = textures.brush.texture.clone();
    let Some(picked) = tileset_browser(ui, textures, thumbnails, &chosen) else {
        return false;
    };
    if let Some(path) = picked {
        // Picking a texture also sets its foliage, from the ground already
        // painted with it. A texture on which no open layer grows anything
        // grows nothing, which matches the shipped ground.
        textures.brush.effect_id = textures::usual_effect(session, &path).unwrap_or(0);
        textures.brush.texture = path;
    }
    true
}

/// The search box and the tileset list by folder, with `chosen` marked.
/// Answers the path clicked this frame, or `None` while the list is still
/// being read. The search and the open folder are the texture tool's, so
/// every list opens where the last one was left.
fn tileset_browser(
    ui: &mut egui::Ui,
    textures: &mut Textures,
    thumbnails: &mut Thumbnails,
    chosen: &str,
) -> Option<Option<String>> {
    ui.add(
        egui::TextEdit::singleline(&mut textures.search)
            .hint_text("search the tilesets")
            .desired_width(f32::INFINITY),
    );
    if !textures.catalogue.iter().any(|path| !path.is_empty()) {
        theme::note(ui, "reading the tileset list…");
        return None;
    }
    // Folders, as in the model picker. `Tileset\Elwynn\` is the set a zone was
    // painted with, and with six hundred names in one scroller every one has
    // to be found by typing. Search still runs flat across the whole
    // catalogue; see [`browse`] for the reason.
    let needle = textures.search.to_ascii_lowercase();
    let row = SIDE as f32 + 6.0;
    let mut picked: Option<String> = None;
    if needle.is_empty() {
        let here = textures.folder.clone();
        let mut go = breadcrumb(ui, &here, "");
        let (folders, files) = split_folder(&textures.catalogue, &here);
        theme::note(
            ui,
            match folders.len() {
                0 => format!("{} tilesets", files.len()),
                n => format!("{n} folders · {} tilesets", files.len()),
            },
        );
        egui::ScrollArea::vertical()
            .id_salt("tilesets")
            .max_height(260.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                go = folder_rows(ui, &here, &folders, row).or(go.take());
                for path in &files {
                    if texture_row(ui, thumbnails, path, chosen) {
                        picked = Some(path.clone());
                    }
                }
            });
        if let Some(folder) = go {
            let mut folder = folder.to_ascii_lowercase();
            if !folder.ends_with(place::SEP) {
                folder.push(place::SEP);
            }
            textures.folder = folder;
        }
    } else {
        let matches: Vec<String> = textures
            .catalogue
            .iter()
            .filter(|path| !path.is_empty() && path.to_ascii_lowercase().contains(&needle))
            .cloned()
            .collect();
        theme::note(
            ui,
            format!("{} of {} match", matches.len(), textures.catalogue.len()),
        );
        // `show_rows` rather than `show`: it builds only the rows on screen,
        // so only those request a thumbnail and the other six hundred are
        // never decoded. See [`super::thumbnails`].
        egui::ScrollArea::vertical()
            .id_salt("tileset-search")
            .max_height(260.0)
            .auto_shrink([false, true])
            .show_rows(ui, row, matches.len(), |ui, range| {
                for path in &matches[range] {
                    if texture_row(ui, thumbnails, path, chosen) {
                        picked = Some(path.clone());
                    }
                }
            });
    }
    Some(picked)
}

/// One row of the tileset list: a picture, a name, and whether it was clicked.
///
/// Laid out by hand rather than built from a `Button`, because the row must be
/// the same height whether or not its picture has been decoded. If rows changed
/// size as thumbnails arrived, the list would move under the pointer.
fn texture_row(ui: &mut egui::Ui, thumbnails: &mut Thumbnails, path: &str, chosen: &str) -> bool {
    let height = SIDE as f32 + 4.0;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    let on = chosen.eq_ignore_ascii_case(path);
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
        if on {
            painter.rect_stroke(
                rect,
                egui::CornerRadius::same(3),
                egui::Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let square = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 3.0, rect.top() + 2.0),
            egui::Vec2::splat(SIDE as f32),
        );
        swatch_at(ui, thumbnails, path, square);
        // Two lines and a clip, because the names do not fit on one. The panel
        // is 288 points wide and `aeriepeaksgravelneedlesbase.blp` is wider;
        // painted as one line it runs off the edge of the window. The file name
        // identifies the texture and the folder groups it, so those are the two
        // lines. The extension is the same on all 621 and is dropped.
        let (folder, file) = split(path);
        let text = ui.painter().with_clip_rect(egui::Rect::from_min_max(
            egui::pos2(square.right() + 6.0, rect.top()),
            rect.right_bottom(),
        ));
        text.text(
            egui::pos2(square.right() + 6.0, rect.center().y - 7.0),
            egui::Align2::LEFT_CENTER,
            file,
            egui::FontId::proportional(12.0),
            match on {
                true => theme::INK,
                false => theme::INK_DIM,
            },
        );
        text.text(
            egui::pos2(square.right() + 6.0, rect.center().y + 7.0),
            egui::Align2::LEFT_CENTER,
            folder,
            egui::FontId::proportional(theme::SMALL),
            theme::INK_FAINT,
        );
    }
    response.on_hover_text(path.to_string()).clicked()
}

/// An archive path as the two parts of a list row: the folder it is in, and its
/// file name without the extension every one of them shares.
fn split(path: &str) -> (&str, &str) {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let rest = &path[..path.len() - file.len()];
    let folder = rest.trim_end_matches(['\\', '/']);
    let folder = folder.rsplit(['\\', '/']).next().unwrap_or("");
    (folder, file.trim_end_matches(".blp"))
}

/// A texture's picture, inline, at [`SIDE`] square.
fn swatch(ui: &mut egui::Ui, thumbnails: &mut Thumbnails, path: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(SIDE as f32), egui::Sense::hover());
    swatch_at(ui, thumbnails, path, rect);
}

/// [`swatch`] into a rectangle the caller has already allocated.
///
/// Draws a placeholder while the picture is being decoded, so the row keeps
/// its shape; see [`texture_row`], and [`super::thumbnails`] for why a picture
/// is not available the first time it is requested.
fn swatch_at(ui: &egui::Ui, thumbnails: &mut Thumbnails, path: &str, rect: egui::Rect) {
    thumbnails.want(path);
    let painter = ui.painter();
    match thumbnails.get(path) {
        Some(texture) => {
            painter.image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(rect, egui::CornerRadius::same(2), theme::SUNK);
        }
    }
}

/// The textures the panel's chunk is painted with, and the controls for
/// freeing a slot when the four-layer limit is reached.
///
/// This section shows why a brush stroke was refused. A chunk carries at most
/// four textures (measured over the shipped tiles, between half and three
/// quarters of chunks already carry four), so a stroke with a fifth is
/// refused. Each of the four is listed with how much of it is actually visible
/// rather than what its own blend map holds, because the decision is which one
/// to remove.
///
/// ## Why the list shows a pinned chunk, not the one under the pointer
///
/// If the list followed the pointer, moving the pointer to the `×` button
/// would take it off the chunk the list was about. Latching when the pointer
/// leaves the viewport does not work either, because the path to the panel
/// crosses other chunks.
///
/// So the chunk is pinned: by `Space`, which needs no pointer movement, and
/// automatically by a refused stroke, which is the case this list is for.
/// Unpinning returns to following the pointer, which is the default for
/// looking around.
#[allow(clippy::too_many_arguments)]
fn chunk_layers(
    ui: &mut egui::Ui,
    thumbnails: &mut Thumbnails,
    textures: &mut Textures,
    session: &mut EditSession,
    cursor: &Cursor,
    assets: &vale_client::assets::GameAssets,
    now: f64,
) {
    theme::heading(ui, "Chunk");
    let pinned = textures.pinned.is_some();
    let Some((coord, chunk)) = textures::shown(textures, cursor) else {
        theme::note(ui, "pointer is not over the ground");
        theme::note(ui, "space pins the chunk under the pointer");
        return;
    };
    let Some(layers) = textures::layers_of(session, coord, chunk) else {
        theme::note(ui, "that tile is not open yet");
        return;
    };

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(match pinned {
                true => "pinned",
                false => "following the pointer",
            })
            .size(theme::SMALL)
            .color(match pinned {
                true => theme::WARN,
                false => theme::INK_FAINT,
            }),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button(match pinned {
                    true => "Unpin",
                    false => "Pin",
                })
                .on_hover_text(
                    "Space over a chunk. A pinned chunk stays in this panel \
                     while the pointer moves to the buttons. Unpinned, the panel \
                     follows the pointer.",
                )
                .clicked()
            {
                textures.pinned = match pinned {
                    true => None,
                    false => Some((coord, chunk)),
                };
            }
        });
    });
    theme::note(
        ui,
        format!(
            "tile {},{} · chunk {chunk} · {} of {} textures",
            coord.0,
            coord.1,
            layers.len(),
            vale_edit::adt::alpha::MAX_LAYERS
        ),
    );

    let mut drop: Option<usize> = None;
    let mut rebase: Option<String> = None;
    let mut swap: Option<(usize, String)> = None;
    let mut grow: Option<(usize, u32)> = None;
    let mut crawl: Option<(usize, u32, u32, bool)> = None;
    // "The chosen texture" here is the brush's texture. There is no second
    // picker: the texture used for painting is also the one used for
    // re-basing or swapping a layer.
    let chosen = match textures.brush.texture.is_empty() {
        true => None,
        false => Some(textures.brush.texture.clone()),
    };
    let mut using: Option<(String, u32)> = None;
    for (index, layer) in layers.iter().enumerate() {
        if index > 0 {
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            swatch(ui, thumbnails, &layer.texture);
            ui.vertical(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(textures::leaf(&layer.texture))
                            .size(theme::SMALL)
                            .color(theme::INK),
                    )
                    .truncate(),
                )
                .on_hover_text(layer.texture.clone());
                // The base is the one layer with no blend map and cannot be
                // removed, so it is labelled "base" instead of showing a
                // coverage it cannot change. The row's buttons share the line.
                ui.horizontal(|ui| {
                    theme::note(
                        ui,
                        match index {
                            0 => "base".to_string(),
                            _ => format!("{:.0}% visible", layer.coverage * 100.0),
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        layer_buttons(ui, index, pinned, chosen.is_some(), &mut |ask| match ask {
                            LayerAsk::Drop => drop = Some(index),
                            LayerAsk::Swap => swap = Some((index, chosen.clone().unwrap_or_default())),
                            LayerAsk::Rebase => rebase = chosen.clone(),
                            LayerAsk::Use => using = Some((layer.texture.clone(), layer.effect_id)),
                        });
                    });
                });
            });
        });
        // The layer's foliage and animation, under the name: the two `MCLY`
        // fields no brush writes. Disabled while the chunk follows the
        // pointer, for the reason the buttons are.
        let picked = theme::row_about(
            ui,
            "foliage",
            "MCLY effectId: what this layer grows, a GroundEffectTexture row.",
            |ui| {
                ground_effect(
                    ui,
                    &format!("layer {index}"),
                    layer.effect_id,
                    pinned,
                    session,
                    &layer.texture,
                    assets,
                )
            },
        );
        if let Some(id) = picked {
            grow = Some((index, id));
        }
        // Only 164 layers in Azeroth are animated, all of them lava, so the
        // direction and speed are shown only once the switch is on.
        theme::row_about(
            ui,
            "animation",
            "MCLY texture animation: scrolls this layer's texture coordinates, as on \
             the Burning Steppes lava.",
            |ui| {
                let (mut turn, mut rate, mut on) = layer.animation;
                ui.add_enabled_ui(pinned, |ui| {
                    let mut moved = ui
                        .checkbox(&mut on, "scrolls")
                        .on_disabled_hover_text("Pin the chunk (press Space over it).")
                        .changed();
                    if on {
                        moved |= crawl_direction(ui, index, &mut turn);
                        moved |= ui
                            .add(egui::DragValue::new(&mut rate).speed(0.1).range(0..=7).prefix("speed "))
                            .on_hover_text("0 to 7: each step doubles it.")
                            .changed();
                        theme::note(ui, crawls((turn, rate, on)));
                    }
                    if moved {
                        crawl = Some((index, turn, rate, on));
                    }
                });
            },
        );
    }
    if let Some((path, effect_id)) = using {
        textures.brush.effect_id = effect_id;
        session.status = format!("painting with {}", textures::leaf(&path));
        textures.brush.texture = path;
    }
    if let Some(index) = drop {
        textures::drop_layer(session, coord, chunk, index);
    }
    if let Some(path) = rebase {
        textures::set_base(session, coord, chunk, &path);
    }
    if let Some((index, path)) = swap {
        textures::swap_layer(session, coord, chunk, index, &path);
    }
    if let Some((index, effect_id)) = grow {
        textures::set_layer_effect(session, coord, chunk, index, effect_id, now);
    }
    if let Some((index, turn, rate, on)) = crawl {
        textures::set_layer_animation(session, coord, chunk, index, turn, rate, on, now);
    }

    // The two buttons that act on more than one layer at a time.
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(
                pinned && layers.len() > 1,
                egui::Button::new("Clear to base"),
            )
            .on_hover_text("Removes every layer from this chunk except the base.")
            .clicked()
        {
            textures::clear_paint(session, coord, chunk);
        }
        if ui
            .add_enabled(chosen.is_some(), egui::Button::new("Base whole tile"))
            .on_hover_text(
                "Makes the chosen texture the base of all 256 chunks of this tile, \
                 under whatever each already carries. One undo entry.",
            )
            .on_disabled_hover_text("Choose a texture below first.")
            .clicked()
        {
            if let Some(path) = chosen.clone() {
                let changed = textures::set_tile_base(session, coord, &path);
                session.status = format!("base texture of {changed} chunks set to {}", textures::leaf(&path));
            }
        }
    });
    if layers.len() >= vale_edit::adt::alpha::MAX_LAYERS {
        ui.add_space(2.0);
        theme::note(
            ui,
            "This chunk has four textures, the maximum. Remove or swap a layer \
             to paint a different texture.",
        );
    }
}

/// What a layer row's buttons asked for.
enum LayerAsk {
    /// Paint with the layer's texture and ground effect.
    Use,
    /// Replace the layer's texture with the chosen one.
    Swap,
    /// Make the chosen texture the chunk's base.
    Rebase,
    /// Remove the layer.
    Drop,
}

/// A layer row's buttons, right to left: remove, swap (or set the base, on
/// the base) and use.
///
/// Use is always available, since it changes the brush and not the chunk. The
/// others are disabled rather than hidden while the chunk follows the pointer,
/// because a press would act on whichever chunk the pointer crossed on the
/// way to the button, with no sign that it was the wrong one.
fn layer_buttons(
    ui: &mut egui::Ui,
    index: usize,
    pinned: bool,
    chosen: bool,
    asked: &mut dyn FnMut(LayerAsk),
) {
    let unpinned = "Pin the chunk first (press Space over it). Unpinned, this would act on \
                    the last chunk the pointer crossed on the way to the button.";
    match index {
        // The base is replaced, never removed; see `Paint::set_base`.
        0 => {
            if ui
                .add_enabled(pinned && chosen, egui::Button::new("Set base").small())
                .on_hover_text(
                    "Make the chosen texture this chunk's base, under everything already \
                     painted on it. Every blend map is kept.",
                )
                .on_disabled_hover_text("Pin the chunk (press Space over it) and choose a texture.")
                .clicked()
            {
                asked(LayerAsk::Rebase);
            }
        }
        _ => {
            if ui
                .add_enabled(pinned, egui::Button::new("×").small())
                .on_hover_text(
                    "Remove this layer from the chunk, freeing a slot for another texture. \
                     Removing a layer at 0% changes nothing on screen.",
                )
                .on_disabled_hover_text(unpinned)
                .clicked()
            {
                asked(LayerAsk::Drop);
            }
            if ui
                .add_enabled(pinned && chosen, egui::Button::new("Swap").small())
                .on_hover_text(
                    "Replace this layer's texture with the chosen texture, keeping its blend \
                     map. On a full chunk this changes a texture without removing a layer.",
                )
                .on_disabled_hover_text("Pin the chunk (press Space over it) and choose a texture.")
                .clicked()
            {
                asked(LayerAsk::Swap);
            }
        }
    }
    if ui
        .button(egui::RichText::new("Use").size(theme::SMALL))
        .on_hover_text("Paint with this texture: it becomes the brush's, with this layer's foliage.")
        .clicked()
    {
        asked(LayerAsk::Use);
    }
}

/// A layer's animation direction as a compass point, from `MCLY`'s 0 to 7:
/// step 0 is north (up the texture) and each step 45° clockwise. Returns
/// whether it changed.
fn crawl_direction(ui: &mut egui::Ui, index: usize, turn: &mut u32) -> bool {
    const COMPASS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let was = *turn;
    egui::ComboBox::from_id_salt(("layer-direction", index))
        .selected_text(COMPASS[(*turn & 7) as usize])
        .width(48.0)
        .show_ui(ui, |ui| {
            for (step, name) in COMPASS.iter().enumerate() {
                ui.selectable_value(turn, step as u32, *name);
            }
        })
        .response
        .on_hover_text("Which way the texture crawls.");
    *turn != was
}

/// **A ground effect, chosen by what it grows** rather than typed as a
/// `GroundEffectTexture` row id. Returns the row picked this frame.
///
/// The list offers nothing, then the effects the open tiles grow on this
/// texture, most used first, then the ones they grow on other textures. Each
/// is named by its models (see [`grows`]). Neither table names a texture, so
/// the open ground is the only record of which effect suits which, and a zone
/// generally reuses a handful. A row id the open tiles do not use can still be
/// typed at the bottom.
///
/// The counts read every open chunk. Worked out every frame the list was
/// open, they took the editor to a crawl, so they are worked out on the frame
/// it opens and kept in egui's memory while it stays open.
fn ground_effect(
    ui: &mut egui::Ui,
    salt: &str,
    current: u32,
    enabled: bool,
    session: &EditSession,
    texture: &str,
    assets: &vale_client::assets::GameAssets,
) -> Option<u32> {
    let mut picked = None;
    let width = ui.available_width().max(80.0);
    ui.add_enabled_ui(enabled, |ui| {
        egui::ComboBox::from_id_salt(("ground-effect", salt))
            .selected_text(egui::RichText::new(grows(assets, current)).size(theme::SMALL))
            .width(width)
            .truncate()
            .show_ui(ui, |ui| {
                ui.set_min_width(width.max(260.0));
                if ui.selectable_label(current == 0, "nothing").clicked() {
                    picked = Some(0);
                }
                let counts = counts_while_open(ui, salt, session, texture);
                let mut section = |ui: &mut egui::Ui, title: &str, list: &[(u32, usize)]| {
                    if list.is_empty() {
                        return;
                    }
                    ui.add_space(4.0);
                    theme::note(ui, title);
                    for &(id, count) in list.iter().take(20) {
                        let label = format!("{}  · {count} layers · row {id}", grows(assets, id));
                        if ui.selectable_label(current == id, label).clicked() {
                            picked = Some(id);
                        }
                    }
                };
                section(ui, "grown on this texture in the open tiles", &counts.with);
                section(ui, "grown on other textures in the open tiles", &counts.elsewhere);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    theme::note(ui, "another row");
                    let mut id = current;
                    if ui
                        .add(egui::DragValue::new(&mut id).speed(1.0).range(0..=u32::MAX))
                        .on_hover_text("A GroundEffectTexture row id, for one the open tiles do not use.")
                        .changed()
                    {
                        picked = Some(id);
                    }
                });
            });
    })
    .response
    .on_disabled_hover_text("Pin the chunk (press Space over it).");
    picked
}

/// [`textures::effect_counts`] for a list that is open: worked out on the
/// first frame and kept while the list is drawn on every frame after it, or
/// again if the texture it was for has changed.
fn counts_while_open(
    ui: &egui::Ui,
    salt: &str,
    session: &EditSession,
    texture: &str,
) -> std::sync::Arc<textures::EffectCounts> {
    type Kept = (u64, String, std::sync::Arc<textures::EffectCounts>);
    let id = egui::Id::new(("ground-effect-counts", salt));
    let pass = ui.ctx().cumulative_pass_nr();
    let kept = ui.data(|data| data.get_temp::<Kept>(id));
    let counts = match kept {
        Some((last, path, counts)) if last + 1 >= pass && path == texture => counts,
        _ => std::sync::Arc::new(textures::effect_counts(session, texture)),
    };
    ui.data_mut(|data| data.insert_temp(id, (pass, texture.to_string(), counts.clone())));
    counts
}

/// What the group lines under the Selection heading asked for.
enum Group {
    Nothing,
    /// Keep the primary and drop the rest.
    Only,
    /// Drop the whole selection.
    Clear,
    /// Add every placement of the primary's model in the open tiles.
    AllOfModel,
    /// Lock every selected placement and drop the selection.
    Lock,
    /// Make this member the primary, from the list of members.
    Primary(u32),
    /// Take this member out of the group, from the list of members.
    Drop(u32),
}

/// How many doodads or WMOs of the map are locked, and a button that unlocks
/// them all. See `crate::session::PlacementLocks`.
fn locked_placements(ui: &mut egui::Ui, session: &mut EditSession, kind: Kind) {
    let noun = match kind {
        Kind::Doodad => "doodads",
        Kind::Wmo => "WMOs",
    };
    let locked = session.placement_locks().of(kind).len();
    if locked == 0 {
        return;
    }
    ui.horizontal(|ui| {
        theme::note(ui, format!("{locked} {noun} locked on this map, marked red"));
        if ui
            .small_button("Unlock all")
            .on_hover_text(format!(
                "Unlock every locked {} on this map. Right-click one to unlock it alone.",
                noun.trim_end_matches('s')
            ))
            .clicked()
        {
            let ids: Vec<u32> = session.placement_locks().of(kind).iter().copied().collect();
            let n = session.set_placements_locked(kind, ids, false);
            session.status = format!("unlocked {n} {noun}");
        }
    });
}

/// The lines about a selection of several, above the primary's own numbers:
/// how many, what they are, and the three buttons that change the group.
///
/// `chosen` is every member's unique id and path, the primary first. With one
/// placement or none this draws only the "Select all of this model" button,
/// which makes a group of one kind in one press. See `crate::tools::group`.
fn group(ui: &mut egui::Ui, chosen: &[(u32, &str)], noun: &str) -> Group {
    let Some(&(_, primary)) = chosen.first() else {
        return Group::Nothing;
    };
    let mut asked = Group::Nothing;
    if chosen.len() > 1 {
        ui.label(
            egui::RichText::new(format!("{} {noun} selected", chosen.len()))
                .strong()
                .color(theme::INK),
        );
        // What the group is made of, most numerous first: a sweep of a forest
        // is thirty of one tree and a hand-picked group is a few of several.
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for (_, path) in chosen {
            let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
            match counts.iter_mut().find(|(had, _)| had.eq_ignore_ascii_case(leaf)) {
                Some((_, count)) => *count += 1,
                None => counts.push((leaf, 1)),
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        for (leaf, count) in counts.iter().take(6) {
            theme::note(ui, format!("{count} × {leaf}"));
        }
        if counts.len() > 6 {
            theme::note(ui, format!("…and {} other models", counts.len() - 6));
        }
        theme::note(
            ui,
            "The values below are the primary selection's (drawn brighter). \
             Dragging, the handles, the keys and Delete act on all of them.",
        );
        let member = |index: usize| {
            let (id, path) = chosen[index];
            super::members::Member {
                id: u64::from(id),
                label: path.rsplit(['\\', '/']).next().unwrap_or(path).to_string(),
                detail: id.to_string(),
            }
        };
        match super::members::list(ui, noun, chosen.len(), member) {
            Some(super::members::Pick::Primary(id)) => asked = Group::Primary(id as u32),
            Some(super::members::Pick::Drop(id)) => asked = Group::Drop(id as u32),
            None => {}
        }
    }
    ui.horizontal_wrapped(|ui| {
        let leaf = primary.rsplit(['\\', '/']).next().unwrap_or(primary);
        if ui
            .small_button("Select all of this model")
            .on_hover_text(format!(
                "Add every {leaf} in the open tiles to the selection."
            ))
            .clicked()
        {
            asked = Group::AllOfModel;
        }
        if ui
            .small_button("Lock")
            .on_hover_text(format!(
                "Lock the selected {noun}: no tool selects, moves or deletes them, and \
                 moving the ground does not carry them, until they are unlocked. \
                 Right-click a locked one to unlock it."
            ))
            .clicked()
        {
            asked = Group::Lock;
        }
        if chosen.len() > 1 {
            if ui
                .small_button("Select primary only")
                .on_hover_text("Keep the primary (brighter) selection and deselect the rest.")
                .clicked()
            {
                asked = Group::Only;
            }
            if ui.small_button("Clear").clicked() {
                asked = Group::Clear;
            }
        }
    });
    ui.add_space(4.0);
    asked
}

/// The doodad tool: what is standing on the ground.
#[allow(clippy::too_many_arguments)]
fn doodad(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    selection: &mut Selection,
    held: &mut crate::tools::doodads::Held,
    gizmo: &mut Gizmo,
    placer: &mut Placing,
    portraits: &mut crate::portraits::Portraits,
    favourites: &mut crate::favourites::Favourites,
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
                for at in crate::tools::doodads::all_of(session, &path) {
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
            if !crate::tools::doodads::lean_onto_ground(session, at) {
                session.status = "the doodad's origin is not over open ground".to_string();
            }
        }
        if ui
            .small_button("Stand upright")
            .on_hover_text("Set the rotation about x and y to zero, keeping the rotation about z. In a group, applies to every member.")
            .clicked()
        {
            each = Some(Lean::Upright);
            crate::tools::doodads::stand_upright(at);
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
                    .range(crate::tools::doodads::SCALE_RANGE)
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
        None => crate::tools::doodads::what_changed(&was, &record),
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
    crate::tools::doodads::write_record(session, at);
    session.publish(tile);
    if grouped {
        match each {
            Some(lean) => {
                for member in selection.also.iter_mut() {
                    match lean {
                        Lean::Ground => {
                            crate::tools::doodads::lean_onto_ground(session, member);
                        }
                        Lean::Upright => crate::tools::doodads::stand_upright(member),
                    }
                    crate::tools::doodads::write_record(session, member);
                }
            }
            None => crate::tools::doodads::carry_members(session, selection, &was),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A row names the file and the folder, without the extension all 621
    /// share. The panel is 288 points wide and a tileset path is wider: painted
    /// whole on one line it runs off the edge of the window.
    #[test]
    fn a_row_splits_into_a_folder_and_a_file() {
        assert_eq!(
            split("tileset\\elwynn\\elwynngrassbase.blp"),
            ("elwynn", "elwynngrassbase")
        );
        assert_eq!(split("tileset/elwynn/grass.blp"), ("elwynn", "grass"));
        // A path with nothing above it has no folder, and keeps its name.
        assert_eq!(split("grass.blp"), ("", "grass"));
        assert_eq!(split(""), ("", ""));
    }
}

/// The sweep tool: a find-and-replace over every tile of the map.
///
/// Find and Replace walk the tiles the same way; only Replace writes. Both are
/// offered because a path is easy to misspell, and a misspelled path reports
/// "no tiles matched", which looks the same as "nothing to do". Find is
/// expected to be pressed first, and it prints the count that decides whether
/// to replace.
///
/// The paths are typed rather than picked. A picker over the archives' listing
/// would offer 5,816 models and 621 tilesets and would be the wrong list for
/// both fields: the find path should be one this map uses, which the listing
/// does not say, and the replacement can be any path. The Chunk list's rows
/// and the placement inspector both print the full path, so the find path can
/// be copied from there.
fn sweep(
    ui: &mut egui::Ui,
    sweep: &mut crate::tools::sweep::Sweep,
    session: &EditSession,
    assets: &vale_client::assets::GameAssets,
) {
    use crate::tools::sweep::{Subject, SUBJECTS};

    theme::heading(ui, "Map");
    theme::note(
        ui,
        format!(
            "{}, {} tiles. Tiles reference textures and models by index into a \
             path list, so replacing a path changes nothing else: no chunk is \
             repainted and no placement moves.",
            session.map,
            session.claimed.len()
        ),
    );

    ui.add_space(4.0);
    let running = sweep.running.is_some();
    ui.add_enabled_ui(!running, |ui| {
        theme::segmented(ui, &mut sweep.subject, &SUBJECTS, |a, b| a == b);
        ui.add_space(4.0);
        theme::heading(ui, "Find path");
        ui.add(
            egui::TextEdit::singleline(&mut sweep.from)
                .hint_text(sweep.subject.root())
                .desired_width(f32::INFINITY),
        );
        theme::heading(ui, "Replace with");
        ui.add(
            egui::TextEdit::singleline(&mut sweep.to)
                .hint_text(match sweep.subject {
                    Subject::Texture => r"Tileset\Barrens\BarrensDirt01.blp",
                    Subject::Model => r"World\Azeroth\Elwynn\PassiveDoodads\Trees\ElwynnTree01.m2",
                    Subject::Building => r"World\wmo\Azeroth\Buildings\Human_Farm\Farm.wmo",
                })
                .desired_width(f32::INFINITY),
        );
    });

    ui.add_space(6.0);
    let named = !sweep.from.trim().is_empty();
    ui.horizontal(|ui| {
        let half = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
        if ui
            .add_enabled(
                !running && named,
                egui::Button::new("Find").min_size(egui::vec2(half, 24.0)),
            )
            .on_hover_text(
                "Count how many tiles name this path, and how many times, \
                 without changing anything.",
            )
            .on_disabled_hover_text("Enter a path to find.")
            .clicked()
        {
            sweep.find(session, assets);
        }
        if ui
            .add_enabled(
                !running && named && !sweep.to.trim().is_empty(),
                egui::Button::new("Replace").min_size(egui::vec2(half, 24.0)),
            )
            .on_hover_text(
                "Replace the path on every tile of this map, as one undo entry. \
                 Each changed tile is written to the project folder.",
            )
            .on_disabled_hover_text("Enter both paths.")
            .clicked()
        {
            sweep.replace(session, assets);
        }
    });

    if let Some(run) = &sweep.running {
        ui.add_space(4.0);
        let (done, total) = (run.done(), run.total());
        theme::progress(
            ui,
            done as f32 / total.max(1) as f32,
            "Sweep",
            &format!("{done} / {total} tiles"),
            ui.available_width(),
        );
    }

    if let Some(report) = &sweep.last {
        ui.add_space(6.0);
        theme::heading(ui, "Last sweep");
        theme::note(
            ui,
            match report.tiles {
                0 => format!(
                    "{} — not found in any tile of this map, {} scanned.",
                    report.what, report.scanned
                ),
                tiles => format!(
                    "{} — {tiles} tile(s), {} occurrence(s), {} scanned.",
                    report.what, report.names, report.scanned
                ),
            },
        );
    }

    ui.add_space(6.0);
    theme::note(
        ui,
        "Replace does not merge duplicates: replacing A with B on a tile that \
         already lists B leaves two B entries. Merging would renumber the list \
         and every record that indexes it.",
    );
}
