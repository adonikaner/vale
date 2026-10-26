//! Startup: the session built from the command line and the project, and the
//! overlay handed to the archive chain.

use vale_client::assets::GameAssets;
use vale_client::render::focus::WorldFocus;
use vale_edit::project::Project;
use vale_edit::undo::History;
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use std::sync::{Arc, RwLock};
use super::{EditSession, Edited, overlay_over};
use super::tiles::claimed_tiles;
use crate::places::map_directories;
use super::originals::Shipped;

/// Build the session and hand the archive chain its overlay.
pub(super) fn open(
    mut commands: Commands,
    args: Res<crate::args::Args>,
    assets: Res<GameAssets>,
    mut focus: ResMut<WorldFocus>,
    mut live: ResMut<vale_client::render::terrain::LiveEdits>,
    mut clock: ResMut<vale_client::render::sky::WorldClock>,
    // The camera is where the focus comes from from the next frame on, so
    // `--at` has to reach it here. Setting only the focus would put the world at
    // the asked-for place for one frame and then move it to the camera's own
    // origin, which is the middle of the map.
    mut camera: ResMut<crate::camera::EditorCamera>,
) {
    // `--project`, else the one opened last, else the default. The last
    // one is a file under `Edit\`, written here and on every switch — see
    // `vale_edit::project::last_opened`.
    let name = args
        .view.project
        .clone()
        .or_else(|| vale_edit::project::last_opened(&assets.root))
        .unwrap_or_else(|| vale_edit::project::DEFAULT.to_string());
    let project = match Project::open(&assets.root, &name) {
        Ok(project) => project,
        Err(e) => {
            error!("could not open the project {name}: {e}");
            return;
        }
    };
    vale_edit::project::remember_opened(&assets.root, &name);

    let edited: Edited = Arc::new(RwLock::new(HashMap::default()));
    assets.set_overlay(Some(overlay_over(&project, &edited)));

    let maps = map_directories(&assets);
    let map_id = maps
        .iter()
        .find(|(_, name)| name.eq_ignore_ascii_case(&args.view.map))
        .map(|(id, _)| *id)
        .unwrap_or(0);

    // A tile is built so its ground can be rewritten after it is on the GPU,
    // and this has to be set before any tile is streamed because it decides how
    // each one is built: the meshes stay in the main world so a stroke can write
    // to them, and each draw group remembers where its vertices came from. See
    // `vale_client::render::terrain::LiveEdits`.
    live.0 = true;
    // The world is lit by hand, because `render::sky::resolve` declines to
    // resolve at all while neither the server nor a person has said what time it
    // is, and only a server sends the clock. The same override `--hour` uses.
    //
    // The game's interface and the login screen's own 3D scene are switched
    // by [`crate::playtest::arrange`], not here. They are off while the editor
    // is editing and on while it is being played, which is one state and not
    // two defaults.
    let noon = 12 * 120;
    *clock = vale_client::render::sky::WorldClock {
        half_minutes: noon,
        from_server: false,
        override_half_minutes: Some(noon),
    };

    // What the map claims, from the project's own copy of the WDT if it has
    // one and the archives' otherwise — so a tile made in a previous session is
    // there when this one opens. See `EditSession::claimed`.
    let claimed = claimed_tiles(&assets, &project, &args.view.map);
    // …and what it changes on the server, which is a file of the project's own
    // because no client file carries a `creature_template` row.
    let rows = crate::server::store::ServerRows::read(&project);
    // …and which of its tiles are the archives' own, which it should not keep.
    // See [`Shipped::look_for_unchanged`].
    let shipped = Shipped::new(&assets);
    let unchanged = Some(shipped.look_for_unchanged(&project));

    let at = args.view.at.unwrap_or((0.0, 0.0));
    camera.go_to(Vec2::new(at.0, at.1));
    // …and `--view`, which is how far back and at what angle. Written after
    // `go_to` because that keeps the framing and only moves the focus.
    if let Some((distance, pitch, yaw)) = args.view.camera {
        camera.distance = distance.max(1.0);
        if let Some(pitch) = pitch {
            camera.pitch = pitch.to_radians();
        }
        if let Some(yaw) = yaw {
            camera.yaw = yaw.to_radians();
        }
    }
    *focus = WorldFocus::at(map_id, args.view.map.clone(), camera.target);

    commands.insert_resource(EditSession {
        status: format!("{} in {}", args.view.map, project.root.display()),
        project,
        map: args.view.map.clone(),
        map_id,
        maps,
        tiles: HashMap::default(),
        tables: std::collections::HashMap::default(),
        history: History::new(),
        dirty: HashMap::default(),
        regrow: HashMap::default(),
        moved: HashMap::default(),
        moved_buildings: HashMap::default(),
        revision: HashMap::default(),
        changes: HashMap::default(),
        unpublished: HashSet::default(),
        repaint: HashMap::default(),
        regroup: HashSet::default(),
        stale: HashSet::default(),
        unsaved: HashSet::default(),
        unsaved_tables: HashSet::default(),
        table_revision: 0,
        restream: HashSet::default(),
        republished: Vec::new(),
        republished_all: false,
        tables_republished: false,
        republished_revision: 0,
        files_revision: 0,
        spelled: Default::default(),
        claimed,
        reread_everything: false,
        claims_for: args.view.map.clone(),
        edited,
        rows,
        applied_signature: None,
        applied: [None; crate::server::stack::Subject::COUNT],
        database_writes: 0,
        shipped: Some(shipped),
        unchanged,
        locks: HashMap::default(),
        placement_locks: None,
    });
}
