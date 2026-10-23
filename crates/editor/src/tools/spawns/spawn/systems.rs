//! The systems over one subject's [`SpawnTool`]: the map read, the spawns and
//! templates this project creates, the picker's search, the near list, the
//! pointer's aim, press, rectangle and drag, the markers, the preview models,
//! the ghost on the cursor, and the two whole rows a form reads.
//!
//! ## The whole map is read in one query, once
//!
//! Map 0 has 24,610 creature spawns and map 1 has 29,010, and one statement
//! reads either in about 0.6 s (`vale spawns Azeroth`). A query per block as
//! the camera moves would put a database round trip on the frame the camera
//! pans. So the map is read once, on a task, and camera movement only changes
//! which of the rows are drawn.
//!
//! The map is read again when the map changes and when a save applies
//! something or a Put back runs (`EditSession::database_writes`). It is not
//! polled: this does not handle another editor writing the same database,
//! and a re-read every few seconds would discard a selection whose form is
//! being typed into.
//!
//! ## Near spawns are drawn as models, the rest as markers
//!
//! The busiest tile on map 0 holds 288 spawns and the editor streams a 7x7
//! block, so drawing every spawn in the block as a model is up to several
//! thousand bodies. [`SpawnTool::model_budget`] is the cap: the nearest N get
//! a model through the client's own entity pipeline, and every other spawn in
//! range gets a marker. Both are pickable, so the budget changes how a spawn
//! is drawn and never whether it can be picked.
//!
//! A model is a synthetic `WorldEntity` standing at the row's position, built
//! the way `crate::preview::stage` builds its own. It goes through
//! `spawn_models`, `dress` and `animate`, so what is on screen is what the
//! game would draw, not a second reading of the same tables in this crate.
//!
//! Nothing is drawn during a playtest. The server spawns the real ones then,
//! and a preview in the same place would be a second copy of each.

use super::{Drag, MapRead, SpawnRow, SpawnSubject, SpawnTool, Spawned, Template, TemplateRow, MATCHES};
use crate::marks::{Look, Marks};
use crate::subjects::listing::Listed;
use crate::session::EditSession;
use crate::tools::placements::group::DRAG_PIXELS;
use crate::tools::Tool;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future};
use std::collections::HashSet;
use std::marker::PhantomData;
use vale_client::render::focus::WorldFocus;
use vale_client::world::camera::WorldCamera;
use vale_mangos::row::Life;
use vale_mangos::schema::RowValue;

/// Marks one of a tool's preview bodies, so they can be despawned together
/// and so nothing else mistakes one for an entity the server sent.
#[derive(Component)]
pub struct Preview<S: SpawnSubject> {
    /// The spawn's own guid.
    pub guid: u64,
    /// What the body was built from. A `WorldEntity`'s model is resolved once,
    /// when `spawn_models` first sees it, so a body whose display id (or a game
    /// object's state) has since changed is despawned and spawned again.
    pub look: S::Look,
    of: PhantomData<fn() -> S>,
}

/// Marks the template on the cursor before it is placed.
///
/// Not [`Preview`]: [`stand_them_up`] despawns every `Preview` that is not a
/// spawn on the map, and a ghost is not a spawn.
#[derive(Component)]
pub struct Ghost<S: SpawnSubject> {
    /// The display id this body was built from, on [`Preview`]'s rule.
    display_id: u32,
    of: PhantomData<fn() -> S>,
}

/// The marker colour of a spawn this project creates, which is in no database
/// yet.
pub(crate) const NEW: Color = Color::srgb(0.45, 0.85, 1.0);

/// The marker colour of a spawn this project removes. It is still drawn until
/// Apply.
const REMOVED: Color = Color::srgb(0.95, 0.30, 0.30);

/// The marker colour of a selected spawn that is not the primary: the
/// primary's gold, paler.
const MEMBER: Color = Color::srgb(0.95, 0.78, 0.50);

/// Read the open map's spawns, on a task, when the map or the project's
/// applied edits have changed.
pub(super) fn read_the_map<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    // Collect a finished read whatever the tool is, so switching away and back
    // does not discard a read that has already run.
    if let Some(task) = tool.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            tool.task = None;
            match done {
                Ok(read) => {
                    info!("{}: {} spawn(s) read", S::WORDS.nouns, read.spawns.len());
                    // What the map query already says about each template, so
                    // a spawn this project creates of one of them is drawn
                    // without a read of its own.
                    for spawn in &read.spawns {
                        if let Some(known) = S::known_of(spawn) {
                            tool.known.insert(known.entry(), known);
                        }
                    }
                    tool.take_read(read.spawns);
                    tool.max_guid = read.highest;
                    tool.max_entry = read.highest_entry;
                    tool.trouble = None;
                    tool.built_at = None;
                    tool.created_for = None;
                }
                Err(e) => {
                    warn!("{}: {e}", S::WORDS.nouns);
                    tool.take_read(Vec::new());
                    tool.trouble = Some(e);
                }
            }
        }
        return;
    }
    if !state.editing() || *chosen != S::TOOL {
        return;
    }
    let Some(session) = session else { return };
    // Keyed on `EditSession::database_writes`, which every apply and every
    // Put back of any subject increments. It was keyed on
    // `EditSession::applied`, which a Put back sets to `None`; in a session
    // that did not make the apply it was already `None`, so the key did not
    // change and spawns the Put back had deleted stayed on the map. It is not
    // keyed on the spell tables' signature either: a spell save would re-read
    // every spawn on the map.
    let key = (session.map_id, session.database_writes);
    if tool.loaded == Some(key) {
        return;
    }
    // Set before the read, not after, so a failed read is not retried on every
    // frame; with no server running that would be a connection attempt sixty
    // times a second.
    tool.loaded = Some(key);
    let Some((at, _source)) = settings.resolve() else {
        tool.take_read(Vec::new());
        tool.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = crate::server::settings::server_patch(&settings);
    let map = session.map_id;
    tool.trouble = None;
    tool.task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let rows = db.rows(&S::spawns_query(map, patch))?;
        // The highest guid in the whole table, not this map's. Guids are
        // global, so a new spawn numbered from one map's highest would collide
        // with a row on another. It adds one statement to a read that already
        // takes half a second.
        let highest = db.row(S::MAX_SPAWN_GUID_QUERY)?.and_then(|row| row.integer("guid")).map(|guid| guid as u64);
        // The highest template entry at any patch, for a new template's
        // number. One numbered from the rows the server loads would collide
        // with a row at a patch the server is not loading.
        let highest_entry = db.row(S::MAX_ENTRY_QUERY)?.and_then(|row| row.integer("entry")).map(|entry| entry as u32);
        Ok(MapRead { spawns: rows.iter().map(S::read_spawn).collect(), highest, highest_entry })
    }));
}

