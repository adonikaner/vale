//! Where a subject is chosen: the rail of world tools on the left, the
//! workspace control on the top bar, and the two pointer tools over the
//! viewport's corner.
//!
//! ## Which subjects are tiles and which are workspaces
//!
//! Tools that act on the world (terrain, grading, holes, texture layers,
//! doodads, area assignment, lights, water) are tiles on the side rail, and
//! editors that do not (Spells, Skills, Items, Quests, Creatures, Game
//! Objects, Zones, Sound, Factions, Spawning, Tables, Assets) are parts of the
//! workspace control on the top bar. [`Tool::surface`] decides it: a subject
//! for which the viewport is the document is a tile on the rail, and a
//! subject whose workspace replaces the viewport is a part of the workspace
//! control.
//!
//! ```text
//! the rail      Terrain  the ground itself: eight tools, each of which writes
//!                        a field of an MCNK chunk
//!               World    what has a place on the ground and is not the ground:
//!                        MDDF, MODF, the spheres of Light.dbc, the taxi nodes
//!                        and paths, the area triggers, the graveyards, and
//!                        Sweep, whose place is every tile of the map at once
//!               Spawns   rows of vmangos' database that stand in the world:
//!                        creature and gameobject spawns, picked and moved in
//!                        the viewport
//! the top bar   World, then the workspaces in `SPECS` order: Spells, Skills,
//!               Items, Quests, Creatures, Game Objects, Zones, Sound,
//!               Factions, Spawning, Tables and Assets. World returns to the
//!               rail's last tool; the others replace the viewport. Tables
//!               reaches every table the others do not. A workspace of several
//!               parts is a menu of them. Which ones a playtest leaves
//!               available is `Tool::survives_playtest`'s answer
//! the corner    Select and Measure, which change nothing and are returned to
//!               rather than started with
//! ```
//!
//! The rail is drawn only while the viewport is the world's: not while a
//! workspace replaces it, and not during a playtest. In a workspace every
//! tile would leave it for the world, which World on the top bar does, and
//! the shell gives the rail's width to the workspace.
//!
//! Lights, Taxi, Triggers and Graveyards edit client tables, but a light's
//! sphere, a taxi node, a trigger and a safe place are picked and dragged in
//! the viewport, so all four answer [`crate::tools::Surface::Inspector`] and
//! are tiles.
//!
//! ## One column or two
//!
//! The rail is one column of tiles when the height the bars leave holds every
//! tile and heading, and two columns otherwise. One column is 54 points wide
//! and leaves the rest to the viewport; two columns take half the height. The
//! height is in egui points, so the UI scale is part of it: at a scale of 1.5
//! a 1440-pixel window is 960 points tall. [`Rail::columns`] decides it from
//! the height measured on the frame before, so a resize changes the rail one
//! frame later.
//!
//! ## Icons
//!
//! A tile and a corner button draw their tool's icon alone
//! ([`super::toolicons`]), painted with egui's painter as the brush presets'
//! icons are. The tool's name and what it does are in the tooltip. Icons
//! rather than names let a tile be square, so the rail is 54 or 96 points wide
//! rather than the width of the longest name. The workspace control on the
//! top bar names its workspaces in text.

use bevy::prelude::Resource;
use bevy_egui::egui;

use super::toolicons::ToolIcon;
use crate::ui::widgets::theme;
use crate::tools::{Place, Surface, Tool, SPECS};

/// One subject: its name, its tool, and what it does. Read from the tool's
/// row of [`SPECS`].
struct Subject {
    /// The name: a top bar button's label, and the first line of a tile's
    /// tooltip.
    name: &'static str,
    tool: Tool,
    /// What it does, for the tooltip.
    about: &'static str,
}

/// The rail's headings, in the order it draws them. A tile names its
/// heading in [`Place::Rail`].
const HEADINGS: [&str; 3] = ["Terrain", "World", "Spawns"];

