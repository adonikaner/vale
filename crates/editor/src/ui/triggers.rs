//! The area trigger tool's panel: the selected trigger's volume, and what the
//! server does when a character is inside it: the template row's label,
//! script, condition and cooldown, which are always shown, then the teleport,
//! the inn, the quest objective and the battleground entrance.
//!
//! The volume writes through `crate::tools::tables::set_fields` under a gesture
//! key, so a value dragged through forty steps is one undo entry. The server
//! half writes rows of the project's store through `crate::tools::triggers`,
//! and is drawn only once the three tables have been read. What the pointer
//! does in the world is described in `crate::tools::triggers`.

use bevy_egui::egui;

use super::theme;
use crate::session::EditSession;
use crate::tools::triggers::{self, Armed, PickFor, Triggers};
use vale_assets::tables::areatrigger::fields as tf;
use vale_edit::dbc::places::{self, Trigger};
use vale_mangos::row::Life;

/// What the panel is drawn from.
pub struct Subject<'a> {
    pub session: &'a mut EditSession,
    pub triggers: &'a mut Triggers,
    pub assets: &'a vale_client::assets::GameAssets,
    pub now: f64,
    /// The top bar's Server… popover, which holds Apply and Put back.
    pub server_panel: &'a mut super::popover::Popover,
    /// The script window's state, which a template's script id opens.
    pub behaviour: &'a mut crate::tools::behaviour::Behaviour,
}

pub fn draw(ui: &mut egui::Ui, subject: Subject<'_>) {
    let Subject {
        session,
        triggers,
        assets,
        now,
        server_panel,
        behaviour,
    } = subject;
    if !session.open_table(assets, places::TRIGGERS) {
        theme::note(ui, "opening AreaTrigger.dbc\u{2026}");
        return;
    }
    if pick_banner(ui, session, triggers) {
        return;
    }

    summary(ui, triggers);
    ui.add_space(4.0);
    controls(ui, triggers);

    egui::ScrollArea::vertical()
        .id_salt("triggers")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            match triggers.selected.and_then(|id| triggers.trigger(id).copied()) {
                Some(trigger) => {
                    ui.add_space(4.0);
                    theme::heading(ui, &format!("Trigger {}", trigger.id));
                    volume(ui, session, triggers, &trigger, now);
                    ui.add_space(6.0);
                    theme::heading(ui, "Server rows");
                    server_half(ui, session, triggers, behaviour, &trigger, now);
                }
                None => theme::note(
                    ui,
                    "Click a trigger in the world. Each is coloured by what the server does \
                     with it: purple a teleport, green an inn, gold a quest objective, red a \
                     battleground entrance, pink a script, blue nothing the editor has read.",
                ),
            }
            ui.add_space(6.0);
            theme::heading(ui, "Server");
            if ui
                .button("Server\u{2026}")
                .on_hover_text(
                    "Opens the Server panel. Client tables writes each changed trigger as an \
                     areatrigger_template row; Area triggers writes what \
                     the triggers do.",
                )
                .clicked()
            {
                server_panel.show();
            }
            theme::note(
                ui,
                "The client reads AreaTrigger.dbc from the project on the next playtest. The \
                 server reads areatrigger_template at startup only, so a new or moved trigger \
                 needs a restart; teleports, inns and quest triggers are live on a reload.",
            );
            ui.add_space(12.0);
        });
}

/// The count, and the warning for a file whose maps are split.
fn summary(ui: &mut egui::Ui, triggers: &Triggers) {
    theme::note(ui, format!("{} triggers on this map", triggers.list.len()));
    if !triggers.blocks.is_empty() {
        let maps: Vec<String> = triggers.blocks.iter().map(u32::to_string).collect();
        ui.label(
            egui::RichText::new(format!(
                "The rows of map(s) {} are not one block of AreaTrigger.dbc. The client reads \
                 only the first block of the current map, so the rows past it never fire.",
                maps.join(", ")
            ))
            .size(theme::SMALL)
            .color(theme::WARN),
        );
    }
}