/// Build the spawns this project creates from the store, whenever the store
/// changes.
///
/// They are not in any database, so no query returns them. The store's own
/// columns are turned into the same spawn the map query produces, and from
/// there they are drawn, picked, dressed and edited by the same code as the
/// others.
///
/// Keyed on `ServerRows::revision`, which changes on every edit, unlike the
/// map read, which is keyed on the last apply. The work is a walk of the
/// store, which holds only the project's edits rather than a whole map, so a
/// walk per edit covers a few rows.
pub(super) fn rebuild_created<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    session: Option<Res<EditSession>>,
    chosen: Res<Tool>,
    popout: Res<crate::popout::Popout>,
    state: Res<crate::playtest::Playtest>,
) {
    // Under the subject's workspace too, in either window: Discard there finds
    // the created template in `templates`.
    if !state.editing() || !(*chosen == S::TOOL || popout.shows(*chosen, S::WORKSPACE)) {
        return;
    }
    let Some(session) = session else { return };
    if tool.created_for == Some(session.rows.revision) {
        return;
    }
    tool.created_for = Some(session.rows.revision);
    // The templates first: a created spawn below is labelled from `known`,
    // which the created templates are put into here, and every entry is
    // folded through the project's moves before anything reads it.
    tool.fold_template_moves(&session.rows.edits);
    tool.rebuild_templates(&session.rows.edits);

    let mut out: Vec<S::Spawn> = Vec::new();
    for (table, key, row) in session.rows.edits.rows() {
        if table != S::SPAWN || row.life != Life::Insert {
            continue;
        }
        let Some(guid) = key.first() else { continue };
        // Skip a spawn the read already returned. Once a created spawn has
        // been applied it is a row in the database, so the next read of the
        // map includes it. Listing it here as well would draw it twice, in two
        // places once one copy was dragged. The read's copy is drawn, with the
        // project's own columns laid over it as for any other row.
        if tool.read_one(guid).is_some() {
            continue;
        }
        // Only this map's spawns. A project can carry spawns on several maps
        // and the viewport shows one; a row for another map drawn here would
        // put a spawn at another map's coordinates.
        if super::stored_number(row, "map").map(|map| map as u32) != Some(session.map_id) {
            continue;
        }
        let entry = super::stored_number(row, "id").unwrap_or(0.0) as u32;
        out.push(S::created(guid, row, tool.known.get(&entry)));
    }
    // The near list indexes both lists, so it is out of date as soon as
    // either changes. An unchanged list, which is every store change that
    // creates no spawn, such as each frame of a drag, keeps it.
    if !same_guids(&tool.created, &out) {
        tool.built_at = None;
    }
    tool.created = out;
    tool.overlay.at = None;
}

/// Read the templates of spawns this project creates and the map does not
/// carry.
///
/// Everything the map query returns is already in [`SpawnTool::known`], and
/// the picker adds whatever was searched for, so this covers one case: a
/// project opened in a later session whose created spawns name a template
/// with no other spawn on this map. Without it those spawns are drawn as a
/// marker labelled by an entry and never get a model.
///
/// One query for every entry that is missing rather than one each, and it does
/// not run again while a read is in flight.
pub(super) fn learn_the_created<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    settings: Res<crate::server::settings::ServerSettings>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    session: Option<Res<EditSession>>,
) {
    // The templates held were read from the database as it stood at the
    // counter; after an apply or a put back they are read again, and a read
    // still running is dropped with them.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    if tool.known.renew(writes) {
        tool.known_task = None;
        tool.created_for = None;
    }
    if let Some(task) = tool.known_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            tool.known_task = None;
            match done {
                Ok(rows) => {
                    for known in rows {
                        tool.known.insert(known.entry(), known);
                    }
                    // What was read changes what a created spawn is drawn as.
                    tool.created_for = None;
                }
                Err(e) => warn!("{} templates: {e}", S::WORDS.noun),
            }
        }
        return;
    }
    if !state.editing() || *chosen != S::TOOL {
        return;
    }
    let missing: Vec<u32> = tool
        .created
        .iter()
        .map(|spawn| spawn.entry())
        .filter(|entry| *entry != 0 && !tool.known.contains_key(entry))
        .collect::<HashSet<u32>>()
        .into_iter()
        .collect();
    if missing.is_empty() {
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        return;
    };
    let patch = crate::server::settings::server_patch(&settings);
    // Recorded before the read, not after. Otherwise two frames with a read
    // in flight would start two reads, and an entry the database has no row
    // for would start a read on every frame.
    for entry in &missing {
        tool.known.insert(*entry, S::unknown(*entry));
    }
    tool.known_task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let Some(sql) = S::templates_query(&missing, patch) else {
            return Ok(Vec::new());
        };
        Ok(db.rows(&sql)?.iter().filter_map(S::read_known).collect())
    }));
}