/// The tiles under `heading`, in the order the rail draws them. A tile shows
/// its tool's icon.
fn tiles(heading: &str) -> Vec<Subject> {
    SPECS
        .iter()
        .filter(|spec| matches!(spec.place, Place::Rail(under) if under == heading))
        .map(|spec| Subject { name: spec.name, tool: spec.tool, about: spec.about })
        .collect()
}

/// The workspaces after *World*, in the order the top bar draws them, under
/// the names the top bar gives them.
fn bar() -> impl Iterator<Item = Subject> {
    SPECS.iter().filter_map(|spec| match spec.place {
        Place::Bar(name) => Some(Subject { name, tool: spec.tool, about: spec.about }),
        _ => None,
    })
}

/// A part's menu entry: every part of a workspace of several parts, by the
/// name the menu gives it. A workspace's menu lists its [`Tool::parts`] in
/// their order, each under its name here.
fn part(tool: Tool) -> Option<Subject> {
    let spec = tool.spec();
    spec.menu.map(|entry| Subject { name: entry.name, tool, about: entry.about })
}

/// The two tools that change nothing, drawn over the viewport's corner.
fn corner() -> impl Iterator<Item = Subject> {
    SPECS
        .iter()
        .filter(|spec| spec.place == Place::Corner)
        .map(|spec| Subject { name: spec.name, tool: spec.tool, about: spec.about })
}

/// The rail's memory between frames.
#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct Rail {
    /// The last tool chosen on the rail or in the corner, which the *World*
    /// part of the workspace control returns to.
    pub(crate) last_world: Tool,
    /// How many columns of tiles the rail draws, 1 or 2. Set by [`draw`]
    /// through [`next_columns`].
    columns: usize,
    /// The width of the widest heading's label, in points. Measured on the
    /// first frame drawn; 0 before it.
    heading: f32,
}

impl Default for Rail {
    fn default() -> Rail {
        Rail {
            last_world: Tool::Select,
            columns: 2,
            heading: 0.0,
        }
    }
}

/// How much taller than one column's content the rail must be before it
/// changes from two columns to one. It changes back as soon as one column
/// does not fit. Without the margin, a height on the boundary could change
/// the rail's width, and so the viewport's, on every frame.
const SLACK: f32 = 8.0;

/// The column count after `columns`, for a rail `room` points tall whose
/// content takes `one` points in one column.
fn next_columns(columns: usize, room: f32, one: f32) -> usize {
    match columns {
        1 if one > room => 2,
        2 if one + SLACK <= room => 1,
        _ => columns,
    }
}

impl Rail {
    /// How many columns of tiles the rail draws: one when one column fits the
    /// rail's height, two otherwise.
    pub(crate) fn columns(&self) -> usize {
        self.columns
    }

    /// How wide the rail's panel is for [`Rail::columns`]. One column is a
    /// tile wide, or as wide as the widest heading when that is wider, so
    /// TERRAIN is not cut off.
    pub(crate) fn width(&self) -> f32 {
        match self.columns() {
            1 => (TILE.max(self.heading) + MARGINS).ceil(),
            _ => theme::RAIL_WIDTH,
        }
    }

    /// Note the tool in use, when it is one the viewport is the document for.
    pub(crate) fn follow(&mut self, tool: Tool) {
        if tool.surface() != Surface::Middle {
            self.last_world = tool;
        }
    }
}

/// Why a subject cannot be chosen now, as a sentence, or `None`.
///
/// `playing` is whether the panels are open over a running playtest. The
/// editor's own tiles were despawned on the way in, so a subject that keeps
/// the viewport is unavailable and a workspace is not
/// ([`Tool::survives_playtest`]). `server`
/// is whether there is a world database to reach; a subject whose rows are in
/// it has nothing to show without one ([`Tool::server_table`]).
fn held(tool: Tool, playing: bool, server: bool) -> Option<&'static str> {
    if playing && !tool.survives_playtest() {
        return Some(
            "Unavailable during a playtest: the viewport shows the game's terrain, \
             streamed around the character. Ctrl+P ends the playtest.",
        );
    }
    if !server && tool.server_table().is_some() {
        return Some(
            "No world database connection. Set this machine's vmangos in \
             Project \u{2192} Settings\u{2026}, or with VALE_MANGOSD.",
        );
    }
    None
}

