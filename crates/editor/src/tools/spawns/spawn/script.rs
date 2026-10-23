//! The command-line flags of a spawn tool: select a spawn and fly to it
//! (`--spawn`, `--object`), place and remove one (`--spawn-add`,
//! `--spawn-delete`, `--pick` and the `--object-` spellings), make and
//! renumber a template (`--template-new`, `--template-entry`, `--object-new`,
//! `--object-entry`), and open the display picker (`--pick-display`).
//!
//! A scripted run cannot otherwise make any of these gestures: selecting is a
//! click on a spawn in the viewport, placing is a click on the ground with the
//! placer armed, removing is a press on a button under a selection, creating
//! is a press on the picker and renumbering is typing into the window's entry
//! box. Without them nothing downstream of each (the store, the undo, the SQL,
//! the apply, the revert) could be checked with nobody at the keyboard. Each
//! subject spells its flags in [`super::Flags`].

use super::{SpawnSubject, SpawnTool, Spawned};
use crate::subjects::listing::Listed;
use crate::session::EditSession;
use crate::tools::Tool;
use bevy::prelude::*;
use vale_client::render::focus::WorldFocus;

/// `--spawn <guid>` or `--object <guid>`: select one spawn and fly to it, once
/// the map has been read.
///
/// Runs once, on the first frame the map's spawns are available. Running
/// repeatedly would move the selection back from whatever the user had
/// clicked.
pub(super) fn on_the_command_line<S: SpawnSubject>(
    args: Res<crate::args::Args>,
    mut tool: ResMut<SpawnTool<S>>,
    mut waypoints: ResMut<crate::tools::spawns::waypoints::Waypoints>,
    mut quests: ResMut<crate::subjects::quests::Quests>,
    mut session: Option<ResMut<EditSession>>,
    mut fly: MessageWriter<crate::camera::FlyTo>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let flag = S::FLAGS.open;
    let Some(guid) = S::script(&args).open else {
        *done = true;
        return;
    };
    // Nothing to choose from yet: the read is still in flight, or there is no
    // database and `trouble` already reports it.
    if tool.spawns.is_empty() {
        if tool.trouble.is_some() {
            *done = true;
        }
        return;
    }
    *done = true;
    let Some(spawn) = tool.spawns.iter().find(|spawn| spawn.guid() == guid).cloned() else {
        warn!("{flag}: no {} with guid {guid} on this map", S::WORDS.noun);
        if let Some(session) = session.as_mut() {
            session.status = format!("no {} spawn {guid} on this map", S::WORDS.noun);
        }
        return;
    };
    let at = spawn.at();
    info!("{flag}: {} at {:.1}, {:.1}, {:.1}", spawn.label(), at.x, at.y, at.z);
    tool.select_only(Some(guid));
    S::opened(&args, &spawn, &mut waypoints);
    // The template window, which is otherwise opened by a button. See
    // [`crate::args::spawns::Spawns::template`].
    tool.template_window = args.spawns.template;
    // The quest window beside it. See
    // [`crate::args::subjects::Subjects::creature_quests`].
    if args.subjects.creature_quests {
        quests.window_for = Some((S::HOLDER, spawn.entry(), spawn.label()));
    }
    fly.write(crate::camera::FlyTo::at(at));
    if let Some(session) = session.as_mut() {
        session.status = format!("{} — guid {guid}", spawn.label());
    }
}

