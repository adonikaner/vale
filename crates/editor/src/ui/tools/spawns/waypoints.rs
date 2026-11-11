//! The waypoint window: a creature's path as a list of points, beside the
//! same path drawn in the world.
//!
//! It is a window on the root context rather than a block in the inspector,
//! for two reasons. The first is the one [`super::spawn::template_window`]
//! gives: a window inside a panel is clipped to it, and this window has to
//! sit where it does not cover the ground the points are on. The second is
//! that the pointer does the editing, so the window has to be movable out of
//! the way of what the pointer is on.
//!
//! ## One selection for the list and the world
//!
//! [`crate::tools::spawns::waypoints::Waypoints::selected`] is an index into the path,
//! and both the row in the list and the ring in the world read it. Clicking a
//! row selects the point; clicking the point selects the row. There is no
//! second selection, so the list and the world cannot disagree.
//!
//! ## Why every number field is a `DragValue`
//!
//! `-8817.5` is typed one character at a time and `-` alone is not a number, so
//! a box rebuilt from the stored value each frame snaps back on the first
//! keystroke. `egui::DragValue` keeps its own draft while it has focus. See
//! [`crate::ui::widgets::rowform::number_cell`].
//!
//! ## A point's script
//!
//! A point's `script_id` names a `creature_movement_scripts` script, which
//! the server runs when the creature reaches the point. A creature that stops
//! and plays an emote does it this way: a point with a waittime runs a script
//! that sets the emote, and the next point runs one that clears it. The row
//! under the id carries the event form's script buttons
//! ([`script_controls`]): open the script in the script window, choose an
//! existing one, make a new one, or clear the id. The chooser cannot write
//! the id itself, because a path is written whole and this window holds it,
//! so it leaves the choice in
//! [`crate::subjects::behaviour::Behaviour::chosen_for_point`] and this
//! window writes it on its next frame.
//!
//! The behaviour tool reads the scripts every point names
//! (`Behaviour::other_scripts`, filled by `crate::ui::subjects::windows`).
//! A script id the table does not hold is shown as a warning on the point and
//! in the window's head, because the server skips that point when it loads
//! the path.

use bevy_egui::egui;

use crate::ui::widgets::theme::{self, field, property};
use crate::server::creatures::{PATHS_VPATH, SQL_VPATH};
use crate::server::store::Gesture;
use crate::session::EditSession;
use crate::subjects::behaviour::{Behaviour, PathPoint, ScriptAnswer};
use crate::tools::spawns::waypoints::Waypoints;
use crate::ui::subjects::behaviour::script_controls;
use vale_mangos::path::{self, Node, Path, Walk, Which};
use vale_mangos::scripts::CREATURE_MOVEMENT;

/// Everything the window needs, as one argument.
pub(crate) struct Subject<'a> {
    pub(crate) session: &'a mut EditSession,
    pub(crate) waypoints: &'a mut Waypoints,
    /// The behaviour tool, which reads the points' scripts and holds the
    /// script window and its chooser.
    pub(crate) behaviour: &'a mut Behaviour,
    /// The frame clock the undo stack folds a gesture by.
    pub(crate) now: f64,
    /// Where the Fly Here button and a double-click on a point send their
    /// request. See [`crate::camera::FlyTo`].
    pub(crate) fly: &'a mut bevy::prelude::Messages<crate::camera::FlyTo>,
}

