//! The building editor: one building drawn on its own in the viewport, where
//! its areas and the doodads its file carries are edited.
//!
//! ## Opening and leaving
//!
//! The WMO tool's Select mode opens it on the selected WMO (Edit Building),
//! or, on a map that is one building, on that building. The camera's place is
//! kept, the camera moves to the building, and the toolbar's modes become the
//! editor's two: Areas ([`super::wmoareas`]) and Doodads
//! ([`super::wmodoodads`]). Back to the World, Escape with nothing to cancel,
//! another mode or tool, a playtest, or another map leaves it, and the camera
//! goes back. Leaving selects the WMO it was opened from again, once its tile
//! is open: the tiles near the old place are closed while the camera is at
//! the building, unless they hold unsaved edits.
//!
//! ## Where the building is drawn
//!
//! The building is drawn once more, off the map's 64x64 grid at [`ANCHOR`],
//! with no turn: a point's place in the world is its place in the file plus
//! [`ANCHOR`], so the file's own axes are the world's and the numbers the
//! panels show are the file's. Nothing else stands there. The ground the
//! pointer can meet is the open tiles', and no tile is open off the grid, so
//! the pointer meets only the building and its doodads.
//!
//! The host is a held tile (`vale_client::render::terrain::HeldTile`) at
//! [`VIEW_TILES`], spawned through `vale_client::render::wmos::spawn_held`, so
//! the renderer builds the building, its doodads and its collision hull as
//! it builds any tile's. Its placement id is [`VIEW_ID`]. It draws the
//! chosen doodad set ([`Viewing::set`]), the one set a placement draws.
//!
//! An edit to the building's files is drawn by a second host, built hidden
//! on the other of [`VIEW_TILES`] beside the one drawn
//! (`crate::session::buildings` counts the edits told to the renderer). The
//! new host is shown, and the old one despawned, once its building is placed
//! and its doodads resolved and spawned, and two frames more for the GPU to
//! have the meshes. Despawning first and building after left the viewport
//! empty for the frames a rebuild takes, which flashed on every edit. Until
//! the new host is shown, `super::wmodoodads` moves the drawn doodads to
//! their places in the edited file.

use bevy::prelude::*;
use vale_assets::world::adt::PlacedModel;
use vale_client::render::wmos::{Interior, WmoPlacement};

use super::place::{Placing, Switch};
use crate::camera::EditorCamera;
use crate::session::EditSession;
use crate::tools::Tool;

/// Where the building's own origin is put, in the world's axes: off the map's
/// grid, which ends 17,067 yards from the map's centre, so no tile is near.
pub(crate) const ANCHOR: Vec3 = Vec3::new(-20_000.0, -20_000.0, 0.0);

/// The held tiles the building is hosted on, the drawn host on one and its
/// replacement on the other: off the grid, and not the tile a WMO-only map's
/// building is hosted on, `(64, 64)`.
pub(crate) const VIEW_TILES: [(u32, u32); 2] = [(65, 65), (66, 66)];

/// The placement id the building is drawn with. Not a `MODF` id any tile or
/// WDT uses: a WMO-only map's building is `u32::MAX`.
pub(crate) const VIEW_ID: u32 = u32::MAX - 1;

/// The `MODF` id of a WMO-only map's building.
const GLOBAL_BUILDING: u32 = u32::MAX;

/// The open building editor.
#[derive(Debug, Clone)]
pub(crate) struct Viewing {
    /// The root file shown and edited.
    pub(crate) path: String,
    /// The placement it was opened from, by its `MODF` id, and that
    /// placement's name set, the second key of the building's area rows.
    pub(crate) from: u32,
    pub(crate) name_set: u16,
    /// The tile whose `MODF` lists that placement; `None` for a WMO-only
    /// map's building.
    pub(crate) tile: Option<(u32, u32)>,
    /// The doodad set drawn, and the one a doodad added goes into.
    pub(crate) set: u16,
    /// The map it was opened on.
    map: String,
    /// The camera before the editor opened, put back when it closes.
    camera: EditorCamera,
    /// Whether the camera has been put on the building, which waits until
    /// the renderer has placed it and its size is known.
    framed: bool,
}