/// `--spawn-add <entry>`, `--spawn-delete <guid>` and `--pick <entry>`, or the
/// `--object-` spellings: add a spawn to the map, remove one, or choose an
/// entry in the picker and place nothing.
///
/// The placement goes through the picker rather than around it, so the same
/// run checks the search: the entry is typed into the box, the match is taken
/// from the results, and the placement is the ordinary one.
///
/// It waits for three things; without any one of them the placement fails or
/// lands in the wrong place:
///
/// * the map read, because the guid comes from the highest the table holds;
/// * the search, because the template has to be known before it can be drawn
///   or placed;
/// * the tile under the camera, because a placement takes the ground's height
///   and a tile that is not open returns none, which leaves the spawn buried
///   in the slope it was meant to stand on.
pub(super) fn placed_on_the_command_line<S: SpawnSubject>(
    args: Res<crate::args::Args>,
    mut tool: ResMut<SpawnTool<S>>,
    mut session: Option<ResMut<EditSession>>,
    focus: Res<WorldFocus>,
    time: Res<Time>,
    mut fly: MessageWriter<crate::camera::FlyTo>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let script = S::script(&args);
    let flags = S::FLAGS;
    if script.add.is_none() && script.delete.is_none() && script.pick.is_none() {
        *done = true;
        tool.scripted_done = true;
        return;
    }
    // The map has to have been read: a removal needs the spawn and a placement
    // needs the highest guid the table holds.
    if tool.reading() || tool.loaded.is_none() {
        return;
    }
    if tool.trouble.is_some() {
        *done = true;
        tool.scripted_done = true;
        warn!("{}/{}: there is no database to work against", flags.add, flags.delete);
        return;
    }
    let Some(session) = session.as_mut() else { return };
    let now = time.elapsed_secs_f64();

    // Once: the system runs again on each frame it waits for the search, and
    // the removal leaves the spawn selected.
    if let Some(guid) = script.delete {
        if tool.selected != Some(guid) {
            match tool.spawns.iter().any(|spawn| spawn.guid() == guid) {
                true => {
                    SpawnTool::<S>::remove(session, guid, now);
                    tool.select_only(Some(guid));
                    info!("{}: spawn {guid} is marked for removal", flags.delete);
                }
                false => warn!("{}: no {} with guid {guid} on this map", flags.delete, S::WORDS.noun),
            }
        }
    }

    // The pick is the placement without the placement; see
    // [`crate::args::spawns::Spawns::pick`]. It takes the same steps up to
    // choosing the template and then stops, which is the state a screenshot of
    // the picker needs.
    let Some(entry) = script.add.or(script.pick) else {
        *done = true;
        tool.scripted_done = true;
        return;
    };
    // Through the picker, so the search is checked by the same run.
    if tool.new_spawn.search.is_empty() {
        tool.mode = crate::tools::placements::place::Mode::Place;
        tool.new_spawn.search = entry.to_string();
        return;
    }
    if tool.new_spawn.searching() || tool.new_spawn.ran.is_none() {
        return;
    }
    let Some(known) = tool.new_spawn.matches.iter().find(|known| known.entry() == entry).cloned() else {
        *done = true;
        tool.scripted_done = true;
        warn!("{}: no {} template with entry {entry}", flags.add, S::WORDS.noun);
        return;
    };
    tool.new_spawn.chosen = Some(known.clone());
    if script.add.is_none() {
        // Chosen, armed, and nothing placed.
        *done = true;
        tool.scripted_done = true;
        info!("{}: {} is on the cursor", flags.pick, known.label());
        return;
    }
    // The ground at the camera's focus. A scripted run has no pointer, so
    // there is no surface under one to use.
    if !focus.present {
        return;
    }
    let tile = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    let Some(ground) = session
        .tiles
        .get(&tile)
        .and_then(|open| vale_edit::adt::heights::height_at(open, focus.position.x, focus.position.y))
    else {
        return;
    };
    *done = true;
    let at = Vec3::new(focus.position.x, focus.position.y, ground);
    let guid = tool.create(session, &known, at, 0.0, now);
    // Back to Select. A click does not do this, but a script must: nothing
    // presses `Escape` afterwards, and a run that ended armed would show a
    // ghost in its screenshot standing over the spawn it had just placed.
    tool.mode = crate::tools::placements::place::Mode::Select;
    tool.new_spawn.chosen = None;
    tool.scripted_done = true;
    fly.write(crate::camera::FlyTo::at(at));
    info!("{}: {} placed as guid {guid} at {:.2}, {:.2}, {:.2}", flags.add, known.label(), at.x, at.y, at.z);
    session.status = format!("{} placed as guid {guid}", known.label());
}

