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
        // To the left of the inspector and clear of the top bar, so it covers
        // neither the button that opened it nor, usually, the path itself.
        .default_pos([300.0, 120.0])
        .resizable(true)
        .frame(
            egui::Frame::default()
                .fill(theme::SHELL)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            head(ui, subject, &path);
            ui.separator();
            controls(ui, subject, &mut path);
            ui.separator();
            // **The list and the chosen point's form are two columns**, each
            // scrolling on its own, so the form is on screen whatever the list
            // is scrolled to. A path is as long as somebody makes it — 227
            // points on the longest one shipped — and stacked vertically the
            // form was reached by scrolling the point being edited out of
            // sight.
            ui.columns(2, |columns| {
                egui::ScrollArea::vertical()
                    .id_salt("waypoint-list")
                    .auto_shrink([false; 2])
                    .show(&mut columns[0], |ui| list(ui, subject, &mut path));
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
        .default_pos([300.0, 120.0])
        .resizable(true)
        .frame(
            egui::Frame::default()
                .fill(theme::SHELL)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(8)),
        )
        .show(ctx, |ui| {
            theme::note(
                ui,
                "Reading this creature's path. The spawn's own is looked for first and its \
                 template's after, which is the order the server resolves them in.",
            );
        });
    if !open {
        subject.waypoints.close();
    }
    response.map(|response| response.response.rect)
}

/// What this path is, what walks it, and whether it is walked at all.
fn head(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &Path) {
    let (which, _owner) = subject.waypoints.open.expect("the window is open");
    // **Which table answered, and what changing it changes.** A creature with
    // no path of its own walks its template's, and editing that moves every
    // spawn of the kind that has none — the same *this spawn* against *its
    // template* split the creature panel makes, and just as invisible in the
    // thing being edited.
    match which {
        Which::Spawn => {
            ui.label(egui::RichText::new(which.about()).small().color(theme::INK_DIM));
        }
        Which::Template => theme::note(
            ui,
            "This spawn has no path of its own, so it walks its template's — which is what the server does when `creature_movement` has no row for the guid. Editing these points moves EVERY spawn of this creature that has none of its own.",
        ),
    }

    // **Whether an edit here does anything**, which is the first thing to say.
    // A path on a creature whose movement type is idle or random is loaded by
    // the server and never used, and nothing else on screen would say so.
    let walk = subject.waypoints.walk();
    match walk {
        Walk::Never => theme::note(
            ui,
            "This creature's movement_type never walks a path. Editing these points is \
             allowed and will change nothing in the game until movement_type is 2 \
             (waypoint) or 3 (cyclic) — both are on the creature's own form.",
        ),
        _ => {
            ui.label(
                egui::RichText::new(format!(
                    "movement_type {} — {}",
                    subject.waypoints.movement_type,
                    walk.about()
                ))
                .small()
                .color(theme::INK_DIM),
            );
        }
    }
    // …and the generator's own floor, which is the other way a path does
    // nothing: a cyclic path of one node is refused outright.
    if path.nodes.len() < walk.needs_nodes() {
        theme::note(
            ui,
            format!(
                "{} point(s). This generator refuses a path shorter than {}, so the creature \
                 stands still.",
                path.nodes.len(),
                walk.needs_nodes()
            ),
        );
    }

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} point(s) · {:.0} yards round · {} ms waited",
                path.nodes.len(),
                path.loop_length(),
                path.total_wait()
            ))
            .color(theme::INK),
        );
    });

    if subject.waypoints.trouble.is_some() {
        let why = subject.waypoints.trouble.clone().unwrap_or_default();
        theme::note(ui, format!("The database did not answer: {why}"));
    }
}