/// What a click on empty ground does, and the size of a new trigger.
fn controls(ui: &mut egui::Ui, triggers: &mut Triggers) {
    ui.horizontal_wrapped(|ui| {
        let armed = triggers.armed == Armed::NewTrigger;
        if ui
            .selectable_label(armed, "New trigger")
            .on_hover_text(
                "Armed, a click on the ground makes a sphere trigger there, resting on \
                 the ground. It is added after this map's last row, because the client \
                 reads only the first block of each map's rows.",
            )
            .clicked()
        {
            triggers.armed = match armed {
                true => Armed::Nothing,
                false => Armed::NewTrigger,
            };
        }
    });
    theme::row(ui, "new radius", |ui| {
        ui.add(egui::DragValue::new(&mut triggers.radius).range(0.5..=200.0).speed(0.25).suffix(" yd"))
            .on_hover_text("Radius of a new trigger.");
    });
    if triggers.armed != Armed::Nothing {
        theme::note(ui, "Escape disarms.");
    }
}

/// The selected trigger's shape, size and place.
fn volume(ui: &mut egui::Ui, session: &mut EditSession, triggers: &mut Triggers, trigger: &Trigger, now: f64) {
    let mut sphere = trigger.is_sphere();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("shape").color(theme::INK_FAINT));
        let a = ui.radio_value(&mut sphere, true, "Sphere");
        let b = ui.radio_value(&mut sphere, false, "Box");
        if a.changed() || b.changed() {
            triggers::set_sphere(session, trigger, sphere, now);
            triggers.stale();
        }
    });
    let mut fields: Vec<(usize, u32)> = Vec::new();
    match trigger.is_sphere() {
        true => {
            let mut radius = trigger.radius;
            theme::row(ui, "radius", |ui| {
                if ui
                    .add(egui::DragValue::new(&mut radius).range(0.1..=500.0).speed(0.1).suffix(" yd"))
                    .changed()
                {
                    fields.push((tf::RADIUS, radius.to_bits()));
                }
            });
        }
        false => {
            let mut extent = trigger.extent;
            for (axis, (label, field)) in [("length", tf::BOX_LENGTH), ("width", tf::BOX_WIDTH), ("height", tf::BOX_HEIGHT)]
                .into_iter()
                .enumerate()
            {
                theme::row(ui, label, |ui| {
                    if ui
                        .add(egui::DragValue::new(&mut extent[axis]).range(0.1..=1000.0).speed(0.1).suffix(" yd"))
                        .on_hover_text("Full side length, not the half-extent.")
                        .changed()
                    {
                        fields.push((field, extent[axis].to_bits()));
                    }
                });
            }
            let mut degrees = trigger.yaw.to_degrees();
            theme::row(ui, "yaw", |ui| {
                if ui
                    .add(egui::DragValue::new(&mut degrees).speed(0.5).suffix("\u{b0}"))
                    .on_hover_text("Rotation of the box about the vertical axis. AreaTrigger.dbc stores it in radians.")
                    .changed()
                {
                    fields.push((tf::BOX_YAW, degrees.rem_euclid(360.0).to_radians().to_bits()));
                }
            });
        }
    }
    let mut at = trigger.at;
    for (axis, (label, field)) in [("x", tf::X), ("y", tf::Y), ("z", tf::Z)].into_iter().enumerate() {
        theme::row(ui, label, |ui| {
            if ui.add(egui::DragValue::new(&mut at[axis]).speed(0.25).suffix(" yd")).changed() {
                fields.push((field, at[axis].to_bits()));
            }
        });
    }
    if !fields.is_empty() {
        triggers::set_volume(session, trigger, &fields, "Edit area trigger", now);
        triggers.stale();
    }
    ui.horizontal(|ui| {
        if ui.button("Fly to").clicked() {
            triggers.fly_to(trigger.at);
        }
        if ui
            .button("Snap to ground")
            .on_hover_text("Places the trigger on the ground under its centre, where the ground is open.")
            .clicked()
        {
            if let Some(ground) = crate::tools::doodads::ground_height(session, at[0], at[1]) {
                let lift = match trigger.is_sphere() {
                    true => trigger.radius * 0.5,
                    false => trigger.extent[2] * 0.5,
                };
                let z = ground + lift;
                triggers::set_volume(session, trigger, &[(tf::Z, z.to_bits())], "Edit area trigger", now);
                triggers.stale();
            }
        }
        let shipped = triggers.is_shipped(trigger.id);
        let remove = ui.button(match shipped {
            true => "Remove\u{2026}",
            false => "Remove",
        });
        if remove
            .on_hover_text(
                "Removes the row from AreaTrigger.dbc, and the teleport, inn and quest rows \
                 that name it from the project's server rows.",
            )
            .clicked()
        {
            match shipped {
                true => triggers.confirm_remove = Some(trigger.id),
                false => {
                    let line = triggers::remove_trigger(session, triggers, trigger.id, now);
                    session.status = line;
                }
            }
        }
    });
    if triggers.confirm_remove == Some(trigger.id) {
        ui.label(
            egui::RichText::new(
                "This is a shipped trigger. Removing it also stops the server behaviour \
                 attached to it (dungeon entrance, inn, quest objective).",
            )
            .size(theme::SMALL)
            .color(theme::WARN),
        );
        ui.horizontal(|ui| {
            if ui.button("Remove trigger").clicked() {
                let line = triggers::remove_trigger(session, triggers, trigger.id, now);
                session.status = line;
            }
            if ui.button("Cancel").clicked() {
                triggers.confirm_remove = None;
            }
        });
    }
}