/// `--template-new` or `--object-new`: make a new template, choose it in the
/// picker and open the template window on it. `--template-entry <n>` or
/// `--object-entry <n>`: renumber the template the window is about, which is
/// the new one's or the selected spawn's.
///
/// It waits for the map read, because a new entry is numbered from the highest
/// the table holds. The renumber waits for a subject: the selecting flag opens
/// its spawn on the frame the map lands, and the new template is chosen here,
/// so the retry is at most a frame.
pub(super) fn template_on_the_command_line<S: SpawnSubject>(
    args: Res<crate::args::Args>,
    mut tool: ResMut<SpawnTool<S>>,
    mut session: Option<ResMut<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    time: Res<Time>,
    mut done: Local<bool>,
    mut made: Local<bool>,
) {
    if *done {
        return;
    }
    let script = S::script(&args);
    let flags = S::FLAGS;
    if !script.new && script.entry.is_none() {
        *done = true;
        tool.scripted_template_done = true;
        return;
    }
    if tool.reading() || tool.loaded.is_none() {
        return;
    }
    if tool.trouble.is_some() {
        *done = true;
        tool.scripted_template_done = true;
        warn!("{}/{}: there is no database to work against", flags.new, flags.entry);
        return;
    }
    let Some(session) = session.as_mut() else { return };
    let now = time.elapsed_secs_f64();
    if script.new && !*made {
        *made = true;
        let patch = crate::server::settings::server_patch(&settings);
        let entry = tool.create_template(session, S::WORDS.new_name, patch, now);
        info!("{}: {} {entry} created at patch {patch}", flags.new, S::TEMPLATE);
        session.status = format!("{} {entry} created", S::TEMPLATE);
    }
    if let Some(to) = script.entry {
        if !script.new && script.open.is_none() {
            *done = true;
            tool.scripted_template_done = true;
            warn!("{}: nothing names a template; give {} or {}", flags.entry, flags.new, flags.open);
            return;
        }
        let Some(subject) = tool.template_subject(&session.rows.edits) else { return };
        match tool.rekey_template(session, &subject, to, now) {
            Ok(()) => info!("{} {to}: {} {} moved", flags.entry, S::TEMPLATE, subject.entry),
            Err(why) => warn!("{} {to}: {why}", flags.entry),
        }
    }
    *done = true;
    tool.scripted_template_done = true;
}

/// `--pick-display [id]`: open the display picker on the template window's
/// first display column, and with an id choose it. The picker is a grid of
/// pictures two clicks in; the bare flag opens it for a screenshot, and an id
/// also writes the column, which checks the whole path with nobody at the
/// keyboard.
///
/// It runs after the template flags and waits for the template's row, because
/// the column's target carries the database's value: a chosen value equal to
/// it takes the edit off rather than writing it. The window is opened so that
/// the row is fetched (`fetch_rows` follows the window's subject) and so that
/// a shot shows the column the choice lands in.
pub(super) fn display_on_the_command_line<S: SpawnSubject>(
    args: Res<crate::args::Args>,
    mut tool: ResMut<SpawnTool<S>>,
    mut session: Option<ResMut<EditSession>>,
    chosen: Res<Tool>,
    time: Res<Time>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    if !args.spawns.pick_display || *chosen != S::TOOL {
        *done = true;
        return;
    }
    if !tool.scripted_template_done {
        return;
    }
    let Some(session) = session.as_mut() else { return };
    let Some(subject) = tool.template_subject(&session.rows.edits) else {
        if tool.scripted_done || tool.trouble.is_some() {
            *done = true;
            warn!("--pick-display: nothing names a template; give {} or {}", S::FLAGS.open, S::FLAGS.new);
        }
        return;
    };
    tool.template_window = true;
    let Some(row) = tool.template.as_ref().filter(|held| held.entry == subject.entry) else {
        return;
    };
    let column_name = S::DISPLAY_COLUMN;
    let column = S::TEMPLATE_TABLE.column(column_name).expect("a template column");
    let key = subject.key();
    let in_database = match subject.is_new() {
        true => None,
        false => column.literal(&row.row),
    };
    let showing: u32 = session
        .rows
        .edits
        .get(S::TEMPLATE, &key, column_name)
        .map(str::to_string)
        .or_else(|| in_database.clone())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let target = crate::subjects::quests::ColumnTarget {
        table: S::TEMPLATE,
        subject: format!("{} {} {column_name}", S::TEMPLATE, key.text()),
        key,
        column: column_name,
        in_database,
        label: S::WORDS.edit,
    };
    match args.spawns.display_id {
        Some(id) => {
            target.write(session, id.to_string(), time.elapsed_secs_f64());
            info!("--pick-display {id}: written to {} {} {column_name}", S::TEMPLATE, subject.entry);
        }
        None => {
            tool.display_pick = Some(crate::subjects::displays::DisplayPick::open(S::DISPLAY_TABLE, target, showing));
            info!("--pick-display: the picker is open on {} {} {column_name}", S::TEMPLATE, subject.entry);
        }
    }
    *done = true;
}