/// The switches, and the three operations that are not about one point.
fn controls(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &mut Path) {
    ui.horizontal_wrapped(|ui| {
        // **Add is armed rather than held**, so a scripted run can build a
        // path — see the tool's module comment.
        let armed = subject.waypoints.adding;
        if ui
            .selectable_label(armed, "Add points")
            .on_hover_text(
                "While this is on, a click on the ground adds a point there — after the \
                 selected one, or at the end when none is selected. It takes the pointer, \
                 so another creature cannot be clicked until it is off.",
            )
            .clicked()
        {
            subject.waypoints.adding = !armed;
        }
        ui.checkbox(&mut subject.waypoints.follow_ground, "On the ground")
            .on_hover_text(
                "A point added or dragged takes the height of the terrain under the pointer. \
                 Turn it off for a creature that flies, or one walking on a building's floor: \
                 the terrain under a building is not the surface it stands on.",
            );
    });

    ui.horizontal_wrapped(|ui| {
        let chosen = subject.waypoints.selected;
        if ui
            .add_enabled(chosen.is_some(), egui::Button::new("Remove point"))
            .on_hover_text("Take the selected point out. The rest are renumbered.")
            .clicked()
        {
            if let Some(index) = chosen {
                path.remove(index);
                write(subject, path, "Remove waypoint");
                // Keep a selection where there is still one to have, so a run
                // of removals does not need a click between each.
                subject.waypoints.selected = match path.nodes.is_empty() {
                    true => None,
                    false => Some(index.min(path.nodes.len() - 1)),
                };
                subject.session.status = format!("removed point {}", index + 1);
            }
        }
        let can_rise = chosen.is_some_and(|index| index > 0);
        if ui
            .add_enabled(can_rise, egui::Button::new("Earlier"))
            .on_hover_text(
                "Move the selected point one place earlier in the walk. It was an up \
                 arrow and the interface's font has no glyph for one, so it drew as an \
                 empty box.",
            )
            .clicked()
        {
            if let Some(index) = chosen {
                path.move_node(index, index - 1);
                write(subject, path, "Reorder waypoints");
                subject.waypoints.selected = Some(index - 1);
            }
        }
        let can_fall = chosen.is_some_and(|index| index + 1 < path.nodes.len());
        if ui
            .add_enabled(can_fall, egui::Button::new("Later"))
            .on_hover_text("Move the selected point one place later in the walk.")
            .clicked()
        {
            if let Some(index) = chosen {
                path.move_node(index, index + 1);
                write(subject, path, "Reorder waypoints");
                subject.waypoints.selected = Some(index + 1);
            }
        }
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
                .button("Give this spawn its own path")
                .on_hover_text(
                    "Copy these points into `creature_movement` under this guid. The template is left as it is, and the server prefers a spawn's own path over its template's, so only this one creature changes.",
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
        ui.add_space(4.0);
    }

    ui.horizontal_wrapped(|ui| {
        let edited = subject.waypoints.edited(Some(subject.session));
        // **Two different undos, and the labels say which.** One gives up this
        // project's claim on the path; the other empties the path, which is an
        // edit that deletes every row.
        if ui
            .add_enabled(edited, egui::Button::new("Put this path back"))
            .on_hover_text(
                "Stop changing this path. The project gives up its claim and what is drawn \
                 goes back to what the database holds. It does not touch the database.",
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
                    label: "Put path back",
                    subject: &subject_name,
                    now: subject.now,
                }),
            );
            subject.waypoints.selected = None;
            subject.session.status = "this project no longer changes that path".to_string();
        }
        if ui
            .add_enabled(
                !path.nodes.is_empty(),
                egui::Button::new("Clear every point"),
            )
            .on_hover_text(
                "Empty the path. That is an edit: it writes a DELETE and no INSERTs, so the \
                 creature stops walking once it is applied.",
            )
            .clicked()
        {
            path.nodes.clear();
            write(subject, path, "Clear path");
            subject.waypoints.selected = None;
            subject.session.status = "path emptied — applying this removes every point".to_string();
        }
    });

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
            true => format!(
                "Applied — the database holds these points. Restart the server to see the \
                 creature walk them; there is no .reload for creature_movement. Editing \
                 further needs another Apply."
            ),
        };
        ui.label(egui::RichText::new(line).small().color(theme::WARN));
    }
}

/// Every point as a row: which it is, where it is, and what it does there.
fn list(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &mut Path) {
    if path.nodes.is_empty() {
        theme::note(
            ui,
            "This creature has no path in either movement table. Turn on Add points and click \
             the ground to build one; it will be this spawn's own, not its template's.",
        );
        return;
    }
    let chosen = subject.waypoints.selected;
    let hovered = subject.waypoints.hovered;
    for (index, node) in path.nodes.iter().enumerate() {
        // One line per point, in a column half the window wide. The wait is a
        // mark rather than a number: which points the creature stops at is
        // readable at a glance, and how long is on the form.
        let label = format!(
            "{:>3}  {:>9.1} {:>8.1} {:>7.1}{}",
            index + 1,
            node.x,
            node.y,
            node.z,
            match node.waittime {
                0 => "",
                _ => "  •",
            }
        );
        let mut text = egui::RichText::new(label).monospace();
        if hovered == Some(index) && chosen != Some(index) {
            text = text.color(theme::INK);
        }
        let row = ui.selectable_label(chosen == Some(index), text);
        if row.clicked() {
            subject.waypoints.selected = Some(index);
        }
        // **Double-click flies to it**, which is the only way to reach a point
        // of a path that runs off the edge of what is streamed.
        if row.double_clicked() {
            subject.waypoints.fly_to = Some(bevy::prelude::Vec3::new(node.x, node.y, node.z));
        }
    }
}