/// Run the picker's search when what is typed or the type filter no longer
/// matches what was last asked.
///
/// One query per change of the box rather than per frame: `ran` holds what
/// was sent, so a frame that redraws the same text sends nothing. A term of
/// fewer than two characters is sent only when a type is chosen: `%a%` over
/// every template is thousands of rows and fifty arbitrary ones, while every
/// template of one type, such as every chest, is a list somebody asked for.
pub(super) fn search<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    settings: Res<crate::server::settings::ServerSettings>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    session: Option<Res<EditSession>>,
) {
    // The matches were read from the database as it stood at the counter; an
    // apply or a put back empties them, drops a search still running, and
    // sends the typed text again.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    if tool.new_spawn.searched_at != Some(writes) {
        tool.new_spawn.searched_at = Some(writes);
        tool.new_spawn.task = None;
        tool.new_spawn.forget();
    }
    if let Some(task) = tool.new_spawn.task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            tool.new_spawn.task = None;
            match done {
                Ok(rows) => {
                    for known in &rows {
                        tool.known.insert(known.entry(), known.clone());
                    }
                    tool.new_spawn.matches = rows;
                    tool.new_spawn.trouble = None;
                    // The matches are read at the entries the database has,
                    // and a template this project moves is listed at its new
                    // one; see `fold_template_moves`.
                    tool.created_for = None;
                }
                Err(e) => {
                    tool.new_spawn.matches.clear();
                    tool.new_spawn.trouble = Some(e);
                }
            }
        }
        return;
    }
    if !state.editing() || *chosen != S::TOOL || tool.mode != crate::tools::placements::place::Mode::Place {
        return;
    }
    let asked = (tool.new_spawn.search.trim().to_string(), tool.new_spawn.of_type);
    if tool.new_spawn.ran.as_ref() == Some(&asked) {
        return;
    }
    tool.new_spawn.ran = Some(asked.clone());
    let (term, of_type) = asked;
    if term.len() < 2 && of_type.is_none() {
        tool.new_spawn.matches.clear();
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        tool.new_spawn.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = crate::server::settings::server_patch(&settings);
    tool.new_spawn.task = Some(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let sql = S::search_query(&term, of_type, patch, MATCHES);
        Ok(db.rows(&sql)?.iter().filter_map(S::read_known).collect())
    }));
}

/// Collect the spawns near enough to draw, nearest first.
///
/// Rebuilt when the focus crosses a tile rather than per frame: it walks every
/// row on the map and sorts what it keeps, which is too much to do every
/// frame. A tile is 533 yards, so the list is never more than that out of
/// date.
pub(super) fn collect_the_near<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    focus: Res<WorldFocus>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *chosen != S::TOOL || !focus.present {
        return;
    }
    let tile = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    if tool.built_at == Some(tile) {
        return;
    }
    tool.built_at = Some(tile);
    // Both lists, the database's spawns and the ones this project creates,
    // under one index space. See [`SpawnTool::base_spawn`].
    tool.build_near(focus.position);
}

/// Whether two lists of created spawns are the same spawns in the same order,
/// which is what the near list's indices depend on.
fn same_guids<P: Spawned>(had: &[P], now: &[P]) -> bool {
    had.len() == now.len() && had.iter().zip(now).all(|(a, b)| a.guid() == b.guid())
}

/// Lay the project's edits over the near spawns once this frame's drag has
/// written them, for the marks, the models and the service marks drawn
/// after it. See [`SpawnTool::lay_edits`].
pub(crate) fn lay_the_edits<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    session: Option<Res<EditSession>>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *chosen != S::TOOL {
        return;
    }
    tool.lay_edits(session.as_deref());
}

/// What a right click landed on, for the right-click menu: the spawn the
/// pointer is over, which [`aim`] found this frame. A spawn outside the
/// selection becomes the selection, so the menu's actions apply to it; a
/// member of the group keeps the group. See `crate::context`.
pub fn menu_target<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    mut menu: ResMut<crate::context::ContextMenu>,
    chosen: Res<Tool>,
) {
    if *chosen != S::TOOL || !menu.asked() {
        return;
    }
    let Some(guid) = tool.hovered else { return };
    if !tool.holds(guid) {
        tool.select_only(Some(guid));
    }
    menu.object = Some(S::target(guid));
}

/// Find the spawn the pointer is over.
///
/// The pick radius is [`SpawnSubject::HANDLE_PIXELS`] on screen, for the
/// reason the light tool gives: a position is a point, and a radius in yards
/// is too small to click far away and covers the screen close up. The body's
/// half height is the minimum, so a spawn close to the camera is picked by its
/// body rather than by a dot at its feet.
#[allow(clippy::too_many_arguments)]
pub(super) fn aim<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    session: Option<Res<EditSession>>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if !state.editing() || *chosen != S::TOOL {
        tool.hovered = None;
        return;
    }
    // A held drag keeps what it grabbed.
    if tool.drag.is_some() {
        return;
    }
    // Nothing is hovered while a template is on the cursor. A highlighted
    // marker under a ghost would indicate that a click selects it, and the
    // click places instead.
    if tool.placing() || !crate::ui::over_the_world(&viewport, &wants, &windows) {
        tool.hovered = None;
        return;
    }
    let Some((origin, direction)) = crate::pick::ray(&windows, &camera) else {
        tool.hovered = None;
        return;
    };
    let Ok(window) = windows.single() else { return };
    let Ok((camera, _)) = camera.single() else { return };
    let height = window.height().max(1.0);
    // The projection's [1][1] is `1 / tan(fov / 2)`, which turns a pixel height
    // into an angle without asking what kind of projection it is.
    let fov = match camera.clip_from_view().to_cols_array()[5] {
        y if y.abs() > f32::EPSILON => y,
        _ => return,
    };
    // The pointer aims at where a spawn is drawn, which is where the
    // project's edits put it rather than where the database does.
    tool.lay_edits(session.as_deref());
    let mut best: Option<(f32, u64)> = None;
    for rank in 0..tool.near.len() {
        let Some(spawn) = tool.near_drawn(rank) else { continue };
        let middle = spawn.at() + Vec3::Z * (S::BODY / 2.0);
        let to = middle - origin;
        let along = to.dot(direction);
        if along <= 0.0 {
            continue;
        }
        let grabbable = ((S::HANDLE_PIXELS / height) * 2.0 * along / fov).max(S::BODY / 2.0);
        if (to - direction * along).length() > grabbable {
            continue;
        }
        if best.is_none_or(|(nearest, _)| along < nearest) {
            best = Some((along, spawn.guid()));
        }
    }
    tool.hovered = best.map(|(_, guid)| guid);
}