/// Draws the window, or nothing when no creature's path is open.
///
/// Returns its rectangle so the viewport can keep a click inside it from also
/// being a click on the ground behind it.
pub(crate) fn window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.waypoints.showing {
        return None;
    }
    // Open with nothing selected: the window has nothing to show and gets out
    // of the way, as the template window beside it does. It has not been shut
    // — `showing` is still on — so clicking another creature brings it back.
    subject.waypoints.subject.as_ref()?;
    // A creature whose path is still being read keeps the window up. The read
    // decides which table answers, so for a frame or two there is a creature
    // and no path. Hiding the window in those frames made it blink on every
    // click, which looks like the window closing itself.
    let Some((which, owner)) = subject.waypoints.open else {
        return reading(ctx, subject);
    };
    let mut path = subject.waypoints.path(Some(subject.session))?;
    take_the_chosen_script(subject, &mut path);
    let mut open = true;
    // The title names the table as well as the id. `id 80849` and
    // `entry 330` are different rows, an edit to each reaches a different set
    // of spawns, and nothing but the window says which one is open.
    // The reading form below shares the window's id, so the switch between
    // the two moves nothing on screen.
    let response = theme::window(format!("Waypoints — {} {owner}", which.table()), "waypoints")
        .open(&mut open)
        // Wide enough for the list and the form side by side; see the comment
        // on the two columns below. In one column the form sat under the list,
        // which is as long as the path, so reaching the form for point 40 of 60
        // scrolled point 40 off the screen.
        .default_size([640.0, 560.0])
        // Narrower than this, the toolbar's two ends meet in the middle.
        .min_width(560.0)
        // To the left of the inspector and clear of the top bar, so it covers
        // neither the button that opened it nor, usually, the path itself.
        .default_pos([300.0, 120.0])
        .show(ctx, |ui| {
            head(ui, subject, &path);
            ui.add_space(4.0);
            controls(ui, subject, &mut path);
            // The list and the selected point's form are two columns, each
            // with its own scroll, so the form stays on screen wherever the
            // list is scrolled. A path has as many points as somebody gives it
            // (the longest shipped path has 227), and with the form stacked
            // under the list, reaching the form scrolled the point being
            // edited out of sight.
            ui.columns(2, |columns| {
                theme::heading(&mut columns[0], "Points");
                list(&mut columns[0], subject, &mut path);
                egui::ScrollArea::vertical()
                    .id_salt("waypoint-form")
                    .auto_shrink([false; 2])
                    .show(&mut columns[1], |ui| node_form(ui, subject, &mut path));
            });
        });

    if !open {
        subject.waypoints.close();
    }
    response.map(|response| response.response.rect)
}

/// The window while the path is still being read: the same window, with one
/// line in it.
///
/// It is drawn rather than skipped so that clicking from creature to creature
/// does not hide the window and show it again. It keeps the title bar and the
/// frame, so only the contents change on screen.
fn reading(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    let guid = subject.waypoints.subject.map(|open| open.guid)?;
    let mut open = true;
    let response = theme::window(format!("Waypoints — creature {guid}"), "waypoints")
        .open(&mut open)
        .default_size([640.0, 560.0])
        .min_width(560.0)
        .default_pos([300.0, 120.0])
        .show(ctx, |ui| {
            theme::waiting(
                ui,
                "Reading this creature's path: creature_movement for the spawn first, then \
                 creature_movement_template, the order the server resolves them in.",
            );
        });
    if !open {
        subject.waypoints.close();
    }
    response.map(|response| response.response.rect)
}

/// Write a script chosen in the script chooser into the point it was chosen
/// for. The chooser cannot write it, because the path is written whole and
/// this window holds it. A choice for a path that is no longer open is
/// dropped, and the status line says so.
fn take_the_chosen_script(subject: &mut Subject<'_>, path: &mut Path) {
    let Some((point, id)) = subject.behaviour.chosen_for_point.take() else {
        return;
    };
    let index = point.index;
    if (point.which, point.owner) != (path.which, path.owner) || index >= path.nodes.len() {
        subject.session.status = format!(
            "script {id} not written: {} {} point {} is not open",
            point.which.table(),
            point.owner,
            index + 1
        );
        return;
    }
    path.nodes[index].script_id = id;
    write(subject, path, "Choose waypoint script");
    subject.session.status = format!("point {} runs {CREATURE_MOVEMENT} {id}", index + 1);
}

/// Whether `creature_movement_scripts` holds a script once this project is
/// applied: the project's script where it has one, otherwise the database's.
/// `None` while the database's rows are being read.
fn script_exists(subject: &Subject<'_>, id: u32) -> Option<bool> {
    let script = subject.behaviour.script_of(CREATURE_MOVEMENT, id, subject.session)?;
    Some(!script.rows.is_empty())
}

/// Small text in the warning colour, for what makes an edit here do
/// something other than it seems to.
fn warning(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text.into()).size(theme::SMALL).color(theme::WARN));
}

