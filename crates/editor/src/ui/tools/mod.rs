//! The inspector and the pointer tools' own panels and windows. A file here
//! has the same path and stem as the state it draws in `crate::tools`.
//!
//! ```text
//! ground/       the panels of the ground tools: terrain, grade, road, shading,
//!               textures, holes, areas, chunks and water
//! placements/   the panels of the doodad and WMO tools: the placement picker,
//!               a placement's numbers, and a group's
//! spawns/       the creature and game object forms and the waypoint window
//! places/       the panels of the flight path, area trigger and graveyard
//!               tools, and a light's form and bands
//! measure.rs    the Measure tool's panel: a point, a distance, and the slope
//! history.rs    what undo and redo would do, at the top of an editing tool's
//!               panel
//! tablet.rs     the drawing tablet's settings and the pen's state, under the
//!               three brushes a pen's pressure scales
//! ```
//!
//! This file is the inspector: the panel beside the viewport, which shows the
//! chosen tool's settings. [`draw`] chooses the tool's panel; the panels are in
//! the files above.
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
//! The conversion belongs to `vale_assets`, both ways, for positions and for
//! angles: a position through `placement_to_world` and the three angles through
//! `placement_euler_to_world`, which states that the file's second angle is
//! the turn about the world's z. See [`crate::tools::placements::doodads`], where this is
//! written down in full.
//!
//! ## Why the undo history is on this panel and not on the bar
//!
//! Undo applies to the edit, not to the session; [`crate::ui::shell::topbar`] holds the
//! session controls. Undo is also the control a person reaches for without
//! looking, so it sits at the top of the panel they are already looking at.

pub mod ground;
pub mod history;
pub mod measure;
pub mod placements;
pub mod places;
pub mod spawns;
pub mod tablet;

use bevy_egui::egui;

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use crate::ui::Editing;
use crate::ui::map::sweep::sweep;
use crate::ui::tables::table;
use crate::ui::tools::ground::areas::zones;
use crate::ui::tools::ground::chunks::chunks;
use crate::ui::tools::ground::grade::grading;
use crate::ui::tools::ground::holes::holes;
use crate::ui::tools::ground::road::road;
use crate::ui::tools::ground::shading::shading;
use crate::ui::tools::ground::terrain::brush;
use crate::ui::tools::ground::textures::paint;
use crate::ui::tools::ground::water::water;
use crate::ui::tools::history::history;
use crate::ui::tools::measure::measure;
use crate::ui::tools::placements::doodads::doodad;
use crate::ui::tools::placements::wmos::wmo;
use crate::ui::tools::tablet::tablet;
use crate::ui::widgets::theme;

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
    /// same panels link to it; see [`crate::ui::shell::sync`].
    pub server_panel: &'a mut crate::ui::shell::popover::Popover,
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
        // A workspace in the main window can be moved to a window of its own.
        // See `crate::popout`.
        if tool.covers_viewport() && !editing.popout.holds(tool) {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let pop = ui
                    .add_enabled(editing.popout.editing, egui::Button::new("Pop Out").small())
                    .on_hover_text(
                        "Open this workspace in a window of its own, and return this window \
                         to the world. Dock or close that window to bring it back. Each \
                         workspace can have a window, except that the table workspaces share \
                         one.",
                    )
                    .on_disabled_hover_text("Not during a playtest.");
                if pop.clicked() {
                    editing.popout.asks.push(crate::popout::Ask::Open(tool));
                }
            });
        }
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
        Tool::Lights => places::lights::light_inspector(
            ui,
            crate::ui::tables::Workspace {
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
        Tool::Flightpaths => places::flightpaths::draw(
            ui,
            places::flightpaths::Subject {
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
        Tool::Triggers => places::triggers::draw(
            ui,
            places::triggers::Subject {
                session,
                triggers: &mut editing.triggers,
                assets,
                now,
                server_panel,
                behaviour: &mut editing.behaviour,
            },
        ),
        Tool::Graveyards => places::graveyards::draw(
            ui,
            places::graveyards::Subject {
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
        // `spawns::creatures`.
        Tool::Creatures => spawns::creatures::draw(
            ui,
            spawns::creatures::Subject {
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
        // `spawns::gameobjects`.
        Tool::GameObjects => spawns::gameobjects::draw(
            ui,
            spawns::gameobjects::Subject {
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
        // which is this panel's view of the selection. `crate::ui::subjects::items` says why
        // it is not a column of its own.
        Tool::Items => crate::ui::subjects::items::inspector(
            ui,
            crate::ui::subjects::items::Looking {
                session,
                items: &mut editing.items,
                assets,
                thumbnails: &mut editing.thumbnails,
                portraits: &mut editing.portraits,
            },
        ),
        // The quest workspace's third part, following the shell's rule that the
        // inspector shows the selection: the open quest as the log would show
        // it, who gives and takes it, and its chain. `crate::ui::subjects::quests` says why
        // it is not a column of its own.
        Tool::Quests => crate::ui::subjects::quests::inspector(
            ui,
            crate::ui::subjects::quests::Workspace {
                session,
                quests: &mut editing.quests,
                items: &mut editing.items,
                assets,
                thumbnails: &mut editing.thumbnails,
                now,
            },
        ),
        // The Assets workspace's third part: the chosen file, drawn. See
        // `crate::ui::import::assets`.
        Tool::Assets => crate::ui::import::assets::inspector(
            ui,
            crate::ui::import::assets::Work {
                session,
                assets: &mut editing.assets,
                game: assets,
                thumbnails: &mut editing.thumbnails,
                portraits: &mut editing.portraits,
            },
        ),
        // The Creatures and Game Objects workspaces' third part: the open
        // template's model, a summary, and the windows for its other tables.
        // See `crate::ui::subjects::creature_data` and `crate::ui::subjects::gameobject_data`.
        Tool::CreatureData => crate::ui::subjects::creature_data::inspector(
            ui,
            spawns::creatures::Subject {
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
                gizmo: None,
            },
        ),
        Tool::GameObjectData => crate::ui::subjects::gameobject_data::inspector(
            ui,
            spawns::gameobjects::Subject {
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
                gizmo: None,
            },
        ),
        // A table's inspector shows what the open row points at. The form
        // itself is in the middle, where the viewport would be; see
        // `crate::ui::tables`. This follows the shell's rule that the inspector
        // shows the selection.
        Tool::Spells
        | Tool::ItemSets
        | Tool::Tables
        | Tool::Zones
        | Tool::CreatureDisplays
        | Tool::ObjectDisplays
        | Tool::ItemDisplays => table(
            ui,
            crate::ui::tables::Workspace {
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
    // The three brushes whose strength and radius a pen's pressure scales.
    if matches!(tool, Tool::Terrain | Tool::Textures | Tool::Shading) {
        tablet(ui, &mut editing.tablet, &editing.pen);
    }
}