impl Viewing {
    /// The root file's name without its folders.
    pub(crate) fn leaf(&self) -> &str {
        self.path.rsplit(['\\', '/']).next().unwrap_or(&self.path)
    }

    /// Whether it was opened on a WMO-only map's building.
    pub(crate) fn opened_on_the_map(&self) -> bool {
        self.from == GLOBAL_BUILDING
    }
}

/// The building editor's state.
#[derive(Resource, Debug)]
pub(crate) struct WmoView {
    pub(crate) open: Option<Viewing>,
    /// A request to open it on a drawn placement, by its `MODF` id.
    pub(crate) asked: Option<u32>,
    /// A request to leave it.
    pub(crate) leave: bool,
    /// The mode it opens in: the last one used.
    pub(crate) tab: Switch,
    /// The root file of the WMO-only map's building, while one is drawn.
    pub(crate) global: Option<String>,
    /// The WMO to select again after leaving, by its `MODF` id, until its
    /// tile is open.
    reselect: Option<u32>,
    /// The host being built hidden to replace the one drawn, and how many
    /// frames it has been ready.
    coming: Option<(Entity, u8)>,
}

impl Default for WmoView {
    fn default() -> WmoView {
        WmoView { open: None, asked: None, leave: false, tab: Switch::Areas, global: None, reselect: None, coming: None }
    }
}

/// The host entity and what it was spawned with: the file, the set, and the
/// count of edits told to the renderer.
#[derive(Component)]
pub(crate) struct Host {
    path: String,
    set: u16,
    told: u64,
    coord: (u32, u32),
}

/// How a new host's building and doodads stand, for the swap.
type Built<'a> = (
    &'a vale_client::render::wmos::PendingWmos,
    &'a vale_client::render::doodads::PendingDoodads,
    &'a vale_client::render::doodads::ResidentDoodads,
);