/// What this path is, what walks it, and whether it is walked at all.
fn head(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &Path) {
    let (which, _owner) = subject.waypoints.open.expect("the window is open");

    // The totals come first: each number in the body colour, and the unit it
    // counts in the dim colour.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let count = path.nodes.len();
        let wait = path.total_wait();
        let stats = [
            (count.to_string(), if count == 1 { "point" } else { "points" }),
            (format!("{:.0}", path.loop_length()), "yd loop"),
            match wait {
                0 => (String::new(), "no waits"),
                _ => (format!("{:.1}", wait as f64 / 1000.0), "s total wait"),
            },
        ];
        for (i, (number, unit)) in stats.into_iter().enumerate() {
            if i > 0 {
                ui.label(egui::RichText::new("·").color(theme::INK_FAINT));
            }
            if !number.is_empty() {
                ui.label(egui::RichText::new(number).color(theme::INK));
            }
            ui.label(egui::RichText::new(unit).color(theme::INK_DIM));
        }
    });

    // Which table answered, and which spawns an edit changes. A creature with
    // no path of its own walks its template's, and editing that path changes
    // every spawn of the creature that has none. The creature panel makes the
    // same split between this spawn and its template, and the points
    // themselves do not show which of the two is being edited.
    match which {
        Which::Spawn => theme::note(ui, which.about()),
        Which::Template => warning(
            ui,
            "This spawn has no creature_movement rows, so the server uses its template's path \
             from creature_movement_template. Editing these points changes the path of every \
             spawn of this creature that has no creature_movement rows.",
        ),
    }

    // Whether an edit here has any effect. The server loads the path of a
    // creature whose movement type is idle or random but never uses it, and
    // nothing else on screen says so.
    let walk = subject.waypoints.walk();
    match walk {
        Walk::Never => warning(
            ui,
            "This creature's movement_type does not use a path. Edits to these points \
             have no effect in the game until movement_type is 2 (waypoint) or 3 \
             (cyclic), set on the creature's form.",
        ),
        _ => theme::note(
            ui,
            format!("movement_type {} — {}", subject.waypoints.movement_type, walk.about()),
        ),
    }
    // The movement generator's minimum number of nodes is the other way a
    // path has no effect: it refuses a cyclic path of one node.
    if path.nodes.len() < walk.needs_nodes() {
        let has = match path.nodes.len() {
            1 => "1 point".to_string(),
            n => format!("{n} points"),
        };
        warning(
            ui,
            format!(
                "{has}. This movement generator needs at least {} points, so the creature stands \
                 still.",
                walk.needs_nodes()
            ),
        );
    }

    // A point whose script the table does not hold is dropped by the server
    // when it loads the path, so the creature walks a shorter path than the
    // one drawn here.
    let missing: Vec<String> = path
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.script_id != 0 && script_exists(subject, node.script_id) == Some(false))
        .map(|(index, _)| (index + 1).to_string())
        .collect();
    if !missing.is_empty() {
        let (points, name, those) = match missing.len() {
            1 => ("Point", "names", "that point"),
            _ => ("Points", "name", "those points"),
        };
        warning(
            ui,
            format!(
                "{points} {} {name} a script id that {CREATURE_MOVEMENT} does not hold. When the server \
                 loads this path it skips {those} and writes the reason to its database error log.",
                missing.join(", ")
            ),
        );
    }

    if subject.waypoints.trouble.is_some() {
        let why = subject.waypoints.trouble.clone().unwrap_or_default();
        ui.label(
            egui::RichText::new(format!("The database query failed: {why}"))
                .size(theme::SMALL)
                .color(theme::BAD),
        );
    }
}