/// The tooltip for a subject: what it does, and why it is unavailable when it
/// is.
fn tooltip(subject: &Subject, held: Option<&str>) -> String {
    let mut text = format!("{}\n\n{}", subject.name, subject.about);
    if let Some(why) = held {
        text.push_str(&format!("\n\n{why}"));
    }
    text
}

/// How tall a tile is. Two tiles and [`GAP`] fill the two-column rail's 80
/// points inside its margins, and one tile fills the one-column rail's 38, so
/// a tile is square. See `theme::RAIL_WIDTH` and `theme::RAIL_NARROW`.
const TILE: f32 = 38.0;

/// The icon's size on a tile.
const PICTURE: f32 = 24.0;

/// How large a corner button is, and its icon.
const CORNER: f32 = 28.0;
const CORNER_PICTURE: f32 = 20.0;

/// Space between two tiles, across and down.
const GAP: f32 = 4.0;

/// The rail panel's left and right margins together: 8 points each, as the
/// side panels' frame sets them.
const MARGINS: f32 = 16.0;

/// How many rows of tiles the rail draws in `columns` columns.
fn rows(columns: usize) -> usize {
    HEADINGS.iter().map(|heading| tiles(heading).len().div_ceil(columns)).sum()
}

/// Draw the rail, in [`Rail::columns`] columns.
///
/// The rail's height at 1280x720 is what the top bar, the view bar and the
/// status line leave, about 615 points. Ten rows of tiles and three headings
/// take about 500, so the rail is two columns there. The rail is a
/// `ScrollArea` for windows shorter than that; the scroll area draws nothing
/// while everything fits.
pub(crate) fn draw(ui: &mut egui::Ui, tool: &mut Tool, rail: &mut Rail, playing: bool, server: bool) {
    rail.follow(*tool);
    let columns = rail.columns();
    let room = ui.available_height();
    let output = egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            ui.add_space(4.0);
            for heading in HEADINGS {
                theme::heading(ui, heading);
                // A row is centred, which matters in one column when a
                // heading is wider than a tile.
                let across = columns as f32 * TILE + (columns - 1) as f32 * GAP;
                let indent = ((ui.available_width() - across) / 2.0).max(0.0).floor();
                for row in tiles(heading).chunks(columns) {
                    ui.horizontal(|ui| {
                        if indent > 0.0 {
                            ui.add_space(indent);
                        }
                        for subject in row {
                            let why = held(subject.tool, playing, server);
                            let size = egui::Vec2::splat(TILE);
                            if tile(ui, subject, *tool == subject.tool, why, size, PICTURE) {
                                *tool = subject.tool;
                                rail.follow(subject.tool);
                            }
                        }
                    });
                }
                ui.add_space(2.0);
            }
        });
    // The headings and spacing are the same in either layout, so the height
    // of one column is the drawn height plus one tile and one gap for each row
    // one column adds.
    let added = (rows(1) - rows(columns)) as f32 * (TILE + GAP);
    let next = next_columns(columns, room, output.content_size.y + added);
    // Written only on a change, so the resource is not marked changed every
    // frame.
    if next != columns {
        rail.columns = next;
    }
    // The label as `theme::heading` draws it: upper case, at `theme::SMALL`.
    // A size in points does not change with the UI scale, so it is measured
    // once.
    if rail.heading == 0.0 {
        rail.heading = HEADINGS
            .iter()
            .map(|heading| {
                let text = egui::WidgetText::from(egui::RichText::new(heading.to_uppercase()).size(theme::SMALL));
                text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body)
                    .size()
                    .x
            })
            .fold(0.0, f32::max);
    }
}