/// Place the chosen template on the surface under the pointer.
///
/// This is the only action of the tool that creates a row rather than
/// changing one. It is armed by the panel and acts on the next click, the same
/// two-gesture rule a move follows: arming is deliberate, and until it has
/// happened a click on the world selects.
///
/// The spawn faces the way the ghost does, which starts toward the camera.
/// vmangos' own `.npc add` gives a creature the player's orientation, so it
/// faces the way the player placing it is looking; the editor's camera looks
/// at the spot rather than from it, so the equivalent here is facing the
/// camera.
#[allow(clippy::too_many_arguments)]
fn place_one<S: SpawnSubject>(
    tool: &mut SpawnTool<S>,
    session: &mut Option<ResMut<EditSession>>,
    cursor: &crate::pick::Cursor,
    viewport: &crate::ui::Viewport,
    wants: &bevy_egui::input::EguiWantsInput,
    windows: &Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: &Query<&GlobalTransform, With<WorldCamera>>,
    time: &Time,
) {
    if !crate::ui::over_the_world(viewport, wants, windows) {
        return;
    }
    let Some(known) = tool.new_spawn.chosen.clone() else { return };
    let Some(session) = session.as_mut() else { return };
    // Without a surface under the pointer nothing is placed. A height guessed
    // from the camera would put the spawn inside a hill or in the air, and the
    // server drops a creature with a wrong `position_z` to the floor of
    // whatever it is over. The click is ignored and the placer stays armed,
    // so the next click on something solid places.
    //
    // The surface and not the ground, so that clicking a bridge, a dock or an
    // inn's upper floor stands the spawn on it. See [`crate::pick`].
    let Some(ground) = cursor.surface else {
        session.status = "no surface under the pointer; nothing placed".into();
        return;
    };
    // The ghost's own facing, so the row written matches the model that was
    // on screen. The camera fallback covers a click before the ghost has been
    // given a facing, which is the frame the placer is armed on.
    let facing = tool.ghost_facing.unwrap_or_else(|| match camera.single() {
        Ok(camera) => {
            let eye = Vec3::from(vale_client::render::axes::to_wow(camera.translation()));
            (eye.y - ground.y).atan2(eye.x - ground.x)
        }
        Err(_) => 0.0,
    });
    let guid = tool.create(session, &known, ground, facing, time.elapsed_secs_f64());
    tool.template_window = false;
    session.status = format!("{} placed as guid {guid}", known.label());
    info!(
        "{}: placed {} as guid {guid} at {:.1}, {:.1}, {:.1}",
        S::WORDS.nouns,
        known.label(),
        ground.x,
        ground.y,
        ground.z
    );
}