/// The window's bar: the two switches, and the operations on the whole path.
/// What acts on one point is on that point's form.
///
/// The row is laid out as a viewport toolbar is. Add Points and Snap to
/// Ground are each on or off by itself, so each is a [`theme::bar_switch`],
/// and the buttons are the bar's height.
fn controls(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &mut Path) {
    ui.horizontal(|ui| {
        // Add Points stays on after a click rather than acting only while
        // held, so a scripted run can build a path; see the module comment of
        // `crate::tools::spawns::waypoints`.
        let armed = subject.waypoints.adding;
        if theme::bar_switch(ui, armed, "Add Points")
            .on_hover_text(
                "While on, a click on the ground adds a point there: after the selected \
                 point, or at the end when none is selected. Clicks do not select other \
                 creatures until it is off. A path created this way for a creature with no \
                 rows is written to creature_movement for this spawn.",
            )
            .clicked()
        {
            subject.waypoints.adding = !armed;
        }
        let follows = subject.waypoints.follow_ground;
        if theme::bar_switch(ui, follows, "Snap to Ground")
            .on_hover_text(
                "A point added or dragged takes the height of the terrain under the pointer. \
                 Turn it off for a creature that flies, or one walking on a building's floor: \
                 the terrain under a building is not the surface it stands on.",
            )
            .clicked()
        {
            subject.waypoints.follow_ground = !follows;
        }

        // The two that undo or delete the whole path, at the other end of the
        // row from the switches.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Two different undos, with labels that say which. Discard Path
            // Edits gives up this project's claim on the path; Clear All
            // Points empties the path, which is an edit that deletes every row.
            if ui
                .add_enabled(
                    !path.nodes.is_empty(),
                    egui::Button::new(egui::RichText::new("Clear All Points").color(theme::BAD))
                        .min_size(egui::vec2(0.0, theme::BAR_HEIGHT)),
                )
                .on_hover_text(
                    "Removes every point. This is an edit: Apply writes a DELETE and no INSERTs, \
                     so the creature stops walking once it is applied.",
                )
                .clicked()
            {
                path.nodes.clear();
                write(subject, path, "Clear path");
                subject.waypoints.selected = None;
                subject.session.status = "path emptied — applying this removes every point".to_string();
            }
            let edited = subject.waypoints.edited(Some(subject.session));
            if ui
                .add_enabled(
                    edited,
                    egui::Button::new("Discard Path Edits").min_size(egui::vec2(0.0, theme::BAR_HEIGHT)),
                )
                .on_hover_text(
                    "Discards this project's edits to the path. The window and the world show \
                     the database's rows again. The database is not changed.",
                )
                .clicked()
            {
                let (which, owner) = subject.waypoints.open.expect("the window is open");
                let subject_name = format!("{} {owner} path", which.table());
                subject.session.set_server_path(
                    which,
                    owner,
                    None,
                    Some(Gesture {
                        label: "Discard path edits",
                        subject: &subject_name,
                        now: subject.now,
                    }),
                );
                subject.waypoints.selected = None;
                subject.session.status = "path edits discarded; the database's rows are shown".to_string();
            }
        });
    });

    // Copy Path to Spawn, offered while the template's path is open. A
    // template path is shared, so changing one spawn's route means giving
    // that spawn its own copy. The server's `GetDefaultPath` takes the spawn's
    // own path first, so the copy is used from the moment it is applied, and
    // the template's path is unchanged for every other spawn.
    if subject.waypoints.editing_the_template() {
        let guid = subject.waypoints.subject.map(|open| open.guid);
        if let Some(guid) = guid {
            if ui
                .button("Copy Path to Spawn")
                .on_hover_text(
                    "Copies these points into creature_movement under this guid. \
                     creature_movement_template is not changed. The server uses a spawn's \
                     creature_movement rows before its template's, so only this spawn changes.",
                )
                .clicked()
            {
                let own = Path { which: Which::Spawn, owner: guid, nodes: path.nodes.clone() };
                let name = format!("creature_movement {guid} path");
                subject.session.set_server_path(
                    Which::Spawn,
                    guid,
                    Some(&own),
                    Some(Gesture { label: "Copy path to spawn", subject: &name, now: subject.now }),
                );
                // The window is about the spawn's path from here on: the read
                // is dropped so the next frame resolves it again, and the
                // resolve prefers a path the project has given the spawn.
                subject.waypoints.forget();
                subject.session.status =
                    format!("{} point(s) copied to spawn {guid}", own.nodes.len());
            }
        }
    }

    if subject.waypoints.edited(Some(subject.session)) {
        // The revert file tells an applied path from an unapplied one: a path
        // named there has had its rows written. The project's claim on the
        // path cannot tell them apart, because the store keeps the claim after
        // Apply; a line that tested the claim showed "Not applied" for a
        // change that was already in the database.
        let applied = crate::server::creatures::path_applied(subject.session, path);
        let line = match applied {
            false => format!(
                "Not applied. Save writes {SQL_VPATH} and {PATHS_VPATH}; Apply on the Server \
                 panel, under Creatures, puts it in the database."
            ),
            true => "Applied — the database holds these points. Restart the server to see the \
                     creature walk them; there is no .reload for creature_movement. Editing \
                     further needs another Apply."
                .to_string(),
        };
        warning(ui, line);
    }
}