/// Open, keep and leave the building editor. See the module comment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow(
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    mut view: ResMut<WmoView>,
    mut placing: ResMut<Placing>,
    mut camera: ResMut<EditorCamera>,
    mut selection: ResMut<super::wmos::Selection>,
    placed: Query<(&WmoPlacement, &Interior)>,
    (hosts, built, buildings): (Query<(Entity, &Host)>, Query<Built>, Query<&ChildOf, With<WmoPlacement>>),
    session: Option<Res<EditSession>>,
    mut commands: Commands,
) {
    view.global = placed.iter().find(|(placement, _)| placement.unique_id == GLOBAL_BUILDING).map(|(placement, _)| placement.path.clone());
    let Some(session) = session else { return };
    let in_editor = super::place::in_building_editor(placing.switch());

    if let Some(id) = view.reselect {
        if *tool != Tool::Wmos || view.open.is_some() || view.asked.is_some() {
            view.reselect = None;
        } else if let Some(at) = super::wmos::find(&session, id) {
            selection.only(Some(at));
            view.reselect = None;
        }
    }

    if let Some(from) = view.asked {
        if *tool != Tool::Wmos || !state.editing() {
            view.asked = None;
        } else if let Some((placement, interior)) = placed.iter().find(|(placement, _)| placement.unique_id == from) {
            view.asked = None;
            let held = view.open.take().map_or_else(|| camera.clone(), |open| open.camera);
            view.open = Some(Viewing {
                path: placement.path.clone(),
                from,
                name_set: interior.name_set,
                tile: super::wmos::find(&session, from).map(|at| at.tile),
                set: placement.doodad_set,
                map: session.map.clone(),
                camera: held,
                framed: false,
            });
            camera.target = ANCHOR;
            camera.wants_the_ground = false;
            let tab = view.tab;
            placing.set_switch(tab);
            selection.only(None);
            return;
        }
    }

    let Some(open) = view.open.as_ref() else {
        for (host, _) in &hosts {
            commands.entity(host).despawn();
        }
        // A mode of the editor with the editor closed is Select.
        if in_editor && view.asked.is_none() {
            placing.set_switch(Switch::Select);
        }
        return;
    };

    let other_map = !open.map.eq_ignore_ascii_case(&session.map);
    if view.leave || *tool != Tool::Wmos || !state.editing() || !in_editor || other_map {
        let open = view.open.take().expect("checked above");
        view.leave = false;
        if in_editor {
            view.tab = placing.switch();
            placing.set_switch(Switch::Select);
        }
        if !other_map {
            if !open.opened_on_the_map() {
                view.reselect = Some(open.from);
            }
            *camera = open.camera;
        }
        for (host, _) in &hosts {
            commands.entity(host).despawn();
        }
        view.coming = None;
        return;
    }

    // The host: spawned when there is none, and built again beside the one
    // drawn when the file, the set or the file's bytes change. See the module
    // comment.
    let open = open.clone();
    let told = session.building_redraw.told;
    let wanted = |host: &Host| host.path == open.path && host.set == open.set && host.told == told;
    if let Some((coming, _)) = view.coming {
        let alive = hosts.get(coming).ok().filter(|(_, host)| wanted(host));
        if alive.is_none() {
            if hosts.get(coming).is_ok() {
                commands.entity(coming).despawn();
            }
            view.coming = None;
        }
    }
    let shown = hosts.iter().any(|(entity, host)| wanted(host) && view.coming.is_none_or(|(coming, _)| coming != entity));
    if !shown && view.coming.is_none() {
        let used: Vec<(u32, u32)> = hosts.iter().map(|(_, host)| host.coord).collect();
        let coord = VIEW_TILES.into_iter().find(|coord| !used.contains(coord)).unwrap_or(VIEW_TILES[0]);
        let placement = PlacedModel {
            path: open.path.clone(),
            unique_id: VIEW_ID,
            position: ANCHOR.to_array(),
            matrix: Mat4::from_translation(ANCHOR).to_cols_array(),
            scale: 1.0,
            doodad_set: open.set,
            name_set: open.name_set,
        };
        let host = vale_client::render::wmos::spawn_held(&mut commands, coord, vec![placement]);
        commands.entity(host).insert(Host { path: open.path.clone(), set: open.set, told, coord });
        if hosts.is_empty() {
            // Nothing drawn to keep: shown at once.
        } else {
            commands.entity(host).insert(Visibility::Hidden);
            view.coming = Some((host, 0));
        }
    }
    if let Some((coming, frames)) = view.coming {
        let placed_here = buildings.iter().any(|parent| parent.parent() == coming);
        let ready = placed_here
            && built.get(coming).is_ok_and(|(wmos, doodads, residents)| wmos.0.is_empty() && doodads.is_empty() && residents.streamed());
        match (ready, frames) {
            (false, _) => view.coming = Some((coming, 0)),
            (true, frames) if frames < 2 => view.coming = Some((coming, frames + 1)),
            (true, _) => {
                commands.entity(coming).insert(Visibility::Inherited);
                for (host, _) in &hosts {
                    if host != coming {
                        commands.entity(host).despawn();
                    }
                }
                view.coming = None;
            }
        }
    }

    // The camera on the building, once it is placed and its size is known.
    if !open.framed {
        if let Some((_, interior)) = placed.iter().find(|(placement, _)| placement.unique_id == VIEW_ID) {
            let [low, high] = interior.bounds;
            let (low, high) = (Vec3::from(low), Vec3::from(high));
            camera.target = ANCHOR + (low + high) / 2.0;
            camera.distance = ((high - low).length() * 0.7).clamp(20.0, 900.0);
            camera.pitch = 0.6;
            camera.wants_the_ground = false;
            if let Some(open) = view.open.as_mut() {
                open.framed = true;
            }
        }
    }
}

/// Escape in Areas mode leaves the editor. Doodads mode has its own, which
/// first puts down a model and lets go of a doodad.
fn escape(
    tool: Res<Tool>,
    placing: Res<Placing>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    mut view: ResMut<WmoView>,
) {
    if *tool == Tool::Wmos
        && view.open.is_some()
        && placing.switch() == Switch::Areas
        && keys.just_pressed(KeyCode::Escape)
        && !wants.wants_keyboard_input()
    {
        view.leave = true;
    }
}

pub(crate) struct WmoViewPlugin;

impl Plugin for WmoViewPlugin {
    fn build(&self, app: &mut App) {
        // The two modes' systems run after `follow`, so they find the
        // building the frame it is opened.
        app.init_resource::<WmoView>().add_systems(Update, (escape, follow).chain().after(crate::pick::aim));
    }
}