/// Select the spawn under the pointer.
///
/// A press on a spawn that is not selected selects it and does nothing else.
/// A press on one that is already selected arms a drag, which moves nothing
/// until the pointer has travelled `DRAG_PIXELS`. Moving a spawn is therefore
/// two gestures, click then drag, and the first cannot move it.
#[allow(clippy::too_many_arguments)]
pub fn press<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    mut session: Option<ResMut<EditSession>>,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<crate::pick::Cursor>,
    time: Res<Time>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    waypoints: Option<Res<crate::tools::spawns::waypoints::Waypoints>>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
    gizmo: Res<crate::tools::placements::gizmo::Gizmo>,
    keys: Res<ButtonInput<KeyCode>>,
    mut marquee: ResMut<crate::tools::placements::group::Marquee>,
) {
    if !state.editing() || *chosen != S::TOOL {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    // A press on a gizmo handle is the gizmo's, which runs first; it must
    // not also select or arm a drag on what is behind the handle.
    if gizmo.holding() {
        return;
    }
    // Another mode of the tool takes the pointer when it is using it; see
    // `SpawnSubject::yields_press`. It is checked ahead of the placer.
    if S::yields_press(waypoints.as_deref()) {
        return;
    }
    // A click in Place creates a spawn and does nothing else. It does not
    // also select whatever was under the pointer, which would open the form
    // on a spawn the user did not click.
    if tool.placing() {
        place_one(&mut tool, &mut session, &cursor, &viewport, &wants, &windows, &camera, &time);
        return;
    }
    let adding = crate::tools::placements::group::shift(&keys);
    let pointer = windows.single().ok().and_then(|window| window.cursor_position()).unwrap_or_default();
    let Some(guid) = tool.hovered else {
        // Nothing under the pointer: a rectangle begins. A click on nothing
        // leaves the selection alone, unlike the doodad tool's: the form,
        // the template window and the creature's waypoint window all follow
        // the selected spawn, and an empty click is how a waypoint node is let
        // go of.
        if crate::ui::over_the_world(&viewport, &wants, &windows) {
            marquee.begin(S::TOOL, pointer, adding);
        }
        return;
    };
    if adding {
        tool.toggle(guid);
        return;
    }
    if !tool.holds(guid) {
        // The form draws the two whole rows, and they belong to the selection
        // rather than to the frame. `select_only` clears them, which makes
        // `fetch_rows` read the new selection's rows.
        //
        // No drag is armed. The press that selects a spawn does not move it;
        // see this function's doc.
        tool.select_only(Some(guid));
        return;
    }
    // A press on a member of the group keeps the group, so the drag that
    // follows moves all of it.
    let collapse = !tool.also.is_empty();
    tool.promote(guid);
    // Where each is drawn, which is where the project's edits put it.
    let edits = session.as_ref().map(|session| &session.rows.edits);
    let from = tool.chosen_edited(edits).map(|spawn| spawn.at()).unwrap_or_default();
    let group_from: Vec<(u64, Vec3)> =
        tool.also.iter().filter_map(|&guid| tool.edited(guid, edits).map(|spawn| (guid, spawn.at()))).collect();
    tool.group_from = group_from;
    tool.drag = Some(Drag { guid, from, pressed_at: pointer, grab: None, moving: false, collapse });
}

/// Select every drawn spawn whose marker is inside a finished rectangle.
///
/// The spawns considered are the ones drawn — [`SpawnTool::near`] — for the
/// reason the doodad tool encloses only what is drawn: what is not on screen
/// cannot be seen to be inside the rectangle. See
/// [`crate::tools::placements::group`].
pub(super) fn enclose<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    mut marquee: ResMut<crate::tools::placements::group::Marquee>,
    session: Option<Res<EditSession>>,
    chosen: Res<Tool>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    if *chosen != S::TOOL {
        return;
    }
    let Some(done) = marquee.finished(S::TOOL) else {
        return;
    };
    let Some((camera, eye, window)) = crate::tools::placements::group::view(&camera, &windows) else {
        return;
    };
    let edits = session.as_ref().map(|session| &session.rows.edits);
    let mut found: Vec<(f32, u64)> = Vec::new();
    for index in &tool.near {
        let Some(spawn) = tool.spawn_at(*index, edits) else {
            continue;
        };
        let middle = spawn.at() + Vec3::Z * (S::BODY / 2.0);
        let Some(point) = crate::tools::placements::group::on_screen(camera, eye, window, middle) else {
            continue;
        };
        if done.rect.contains(point) {
            found.push((point.distance(done.rect.center()), spawn.guid()));
        }
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    if !done.adding {
        tool.select_only(None);
    }
    for (_, guid) in found {
        if tool.holds(guid) {
            continue;
        }
        match tool.selected {
            None => tool.make_primary(Some(guid)),
            Some(_) => tool.also.push(guid),
        }
    }
}

/// Move the held spawn, once the pointer has travelled far enough.
///
/// The three position columns are written to the project's store on every
/// frame of the drag, under one gesture. This is what makes the spawn follow
/// the pointer: the drawing reads the store over the database's reading, so a
/// drag that wrote only at the end would move nothing until the button was
/// released.
///
/// It adds nothing to the file or the undo stack. The store is a map keyed by
/// column, so sixty writes to `position_x` are one entry; the gesture names
/// the row, so the three columns fold into one undo entry rather than three;
/// and `set_server_edit` returns early when the value has not changed, so a
/// held pointer that is not moving writes nothing.
///
/// The drag follows the surface under the pointer, not a plane at the spawn's
/// height, so it is not `crate::tools::points::PointDrag`; the height is the
/// subject's [`SpawnSubject::drag_height`].
#[allow(clippy::too_many_arguments)]
pub fn drag<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    mut session: Option<ResMut<EditSession>>,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<crate::pick::Cursor>,
    time: Res<Time>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
) {
    if !state.editing() || *chosen != S::TOOL {
        tool.drag = None;
        return;
    }
    let Some(held) = tool.drag else { return };
    let Some(session) = session.as_mut() else { return };

    if !buttons.pressed(MouseButton::Left) {
        tool.drag = None;
        tool.group_from.clear();
        session.history.release();
        if held.moving {
            let now = tool.chosen_edited(Some(&session.rows.edits)).map(|spawn| spawn.at()).unwrap_or(held.from);
            session.status = match tool.also.len() {
                0 => format!("moved {} {} by {:.1} yards", S::WORDS.spawn, held.guid, now.distance(held.from)),
                more => format!("moved {} {} by {:.1} yards", more + 1, S::WORDS.spawns, now.distance(held.from)),
            };
        } else if held.collapse {
            // A press on a member that never moved was a click on it.
            tool.also.clear();
        }
        return;
    }

    // Nothing moves until the pointer has travelled from the press point:
    // without it a click moved the spawn a fraction of a yard.
    if !held.moving {
        let at = windows.single().ok().and_then(|window| window.cursor_position()).unwrap_or(held.pressed_at);
        if at.distance(held.pressed_at) >= DRAG_PIXELS {
            tool.drag = Some(Drag { moving: true, ..held });
        }
        return;
    }

    // The surface under the pointer: the ground, or a building or scenery
    // model standing on it. Without a surface nothing moves; a height guessed
    // from the camera would drop the spawn through the floor of whatever it
    // is standing in.
    let Some(ground) = cursor.surface else { return };
    let grab = match held.grab {
        Some(grab) => grab,
        None => {
            // Measured on the first frame the drag is moving, and nothing is
            // written on that frame. The offset is the difference between the
            // spawn's position and the point where the pointer met the
            // surface; keeping it stops the spawn jumping under the cursor.
            tool.drag = Some(Drag { grab: Some(held.from - ground), ..held });
            return;
        }
    };
    // The horizontal offset is kept, and the height is the subject's rule.
    let want = Vec3::new(ground.x + grab.x, ground.y + grab.y, S::drag_height(ground, grab));
    let now = time.elapsed_secs_f64();
    // One entry from the first write to the release, whatever pauses the
    // pointer makes on the way. See `vale_edit::undo::History::hold`.
    session.history.hold(now);
    if tool.group_from.is_empty() {
        super::move_one::<S>(session, held.guid, want, now);
        return;
    }
    // The rest of the group by the same step across the ground, each keeping
    // its own height above the terrain — see
    // [`crate::tools::placements::group::carried`].
    let step = want - held.from;
    let label = format!("Move {} {}", tool.group_from.len() + 1, S::WORDS.nouns);
    let subject = format!("{} group {} position", S::SPAWN, held.guid);
    let members = tool.group_from.clone();
    session.as_one(&label, &subject, |session| {
        super::move_one::<S>(session, held.guid, want, now);
        for (guid, from) in members {
            let to = crate::tools::placements::group::carried(session, from, from + step);
            super::move_one::<S>(session, guid, to, now);
        }
    });
}