/// How tall one point's row is in the list.
const ROW: f32 = 20.0;

/// Where each of the list's columns ends, from the row's left edge: the
/// number, x, y and z, which are right-aligned to it, and the wait mark,
/// which is centred on it.
const COLUMNS: [f32; 5] = [30.0, 106.0, 182.0, 248.0, 268.0];

/// The list's numbers.
fn row_font() -> egui::FontId {
    egui::FontId::monospace(12.0)
}

/// Every point as a row: which it is, where it is, and whether it waits there.
///
/// Painted rather than built from labels, so a row is a full-width target
/// with its numbers in columns under the heading's names.
fn list(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &mut Path) {
    if path.nodes.is_empty() {
        // How a path is created is on the Add Points switch's hover.
        theme::note(ui, "This creature has no rows in creature_movement or creature_movement_template.");
        return;
    }

    // The columns' names, which stay put while the rows scroll under them.
    let (header, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::hover());
    let painter = ui.painter();
    for (name, at, align) in [
        ("#", COLUMNS[0], egui::Align2::RIGHT_CENTER),
        ("X", COLUMNS[1], egui::Align2::RIGHT_CENTER),
        ("Y", COLUMNS[2], egui::Align2::RIGHT_CENTER),
        ("Z", COLUMNS[3], egui::Align2::RIGHT_CENTER),
        ("Wait", COLUMNS[4], egui::Align2::CENTER_CENTER),
    ] {
        painter.text(
            egui::pos2(header.left() + 6.0 + at, header.center().y),
            align,
            name,
            egui::FontId::proportional(theme::SMALL),
            theme::INK_FAINT,
        );
    }

    let chosen = subject.waypoints.selected;
    let hovered = subject.waypoints.hovered;
    // `show_rows` adds this ui's spacing to every row's height when it works
    // out which rows are on screen, so it has to be the spacing the rows are
    // drawn with. With the default, the list stopped a few rows short of the
    // bottom of the window.
    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical()
        .id_salt("waypoint-list")
        .auto_shrink([false; 2])
        .show_rows(ui, ROW, path.nodes.len(), |ui, rows| {
            for index in rows {
                let node = &path.nodes[index];
                let (rect, row) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW), egui::Sense::click());
                let painter = ui.painter();
                // A selected row is drawn as a selected list row is everywhere
                // in the editor. A hovered row, hovered here or as the point
                // under the pointer in the world, gets a raised fill.
                if chosen == Some(index) {
                    painter.rect(
                        rect,
                        egui::CornerRadius::same(3),
                        theme::ACCENT_SUNK,
                        egui::Stroke::new(1.0, theme::ACCENT),
                        egui::StrokeKind::Inside,
                    );
                } else if row.hovered() || hovered == Some(index) {
                    painter.rect_filled(rect, egui::CornerRadius::same(3), theme::RAISED);
                }
                let y = rect.center().y;
                let left = rect.left() + 6.0;
                painter.text(
                    egui::pos2(left + COLUMNS[0], y),
                    egui::Align2::RIGHT_CENTER,
                    (index + 1).to_string(),
                    row_font(),
                    theme::INK_FAINT,
                );
                for (value, at) in [(node.x, COLUMNS[1]), (node.y, COLUMNS[2]), (node.z, COLUMNS[3])] {
                    painter.text(
                        egui::pos2(left + at, y),
                        egui::Align2::RIGHT_CENTER,
                        format!("{value:.1}"),
                        row_font(),
                        theme::INK,
                    );
                }
                // The wait is a mark rather than a number: which points the
                // creature stops at is readable at a glance, and how long is
                // on the form.
                if node.waittime > 0 {
                    painter.circle_filled(egui::pos2(left + COLUMNS[4], y), 3.0, theme::WARN);
                }
                if row.clicked() {
                    subject.waypoints.selected = Some(index);
                }
                // A double-click flies the camera to the point, which is the only
                // way to reach a point of a path that runs past the streamed area.
                if row.double_clicked() {
                    subject.fly.write(crate::camera::FlyTo::at(bevy::prelude::Vec3::new(node.x, node.y, node.z)));
                }
            }
        });
}

