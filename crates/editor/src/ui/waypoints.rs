//! **The waypoint window**: a creature's path as a list of points, beside the
//! same path drawn in the world.
//!
//! A window on the root context rather than a block in the inspector, for
//! [`super::creatures::template_window`]'s reason and one of its own. The
//! reason it shares: a window inside a panel is clipped to it, and this wants
//! to sit where it does not cover the ground the points are on. The reason of
//! its own: the pointer is doing the editing, so the panel has to be somewhere
//! a person can put it out of the way of the thing they are pointing at.
//!
//! ## The list and the world are one selection
//!
//! [`crate::tools::waypoints::Waypoints::selected`] is an index into the path,
//! and both the row here and the ring out there read it. Clicking a row selects
//! the point; clicking the point selects the row. There is no second notion of
//! what is chosen, which is what stops the two disagreeing.
//!
//! ## Every number is a drag, and the reason is in the creature form
//!
//! `-8817.5` is typed one character at a time and `-` alone is not a number, so
//! a box rebuilt from the stored value each frame snaps back on the first
//! keystroke. `egui::DragValue` keeps its own draft while it has focus. See
//! `super::creatures`, where this was found.

use bevy_egui::egui;

use super::theme;
use crate::server::creatures::{PATHS_VPATH, SQL_VPATH};
use crate::session::{EditSession, Gesture};
use crate::tools::waypoints::Waypoints;
use vale_mangos::path::{self, Node, Path, Walk, Which};

/// Everything the window needs, as one argument.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub waypoints: &'a mut Waypoints,
    /// The frame clock the undo stack folds a gesture by.
    pub now: f64,
}