/// While a place is being picked, what the pointer will do and how to stop.
/// Answers whether the rest of the panel is left out, which it is while the
/// pick is on another map than the trigger's.
fn pick_banner(ui: &mut egui::Ui, session: &mut EditSession, triggers: &mut Triggers) -> bool {
    let Some(pick) = triggers.pick.clone() else {
        return false;
    };
    let map = session
        .maps
        .iter()
        .find(|(id, _)| *id == pick.map)
        .map_or_else(|| format!("map {}", pick.map), |(_, name)| name.clone());
    ui.label(
        egui::RichText::new(format!("Picking trigger {}'s {} on {map}", pick.trigger, pick.what.noun()))
            .strong()
            .color(theme::INK),
    );
    theme::note(
        ui,
        "Press on the ground to set the position, drag before releasing to set the \
         facing, and release to write both. Escape cancels.",
    );
    theme::row(ui, "facing", |ui| {
        ui.label(theme::number(format!("{:.0}\u{b0}", pick.facing.to_degrees().rem_euclid(360.0))));
    });
    if ui.button("Cancel").clicked() {
        triggers.end_pick();
        session.status = "pick cancelled".to_string();
    }
    ui.add_space(6.0);
    pick.home.is_some()
}

/// The buttons that pick a place on `map` for one of the trigger's rows: on
/// this map, or on the row's map, which the editor opens for the pick.
fn pick_buttons(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    triggers: &mut Triggers,
    what: PickFor,
    trigger: u32,
    map: u32,
    at: [f32; 3],
    facing: f32,
) {
    ui.horizontal(|ui| {
        let here = map == session.map_id;
        let name = session
            .maps
            .iter()
            .find(|(id, _)| *id == map)
            .map_or_else(|| format!("map {map}"), |(_, name)| name.clone());
        let label = match here {
            true => "Pick on this map".to_string(),
            false => format!("Pick on {name}\u{2026}"),
        };
        let tip = match here {
            true => "Press on the ground to set the position and drag to set the facing.".to_string(),
            false => format!(
                "Opens {name} with the camera over the row's current position. Press on the \
                 ground to set the position and drag to set the facing; the editor then \
                 returns to this trigger. Unsaved tiles are saved first, as on any map change."
            ),
        };
        if ui.button(label).on_hover_text(tip).clicked() {
            if let Err(why) = triggers.start_pick(session, what, trigger, map, at, facing) {
                session.status = why;
            }
        }
        if ui
            .add_enabled(here, egui::Button::new("Fly to"))
            .on_disabled_hover_text("The place is on another map.")
            .clicked()
        {
            triggers.fly_to(at);
        }
    });
}

/// A map, chosen from the session's list.
fn map_choice(ui: &mut egui::Ui, session: &EditSession, salt: (&str, u32), map: &mut u32) {
    let name = |map: u32| {
        session
            .maps
            .iter()
            .find(|(known, _)| *known == map)
            .map_or_else(|| format!("map {map}"), |(_, name)| name.clone())
    };
    egui::ComboBox::from_id_salt(salt)
        .selected_text(format!("{map} {}", name(*map)))
        .height(320.0)
        .show_ui(ui, |ui| {
            for (known, known_name) in &session.maps {
                ui.selectable_value(map, *known, format!("{known:>4}  {known_name}"));
            }
        });
}