/// One tile: the tool's icon, `picture` points square, lit when it is the
/// tool in use, greyed with a reason when it is unavailable. The name is in
/// the tooltip. Answers whether it was clicked. The corner buttons are tiles
/// too.
fn tile(ui: &mut egui::Ui, subject: &Subject, on: bool, held: Option<&str>, size: egui::Vec2, picture: f32) -> bool {
    let sense = match held {
        Some(_) => egui::Sense::hover(),
        None => egui::Sense::click(),
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let hovered = response.hovered() && held.is_none();
    let fill = match (held.is_some(), on, hovered) {
        (true, _, _) => theme::DEAD,
        (false, true, _) => theme::ACCENT_SUNK,
        (false, false, true) => theme::RAISED_HOT,
        (false, false, false) => theme::RAISED,
    };
    let stroke = match on {
        true => egui::Stroke::new(1.0, theme::ACCENT),
        false => egui::Stroke::new(1.0, theme::LINE),
    };
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        egui::CornerRadius::same(3),
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    let ink = match (held.is_some(), on) {
        (true, _) => theme::INK_FAINT,
        (false, true) => theme::INK,
        (false, false) => theme::INK_DIM,
    };
    let at = egui::Rect::from_center_size(rect.center(), egui::Vec2::splat(picture));
    match ToolIcon::of(subject.tool) {
        Some(icon) => icon.paint(&painter, at, ink, fill),
        // A tool with no icon yet is drawn by name. Every rail and corner
        // tool has one; see `every_rail_and_corner_tool_has_an_icon`.
        None => {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, subject.name, egui::FontId::proportional(theme::SMALL), ink);
        }
    }
    let response = response.on_hover_text(tooltip(subject, held));
    held.is_none() && response.clicked()
}

/// The workspace control on the top bar: *World*, then each workspace that
/// replaces the viewport. *World* is lit while a rail or corner tool is in use
/// and returns to the last one chosen. It is disabled during a playtest, when
/// no tool that keeps the viewport can be used.
///
/// `popped` is the workspaces in pop-out windows: their buttons are lit too,
/// and pressing one sends the tool to its window and brings the window
/// forward (see `crate::popout`). A pop-out window draws the same control
/// with `world` false, since it has no world to return to.
pub(crate) fn workspaces(
    ui: &mut egui::Ui,
    tool: &mut Tool,
    rail: &mut Rail,
    playing: bool,
    server: bool,
    popped: &[Tool],
    world: bool,
) {
    let in_world = tool.surface() != Surface::Middle;
    ui.spacing_mut().item_spacing.x = 2.0;
    if !world {
        draw_workspaces(ui, tool, playing, server, popped);
        return;
    }
    let world = ui
        .add_enabled(!playing, egui::Button::selectable(in_world, "World"))
        .on_hover_text(
            "Shows the viewport with the rail tools, Select and Measure. Returns \
             to the tool last used.",
        )
        .on_disabled_hover_text(
            "Unavailable during a playtest: the viewport shows the game's terrain, \
             streamed around the character. Ctrl+P ends the playtest.",
        );
    if world.clicked() {
        // With no database the last tool may be a spawn tool that cannot be
        // used; Select can always be.
        let back = rail.last_world;
        *tool = match held(back, playing, server) {
            None => back,
            Some(_) => Tool::Select,
        };
    }
    draw_workspaces(ui, tool, playing, server, popped);
}