/// **The window**, or nothing when no creature's path is open.
///
/// Returns its rectangle so the viewport can keep a click inside it from also
/// being a click on the ground behind it.
pub fn window(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    if !subject.waypoints.showing {
        return None;
    }
    // Open with nothing selected: the window has nothing to show and gets out
    // of the way, as the template window beside it does. It has not been shut
    // — `showing` is still on — so clicking another creature brings it back.
    subject.waypoints.subject.as_ref()?;
    // **…but a creature whose read is still in flight keeps the window up.**
    // Which table answers is decided by that read, so there is a frame or two
    // with a creature and no path; letting the window vanish there made it
    // blink on every click, which reads as the panel closing itself.
    let Some((which, owner)) = subject.waypoints.open else {
        return reading(ctx, subject);
    };
    let mut path = subject.waypoints.path(Some(subject.session))?;
    let mut open = true;
    // **The title names the table, not just the id.** `id 80849` and
    // `entry 330` are different rows with different blast radiuses and the
    // window is the only thing that can say which one is open.
    let response = egui::Window::new(format!("Waypoints — {} {owner}", which.table()))
        // One id whatever the title says, so the window keeps its place when
        // another creature is clicked — see `super::creatures::template_window`.
        // The reading form below shares it, so the switch between the two
        // moves nothing on screen.
        .id(egui::Id::new("waypoints"))
        .open(&mut open)
        // **Wide enough for the list and the form side by side** — see the
        // note on the two columns below. In one column the form sat under a
        // list that is as long as the path, so editing point 40 of 60 meant
        // scrolling the thing being edited off the screen to reach it.
        .default_size([640.0, 560.0])
        // Narrower than this, the toolbar's two ends meet in the middle.
        .min_width(560.0)
        // To the left of the inspector and clear of the top bar, so it covers
        // neither the button that opened it nor, usually, the path itself.
        .default_pos([300.0, 120.0])
        .resizable(true)
        .frame(frame())
        .show(ctx, |ui| {
            head(ui, subject, &path);
            ui.add_space(4.0);
            controls(ui, subject, &mut path);
            ui.add_space(2.0);
            // **The list and the chosen point's form are two columns**, each
            // scrolling on its own, so the form is on screen whatever the list
            // is scrolled to. A path is as long as somebody makes it — 227
            // points on the longest one shipped — and stacked vertically the
            // form was reached by scrolling the point being edited out of
            // sight.
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

/// The window's frame, the same for the path and for the reading form.
fn frame() -> egui::Frame {
    egui::Frame::default()
        .fill(theme::SHELL)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(10))
}

/// **The window while the path is still being read**, which is the same window
/// with one line in it.
///
/// Drawn rather than skipped so that clicking from creature to creature does
/// not blink the panel out and back. It keeps the title and the frame, so the
/// thing that moves on screen is the contents rather than the window.
fn reading(ctx: &egui::Context, subject: &mut Subject<'_>) -> Option<egui::Rect> {
    let guid = subject.waypoints.subject.map(|open| open.guid)?;
    let mut open = true;
    let response = egui::Window::new(format!("Waypoints — creature {guid}"))
        .id(egui::Id::new("waypoints"))
        .open(&mut open)
        .default_size([640.0, 560.0])
        .min_width(560.0)
        .default_pos([300.0, 120.0])
        .resizable(true)
        .frame(frame())
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

/// Small text in the warning colour, for what makes an edit here do
/// something other than it seems to.
fn warning(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text.into()).size(theme::SMALL).color(theme::WARN));
}

/// What this path is, what walks it, and whether it is walked at all.
fn head(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &Path) {
    let (which, _owner) = subject.waypoints.open.expect("the window is open");

    // The totals first, as the line the eye lands on: a number in the body
    // colour and what it counts in the dim one.
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

    // **Which table answered, and what changing it changes.** A creature with
    // no path of its own walks its template's, and editing that moves every
    // spawn of the kind that has none — the same *this spawn* against *its
    // template* split the creature panel makes, and just as invisible in the
    // thing being edited.
    match which {
        Which::Spawn => theme::note(ui, which.about()),
        Which::Template => warning(
            ui,
            "This spawn has no `creature_movement` rows, so the server uses its template's path from `creature_movement_template`. Editing these points changes the path of every spawn of this creature that has no `creature_movement` rows.",
        ),
    }

    // **Whether an edit here does anything**, which is the first thing to say.
    // A path on a creature whose movement type is idle or random is loaded by
    // the server and never used, and nothing else on screen would say so.
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
    // …and the generator's own floor, which is the other way a path does
    // nothing: a cyclic path of one node is refused outright.
    if path.nodes.len() < walk.needs_nodes() {
        warning(
            ui,
            format!(
                "{} point(s). This movement generator needs at least {} points, so the \
                 creature stands still.",
                path.nodes.len(),
                walk.needs_nodes()
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

/// The switches, and the operations on the whole path. What acts on one point
/// is on that point's form.
fn controls(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &mut Path) {
    ui.horizontal(|ui| {
        // **Add is armed rather than held**, so a scripted run can build a
        // path — see the tool's module comment. Armed, it is drawn the way a
        // chosen segment is, so it reads as a mode that is on.
        let armed = subject.waypoints.adding;
        let add = egui::Button::new(egui::RichText::new("Add points").color(match armed {
            true => theme::INK,
            false => theme::INK_DIM,
        }))
        .fill(match armed {
            true => theme::ACCENT_SUNK,
            false => theme::RAISED,
        })
        .stroke(match armed {
            true => egui::Stroke::new(1.0, theme::ACCENT),
            false => egui::Stroke::NONE,
        });
        if ui
            .add(add)
            .on_hover_text(
                "While on, a click on the ground adds a point there: after the selected \
                 point, or at the end when none is selected. Clicks do not select other \
                 creatures until it is off.",
            )
            .clicked()
        {
            subject.waypoints.adding = !armed;
        }
        ui.checkbox(&mut subject.waypoints.follow_ground, "Snap to ground")
            .on_hover_text(
                "A point added or dragged takes the height of the terrain under the pointer. \
                 Turn it off for a creature that flies, or one walking on a building's floor: \
                 the terrain under a building is not the surface it stands on.",
            );

        // The two that undo or delete the whole path, at the other end of the
        // row from the switches.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // **Two different undos, and the labels say which.** One gives up
            // this project's claim on the path; the other empties the path,
            // which is an edit that deletes every row.
            if ui
                .add_enabled(
                    !path.nodes.is_empty(),
                    egui::Button::new(egui::RichText::new("Clear all points").color(theme::BAD)),
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
                .add_enabled(edited, egui::Button::new("Discard path edits"))
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

    // **The narrower edit**, offered where it is the one somebody means: a
    // template path is shared, and giving this guard a copy is what "change
    // this one guard's route" actually is. `GetDefaultPath` takes the spawn's
    // own path first, so the copy wins from the moment it is applied and the
    // template is left as it was for everything else.
    if subject.waypoints.editing_the_template() {
        let guid = subject.waypoints.subject.map(|open| open.guid);
        if let Some(guid) = guid {
            if ui
                .button("Copy path to spawn")
                .on_hover_text(
                    "Copies these points into `creature_movement` under this guid. `creature_movement_template` is not changed. The server uses a spawn's `creature_movement` rows before its template's, so only this spawn changes.",
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
        // **Two states and they read very differently.** The first draft said
        // "Not applied" whenever the project claimed the path, which stays true
        // after an apply — the store is the project's claim and outlives the
        // button — so the window went on denying a change that was in the
        // database. What separates them is the revert file: a path named there
        // is a path whose rows have been written.
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
        theme::note(
            ui,
            "This creature has no rows in creature_movement or creature_movement_template. \
             Turn on Add points and click the ground to create a path; it is written to \
             creature_movement for this spawn.",
        );
        return;
    }

    // The columns' names, which stay put while the rows scroll under them.
    let (header, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::hover());
    let painter = ui.painter();
    for (name, at, align) in [
        ("#", COLUMNS[0], egui::Align2::RIGHT_CENTER),
        ("x", COLUMNS[1], egui::Align2::RIGHT_CENTER),
        ("y", COLUMNS[2], egui::Align2::RIGHT_CENTER),
        ("z", COLUMNS[3], egui::Align2::RIGHT_CENTER),
        ("wait", COLUMNS[4], egui::Align2::CENTER_CENTER),
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
                // Selected as a list row is everywhere in the editor; hovered,
                // here or as the point under the pointer in the world, as a
                // raised fill.
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
                // **Double-click flies to it**, which is the only way to reach a
                // point of a path that runs off the edge of what is streamed.
                if row.double_clicked() {
                    subject.waypoints.fly_to = Some(bevy::prelude::Vec3::new(node.x, node.y, node.z));
                }
            }
        });
}

/// How wide the form's number boxes are, so they line up in one column.
const FIELD: f32 = 120.0;

/// The selected point's own columns, and what can be done to the point.
fn node_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &mut Path) {
    let Some(index) = subject.waypoints.selected else {
        theme::heading(ui, "Point");
        theme::note(ui, "Click a point, here or in the world, to edit it.");
        return;
    };
    let Some(node) = path.nodes.get(index).cloned() else {
        return;
    };
    theme::heading(ui, &format!("Point {} of {}", index + 1, path.nodes.len()));

    let mut edited = node.clone();
    let mut changed = false;
    let field = |ui: &mut egui::Ui, value: egui::DragValue<'_>| {
        ui.add_sized([FIELD, ui.spacing().interact_size.y], value).changed()
    };
    egui::Grid::new("waypoint-node")
        .num_columns(2)
        .spacing([10.0, 4.0])
        .show(ui, |ui| {
            for (name, value, about) in [
                (
                    "position_x",
                    &mut edited.x,
                    "the point's position, in world coordinates",
                ),
                ("position_y", &mut edited.y, ""),
                ("position_z", &mut edited.z, ""),
            ] {
                ui.label(egui::RichText::new(name).color(theme::INK_DIM))
                    .on_hover_text(about);
                changed |= field(ui, egui::DragValue::new(value).speed(0.25).fixed_decimals(2));
                ui.end_row();
            }

            ui.label(egui::RichText::new("waittime").color(theme::INK_DIM))
                .on_hover_text(
                    "How long the creature waits here, in milliseconds. The server applies \
                     the point's orientation only at a point with a waittime.",
                );
            changed |= field(ui, egui::DragValue::new(&mut edited.waittime).speed(100.0).suffix(" ms"));
            ui.end_row();

            ui.label(egui::RichText::new("wander_distance").color(theme::INK_DIM))
                .on_hover_text("How far the creature may wander from the point while it waits.");
            changed |= field(ui, egui::DragValue::new(&mut edited.wander_distance).speed(0.1).suffix(" yd"));
            ui.end_row();

            ui.label(egui::RichText::new("script_id").color(theme::INK_DIM))
                .on_hover_text(
                    "A creature_movement_scripts id, or 0. If that table has no such id, the \
                     server drops this point at load without an error message, and the path \
                     is one point shorter.",
                );
            changed |= field(ui, egui::DragValue::new(&mut edited.script_id).speed(1.0));
            ui.end_row();

            ui.label(egui::RichText::new("path_id").color(theme::INK_DIM))
                .on_hover_text("A creature_movement_special path to run here, or 0.");
            changed |= field(ui, egui::DragValue::new(&mut edited.path_id).speed(1.0));
            ui.end_row();

            // **The facing is a checkbox and a number**, because the column's
            // empty value is 100 rather than 0 and a person typing 0 into it
            // would be setting a real facing of due east. See
            // `vale_mangos::path::NO_FACING`.
            let mut faces = edited.has_orientation();
            if ui
                .checkbox(&mut faces, egui::RichText::new("orientation").color(theme::INK_DIM))
                .on_hover_text(
                    "Whether the creature faces a direction on arrival. Off writes 100, which is \
                     what the server reads as \"no facing\" — not 0, which is a real facing of \
                     due east.",
                )
                .changed()
            {
                edited.orientation = match faces {
                    true => 0.0,
                    false => path::NO_FACING,
                };
                changed = true;
            }
            if faces {
                changed |= field(
                    ui,
                    egui::DragValue::new(&mut edited.orientation)
                        .speed(0.05)
                        .range(0.0..=std::f32::consts::TAU)
                        .suffix(" rad"),
                );
            } else {
                theme::note(ui, "none");
            }
            ui.end_row();
        });
    if edited.has_orientation() && !edited.faces() {
        warning(
            ui,
            "This facing has no effect: the server applies a facing only at a point with a \
             waittime, and this point's waittime is 0.",
        );
    }

    if changed {
        if let Some(slot) = path.nodes.get_mut(index) {
            *slot = edited;
        }
        write(subject, path, "Edit waypoint");
    }

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Fly here").clicked() {
            subject.waypoints.fly_to = Some(bevy::prelude::Vec3::new(node.x, node.y, node.z));
        }
        if ui
            .button("Insert after")
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
    ui.horizontal(|ui| {
        if ui
            .add_enabled(index > 0, egui::Button::new("Earlier"))
            .on_hover_text("Moves this point one place earlier in the path.")
            .clicked()
        {
            path.move_node(index, index - 1);
            write(subject, path, "Reorder waypoints");
            subject.waypoints.selected = Some(index - 1);
        }
        if ui
            .add_enabled(index + 1 < path.nodes.len(), egui::Button::new("Later"))
            .on_hover_text("Moves this point one place later in the path.")
            .clicked()
        {
            path.move_node(index, index + 1);
            write(subject, path, "Reorder waypoints");
            subject.waypoints.selected = Some(index + 1);
        }
        if ui
            .button(egui::RichText::new("Remove point").color(theme::BAD))
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
    });
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