/// Draw a marker over every near spawn, and the selected one's facing.
///
/// A spawn drawn as a model gets a translucent marker around its feet; a
/// spawn with no model gets a smaller, near-opaque one where the model would
/// stand. The shape is the subject's ([`SpawnSubject::mark`]).
///
/// A mark that is being aimed at is [`Look::Ghosted`], so it shows faintly
/// through a wall. Every other mark is [`Look::Solid`] and hidden by whatever
/// is nearer the camera: the tool marks up to a thousand spawns, and in a
/// forest ghosts of all of them would cover the trees.
pub(super) fn draw<S: SpawnSubject>(
    mut marks: ResMut<Marks>,
    tool: Res<SpawnTool<S>>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *chosen != S::TOOL {
        return;
    }
    // Sorted once and searched per marker — see
    // `crate::tools::placements::doodads::draw_marker`.
    let members = crate::tools::placements::group::sorted(tool.also.iter().copied());
    for rank in 0..tool.near.len() {
        // The spawn as drawn, not as read. A spawn this project creates is in
        // neither the database nor `spawns`, and one it removes has to be
        // drawn as removed. The edits were laid over the near spawns by
        // `lay_the_edits`, and a spawn the project does not edit, which is
        // all but a few, is the base borrowed.
        let Some(spawn) = tool.near_drawn(rank) else { continue };
        let chosen = tool.selected == Some(spawn.guid());
        let member = crate::tools::placements::group::holds(&members, spawn.guid());
        let under = tool.hovered == Some(spawn.guid());
        // With the markers off, only what is aimed at. See
        // [`SpawnTool::show_markers`].
        if !tool.show_markers && !(chosen || under || member) {
            continue;
        }
        // A spawn drawn as a model gets a marker around its feet rather than
        // a full one: the model shows where it is.
        let has_model = tool.show_models && rank < tool.model_budget && S::marks_as_model(spawn);
        // Edited spawns are drawn in the warning colour whether or not they are
        // chosen, so the user can see which spawns have been changed.
        let edited = tool.near_touched(rank);
        // A removal is red in every state, including while it is selected.
        // The other colours all mean the project changes the spawn, and a
        // removal is a different claim. A creation has its own colour for the
        // same reason, at lower priority.
        let colour = match (spawn.claim(), chosen, under, edited) {
            (Life::Delete, _, _, _) => REMOVED,
            (_, true, _, _) => Color::srgb(1.0, 0.82, 0.25),
            (_, _, true, _) => Color::WHITE,
            // The rest of a group, paler than the primary.
            _ if member => MEMBER,
            (Life::Insert, _, _, _) => NEW,
            (_, _, _, true) => Color::srgb(0.95, 0.55, 0.20),
            _ => S::ORDINARY,
        };
        let at = vale_client::render::axes::to_bevy(spawn.at().to_array());
        let radius = marks.visible_radius(at, S::MARK_RADIUS);
        let look = match chosen || member || under {
            true => Look::Ghosted,
            false => Look::Solid,
        };
        S::mark(&mut marks, spawn, at, radius, has_model, colour, look);
        if spawn.is_removed() {
            // A cross through it, because the colour alone has to be learned
            // and this is the one state in which a mistake deletes a row.
            // Ghosted, so a spawn marked for removal still shows through a
            // wall in front of it.
            let arm = radius * 1.4;
            for (dx, dy) in [(1.0, 1.0), (1.0, -1.0)] {
                let from = vale_client::render::axes::to_bevy(
                    (spawn.at() + Vec3::new(arm * dx, arm * dy, S::BODY / 2.0)).to_array(),
                );
                let to = vale_client::render::axes::to_bevy(
                    (spawn.at() - Vec3::new(arm * dx, arm * dy, -S::BODY / 2.0)).to_array(),
                );
                marks.wide_line(from, to, 1.6, REMOVED, Look::Ghosted);
            }
        }
        if chosen {
            // Which way it faces, which is hard to read as a number: 4.71
            // radians does not show a direction.
            let orientation = spawn.orientation();
            let facing = Vec3::new(orientation.cos(), orientation.sin(), 0.0);
            let from = vale_client::render::axes::to_bevy((spawn.at() + Vec3::Z * 0.2).to_array());
            let nose =
                vale_client::render::axes::to_bevy((spawn.at() + facing * (radius * 3.0) + Vec3::Z * 0.2).to_array());
            let thickness = marks.line_radius(from) * 1.6;
            marks.arrow(from, nose, thickness, Color::srgb(1.0, 0.95, 0.6), Look::Ghosted);
            S::mark_chosen(&mut marks, spawn, at);
        }
    }
}