/// What the server does with the trigger.
fn server_half(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    triggers: &mut Triggers,
    behaviour: &mut crate::tools::behaviour::Behaviour,
    trigger: &Trigger,
    now: f64,
) {
    if let Some(trouble) = &triggers.trouble {
        theme::note(ui, format!("The server's rows could not be read: {trouble}"));
        return;
    }
    if triggers.held.is_none() {
        theme::note(ui, "reading the five areatrigger tables\u{2026}");
        return;
    }
    let id = trigger.id;
    if !vale_mangos::trigger::fits(id) {
        ui.label(
            egui::RichText::new(format!(
                "areatrigger_template.id holds at most {}; this trigger's id does not fit.",
                vale_mangos::trigger::MAX_ID
            ))
            .color(theme::WARN),
        );
        return;
    }
    // What Apply would skip for this trigger, with the reason, since a skipped
    // row is otherwise only a line on the Server panel.
    let mine = [format!("id={id};"), format!("id={id} ")];
    let refused: Vec<String> = crate::server::places::plan(session, crate::server::places::Group::Triggers)
        .refused
        .into_iter()
        .filter(|line| mine.iter().any(|key| line.contains(key.as_str())))
        .collect();
    for line in refused {
        ui.label(egui::RichText::new(format!("Apply skips this: {line}")).size(theme::SMALL).color(theme::WARN));
    }
    template(ui, session, triggers, behaviour, trigger, now);
    ui.add_space(6.0);
    teleport(ui, session, triggers, trigger, now);
    ui.add_space(4.0);

    let edits = session.server_edits.clone();
    let (inn, _) = triggers.inn(&edits, id).unwrap_or((false, false));
    let mut ticked = inn;
    if ui
        .checkbox(&mut ticked, "Inn")
        .on_hover_text("areatrigger_tavern: a character inside rests, and can make the inn its home.")
        .changed()
    {
        triggers::set_inn(session, triggers, id, ticked, now);
    }

    let quest = triggers.quest(&edits, id).filter(|(_, _, life)| *life != Life::Delete);
    let mut number = quest.map_or(0, |(quest, _, _)| quest);
    theme::row(ui, "quest", |ui| {
        if ui
            .add(egui::DragValue::new(&mut number).speed(1.0))
            .on_hover_text(
                "areatrigger_involvedrelation: standing inside completes this quest's \
                 exploration objective. 0 for none.",
            )
            .changed()
        {
            triggers::set_quest(session, triggers, id, number, now);
        }
    });
    ui.add_space(4.0);
    entrance(ui, session, triggers, trigger, now);
}