/// The selected point's own columns.
fn node_form(ui: &mut egui::Ui, subject: &mut Subject<'_>, path: &mut Path) {
    let Some(index) = subject.waypoints.selected else {
        theme::note(ui, "Click a point, here or in the world, to edit it.");
        return;
    };
    let Some(node) = path.nodes.get(index).cloned() else {
        return;
    };
    theme::heading(ui, &format!("Point {}", index + 1));

    let mut edited = node.clone();
    let mut changed = false;
    egui::Grid::new("waypoint-node")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, value, about) in [
                (
                    "position_x",
                    &mut edited.x,
                    "where it stands, in the world's own axes",
                ),
                ("position_y", &mut edited.y, ""),
                ("position_z", &mut edited.z, ""),
            ] {
                ui.label(egui::RichText::new(name).color(theme::INK_DIM))
                    .on_hover_text(about);
                changed |= ui.add(egui::DragValue::new(value).speed(0.25)).changed();
                ui.end_row();
            }

            ui.label(egui::RichText::new("waittime").color(theme::INK_DIM))
                .on_hover_text(
                    "How long it stands here, in milliseconds. It is also what makes a facing \
                     happen at all: the server applies an orientation only at a point the \
                     creature stops at.",
                );
            changed |= ui
                .add(egui::DragValue::new(&mut edited.waittime).speed(100.0))
                .changed();
            ui.end_row();

            ui.label(egui::RichText::new("wander_distance").color(theme::INK_DIM))
                .on_hover_text("How far it may stray from the point while it waits.");
            changed |= ui
                .add(egui::DragValue::new(&mut edited.wander_distance).speed(0.1))
                .changed();
            ui.end_row();

            ui.label(egui::RichText::new("script_id").color(theme::INK_DIM))
                .on_hover_text(
                    "A creature_movement_scripts id, or 0. An id that table does not have \
                     costs this one point at load, silently — the path comes up short and \
                     nothing says why.",
                );
            changed |= ui
                .add(egui::DragValue::new(&mut edited.script_id).speed(1.0))
                .changed();
            ui.end_row();

            ui.label(egui::RichText::new("path_id").color(theme::INK_DIM))
                .on_hover_text("A creature_movement_special path to run here, or 0.");
            changed |= ui
                .add(egui::DragValue::new(&mut edited.path_id).speed(1.0))
                .changed();
            ui.end_row();
        });

    // **The facing is a checkbox and a number**, because the column's empty
    // value is 100 rather than 0 and a person typing 0 into it would be setting
    // a real facing of due east. See `vale_mangos::path::NO_FACING`.
    ui.horizontal(|ui| {
        let mut faces = edited.has_orientation();
        if ui
            .checkbox(&mut faces, "Faces a direction on arrival")
            .on_hover_text(
                "Off writes 100, which is what the server reads as \"no facing\" — not 0, \
                 which is a real facing of due east.",
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
            changed |= ui
                .add(
                    egui::DragValue::new(&mut edited.orientation)
                        .speed(0.05)
                        .range(0.0..=std::f32::consts::TAU),
                )
                .changed();
        }
    });
    if edited.has_orientation() && !edited.faces() {
        theme::note(
            ui,
            "This facing will never happen: the server applies one only at a point with a \
             waittime, and this point has none.",
        );
    }

    if changed {
        if let Some(slot) = path.nodes.get_mut(index) {
            *slot = edited;
        }
        write(subject, path, "Edit waypoint");
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Fly here").clicked() {
            subject.waypoints.fly_to = Some(bevy::prelude::Vec3::new(node.x, node.y, node.z));
        }
        if ui
            .button("Insert after")
            .on_hover_text(
                "A new point half way to the next one, which is the cheapest way to \
                            put a corner into a straight leg.",
            )
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