/// Spawn a model at each of the nearest spawns, and despawn the models of
/// spawns that have left the list.
///
/// The model is the client's own: a `WorldEntity` with a display id goes
/// through `spawn_models`, `dress` and `animate` as an entity the server sent
/// does. The module comment states why this is used rather than a second
/// reading of the same tables.
pub(super) fn stand_them_up<S: SpawnSubject>(
    mut commands: Commands,
    tool: Res<SpawnTool<S>>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    mut standing: Query<(Entity, &Preview<S>, &mut Transform)>,
) {
    // The nearest of the near list, as `lay_the_edits` left them: the
    // project's edits over the database's reading, so a display id typed
    // into the form changes the model on the next frame rather than after an
    // apply and a re-read.
    //
    // Every other state wants none of them: another tool, a playtest, or the
    // switch off. The models are despawned rather than hidden, so no
    // dressing is held for a tool that is not in use.
    let budget = match (state.editing(), *chosen == S::TOOL, tool.show_models) {
        (true, true, true) => tool.model_budget.min(tool.near.len()),
        _ => 0,
    };
    let wanted = |guid: u64| -> Option<&S::Spawn> {
        let rank = *tool.near_rank.get(&guid).filter(|rank| **rank < budget)?;
        tool.near_drawn(rank).filter(|spawn| spawn.guid() == guid)
    };

    let mut present = HashSet::new();
    for (entity, preview, mut transform) in &mut standing {
        // A changed display id is a different model, and the body was built
        // from the old one, so it is despawned and spawned again rather than
        // moved.
        match wanted(preview.guid) {
            Some(spawn) if S::look(spawn) == preview.look => {
                present.insert(preview.guid);
                // Written only when it moved, so a still body is not marked
                // changed and its hierarchy is not propagated again.
                let stand = S::transform(spawn);
                if *transform != stand {
                    *transform = stand;
                }
            }
            _ => commands.entity(entity).despawn(),
        }
    }

    for rank in 0..budget {
        let Some(spawn) = tool.near_drawn(rank) else { continue };
        let guid = spawn.guid();
        if present.contains(&guid) || spawn.display_id() == 0 {
            continue;
        }
        commands.spawn((
            S::body(S::PREVIEW_GUID_BASE | guid, spawn),
            S::transform(spawn),
            Visibility::default(),
            // Without a `Sheath` the body is never drawn and no warning is
            // logged: `spawn_models` requires it in its query rather than as
            // an option, so an entity without one does not match, and it gets
            // no model, no fallback box and no warning line.
            vale_client::world::entities::Sheath::seeded(0),
            Preview::<S> { guid, look: S::look(spawn), of: PhantomData },
        ));
    }
}

/// The guid the ghost is given, clear of the previews' range and of anything a
/// session puts in the world.
fn ghost_guid<S: SpawnSubject>() -> u64 {
    S::PREVIEW_GUID_BASE | 0xFFFF_FFFF
}

/// Keep the template to be placed standing under the pointer.
///
/// The ghost is a client body, as a preview is, built by the client's own
/// `spawn_models`, `dress`, `pose` and `animate`, so it is posed and animated
/// where the doodad ghost is a still mesh. A creature's ghost is made
/// translucent by the client's own `world::entities::tint::fade_models`; a game
/// object has no vis flags to fade by, so its ghost is drawn solid with a mark
/// under it. See [`SpawnSubject::ghost_body`].
///
/// The height is the surface's under the pointer, because the server stands a
/// creature at its `position_z` and a guessed one drops it through the floor.
///
/// The facing is the doodad placer's turn. It starts toward the camera, on
/// the frame the ghost first has a surface under it, and from then on it is
/// held: moving the pointer or the camera does not turn the ghost. `,` and
/// `.` and `Alt` with the mouse turn it ([`SpawnTool::turn_ghost`]), and it is
/// kept from one placement to the next. Leaving Place or pressing `Escape`
/// forgets it, so the next template armed starts toward the camera again.
#[allow(clippy::too_many_arguments)]
pub(super) fn ghost<S: SpawnSubject>(
    mut commands: Commands,
    mut tool: ResMut<SpawnTool<S>>,
    mut marks: ResMut<Marks>,
    cursor: Res<crate::pick::Cursor>,
    chosen: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
    mut standing: Query<(Entity, &Ghost<S>, &mut Transform)>,
) {
    // Escape clears the chosen template, with `place::aim`'s key and rule:
    // the mode stays, the template on the cursor is removed. Not while a text
    // box has the keyboard, or the `Escape` used to leave the search box
    // would also disarm the placer.
    if tool.placing() && keys.just_pressed(KeyCode::Escape) && !wants.wants_keyboard_input() {
        tool.new_spawn.chosen = None;
    }
    // What the ghost should be, or nothing when another tool is active, the
    // mode is Select, nothing is chosen, the pointer is over a panel, or there
    // is no surface under it. The last is the usual case at the edge of the
    // streamed block, and the ghost is despawned rather than left where the
    // pointer last met the world.
    let wanted = match state.editing()
        && *chosen == S::TOOL
        && tool.placing()
        && crate::ui::over_the_world(&viewport, &wants, &windows)
    {
        true => tool.new_spawn.chosen.clone().zip(cursor.surface).filter(|(known, _)| S::ghost_wanted(known)),
        false => None,
    };
    let Some((known, at)) = wanted else {
        for (entity, _, _) in &standing {
            commands.entity(entity).despawn();
        }
        // The facing is kept while the placer is armed and the pointer is
        // only off the world, and forgotten once nothing is on the cursor.
        if !tool.placing() && tool.ghost_facing.is_some() {
            tool.ghost_facing = None;
        }
        return;
    };

    // Toward the camera the first time, and what it was left at after that.
    // The click reads the same value, so the model on screen and the row
    // written face the same way.
    let facing = match tool.ghost_facing {
        Some(facing) => facing,
        None => {
            let toward = match camera.single() {
                Ok(camera) => {
                    let eye = Vec3::from(vale_client::render::axes::to_wow(camera.translation()));
                    (eye.y - at.y).atan2(eye.x - at.x)
                }
                Err(_) => 0.0,
            };
            tool.ghost_facing = Some(toward);
            toward
        }
    };

    let centre = vale_client::render::axes::to_bevy(at.to_array());
    S::ghost_mark(&mut marks, centre, at, facing);
    let scale = known.scale();
    let stand = Transform {
        translation: centre,
        rotation: vale_client::render::axes::body(facing, 0.0),
        scale: Vec3::splat(match scale > 0.01 {
            true => scale,
            false => 1.0,
        }),
    };
    let mut found = false;
    for (entity, ghost, mut transform) in &mut standing {
        match ghost.display_id == known.display_id() && !found {
            true => {
                found = true;
                *transform = stand;
            }
            // A second one, or one built from a display id that has changed.
            false => commands.entity(entity).despawn(),
        }
    }
    if found || known.display_id() == 0 {
        return;
    }
    commands.spawn((
        S::ghost_body(ghost_guid::<S>(), &known),
        stand,
        Visibility::default(),
        // Without a `Sheath` the body is never drawn and no warning is logged.
        // See [`stand_them_up`], which needs it for the same reason.
        vale_client::world::entities::Sheath::seeded(0),
        Ghost::<S> { display_id: known.display_id(), of: PhantomData },
    ));
}