/// The template row's five server columns: the label, the script and what
/// gates it. Always shown, since for a trigger with no other rows these
/// columns are the only record of what the server does with it.
fn template(
    ui: &mut egui::Ui,
    session: &mut EditSession,
    triggers: &mut Triggers,
    behaviour: &mut crate::tools::behaviour::Behaviour,
    trigger: &Trigger,
    now: f64,
) {
    let id = trigger.id;
    let edits = session.server_edits.clone();
    let (template, in_database) = triggers
        .template(&edits, id)
        .unwrap_or((vale_mangos::trigger::Template { id, ..Default::default() }, false));
    let text = vale_mangos::sql::text;
    let mut name = template.name.clone();
    theme::row(ui, "label", |ui| {
        if ui
            .text_edit_singleline(&mut name)
            .on_hover_text("areatrigger_template.name: a description for people reading the table. The game does not show it.")
            .changed()
        {
            triggers::set_template(session, triggers, id, "name", text(&name), now);
        }
    });
    if !in_database {
        theme::note(
            ui,
            "The database has no areatrigger_template row for this trigger yet. Client tables \
             on the Server panel makes it from AreaTrigger.dbc; apply that before these \
             columns, which are written onto that row.",
        );
    } else if template.build != vale_mangos::trigger::BUILD {
        theme::note(
            ui,
            format!(
                "The server uses the row at build {}. An edit here writes a copy of it at build \
                 {}, which the server then uses instead.",
                template.build,
                vale_mangos::trigger::BUILD
            ),
        );
    }

    let mut script_name = template.script_name.clone();
    let suggestions = triggers.script_names();
    theme::row(ui, "C++ script", |ui| {
        let typed = ui
            .text_edit_singleline(&mut script_name)
            .on_hover_text(
                "areatrigger_template.script_name: a script the server's C++ registers under this \
                 name. It runs instead of the script id. Empty for none.",
            )
            .changed();
        let mut chosen = None;
        egui::ComboBox::from_id_salt(("trigger script names", id))
            .selected_text("")
            .width(18.0)
            .show_ui(ui, |ui| {
                ui.label(
                    egui::RichText::new("script names used by other triggers")
                        .size(theme::SMALL)
                        .color(theme::INK_FAINT),
                );
                for known in &suggestions {
                    if ui.selectable_label(*known == script_name, known).clicked() {
                        chosen = Some(known.clone());
                    }
                }
            });
        if let Some(known) = chosen {
            script_name = known;
        }
        if typed || script_name != template.script_name {
            triggers::set_template(session, triggers, id, "script_name", text(script_name.trim()), now);
        }
    });

    let mut script_id = template.script_id;
    theme::row(ui, "script id", |ui| {
        if ui
            .add(egui::DragValue::new(&mut script_id).speed(1.0))
            .on_hover_text("areatrigger_template.script_id: the areatrigger_scripts rows to run, or 0.")
            .changed()
        {
            triggers::set_template(session, triggers, id, "script_id", script_id.to_string(), now);
        }
        let table = vale_mangos::scripts::AREATRIGGER;
        match script_id {
            0 => {
                if ui
                    .button("Create script")
                    .on_hover_text(
                        "Sets the script id to the trigger's own id, the shipped convention, and \
                         opens the script window on it to add steps.",
                    )
                    .clicked()
                {
                    triggers::set_template(session, triggers, id, "script_id", id.to_string(), now);
                    behaviour.open_script(table, id);
                }
            }
            script => {
                if ui.button("Open script").on_hover_text("Opens areatrigger_scripts in the script window.").clicked() {
                    behaviour.open_script(table, script);
                }
            }
        }
    });

    let mut condition = template.condition_id;
    let column = format!("areatrigger_template {id} condition_id");
    theme::row(ui, "condition", |ui| {
        let typed = ui
            .add(egui::DragValue::new(&mut condition).speed(1.0))
            .on_hover_text("areatrigger_template.condition_id: a row of `conditions` the character must meet for the script to run, or 0.")
            .changed();
        let answered = super::conditions::cell(ui, egui::Id::new(("trigger-condition", id)), &column, template.condition_id, None);
        if let Some(condition) = answered.or(typed.then_some(condition)).filter(|condition| *condition != template.condition_id) {
            triggers::set_template(session, triggers, id, "condition_id", condition.to_string(), now);
        }
    });
    let mut cooldown = template.cooldown;
    theme::row(ui, "cooldown", |ui| {
        if ui
            .add(egui::DragValue::new(&mut cooldown).speed(1.0).suffix(" s"))
            .on_hover_text("areatrigger_template.cooldown: seconds before the script can run again on this map instance, or 0.")
            .changed()
        {
            triggers::set_template(session, triggers, id, "cooldown", cooldown.to_string(), now);
        }
    });
    let runs = match (template.script_name.is_empty(), template.script_id) {
        (false, _) => format!("Runs the C++ script {}.", template.script_name),
        (true, 0) => "Runs no script.".to_string(),
        (true, script) => format!("Runs areatrigger_scripts {script}."),
    };
    theme::note(ui, format!("{runs} The template is read at server start, so a change needs a restart."));
}