/// The selected point's own columns, as property rows under the columns'
/// names, and what can be done to the point.
fn node_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &mut Path) {
    let Some(index) = subject.waypoints.selected else {
        theme::heading_about(ui, "Point", "Click a point, here or in the world, to edit it.");
        property(ui, "Selected", "", |ui| {
            super::spawn::value_in(ui, "None", theme::INK_DIM);
        });
        return;
    };
    let Some(node) = path.nodes.get(index).cloned() else {
        return;
    };
    theme::heading(ui, &format!("Point {} of {}", index + 1, path.nodes.len()));

    let mut edited = node.clone();
    let mut changed = false;
    let point = PathPoint { which: path.which, owner: path.owner, index };
    // The rows are named by the column, as the rows of a spawn's and a
    // template's form are, and as the window's warnings name them.
    for (name, value) in [
        ("position_x", &mut edited.x),
        ("position_y", &mut edited.y),
        ("position_z", &mut edited.z),
    ] {
        property(ui, name, "The point's position, in world coordinates.", |ui| {
            changed |= field(ui, egui::DragValue::new(value).speed(0.25).fixed_decimals(2)).changed();
        });
    }
    property(
        ui,
        "waittime",
        "How long the creature waits here, in milliseconds. The server applies the point's \
         orientation only at a point with a waittime.",
        |ui| {
            changed |= field(ui, egui::DragValue::new(&mut edited.waittime).speed(100.0).suffix(" ms")).changed();
        },
    );
    property(
        ui,
        "wander_distance",
        "How far the creature may wander from the point while it waits.",
        |ui| {
            changed |= field(ui, egui::DragValue::new(&mut edited.wander_distance).speed(0.1).suffix(" yd")).changed();
        },
    );
    property(
        ui,
        "script_id",
        "A creature_movement_scripts id, or 0. The script runs when the creature reaches this \
         point. If that table has no such id, the server skips this point when it loads the \
         path, and the path is one point shorter.",
        |ui| {
            changed |= field(ui, egui::DragValue::new(&mut edited.script_id).speed(1.0)).changed();
        },
    );
    // The script's buttons are on a row of their own under the id, in the
    // value column: the id box and four buttons do not fit one row.
    let answered = property(ui, "", "", |ui| {
        script_controls(
            ui,
            subject.session,
            subject.behaviour,
            CREATURE_MOVEMENT,
            edited.script_id,
            None,
            ScriptAnswer::Point(point),
            subject.now,
        )
    });
    if let Some(id) = answered.and_then(|id| id.parse::<u32>().ok()) {
        edited.script_id = id;
        changed = true;
    }
    property(ui, "path_id", "A creature_movement_special path to run here, or 0.", |ui| {
        changed |= field(ui, egui::DragValue::new(&mut edited.path_id).speed(1.0)).changed();
    });
    // The facing is a checkbox and a number, because the column's empty
    // value is 100 rather than 0 and a person typing 0 into it would be
    // setting a real facing of due east. See `vale_mangos::path::NO_FACING`.
    property(
        ui,
        "orientation",
        "Whether the creature faces a direction on arrival. Off writes 100, which is what the \
         server reads as \"no facing\" \u{2014} not 0, which is a real facing of due east.",
        |ui| {
            let mut faces = edited.has_orientation();
            if ui.checkbox(&mut faces, "").changed() {
                edited.orientation = match faces {
                    true => 0.0,
                    false => path::NO_FACING,
                };
                changed = true;
            }
            match faces {
                true => {
                    changed |= field(
                        ui,
                        egui::DragValue::new(&mut edited.orientation)
                            .speed(0.05)
                            .range(0.0..=std::f32::consts::TAU)
                            .suffix(" rad"),
                    )
                    .changed();
                }
                false => {
                    super::spawn::value_in(ui, "None", theme::INK_DIM);
                }
            }
        },
    );
    if edited.has_orientation() && !edited.faces() {
        warning(
            ui,
            "This facing has no effect: the server applies a facing only at a point with a \
             waittime, and this point's waittime is 0.",
        );
    }
    script_line(ui, subject, edited.script_id);

    if changed {
        if let Some(slot) = path.nodes.get_mut(index) {
            *slot = edited;
        }
        write(subject, path, "Edit waypoint");
    }

    // Two rows of two equal buttons, then the removal at the form's width, so
    // the five have common edges.
    ui.add_space(8.0);
    ui.columns(2, |columns| {
        let fly = theme::full_button(&columns[0], "Fly Here");
        if columns[0].add(fly).clicked() {
            subject.fly.write(crate::camera::FlyTo::at(bevy::prelude::Vec3::new(node.x, node.y, node.z)));
        }
        let insert = theme::full_button(&columns[1], "Insert After");
        if columns[1]
            .add(insert)
            .on_hover_text("Inserts a new point half way to the next point.")
            .clicked()
        {
            // Half way to the next node, wrapping — the path is a loop, so the
            // point after the last one is the first.
            let next = path.nodes[(index + 1) % path.nodes.len()].clone();
            let middle = Node::at(
                (node.x + next.x) / 2.0,
                (node.y + next.y) / 2.0,
                (node.z + next.z) / 2.0,
            );
            path.insert(index + 1, middle);
            write(subject, path, "Add waypoint");
            subject.waypoints.selected = Some(index + 1);
        }
    });
    ui.columns(2, |columns| {
        let earlier = theme::full_button(&columns[0], "Earlier");
        if columns[0]
            .add_enabled(index > 0, earlier)
            .on_hover_text("Moves this point one place earlier in the path.")
            .clicked()
        {
            path.move_node(index, index - 1);
            write(subject, path, "Reorder waypoints");
            subject.waypoints.selected = Some(index - 1);
        }
        let later = theme::full_button(&columns[1], "Later");
        if columns[1]
            .add_enabled(index + 1 < path.nodes.len(), later)
            .on_hover_text("Moves this point one place later in the path.")
            .clicked()
        {
            path.move_node(index, index + 1);
            write(subject, path, "Reorder waypoints");
            subject.waypoints.selected = Some(index + 1);
        }
    });
    // `theme::full_button` with its text in the colour of a removal.
    let remove = egui::Button::new(egui::RichText::new("Remove Point").color(theme::BAD))
        .min_size(egui::vec2(ui.available_width(), 24.0));
    if ui
        .add(remove)
        .on_hover_text("Removes this point. Later points are renumbered.")
        .clicked()
    {
        path.remove(index);
        write(subject, path, "Remove waypoint");
        // Keep a selection where there is still one to have, so a run of
        // removals does not need a click between each.
        subject.waypoints.selected = match path.nodes.is_empty() {
            true => None,
            false => Some(index.min(path.nodes.len() - 1)),
        };
        subject.session.status = format!("removed point {}", index + 1);
    }
}