/// Read the selected spawn's two whole rows, on a task.
///
/// The map query reads the columns the drawing needs. A form needs every
/// column of the template and of the spawn, for one row rather than every row
/// on the map, so they are read when a selection is made rather than carried
/// for every spawn.
pub fn fetch_rows<S: SpawnSubject>(
    mut tool: ResMut<SpawnTool<S>>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    chosen: Res<Tool>,
    popout: Res<crate::popout::Popout>,
    state: Res<crate::playtest::Playtest>,
    // The world subject the workspace last followed. See below.
    mut followed: Local<Option<u32>>,
) {
    if let Some(task) = tool.template_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            tool.template_task = None;
            match done {
                Ok(row) => tool.template = row,
                Err(e) => warn!("{} template: {e}", S::WORDS.noun),
            }
        }
    }
    if let Some(task) = tool.spawn_task.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            tool.spawn_task = None;
            match done {
                Ok(row) => tool.spawn_row = row,
                Err(e) => warn!("{} spawn: {e}", S::WORDS.noun),
            }
        }
    }
    let workspace = popout.shows(*chosen, S::WORKSPACE);
    if !state.editing() || !(*chosen == S::TOOL || workspace) {
        return;
    }
    // A row is made again when the database has been written since it was
    // made, or when the project has started or stopped creating it; see
    // `crate::server::fresh`.
    let writes = session.as_ref().map_or(0, |session| session.database_writes);
    let Some((at, _)) = settings.resolve() else {
        return;
    };
    // With the workspace in the pop-out window and the tool in the main one,
    // the two share this template row, so the workspace follows the tool: a
    // spawn selected or a template picked in the world opens in the
    // workspace, and a row opened in the workspace stays until the next one.
    // See `crate::popout`.
    if workspace && *chosen == S::TOOL {
        let world =
            session.as_ref().and_then(|session| tool.template_subject(&session.rows.edits)).map(|subject| subject.entry);
        if world.is_some() && world != *followed {
            tool.listed.open = world;
        }
        *followed = world;
    }
    // The template half follows the window's subject, which in Place is the
    // picker's template and in Select the selected spawn's; under the
    // workspace it is the row open there.
    let subject = session.as_ref().and_then(|session| match workspace {
        true => tool.listed.subject(&session.rows.edits),
        false => tool.template_subject(&session.rows.edits),
    });
    if let Some(subject) = subject {
        let wants_template = tool.template_task.is_none()
            && (tool.template.as_ref().is_none_or(|had| had.entry != subject.entry)
                || tool.template_at != Some((writes, subject.is_new())));
        if wants_template {
            tool.template_at = Some((writes, subject.is_new()));
            if subject.is_new() {
                // A template this project creates has no row to read. Its
                // whole row is in the store, so the form is given an empty
                // row holding only the key, and every column shows the
                // project's own value.
                let mut row = vale_mangos::schema::Row::new();
                row.insert("entry".to_string(), Some(subject.entry.to_string()));
                tool.template = Some(TemplateRow { entry: subject.entry, patch: subject.patch, row });
            } else {
                // Read from where the database has it, which is not the entry
                // the window shows while the project moves the template and
                // has not applied it. The patch is the one the map query
                // already resolved, so the form and the drawing show the same
                // row. A row the database does not hold is an empty row, which
                // the form shows as NULL in every column; answering nothing
                // instead left the row unread and asked for again on every
                // frame.
                let (entry, read_entry, patch) = (subject.entry, subject.read_entry, subject.patch);
                let at = at.clone();
                tool.template_task = Some(crate::server::queue::read(async move {
                    let mut db = vale_mangos::conn::Db::open(&at)?;
                    let row = db.row(&S::template_query(read_entry, patch))?.unwrap_or_default();
                    Ok(Some(TemplateRow { entry, patch, row }))
                }));
            }
        }
    }
    // The spawn half is the tool's alone.
    if *chosen != S::TOOL {
        return;
    }
    let Some(spawn) = tool.chosen().cloned() else { return };
    let guid = spawn.guid();
    // A spawn this project creates has no row to read. Its whole row is in
    // the store, so the form is given an empty row holding only the key:
    // every column then shows the project's own value, and the `guid` field,
    // which is never editable and so never in the store, shows the guid
    // rather than `NULL`.
    if spawn.is_new()
        && (tool.spawn_row.as_ref().is_none_or(|had| had.guid != guid) || tool.spawn_at != Some((writes, true)))
    {
        let mut row = vale_mangos::schema::Row::new();
        row.insert("guid".to_string(), Some(guid.to_string()));
        tool.spawn_at = Some((writes, true));
        tool.spawn_row = Some(SpawnRow { guid, row });
    }
    let wants_spawn = !spawn.is_new()
        && tool.spawn_task.is_none()
        && (tool.spawn_row.as_ref().is_none_or(|had| had.guid != guid) || tool.spawn_at != Some((writes, false)));
    if wants_spawn {
        tool.spawn_at = Some((writes, false));
        tool.spawn_task = Some(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            Ok(db.row(&S::spawn_query(guid))?.map(|row| SpawnRow { guid, row }))
        }));
    }
}