/// The battleground entrance block: off, or the battleground, the side and
/// where a character leaving it returns to.
fn entrance(ui: &mut egui::Ui, session: &mut EditSession, triggers: &mut Triggers, trigger: &Trigger, now: f64) {
    use vale_mangos::trigger::TEAMS;
    let id = trigger.id;
    let edits = session.server_edits.clone();
    let live = triggers
        .entrance(&edits, id)
        .filter(|(_, _, life)| *life != Life::Delete)
        .map(|(entrance, _, _)| entrance);
    let mut on = live.is_some();
    if ui
        .checkbox(&mut on, "Battleground entrance")
        .on_hover_text(
            "areatrigger_bg_entrance: the trigger opens a battleground's list for one side, and \
             says where a character leaving the battleground returns to. Read at server start.",
        )
        .changed()
    {
        match on {
            true => triggers::add_entrance(session, triggers, id, triggers.map, trigger.at, now),
            false => triggers::remove_entrance(session, triggers, id, now),
        }
        return;
    }
    let Some(entrance) = live else { return };
    let f = vale_mangos::sql::float;
    let mut team = entrance.team;
    theme::row(ui, "side", |ui| {
        let named = TEAMS.iter().find(|known| known.value == team).map_or_else(|| team.to_string(), |known| known.name.to_string());
        egui::ComboBox::from_id_salt(("entrance side", id)).selected_text(named).show_ui(ui, |ui| {
            for known in TEAMS {
                ui.selectable_value(&mut team, known.value, known.name);
            }
        });
    });
    if team != entrance.team {
        triggers::set_entrance(session, triggers, id, "team", team.to_string(), now);
    }
    let mut battleground = entrance.bg_template;
    theme::row(ui, "battleground", |ui| {
        if ui
            .add(egui::DragValue::new(&mut battleground).speed(0.1))
            .on_hover_text("A battleground_template id: the battleground whose list opens.")
            .changed()
        {
            triggers::set_entrance(session, triggers, id, "bg_template", battleground.to_string(), now);
        }
    });
    let mut exit_map = entrance.exit_map;
    theme::row(ui, "exit map", |ui| map_choice(ui, session, ("entrance exit map", id), &mut exit_map));
    if exit_map != entrance.exit_map {
        triggers::set_entrance(session, triggers, id, "exit_map", exit_map.to_string(), now);
    }
    let mut exit = entrance.exit;
    for (axis, (label, column)) in [("x", "exit_position_x"), ("y", "exit_position_y"), ("z", "exit_position_z")]
        .into_iter()
        .enumerate()
    {
        theme::row(ui, label, |ui| {
            if ui.add(egui::DragValue::new(&mut exit[axis]).speed(0.25).suffix(" yd")).changed() {
                triggers::set_entrance(session, triggers, id, column, f(exit[axis]), now);
            }
        });
    }
    let mut degrees = entrance.orientation.to_degrees();
    theme::row(ui, "facing", |ui| {
        if ui.add(egui::DragValue::new(&mut degrees).speed(1.0).suffix("\u{b0}")).changed() {
            triggers::set_entrance(session, triggers, id, "exit_orientation", f(degrees.rem_euclid(360.0).to_radians()), now);
        }
    });
    let mut name = entrance.name.clone();
    theme::row(ui, "label", |ui| {
        if ui.text_edit_singleline(&mut name).changed() {
            triggers::set_entrance(session, triggers, id, "name", vale_mangos::sql::text(&name), now);
        }
    });
    for fault in entrance.check() {
        ui.label(egui::RichText::new(fault).size(theme::SMALL).color(theme::WARN));
    }
    pick_buttons(ui, session, triggers, PickFor::BgExit, id, entrance.exit_map, entrance.exit, entrance.orientation);
}