/// The workspace buttons and menus of [`workspaces`], after World.
fn draw_workspaces(ui: &mut egui::Ui, tool: &mut Tool, playing: bool, server: bool, popped: &[Tool]) {
    for subject in bar() {
        // A workspace of several parts is available when any part is.
        let open = match subject.tool.parts() {
            [] => held(subject.tool, playing, server).is_none().then_some(subject.tool),
            parts => parts
                .iter()
                .map(|&(_, part)| part)
                .find(|&part| held(part, playing, server).is_none()),
        };
        let why = match open {
            Some(_) => None,
            None => held(subject.tool, playing, server),
        };
        let in_window = popped.iter().any(|popped| popped.workspace() == subject.tool);
        let lit = tool.workspace() == subject.tool || in_window;
        if subject.tool.parts().is_empty() {
            let response = ui.add_enabled(
                why.is_none(),
                egui::Button::selectable(lit, subject.name),
            );
            let response = match why {
                Some(_) => response.on_disabled_hover_text(tooltip(&subject, why)),
                None => response.on_hover_text(tooltip(&subject, None)),
            };
            if response.clicked() && (!lit || in_window) {
                *tool = open.unwrap_or(subject.tool);
            }
            continue;
        }
        // A workspace of several parts is a menu of them, as WoWEdit's menu
        // bar lists its editors under a subject. A part that cannot be used
        // now is listed disabled with the reason, so the menu shows
        // everything the workspace holds: with no database, Creatures still
        // offers its display tables, which are files.
        let name = format!("{} \u{23F7}", subject.name);
        // The menu is drawn in a child of the row, which a wrapping row
        // places before the menu's width is known, so the row is ended here
        // when the button would not fit.
        if ui.layout().main_wrap() && ui.cursor().min.x > ui.max_rect().min.x {
            let text = egui::WidgetText::from(name.as_str());
            let galley = text.into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
            // A wrapping row's available width is the whole row's, not what is
            // left of it.
            let left = ui.max_rect().right() - ui.cursor().min.x;
            if galley.size().x + 2.0 * ui.spacing().button_padding.x > left {
                ui.end_row();
            }
        }
        ui.add_enabled_ui(why.is_none(), |ui| {
            let button = egui::Button::selectable(lit, name);
            let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                for &(_, part_tool) in subject.tool.parts() {
                    let Some(entry) = part(part_tool) else { continue };
                    let blocked = held(part_tool, playing, server);
                    let on = *tool == part_tool || popped.contains(&part_tool);
                    let item = ui.add_enabled(blocked.is_none(), egui::Button::selectable(on, entry.name));
                    let item = match blocked {
                        Some(_) => item.on_disabled_hover_text(tooltip(&entry, blocked)),
                        None => item.on_hover_text(entry.about),
                    };
                    if item.clicked() {
                        *tool = part_tool;
                        ui.close();
                    }
                }
            });
            match why {
                Some(_) => response.on_disabled_hover_text(tooltip(&subject, why)),
                None => response.on_hover_text(tooltip(&subject, None)),
            };
        });
    }
}

/// The strip at the head of a workspace's list that switches between the
/// workspace's parts ([`Tool::parts`]). Draws nothing for a workspace of one
/// tool. Answers the part pressed when it is not the one in use; the shell
/// switches to it after the frame is drawn, since the tool is borrowed while
/// a workspace draws.
pub(crate) fn parts(ui: &mut egui::Ui, tool: Tool) -> Option<Tool> {
    let parts = tool.parts();
    if parts.is_empty() {
        return None;
    }
    let mut chosen = tool;
    // At most `TAB_ROW` (four) parts to a row, as the tabs are drawn. The
    // segments share the row's width, so the Sound workspace's seven names on
    // one row would be truncated. The rows are balanced: seven parts are four
    // and three, and the Skills workspace's six are three and three rather
    // than four and two.
    let rows = parts.len().div_ceil(crate::tables::tabs::TAB_ROW);
    for row in parts.chunks(parts.len().div_ceil(rows)) {
        ui.allocate_ui(egui::vec2(ui.available_width(), 24.0), |ui| {
            theme::segmented(ui, &mut chosen, row, |a, b| a == b);
        });
    }
    ui.add_space(6.0);
    (chosen != tool).then_some(chosen)
}