/// The line under the form that says what a point's script is: how many
/// steps it has and its first step's comment, or that the table does not
/// hold it. Nothing for a point with no script.
fn script_line(ui: &mut egui::Ui, subject: &Subject<'_>, id: u32) {
    if id == 0 {
        return;
    }
    let Some(script) = subject.behaviour.script_of(CREATURE_MOVEMENT, id, subject.session) else {
        match subject.behaviour.trouble() {
            Some(why) => warning(ui, format!("The database query for {CREATURE_MOVEMENT} {id} failed: {why}")),
            None => theme::waiting(ui, format!("Reading {CREATURE_MOVEMENT} {id}\u{2026}")),
        }
        return;
    };
    if script.rows.is_empty() {
        warning(
            ui,
            format!(
                "{CREATURE_MOVEMENT} has no script {id}. When the server loads this path it skips \
                 this point. Use + new to create the script, or clear to remove the id."
            ),
        );
        return;
    }
    let steps = script.rows.len();
    let comment = script.sorted().first().map(|row| row.comments.clone()).unwrap_or_default();
    let plural = if steps == 1 { "step" } else { "steps" };
    match comment.is_empty() {
        true => theme::note(ui, format!("{CREATURE_MOVEMENT} {id}: {steps} {plural}")),
        false => theme::note(ui, format!("{CREATURE_MOVEMENT} {id}: {steps} {plural}, \"{comment}\"")),
    }
}

/// Put the working path into the project's store, under one undo entry.
fn write(subject: &mut Subject<'_>, path: &Path, label: &'static str) {
    let subject_name = format!("{} {} path", path.which.table(), path.owner);
    subject.session.set_server_path(
        path.which,
        path.owner,
        Some(path),
        Some(Gesture {
            label,
            subject: &subject_name,
            now: subject.now,
        }),
    );
}