/// The teleport block: off, or the target and its requirements.
fn teleport(ui: &mut egui::Ui, session: &mut EditSession, triggers: &mut Triggers, trigger: &Trigger, now: f64) {
    let id = trigger.id;
    let edits = session.server_edits.clone();
    let shown = triggers.teleport(&edits, id);
    let live = shown.as_ref().filter(|(_, _, life)| *life != Life::Delete);
    let mut on = live.is_some();
    if ui
        .checkbox(&mut on, "Teleport")
        .on_hover_text(
            "areatrigger_teleport: the server sends a character inside to a map and a \
             position. When first enabled, the target is the trigger's own centre until \
             a target is picked.",
        )
        .changed()
    {
        match on {
            true => triggers::add_teleport(session, triggers, id, triggers.map, trigger.at, now),
            false => triggers::remove_teleport(session, triggers, id, now),
        }
        return;
    }
    let Some((teleport, _, _)) = live.cloned() else {
        return;
    };
    let lacking = triggers::missing(
        &session.server_edits,
        vale_mangos::trigger::TELEPORT,
        &vale_mangos::trigger::teleport_key(id, teleport.patch),
    );
    if !lacking.is_empty() {
        ui.label(
            egui::RichText::new(format!(
                "This teleport has no {}, so Apply skips it. Enter the value, or pick the target \
                 again to write all of them.",
                lacking.join(", ")
            ))
            .size(theme::SMALL)
            .color(theme::WARN),
        );
    }
    let f = vale_mangos::sql::float;
    let mut target_map = teleport.target_map;
    theme::row(ui, "target map", |ui| map_choice(ui, session, ("trigger target map", id), &mut target_map));
    if target_map != teleport.target_map {
        triggers::set_teleport(session, triggers, id, "target_map", target_map.to_string(), now);
    }
    let mut target = teleport.target;
    for (axis, (label, column)) in [
        ("x", "target_position_x"),
        ("y", "target_position_y"),
        ("z", "target_position_z"),
    ]
    .into_iter()
    .enumerate()
    {
        theme::row(ui, label, |ui| {
            if ui.add(egui::DragValue::new(&mut target[axis]).speed(0.25).suffix(" yd")).changed() {
                triggers::set_teleport(session, triggers, id, column, f(target[axis]), now);
            }
        });
    }
    let mut degrees = teleport.orientation.to_degrees();
    theme::row(ui, "facing", |ui| {
        if ui
            .add(egui::DragValue::new(&mut degrees).speed(1.0).suffix("\u{b0}"))
            .on_hover_text("The character's facing on arrival. The row stores it in radians.")
            .changed()
        {
            let radians = degrees.rem_euclid(360.0).to_radians();
            triggers::set_teleport(session, triggers, id, "target_orientation", f(radians), now);
        }
    });
    let mut level = teleport.required_level;
    theme::row(ui, "level", |ui| {
        if ui
            .add(egui::DragValue::new(&mut level).range(0..=60).speed(0.2))
            .on_hover_text("Minimum character level for the teleport. 0 for any.")
            .changed()
        {
            triggers::set_teleport(session, triggers, id, "required_level", level.to_string(), now);
        }
    });
    let mut condition = teleport.required_condition;
    let column = format!("areatrigger_teleport {id} required_condition");
    theme::row(ui, "condition", |ui| {
        let typed = ui
            .add(egui::DragValue::new(&mut condition).speed(1.0))
            .on_hover_text("required_condition: a row of `conditions` the character must meet to be sent, or 0.")
            .changed();
        let answered = super::conditions::cell(ui, egui::Id::new(("teleport-condition", id)), &column, teleport.required_condition, None);
        if let Some(condition) = answered.or(typed.then_some(condition)).filter(|condition| *condition != teleport.required_condition) {
            triggers::set_teleport(session, triggers, id, "required_condition", condition.to_string(), now);
        }
    });
    let mut message = teleport.message.clone();
    theme::row(ui, "message", |ui| {
        if ui
            .text_edit_singleline(&mut message)
            .on_hover_text("Message shown to a character who does not meet the requirements.")
            .changed()
        {
            triggers::set_teleport(session, triggers, id, "message", vale_mangos::sql::text(&message), now);
        }
    });
    let mut name = teleport.name.clone();
    theme::row(ui, "label", |ui| {
        if ui
            .text_edit_singleline(&mut name)
            .on_hover_text("The row's name column, for people reading the table. The game does not show it.")
            .changed()
        {
            triggers::set_teleport(session, triggers, id, "name", vale_mangos::sql::text(&name), now);
        }
    });
    for fault in teleport.check() {
        ui.label(egui::RichText::new(fault).size(theme::SMALL).color(theme::WARN));
    }
    if !session.maps.iter().any(|(known, _)| *known == teleport.target_map) {
        ui.label(
            egui::RichText::new(format!(
                "Map {} is not in Map.dbc, and the server skips a teleport to a map it has no \
                 map_template row for.",
                teleport.target_map
            ))
            .size(theme::SMALL)
            .color(theme::WARN),
        );
    }
    pick_buttons(
        ui,
        session,
        triggers,
        PickFor::Teleport,
        id,
        teleport.target_map,
        teleport.target,
        teleport.orientation,
    );
}