/// Select and Measure, drawn over the viewport's top-left corner. Answers the
/// rectangle drawn, which the caller adds to the viewport's floating list so a
/// click on a button is not also a click on the ground behind it.
pub(crate) fn pointer(ctx: &egui::Context, viewport: egui::Rect, tool: &mut Tool, rail: &mut Rail) -> Option<egui::Rect> {
    if !viewport.is_positive() {
        return None;
    }
    let area = egui::Area::new(egui::Id::new("editor-pointer-tools"))
        .order(egui::Order::Middle)
        .fixed_pos(viewport.min + egui::vec2(8.0, 8.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::PANEL)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        for subject in corner() {
                            let on = *tool == subject.tool;
                            if tile(ui, &subject, on, None, egui::Vec2::splat(CORNER), CORNER_PICTURE) {
                                *tool = subject.tool;
                                rail.follow(subject.tool);
                            }
                        }
                    });
                });
        });
    Some(area.response.rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rail_tools() -> Vec<Tool> {
        HEADINGS
            .iter()
            .flat_map(|heading| tiles(heading))
            .map(|subject| subject.tool)
            .collect()
    }

    fn corner_tools() -> Vec<Tool> {
        corner().map(|subject| subject.tool).collect()
    }

    /// Every rail and corner tool has an icon, so no tile falls back to its
    /// name.
    #[test]
    fn every_rail_and_corner_tool_has_an_icon() {
        for tool in rail_tools().into_iter().chain(corner_tools()) {
            assert!(ToolIcon::of(tool).is_some(), "{} has no icon", tool.name());
        }
    }

    /// Every heading a tile names is one the rail draws, and every heading
    /// has a tile under it.
    #[test]
    fn every_tile_is_under_a_drawn_heading() {
        for spec in SPECS {
            if let Place::Rail(heading) = spec.place {
                assert!(HEADINGS.contains(&heading), "{} is under {heading}", spec.name);
            }
        }
        for heading in HEADINGS {
            assert!(!tiles(heading).is_empty(), "{heading} has no tile");
        }
    }

    /// Every tool is in exactly one of the three places: a tile, a workspace
    /// or one of its parts, or a corner button. A tool reachable only by a
    /// shortcut is not found by a person who does not already know the
    /// shortcut, and a tool in two places is two answers to where it is.
    #[test]
    fn every_tool_is_in_exactly_one_place() {
        // A workspace of several tools places each of them, on the strip at
        // the head of its list.
        let in_workspaces = bar().flat_map(|subject| match subject.tool.parts() {
            [] => vec![subject.tool],
            parts => parts.iter().map(|&(_, part)| part).collect(),
        });
        let placed: Vec<Tool> = rail_tools()
            .into_iter()
            .chain(in_workspaces)
            .chain(corner_tools())
            .collect();
        for tool in crate::tools::ALL {
            let count = placed.iter().filter(|had| **had == tool).count();
            assert_eq!(count, 1, "{} is placed {count} times", tool.name());
        }
    }

    /// The rail and the corner hold the subjects the viewport is the document
    /// for, and the workspaces hold the ones that replace it, as
    /// [`Tool::surface`] decides.
    #[test]
    fn the_places_agree_with_the_surfaces() {
        for tool in rail_tools().into_iter().chain(corner_tools()) {
            assert_ne!(tool.surface(), Surface::Middle, "{} replaces the viewport", tool.name());
        }
        for subject in bar() {
            assert_eq!(
                subject.tool.surface(),
                Surface::Middle,
                "{} keeps the viewport, so it is not a workspace",
                subject.name
            );
        }
    }

    /// The *Spawns* tiles are rows in vmangos' database, and no other tile is.
    /// [`held`] greys a tool with a server table when there is no database, so
    /// a spawn tile that named none would open an empty panel on such a
    /// machine.
    #[test]
    fn the_spawn_tiles_and_only_they_name_a_server_table() {
        for group in HEADINGS {
            for subject in tiles(group) {
                assert_eq!(
                    subject.tool.server_table().is_some(),
                    group == "Spawns",
                    "{} is in {group}",
                    subject.name
                );
            }
        }
    }

    /// Every part of a workspace of several parts has a menu entry, and
    /// every menu entry is a part of a workspace on the bar.
    #[test]
    fn every_part_is_on_its_workspaces_menu() {
        for subject in bar() {
            for &(_, tool) in subject.tool.parts() {
                assert!(part(tool).is_some(), "{} has no menu entry", tool.name());
            }
        }
        for entry in SPECS.iter().filter_map(|spec| part(spec.tool)) {
            assert!(
                bar().any(|subject| subject.tool.parts().iter().any(|&(_, tool)| tool == entry.tool)),
                "{} is on no workspace's menu",
                entry.name
            );
        }
    }

    /// Every subject says what it is for, in more than a word.
    #[test]
    fn every_subject_says_what_it_is_for() {
        for subject in HEADINGS
            .iter()
            .flat_map(|heading| tiles(heading))
            .chain(bar())
            .chain(SPECS.iter().filter_map(|spec| part(spec.tool)))
            .chain(corner())
        {
            assert!(!subject.name.is_empty());
            assert!(subject.about.len() > 20, "{} says too little", subject.name);
        }
    }

    /// The rail starts at two columns, changes to one when one column fits
    /// with [`SLACK`] to spare, and back to two when one column does not fit.
    /// A height between the two keeps the count it has. One column is at
    /// least a tile wide and widens for a heading wider than a tile.
    #[test]
    fn the_rail_is_one_column_when_one_column_fits() {
        let mut rail = Rail::default();
        assert_eq!((rail.columns(), rail.width()), (2, theme::RAIL_WIDTH));
        assert_eq!(next_columns(2, 900.0, 880.0), 1);
        assert_eq!(next_columns(2, 884.0, 880.0), 2);
        assert_eq!(next_columns(1, 884.0, 880.0), 1);
        assert_eq!(next_columns(1, 879.0, 880.0), 2);
        rail.columns = 1;
        assert_eq!(rail.width(), TILE + MARGINS);
        rail.heading = 50.0;
        assert_eq!(rail.width(), 50.0 + MARGINS);
        assert!(rows(1) > rows(2));
    }

    /// *World* returns to the last rail or corner tool, and a workspace does
    /// not replace it.
    #[test]
    fn world_returns_to_the_last_world_tool() {
        let mut rail = Rail::default();
        rail.follow(Tool::Flightpaths);
        rail.follow(Tool::Spells);
        assert_eq!(rail.last_world, Tool::Flightpaths);
        rail.follow(Tool::Measure);
        assert_eq!(rail.last_world, Tool::Measure);
    }

    /// During a playtest every tile is unavailable and a workspace is not;
    /// with no database, the spawn tiles and the two server workspaces are
    /// unavailable.
    #[test]
    fn a_subject_is_held_for_the_stated_reasons() {
        for tool in rail_tools().into_iter().chain(corner_tools()) {
            assert!(held(tool, true, true).is_some(), "{}", tool.name());
        }
        for subject in bar() {
            assert_eq!(
                held(subject.tool, true, true).is_none(),
                subject.tool.survives_playtest(),
                "{}",
                subject.name
            );
        }
        assert!(held(Tool::Zones, true, true).is_some());
        assert!(held(Tool::CreatureData, true, true).is_some());
        assert!(held(Tool::GameObjectData, false, false).is_some());
        assert!(held(Tool::Spells, true, true).is_none());
        assert!(held(Tool::Creatures, false, false).is_some());
        assert!(held(Tool::Items, false, false).is_some());
        assert!(held(Tool::Spells, false, false).is_none());
        assert!(held(Tool::Terrain, false, false).is_none());
    }
}
