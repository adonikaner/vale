//! The WMO pass: buildings, the rooms inside them, and their furniture.
//!
//! `vale_assets::world::wmo` does the parsing. This module turns a `WmoModel`
//! into entities, and makes three decisions about how.
//!
//! ## A building is drawn through the model material
//!
//! The WebGL renderer did the same: `web/wmos.js` drew buildings with
//! `models.js`' program and shader, and `WmoModel::assemble` shapes a `WmoDraw`
//! like an `M2Batch` so that it can. The two differ in two parameters, not in
//! shader branches: the alpha-key cutoff arrives through its own constant,
//! [`WMO_ALPHA_KEY`] (the same 224/255 as an M2's blend mode 1), and the
//! lighting comes from baked `MOCV` vertex colours rather than from the sun. A
//! draw here therefore goes through [`models::upload_prepared`] like any
//! doodad's.
//!
//! ## One entity per batch, culled per batch
//!
//! `web/wmos.js` culled twice by hand: once per placement against the whole
//! model's bounding sphere, then once per group (a room or a wall).
//! `WmoGroupDraw` owns a contiguous range of the draw list so that the second
//! test is a range check rather than a scan.
//!
//! Here a placement is an entity and each of its batches is a child entity with
//! its own mesh, so Bevy's per-entity frustum cull replaces both tests and is
//! finer than either: a batch is a subset of a group, so its `Aabb` is at least
//! as tight. The group ranges are unused by the cull; `WmoGroupDraw` still says
//! whether a draw is vertex-lit.
//!
//! ## Interior doodads go through the ordinary doodad path
//!
//! `MODS`/`MODN`/`MODD` furniture is the same M2s from the same cache as the
//! trees outside. The only difference is that `WmoDoodad::matrix` is WMO-local.
//! Folding the placement's matrix into it (`wmo::mul4`, which exists for this)
//! turns each spawn into an ordinary `PlacedModel`, so the spawns are handed to
//! [`crate::doodads`] rather than to a second spawner that could diverge from
//! it. They cannot be handed over earlier, because the set of doodads is not
//! known until the WMO has been read: a `MODF` picks one doodad set by index,
//! which is how the same building stands furnished in one place and empty in
//! another.

use crate::axes;
use crate::render::doodads::PendingDoodads;
use crate::world::session::Solids;
use crate::render::terrain::TerrainTile;
use crate::render::models::{self, Materials, ModelDraw, PreparedDraw, RawDraw, RawTexture, RoomLight};
use vale_assets::{
    world::adt::PlacedModel,
    world::blp,
    world::collision::{Collider, CollisionMesh, Solid, SolidId},
    world::wmo::{self, WmoDoodadSet, WmoDraw, WmoModel},
    Assets as Archive,
};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;
use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex};

/// The alpha-key cutoff for a WMO material (blend mode 1).
///
/// 224/255, the same value an M2 uses for blend mode 1. The 1.12.1 client
/// alpha-tests a WMO material and an M2 material with the same per-blend-mode
/// reference, so there is no WMO-specific difference on window lattices or
/// elsewhere; see [`models::M2_ALPHA_KEY`]. The two constants are separate
/// names because the cutoff is a per-material uniform: `Models.applyBlend`
/// takes it as a parameter and so does [`models::upload_prepared`]. They are
/// not two policies.
pub const WMO_ALPHA_KEY: f32 = 224.0 / 255.0;

/// How much of a building to turn into GPU assets in one frame — one unit is
/// one texture or one batch.
///
/// The unit is not a whole building, because a building can be very large:
/// `stormwind.wmo` is 844,627 vertices in 3,042 batches naming 155 textures.
/// With a budget of two buildings a frame, converting all of it in one frame
/// was the largest single hitch a tile crossing produced. The meshes are
/// prebuilt on the loader thread, but the render world still uploads every
/// byte added in a frame. At 256 units that building spreads over ~13 frames
/// and appears all at once when the last batch lands, about a fifth of a
/// second later than in one frame. The tile itself takes seconds to stream
/// in, so the delay is not visible.
const UPLOAD_UNITS: usize = 256;

/// One placed building. Its batches are its children.
#[derive(Component)]
pub struct WmoPlacement {
    /// `MODF`'s id, which is what `vale wmos <Map> <x> <y>` prints.
    pub unique_id: u32,
    /// The archive path this was built from.
    ///
    /// Carried so that a building standing in the world does not age out of
    /// [`WmoCache`]. A placement asks the cache once, when its tile hands it
    /// over, and not again, so without this the idle clock would expire under
    /// a city the player is standing in, and the next tile reload would
    /// re-read the whole of `stormwind.wmo` (311 groups, 844,627 vertices)
    /// instead of hitting the cache. The residency sweep touches these paths,
    /// which is a dozen strings per tile.
    ///
    /// Doodads carry no equivalent: there are twelve thousand of them to a
    /// building's dozen, and re-reading a tree costs milliseconds on a loader
    /// thread, against a whole city's staging for a building.
    pub path: String,
}

/// One drawn batch of one building, for counting.
#[derive(Component)]
pub struct WmoPart;

/// The rooms of one placed building, the light an entity standing in one
/// takes, and which of the game's areas each of its groups is.
///
/// A `MODD` spawn does not need this part of the interior lighting: furniture
/// arrives with its own baked colour and never moves, so it is dressed once. A
/// player walks in and out through the same door, and no file or packet says
/// which side of it they are on, so the test is geometric: the building's own
/// indoor groups, from [`wmo::WmoModel::interior_bounds`].
///
/// The boxes are kept in the building's own frame, and the entity's position
/// is transformed into it. The alternative, transforming eight corners per
/// room into the world and testing against their axis-aligned hull, is looser
/// (a rotated room's hull is bigger than the room) and more work. A `MODF`
/// matrix is a rotation and a uniform scale, so its inverse is exact and costs
/// one transform per building per entity.
#[derive(Component)]
pub struct Interior {
    /// World -> this building's local space, in WoW axes: the entity's
    /// position arrives from the object manager in those, and the group boxes
    /// are in the file's own.
    pub inverse: Mat4,
    /// The whole building, for rejecting the entities that are nowhere near it
    /// before walking three hundred rooms.
    pub bounds: [[f32; 3]; 2],
    /// One box per indoor group, WMO-local.
    pub rooms: Vec<[[f32; 3]; 2]>,
    /// The mean of this placement's own doodad set's baked lights — see
    /// [`wmo::mean_light`].
    pub light: RoomLight,
    /// One box per group of any type, with the id that names the place. See
    /// [`wmo::WmoModel::area_bounds`] and [`vale_assets::tables::wmoarea`].
    ///
    /// A second list rather than a flag on `rooms`, because the two questions
    /// have different answers: Stormwind's districts are `EXTERIOR` groups, so
    /// the room list is empty over most of the city and the area list is not.
    pub areas: Vec<([[f32; 3]; 2], u32)>,
    /// The two keys the area lookup needs beside the group: the building's own
    /// `MOHD` id and this placement's `MODF` name set.
    pub wmo_id: u32,
    pub name_set: u16,
}

impl Interior {
    /// Whether a world point (WoW axes) is within `reach` yards of this
    /// building's whole bounds. This is the cheap per-building test in front
    /// of a batch of [`Self::holds`] calls; the weather uses it to skip every
    /// particle test in open country. A `MODF` placement's scale is uniform,
    /// so a local yard is a world yard and the box can be widened in local
    /// space.
    pub fn near(&self, world: [f32; 3], reach: f32) -> bool {
        let local = self.inverse.transform_point3(Vec3::from(world)).to_array();
        (0..3).all(|axis| {
            local[axis] >= self.bounds[0][axis] - reach
                && local[axis] <= self.bounds[1][axis] + reach
        })
    }

    /// Whether a world point (WoW axes) is inside one of this building's rooms.
    pub fn holds(&self, world: [f32; 3]) -> bool {
        self.holds_within(world, 0.0)
    }

    /// [`Self::holds`] with the boxes widened by `margin` yards, for an object
    /// with extent: a weather mist sprite is up to nine yards across, and a
    /// centre test alone lets its sheet clip through the wall its centre is
    /// just outside of. A `MODF` placement's scale is uniform, so a local yard
    /// is a world yard.
    pub fn holds_within(&self, world: [f32; 3], margin: f32) -> bool {
        let local = self.inverse.transform_point3(Vec3::from(world)).to_array();
        let within = |bounds: &[[f32; 3]; 2]| {
            (0..3).all(|axis| {
                local[axis] >= bounds[0][axis] - margin
                    && local[axis] <= bounds[1][axis] + margin
            })
        };
        within(&self.bounds) && self.rooms.iter().any(within)
    }

    /// Which of this building's groups a world point is in: the first one
    /// whose box holds it, or `None` for a point outside the building.
    ///
    /// First rather than smallest: a city's district boxes overlap and the file
    /// states no priority between them, so picking the tightest would be a rule
    /// this client made up. See [`wmo::WmoModel::area_bounds`].
    pub fn group_at(&self, world: [f32; 3]) -> Option<u32> {
        let local = self.inverse.transform_point3(Vec3::from(world)).to_array();
        if !wmo::box_contains(&self.bounds, local) {
            return None;
        }
        self.areas
            .iter()
            .find(|(bounds, _)| wmo::box_contains(bounds, local))
            .map(|&(_, group)| group)
    }
}

/// The `MODF` placements a tile owns that have not been spawned yet.
///
/// Attached by the terrain pass and drained as each model becomes available,
/// as [`PendingDoodads`] is. Empty is the steady state.
#[derive(Component)]
pub struct PendingWmos(pub Vec<PlacedModel>);

/// A building, loaded once and shared by every placement of it.
pub struct WmoReady {
    draws: Vec<ModelDraw>,
    /// The root's `MOHD` ambient, kept past the material build because it is
    /// also what a `MODD` spawn naming no colour of its own is lit by.
    ambient: [f32; 3],
    doodad_sets: Vec<WmoDoodadSet>,
    /// The lights the building's `MOLT` chunk lists; see
    /// [`vale_assets::world::wmo::WmoLight`]. Kept on the ready model rather
    /// than resolved at load because the position depends on the placement:
    /// the same inn stands in Goldshire and in Menethil.
    lights: Vec<wmo::WmoLight>,
    /// Every `MODD` spawn, in WMO-local space. Sliced per placement by set.
    doodads: Vec<wmo::WmoDoodad>,
    /// The building's solid triangles, in WMO-local space. A different set
    /// from `draws`, for the reasons in [`vale_assets::world::collision`].
    /// Shared across every placement of the model and transformed per
    /// placement.
    collision: Arc<CollisionMesh>,
    /// One box per indoor group, WMO-local. See [`Interior`].
    interior: Vec<[[f32; 3]; 2]>,
    /// One box per group of any type, with its `MOGP` id. See
    /// [`Interior::areas`].
    areas: Vec<([[f32; 3]; 2], u32)>,
    /// `MOHD`'s own id, the first key into `WMOAreaTable`.
    wmo_id: u32,
    /// Every group's box together, as the cheap reject in front of them.
    bounds: [[f32; 3]; 2],
}

impl WmoReady {
    /// The batches, for a placement that is not a `MODF` row. A continent
    /// transport is a building the server spawns and moves, so it has no
    /// placement in any file and cannot go through [`spawn_wmos`]. See
    /// [`crate::render::ships`], which spawns the same batches under an entity
    /// whose transform is written every frame.
    pub fn draws(&self) -> &[ModelDraw] {
        &self.draws
    }

    /// The building's solid triangles, in its own space. Shared, so a caller
    /// placing it somewhere new pays only the transform.
    pub fn collision(&self) -> &Arc<CollisionMesh> {
        &self.collision
    }

    /// The spawns belonging to one `MODF` doodad set.
    ///
    /// Only the named set, as `Wmos.doodadsOf` does; set 0 is not drawn in
    /// addition. That behaviour was compared against the game's interiors, and
    /// this pass ports it.
    fn doodads_in_set(&self, set: u16) -> &[wmo::WmoDoodad] {
        let Some(s) = self.doodad_sets.get(set as usize) else {
            return &[];
        };
        let start = (s.start as usize).min(self.doodads.len());
        let end = start
            .saturating_add(s.count as usize)
            .min(self.doodads.len());
        &self.doodads[start..end]
    }
}

/// What the cache knows about a path.
pub enum Lookup {
    Ready(Arc<WmoReady>),
    Loading,
    Failed,
}

/// Every building that has been asked for, by archive path.
///
/// These are the largest single entries the client holds: `stormwind.wmo`
/// alone is 311 groups and 844,627 vertices. Without eviction, a session that
/// visited three cities held all three, meshes and textures, for the life of
/// the process. [`Self::evict`] drops a building nothing has placed for
/// [`crate::render::residency::IDLE_SECS`], on the same sweep and for the same
/// reason as [`crate::render::models::ModelCache`].
#[derive(Resource, Default)]
pub struct WmoCache {
    ready: HashMap<String, ReadyWmo>,
    failed: HashSet<String>,
    pending: HashSet<String>,
    arrived: Vec<Loaded>,
    /// The building currently being converted to GPU assets, part-way through.
    /// One at a time: the staging exists to bound the per-frame work, and two
    /// buildings in flight would be two budgets.
    staging: Option<Staging>,
    loader: Option<Loader>,
    /// The one magenta placeholder every unfilled texture slot shares. See
    /// [`Self::missing_texture`]; `ModelCache` keeps its own for the same reason.
    missing: Option<Handle<Image>>,
    /// The renderer's clock, for the stamps [`Self::evict`] reads. Written
    /// once a frame by [`crate::render::residency::tick`], for the same reason
    /// `ModelCache` keeps one.
    now: f32,
}

/// A loaded building and when a placement last asked for it.
struct ReadyWmo {
    wmo: Arc<WmoReady>,
    used: f32,
}

impl WmoCache {
    /// The shared magenta texture, built on first use.
    ///
    /// Shared rather than made per slot because a distinct `AssetId` is a
    /// distinct [`crate::render::models::MaterialPool`] key, and two untextured batches
    /// that key apart are two draw calls where one would do.
    fn missing_texture(&mut self, images: &mut Assets<Image>) -> Handle<Image> {
        self.missing
            .get_or_insert_with(|| images.add(models::missing_image()))
            .clone()
    }

    /// Where `path` has got to, requesting it if this is the first time.
    ///
    /// It takes no world position. The water inside a building does not
    /// depend on where the building stands: every liquid surface in the world
    /// takes one colour, resolved where the camera is (see
    /// [`crate::render::water`]). A building cached by path therefore carries
    /// nothing from the place it was first seen.
    pub fn lookup(&mut self, path: &str) -> Lookup {
        if let Some(ready) = self.ready.get_mut(path) {
            ready.used = self.now;
            return Lookup::Ready(Arc::clone(&ready.wmo));
        }
        if self.failed.contains(path) {
            return Lookup::Failed;
        }
        if !self.pending.contains(path) {
            match &self.loader {
                Some(loader) if loader.request(path) => {
                    self.pending.insert(path.to_string());
                }
                // No loader or the thread is gone: fail it here rather than
                // leaving every placement of it waiting forever.
                _ => {
                    self.failed.insert(path.to_string());
                    return Lookup::Failed;
                }
            }
        }
        Lookup::Loading
    }

    /// How many buildings are loaded and how many would not read, for the HUD.
    pub fn counts(&self) -> (usize, usize) {
        (self.ready.len(), self.failed.len())
    }

    /// How many buildings this pass is still working on: requested and not
    /// returned, returned and not staged, and the one part-way through
    /// staging.
    ///
    /// Read by [`crate::glue::loading`] for the same reason as
    /// [`crate::render::terrain::LoadedTiles::settling`]: the ground can be
    /// loaded before the city on it, and taking the loading screen down
    /// between the two would show Stormwind assembling. `Standing::floor`
    /// handles the movement side of the same gap by holding the character's
    /// altitude.
    ///
    /// A building that will not read leaves `pending` for `failed`, so this
    /// reaches zero whatever the archives contain.
    pub fn busy(&self) -> usize {
        self.pending.len() + self.arrived.len() + usize::from(self.staging.is_some())
    }

    /// The renderer's clock. See [`Self::now`].
    pub fn tick(&mut self, now: f32) {
        self.now = now;
    }

    /// Mark a building as wanted now — see [`WmoPlacement::path`].
    pub fn touch(&mut self, path: &str) {
        let now = self.now;
        if let Some(ready) = self.ready.get_mut(path) {
            ready.used = now;
        }
    }

    /// Drop every building nothing has placed since `before`, and answer how
    /// many went.
    ///
    /// Dropping the entry drops this cache's handles on its meshes and images.
    /// Anything still standing in the world holds its own, so a building whose
    /// tile is still loaded is unaffected however long ago the lookup was;
    /// this is why the sweep interval must be shorter than the idle window. A
    /// re-entry costs the same read as the first time: a background thread and
    /// a few frames of staging.
    pub fn evict(&mut self, before: f32) -> usize {
        let count = self.ready.len();
        self.ready.retain(|_, ready| ready.used >= before);
        count - self.ready.len()
    }

    /// Forgets which buildings would not read, on leaving the world.
    ///
    /// [`Self::failed`] stops a placement asking again for something that is
    /// not in the archives. That is correct within a session and wrong across
    /// sessions: nothing evicts the set, so a building that failed to read
    /// once (an archive open that lost a race on a busy login, a loader thread
    /// that had exited) stayed failed for the rest of the process, and every
    /// placement of it drew nothing, with no message. On a map whose whole
    /// geometry is one building, the result is a room with no floor.
    ///
    /// Cleared here rather than on a timer because a session boundary is the
    /// one moment nothing holds a placement, and because logging out and back
    /// in is what a player tries first. Without this, that did not recover the
    /// building either.
    ///
    /// Called by [`crate::render::residency::leave_world`]. Returns how many
    /// were forgotten, for the log line.
    pub fn forget_failures(&mut self) -> usize {
        let count = self.failed.len();
        self.failed.clear();
        count
    }
}

pub struct WmoPlugin;

impl Plugin for WmoPlugin {
    fn build(&self, app: &mut App) {
        // No `MaterialPlugin` here: buildings use the model pass's material, and
        // registering it twice would be two plugins fighting over one asset type.
        app.init_resource::<WmoCache>()
            .add_systems(Startup, start_loader)
            .add_systems(Update, (receive_wmos, spawn_wmos, retire_colliders).chain())
            // Switches the placement rather than the batch, so a building is
            // hidden as a whole. Its `MODD` furniture is not affected: it is a
            // child of the tile and follows the doodad switch with the trees.
            // See [`crate::render::tuning`].
            .add_systems(
                Update,
                crate::render::tuning::switch::<WmoPlacement>(|tuning| tuning.buildings),
            );
    }
}

fn start_loader(mut cache: ResMut<WmoCache>, assets: Res<crate::assets::GameAssets>) {
    cache.loader = Some(Loader::start(assets.gamedata_dir.clone()));
}

// ---------------------------------------------------------------------------
// The loader thread
// ---------------------------------------------------------------------------

struct Loaded {
    path: String,
    model: Option<RawWmo>,
}

struct RawWmo {
    /// Parallel to `WmoModel::textures`, already built into [`Image`]s on the
    /// loader thread — see [`RawTexture::into_image`].
    textures: Vec<Option<Image>>,
    /// Meshes prebuilt on the loader thread; the material half waits for the
    /// main thread, which knows the map and holds the material pool.
    draws: Vec<PreparedDraw>,
    /// The root's `MOHD` ambient, which the shader adds back to the vertex
    /// colours `shaded_colours` took it out of.
    ambient: Vec3,
    doodad_sets: Vec<WmoDoodadSet>,
    lights: Vec<wmo::WmoLight>,
    doodads: Vec<wmo::WmoDoodad>,
    collision: CollisionMesh,
    interior: Vec<[[f32; 3]; 2]>,
    areas: Vec<([[f32; 3]; 2], u32)>,
    wmo_id: u32,
    bounds: [[f32; 3]; 2],
}

/// The thread that reads WMOs, and the two channels to it.
///
/// Its own thread and its own archive chain rather than the model pass's, for
/// the same reason that pass has its own: `Assets::read` needs `&mut`, and a
/// building is dozens of files (a root plus one per group), so a WMO load and a
/// tile's 130 doodads should not queue behind each other. Both ends are behind a
/// `Mutex` because a Bevy resource must be `Sync`; neither is ever contended.
struct Loader {
    requests: Mutex<Sender<String>>,
    results: Mutex<Receiver<Loaded>>,
}

impl Loader {
    fn start(gamedata_dir: String) -> Loader {
        let (request_tx, request_rx) = mpsc::channel::<String>();
        let (result_tx, result_rx) = mpsc::channel::<Loaded>();
        std::thread::Builder::new()
            .name("wmo-loader".into())
            .spawn(move || {
                let mut archive = match Archive::open(&gamedata_dir) {
                    Ok(archive) => Some(archive),
                    Err(e) => {
                        error!("wmo loader: {e}");
                        None
                    }
                };
                // Every request gets an answer even with no archive, so the cache
                // marks those failed rather than pending forever.
                while let Ok(path) = request_rx.recv() {
                    let model = archive.as_mut().and_then(|a| read_wmo(a, &path));
                    if model.is_none() {
                        warn!("wmo {path} will not read");
                    }
                    if result_tx.send(Loaded { path, model }).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn the wmo loader thread");
        Loader {
            requests: Mutex::new(request_tx),
            results: Mutex::new(result_rx),
        }
    }

    fn request(&self, path: &str) -> bool {
        let tx = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        tx.send(path.to_string()).is_ok()
    }

    fn drain(&self) -> Vec<Loaded> {
        let rx = self.results.lock().unwrap_or_else(|e| e.into_inner());
        rx.try_iter().collect()
    }
}

/// One WMO (root, groups and textures), decoded.
///
/// A group that will not read loses that room, not the whole building.
/// `wmo::load` already skips such groups and reports which, so they are logged
/// once here rather than once per placement.
fn read_wmo(archive: &mut Archive, path: &str) -> Option<RawWmo> {
    let (model, failed_groups) = wmo::load(archive, path).ok()?;
    for failure in &failed_groups {
        warn!("wmo {path}: {failure}");
    }

    let textures = model
        .textures
        .iter()
        .map(|name| {
            let raw = archive.read(name).ok()?;
            Some(RawTexture::from_blp(blp::decode_mipped(&raw).ok()?).into_image())
        })
        .collect();

    let draws = model
        .draws
        .iter()
        .filter_map(|draw| batch_draw(&model, draw))
        // Meshes are built here, on the loader thread, so the main thread
        // does not copy a city's 3,042 batches.
        .map(PreparedDraw::from_raw)
        .collect();

    Some(RawWmo {
        textures,
        draws,
        ambient: Vec3::from_array(model.ambient),
        doodad_sets: model.doodad_sets.clone(),
        lights: model.lights.clone(),
        doodads: model.doodads.clone(),
        // Where the inside of this building is. Read here with the geometry
        // because the groups it comes from are dropped when `wmo::load`
        // returns, as the collision hull is.
        interior: model.interior_bounds(),
        // Where its areas are, which is a different subset of the same
        // groups. See `Interior::areas`.
        areas: model.area_bounds(),
        wmo_id: model.wmo_id,
        bounds: model.bounds,
        // Built on this thread with the geometry, because it is the same parse:
        // `wmo::load` reads `MOPY` alongside `MOVT`, and the groups it read them
        // from are dropped when it returns.
        collision: model.collision.clone(),
    })
}

/// One `WmoDraw` as its own vertex buffer, in Bevy's axes.
///
/// The same remap the terrain groups and the M2 batches get, for the same
/// reason: the batch carries only the vertices it uses, so its `Aabb` is tight
/// and Bevy's cull does useful work on it.
///
/// The winding is not changed, as an M2's is not. `MOVT` vertices are in the
/// same model space as an M2's: vmangos writes M2 vertices through
/// `fixCoordSystem` and straight back through its inverse and writes `MOVT`
/// raw, so both reach `ModelInstance` in the file's own axes, and `models.js`
/// drew both with `CULL_FACE` on and the default front face. A change of basis
/// is a rotation and cannot reverse the winding.
fn batch_draw(model: &WmoModel, draw: &WmoDraw) -> Option<RawDraw> {
    let range = draw.index_start as usize..(draw.index_start + draw.index_count) as usize;
    let indices = model.indices.get(range)?;
    if indices.is_empty() {
        return None;
    }
    // A batch pointing past the buffers is dropped rather than panicked on.
    let vertices = model
        .positions
        .len()
        .min(model.normals.len())
        .min(model.uvs.len())
        .min(model.colours.len());
    if indices.iter().any(|&i| i as usize >= vertices) {
        return None;
    }

    let mut remap: HashMap<u32, u32> = HashMap::default();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut colours: Vec<[f32; 4]> = Vec::new();
    let mut out: Vec<u32> = Vec::with_capacity(indices.len());

    for &index in indices {
        let next = remap.len() as u32;
        let mapped = *remap.entry(index).or_insert_with(|| {
            let i = index as usize;
            positions.push(axes::to_bevy(model.positions[i]).to_array());
            normals.push(axes::to_bevy(model.normals[i]).to_array());
            uvs.push(model.uvs[i]);
            // The shaded `MOCV` light, normalised. Kept as stored rather than
            // sRGB-decoded, because MODEL_FRAG used the byte value unchanged
            // and this is a port of it. `vale wmos` reports the resulting
            // range (Darkshire: mean peak channel 95/255, nothing saturated),
            // so the choice can be checked against the files.
            colours.push(model.colours[i].map(|c| c as f32 / 255.0));
            next
        });
        out.push(mapped);
    }

    // Only vertex-lit batches need the attribute, and its presence is what
    // compiles the branch in `m2.wgsl`, so a sunlit exterior wall would
    // otherwise pay for a colour it never reads.
    //
    // A liquid surface keeps it for the alpha. Its RGB is black and it is lit
    // by the sun like any exterior, but the alpha is `MLIQ`'s per-vertex depth
    // and is what makes the water visible. Clearing it here left the canals
    // blended by `lake_a`'s foam mask.
    if draw.light == vale_assets::world::wmo::BatchLight::Sun && draw.liquid.is_none() {
        colours.clear();
    }

    Some(RawDraw {
        positions,
        normals,
        uvs,
        colours,
        indices: out,
        // A building has no skeleton — `MODD` furniture animates as ordinary
        // doodads, and the walls do not move.
        joints: Vec::new(),
        weights: Vec::new(),
        // No skeleton, so no subset — see [`RawDraw::bones`].
        bones: Vec::new(),
        // …and no folded layers: an environment map over the same triangles
        // is an M2 authoring pattern. See [`RawDraw::overlays`].
        overlays: Default::default(),
        // A building has no appearance variants either: every batch of a group
        // is drawn, so they all sit in the one geoset a creature's body does.
        geoset: 0,
        texture: draw.texture.map(|t| t as usize),
        // The same table an M2 material uses. Anything outside it falls back to
        // ordinary alpha blending, which is what `specialize` does with it too.
        blend: u16::try_from(draw.blend).unwrap_or(2),
        unlit: draw.unlit,
        two_sided: draw.two_sided,
        // A WMO material has no equivalent of M2 flag 0x10.
        no_depth_write: false,
        light: draw.light,
        liquid: draw.liquid,
        // `M2Color` is an M2 block; masonry does not fade.
        tint: None,
        baked_tint: None,
        // A WMO's textures do not move: `M2TextureTransform` is an M2 block.
        uv: None,
        // A ground quad is an M2 authoring pattern; masonry is never one.
        ground: None,
    })
}

// ---------------------------------------------------------------------------
// Decoded building -> Bevy assets
// ---------------------------------------------------------------------------

/// A building part-way through becoming GPU assets.
///
/// The meshes arrive prebuilt from the loader thread, so what is left here is
/// `Assets::add` and the materials. The render world still uploads every byte
/// added in a frame, so this part is budgeted too. A building spawns nowhere
/// until its last batch lands: `WmoReady` is inserted whole, so `spawn_wmos`
/// never sees a partly staged building.
struct Staging {
    path: String,
    /// Handles for the slots already added, parallel to the model's texture
    /// list as [`Self::pending_textures`] drains into it.
    textures: Vec<Handle<Image>>,
    pending_textures: VecDeque<Option<Image>>,
    draws: Vec<ModelDraw>,
    pending_draws: VecDeque<PreparedDraw>,
    ambient: Vec3,
    doodad_sets: Vec<WmoDoodadSet>,
    lights: Vec<wmo::WmoLight>,
    doodads: Vec<wmo::WmoDoodad>,
    collision: CollisionMesh,
    interior: Vec<[[f32; 3]; 2]>,
    areas: Vec<([[f32; 3]; 2], u32)>,
    wmo_id: u32,
    bounds: [[f32; 3]; 2],
}

impl Staging {
    fn new(path: String, raw: RawWmo) -> Staging {
        Staging {
            path,
            textures: Vec::with_capacity(raw.textures.len()),
            pending_textures: raw.textures.into(),
            draws: Vec::with_capacity(raw.draws.len()),
            pending_draws: raw.draws.into(),
            ambient: raw.ambient,
            doodad_sets: raw.doodad_sets,
            lights: raw.lights,
            doodads: raw.doodads,
            collision: raw.collision,
            interior: raw.interior,
            areas: raw.areas,
            wmo_id: raw.wmo_id,
            bounds: raw.bounds,
        }
    }
}

/// Turn finished WMO loads into meshes, images and materials, up to
/// [`UPLOAD_UNITS`] of work a frame.
fn receive_wmos(
    mut cache: ResMut<WmoCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: Materials,
    // The water's colour. `lake_a` and `ocean_h` carry no colour of their own
    // (see `vale_assets::tables::light`), so an `MLIQ` surface is tinted from
    // `Light.dbc`, and all water in the world takes one colour, resolved where
    // the camera is. `Option` because the headless harnesses build worlds with
    // no render plugins. See [`crate::render::water`], which also keeps the
    // colour current afterwards: a building is cached by path and its batches
    // outlive the place they were first seen from.
    palette: Option<Res<crate::render::water::LiquidPalette>>,
    // The liquid flipbook, for the same reason and on the same terms: a canal
    // takes the same thirty frames a lake does, from one shared set.
    flipbook: Option<Res<crate::render::water::LiquidFlipbook>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Wmos);
    if let Some(loader) = &cache.loader {
        let arrived = loader.drain();
        cache.arrived.extend(arrived);
    }
    if cache.arrived.is_empty() && cache.staging.is_none() {
        return;
    }

    // One magenta image for the whole pass, not one per unfilled slot. A
    // fresh `images.add` per slot is a different `AssetId`, so two otherwise
    // identical materials would key differently and land in separate batch
    // sets, which is the cost `MaterialPool` exists to remove. It would also
    // be a texture upload per missing slot.
    let missing = cache.missing_texture(&mut images);

    let mut budget = UPLOAD_UNITS;
    while budget > 0 {
        if cache.staging.is_none() {
            if cache.arrived.is_empty() {
                break;
            }
            let loaded = cache.arrived.remove(0);
            cache.pending.remove(&loaded.path);
            let Some(raw) = loaded.model else {
                cache.failed.insert(loaded.path);
                continue;
            };
            cache.staging = Some(Staging::new(loaded.path, raw));
        }
        let staging = cache.staging.as_mut().expect("set above");

        // Textures first: a draw's material needs its handle, and the slots are
        // indexed, so the whole list has to exist before any draw does.
        while budget > 0 {
            let Some(texture) = staging.pending_textures.pop_front() else {
                break;
            };
            staging.textures.push(match texture {
                Some(image) => images.add(image),
                None => missing.clone(),
            });
            budget -= 1;
        }
        if !staging.pending_textures.is_empty() {
            break;
        }

        while budget > 0 {
            let Some(draw) = staging.pending_draws.pop_front() else {
                break;
            };
            // A liquid batch does not use the building's texture, although
            // `WmoModel::add_liquid` names one in `MOTX` so that the file stays
            // self-describing. It uses the shared flipbook's anchor frame,
            // which puts a canal and a lake on one material and keeps them
            // animating together. The anchor frame keys the material; it is
            // not the frame shown. See
            // [`crate::render::water::LiquidFlipbook::anchor`] for the cost of
            // keying a material on the wall clock.
            let texture = draw
                .liquid
                .and_then(|kind| flipbook.as_ref()?.anchor(kind))
                .or_else(|| draw.texture.and_then(|i| staging.textures.get(i).cloned()))
                .unwrap_or_else(|| missing.clone());
            let liquid = draw
                .liquid
                .and_then(|kind| palette.as_ref().and_then(|p| p.tint(kind)));
            let uploaded = models::upload_prepared(
                draw,
                texture,
                WMO_ALPHA_KEY,
                staging.ambient,
                liquid,
                &mut meshes,
                &mut materials,
            );
            staging.draws.push(uploaded);
            budget -= 1;
        }
        if !staging.pending_draws.is_empty() {
            break;
        }

        // Every batch is in the store: the building becomes visible to
        // `spawn_wmos`, whole.
        let staging = cache.staging.take().expect("still set");
        let used = cache.now;
        cache.ready.insert(
            staging.path,
            ReadyWmo {
                // Freshly arrived: it survives the next sweep whatever the
                // placement that asked for it has done since.
                used,
                wmo: Arc::new(WmoReady {
                    draws: staging.draws,
                    ambient: staging.ambient.to_array(),
                    doodad_sets: staging.doodad_sets,
                    lights: staging.lights,
                    doodads: staging.doodads,
                    collision: Arc::new(staging.collision),
                    interior: staging.interior,
                    areas: staging.areas,
                    wmo_id: staging.wmo_id,
                    bounds: staging.bounds,
                }),
            },
        );
    }
}

/// Spawn whatever placements have a building ready.
///
/// No frame budget: a tile holds a dozen buildings, not twelve hundred trees,
/// and the expensive part is [`receive_wmos`], which has one.
///
/// Public so the doodad pass can order itself after it. A building's interior
/// `MODD` furniture is handed over through the tile's `PendingDoodads`, and
/// without an explicit order that hand-over is sequenced only by the two
/// systems' conflicting access to that component. See `doodads::DoodadPlugin`.
pub fn spawn_wmos(
    mut commands: Commands,
    mut cache: ResMut<WmoCache>,
    mut tiles: Query<(Entity, &TerrainTile, &mut PendingWmos, &mut PendingDoodads)>,
    focus: Res<crate::render::focus::WorldFocus>,
    solids: Res<Solids>,
    // The terrain cache the mover reads, for the baked shadow under an
    // exterior prop. Absent with no session, where every prop counts as lit.
    session: Option<Res<crate::world::session::Session>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Wmos);
    let active = session.as_ref().and_then(|s| s.active.as_ref());
    if !focus.present {
        return;
    }
    let map_id = focus.map_id;
    for (tile, coord, mut pending, mut tile_doodads) in &mut tiles {
        let coord = coord.coord;
        pending.0.retain(|placement| {
            // Every path out of this closure except `Loading` settles the
            // placement. The mover refuses to stand on the ground under a
            // `MODF` box it has no answer for (`Standing::floor`), so a
            // building that has nothing solid in it, or that will not load,
            // must be settled here, or it is a permanent hole nobody can land
            // in. A hull that is still coming settles when it arrives, inside
            // `CollisionWorld::insert`.
            let ready = match cache.lookup(&placement.path) {
                Lookup::Ready(ready) => ready,
                // Still reading it: keep the placement and try next frame.
                // This is the only state that means "not yet".
                Lookup::Loading => return true,
                // Reported once by the loader.
                Lookup::Failed => {
                    solids.0.settle(map_id, coord, placement.unique_id);
                    return false;
                }
            };
            if ready.draws.is_empty() || ready.collision.is_empty() {
                solids.0.settle(map_id, coord, placement.unique_id);
                if ready.draws.is_empty() {
                    return false;
                }
            }

            // Composed in the file's own frame, including the 180° term (which
            // `vale wmos` matches to the game's own `MODF` boxes to 0.00
            // yards), and conjugated once here. A rotation and a uniform scale,
            // so the decomposition into a `Transform` is exact.
            let placement_matrix = Mat4::from_cols_array(&placement.matrix);
            // The rooms of this placement and the light they give. A
            // component of the building, which is a child of the tile, so
            // walking out of range retires it with the building. An interior
            // left behind would be a lit room standing in an empty field: the
            // same failure `retire_colliders` prevents for colliders.
            let interior = Interior {
                inverse: placement_matrix.inverse(),
                bounds: ready.bounds,
                rooms: ready.interior.clone(),
                light: RoomLight::new(wmo::mean_light(
                    ready.doodads_in_set(placement.doodad_set),
                    ready.ambient,
                )),
                // Which places this building's groups are. This depends on the
                // placement, not only the model: the same inn stands in
                // Goldshire and in Menethil, and only the `MODF` name set says
                // which of them this one is. See `interface::worldmap`, which
                // reads it.
                areas: ready.areas.clone(),
                wmo_id: ready.wmo_id,
                name_set: placement.name_set,
            };
            let transform = Transform::from_matrix(axes::to_bevy_affine(placement_matrix));
            // The lights the building's `MOLT` chunk lists, resolved to world
            // space once here because a building does not move. `None` for a
            // barn and ten for the Goldshire inn. See `render::lamps`. The
            // 1.12.1 client does not use these records; lighting from them is
            // a deviation of this client.
            let lamps = crate::render::lamps::StaticLamps::of_building(&ready.lights, &transform);
            let building = commands
                .spawn((
                    WmoPlacement {
                        unique_id: placement.unique_id,
                        path: placement.path.clone(),
                    },
                    interior,
                    transform,
                    Visibility::default(),
                    // A child of the tile, so walking out of range takes the
                    // building and its furniture with the ground it stands on.
                    ChildOf(tile),
                ))
                .id();
            if let Some(lamps) = lamps {
                commands.entity(building).insert(lamps);
            }

            for draw in &ready.draws {
                let batch = commands
                    .spawn((
                        WmoPart,
                        Mesh3d(draw.mesh.clone()),
                        MeshMaterial3d(draw.material.clone()),
                        ChildOf(building),
                    ))
                    .id();
                // A building's `MLIQ` batches carry the terrain pass's water
                // marker, so that one switch reaches every liquid surface on
                // screen. See [`crate::render::tuning::WorldTuning::water`].
                if let Some(kind) = draw.liquid {
                    commands
                        .entity(batch)
                        .insert(crate::render::terrain::TerrainWater(kind));
                }
            }

            // The collision hull. The matrix is only known here (the loader
            // has the model, this system has the placement), but the
            // transform runs on the compute pool: `Collider::place` walks
            // every solid triangle, and Stormwind's five placements cost
            // 27.6 ms (measured by `vale collision Azeroth 31 48`), a dropped
            // frame on the main thread when a city arrives. The task inserts
            // directly into [`Solids`], which is an `Arc` around a lock the
            // session thread already shares. A task that completes after its
            // tile has been unloaded leaves a hull that `retire_colliders`
            // removes on its next pass; that check runs every frame so that a
            // stale collider does not persist.
            if !ready.collision.is_empty() {
                let collision = Arc::clone(&ready.collision);
                let matrix = placement.matrix;
                let id = SolidId::placement(placement.unique_id);
                let solids = Arc::clone(&solids.0);
                AsyncComputeTaskPool::get()
                    .spawn(async move {
                        let collider = Collider::place(&collision, &matrix);
                        solids.insert(map_id, coord, id, Solid::Building, Arc::new(collider));
                    })
                    .detach();
            }

            // The furniture, folded into world space and handed to the doodad
            // pass; see the module doc. This is the earliest it can happen:
            // the set of doodads is not known until the model has arrived.
            tile_doodads.indoor.extend(
                ready
                    .doodads_in_set(placement.doodad_set)
                    .iter()
                    .filter(|d| d.drawable())
                    .map(|d| {
                        let placed = interior_placement(
                            placement,
                            &wmo::mul4(&placement.matrix, &d.matrix),
                            &d.path,
                        );
                        // A prop referenced only by exterior groups is lit by
                        // the sun, and its sun scale is the terrain doodad's:
                        // 0.5 over the ground's baked shadow, otherwise 1.0. A
                        // prop in a room is lit by the spawn's own colour, or
                        // the building's `MOHD` ambient when the spawn names
                        // none, and takes no sun (see `WmoDoodad::light` and
                        // `WmoDoodad::exterior_lit`).
                        let (light, sun_scale) = if d.exterior_lit {
                            let shadowed = active.is_some_and(|a| {
                                a.terrain_shadowed(map_id, placed.position[0], placed.position[1])
                            });
                            (None, crate::render::doodads::doodad_sun_scale(shadowed))
                        } else {
                            (
                                Some(RoomLight::new(d.light().unwrap_or(ready.ambient))),
                                models::sun_scale::NEUTRAL,
                            )
                        };
                        crate::render::doodads::PlacedDoodad {
                            placement: placed,
                            light,
                            sun_scale,
                        }
                    }),
            );
            false
        });
    }
}

/// Drop the collision hulls of tiles that have walked out of range.
///
/// Keyed off the tiles that still exist rather than off a despawn hook,
/// because a building is a child of its tile and Bevy's recursive despawn does
/// not tell this pass which children went with it. A collider left behind is
/// an invisible wall standing in an empty field. Nine coordinates a frame
/// with an early-out when nothing has changed, which is the steady state.
///
/// The hulls of the tiles that went are freed on the task pool: a block of
/// them is every triangle of every building and doodad on it, and freeing them
/// here took up to 19 ms of a frame. See `CollisionWorld::retain_tiles`.
fn retire_colliders(tiles: Query<&TerrainTile>, solids: Res<Solids>) {
    let live: std::collections::HashSet<(u32, u32)> = tiles.iter().map(|t| t.coord).collect();
    let retired = solids.0.retain_tiles(&live);
    if !retired.is_empty() {
        AsyncComputeTaskPool::get().spawn(async move { drop(retired) }).detach();
    }
}

/// One interior doodad as an ordinary [`PlacedModel`].
///
/// `matrix` is already the product of the building's placement and the `MODD`
/// spawn's own, so this is only the packaging. The `unique_id` is the building's:
/// a `MODD` spawn has none of its own, and the id is used for reporting rather
/// than for the seam deduplication an `MDDF` needs it for (a doodad inside a
/// building is claimed by the tile that claimed the building).
fn interior_placement(placement: &PlacedModel, matrix: &[f32; 16], path: &str) -> PlacedModel {
    PlacedModel {
        path: path.to_string(),
        unique_id: placement.unique_id,
        // The translation column, which is where the composed matrix puts it.
        position: [matrix[12], matrix[13], matrix[14]],
        matrix: *matrix,
        scale: 1.0,
        doodad_set: 0,
        // Both unused for an M2: a doodad has no groups and no name sets.
        name_set: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::wmo::{doodad_matrix, WmoDoodad};

    /// A model with two triangles over four vertices, and one `MOCV` colour each.
    /// Whether a batch is vertex-lit belongs to the `WmoDraw`, not here.
    fn sample() -> WmoModel {
        WmoModel {
            path: "test.wmo".into(),
            wmo_id: 0,
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0]; 4],
            colours: vec![[10, 20, 30, 255], [40, 50, 60, 255], [0, 0, 0, 255], [255; 4]],
            indices: vec![0, 1, 2, 0, 2, 3],
            textures: vec!["t.blp".into()],
            ambient: [0.2, 0.2, 0.2],
            collision: CollisionMesh::default(),
            draws: Vec::new(),
            groups: Vec::new(),
            doodad_sets: Vec::new(),
            lights: Vec::new(),
            doodads: Vec::new(),
            bounds: [[0.0; 3], [1.0; 3]],
            local_bounds: [[0.0; 3], [1.0; 3]],
            radius: 1.0,
        }
    }

    fn draw(index_start: u32, index_count: u32, vertex_lit: bool) -> WmoDraw {
        let light = if vertex_lit {
            vale_assets::world::wmo::BatchLight::Bake
        } else {
            vale_assets::world::wmo::BatchLight::Sun
        };
        WmoDraw {
            group: 0,
            index_start,
            index_count,
            texture: Some(0),
            blend: 0,
            unlit: false,
            two_sided: false,
            light,
            section: vale_assets::world::wmo::BatchSection::Ext,
            liquid: None,
        }
    }

    /// Splitting a building into per-batch meshes must not lose a triangle, and a
    /// batch must carry only the vertices it uses. That is what makes its
    /// bounding box, and therefore Bevy's cull, tighter than the per-group cull
    /// it replaces.
    #[test]
    fn a_batch_keeps_its_triangles_and_only_its_vertices() {
        let model = sample();
        let whole = batch_draw(&model, &draw(0, 6, true)).expect("a draw");
        assert_eq!(whole.indices.len(), 6);
        assert_eq!(whole.positions.len(), 4);

        let half = batch_draw(&model, &draw(0, 3, true)).expect("a draw");
        assert_eq!(half.indices.len(), 3);
        assert_eq!(half.positions.len(), 3);
    }

    /// The colours stay parallel to the positions through the remap.
    /// `WmoModel::assemble` emits one colour per vertex even for groups with no
    /// `MOCV`, so that this holds across the group concatenation. A remap that
    /// dropped or reordered them would light each vertex with another vertex's
    /// light, which renders as a plausibly lit room rather than as an error.
    #[test]
    fn the_baked_light_follows_its_own_vertex() {
        let model = sample();
        let batch = batch_draw(&model, &draw(3, 3, true)).expect("a draw");
        assert_eq!(batch.colours.len(), batch.positions.len());

        // Indices 0, 2, 3 of the source, in the order the remap first met them.
        for (slot, source) in [0usize, 2, 3].into_iter().enumerate() {
            let expected = model.colours[source].map(|c| c as f32 / 255.0);
            assert_eq!(
                batch.colours[slot], expected,
                "vertex {slot} carries the wrong vertex's light"
            );
        }
    }

    /// A batch the sun lights gets no colour attribute at all, not a black one:
    /// the attribute's presence is what compiles the vertex-lit branch, so an
    /// exterior wall would otherwise pay for a lookup it never uses.
    #[test]
    fn a_sunlit_batch_carries_no_baked_light() {
        let model = sample();
        let batch = batch_draw(&model, &draw(0, 6, false)).expect("a draw");
        assert!(batch.colours.is_empty());
        assert_eq!(batch.light, vale_assets::world::wmo::BatchLight::Sun);
    }

    /// A liquid batch keeps its colour attribute. A liquid surface is not
    /// vertex-lit (the sun lights water), so it once fell into the same branch
    /// as a sunlit exterior wall and had its colour attribute cleared, which
    /// left Stormwind's canals empty. That alpha is `MLIQ`'s depth, and the
    /// fragment shader mixes the shallow and deep opacities with it; without
    /// it the whole surface draws at the bank's alpha.
    #[test]
    fn a_liquid_batch_keeps_its_colours_although_it_is_not_vertex_lit() {
        let model = sample();
        let mut liquid = draw(0, 6, false);
        liquid.liquid = Some(vale_assets::world::wmo::Liquid::Water);

        let batch = batch_draw(&model, &liquid).expect("a draw");
        assert_eq!(batch.liquid, Some(vale_assets::world::wmo::Liquid::Water));
        assert_eq!(batch.light, vale_assets::world::wmo::BatchLight::Sun, "water is lit by the sun");
        assert_eq!(
            batch.colours.len(),
            batch.positions.len(),
            "the alpha is the opacity and has to survive"
        );
        assert_eq!(batch.colours[0][3], 1.0, "255 in the file is fully opaque");

        // And a batch that is neither still pays for nothing.
        let wall = batch_draw(&model, &draw(0, 6, false)).expect("a draw");
        assert!(wall.colours.is_empty());
    }

    #[test]
    fn a_batch_outside_its_buffers_is_dropped() {
        let model = sample();
        assert!(batch_draw(&model, &draw(0, 0, true)).is_none());
        assert!(batch_draw(&model, &draw(4, 6, true)).is_none());
    }

    /// The winding survives the change of basis, as an M2's does, for the same
    /// reason: `MOVT` vertices are in the same model space. Bevy back-face
    /// culls by default, so a wrong winding turns the surface inside out, as it
    /// once did for the terrain.
    #[test]
    fn a_batch_keeps_the_winding_the_file_gave_it() {
        let model = sample();
        let batch = batch_draw(&model, &draw(0, 3, true)).expect("a draw");
        let p: Vec<Vec3> = batch
            .indices
            .iter()
            .map(|&i| Vec3::from_array(batch.positions[i as usize]))
            .collect();
        let geometric = (p[1] - p[0]).cross(p[2] - p[0]);
        let shading = axes::to_bevy(model.normals[0]);
        assert!(
            geometric.dot(shading) > 0.0,
            "the batch was reversed: {geometric:?} vs {shading:?}"
        );
    }

    /// A `MODD` spawn's matrix is WMO-local, so a table inside a building is
    /// placed by the placement's matrix times its own. Multiplying in the wrong
    /// order puts the furniture at plausible world coordinates elsewhere on the
    /// map, so this is checked against the point transformed one matrix at a
    /// time rather than by eye.
    #[test]
    fn a_doodad_inside_a_building_lands_where_both_matrices_say() {
        let building = vale_assets::world::adt::placement_matrix(
            vale_assets::world::adt::placement_to_world([1000.0, 50.0, 2000.0]),
            [0.0, 60.0, 0.0],
            1.0,
        );
        // A table three yards along the building's own X, tilted.
        let local = doodad_matrix([3.0, 0.0, 0.0], [0.0, 0.0, 0.383, 0.924], 1.0);

        let placement = PlacedModel {
            path: "b.wmo".into(),
            unique_id: 7,
            position: [0.0; 3],
            matrix: building,
            scale: 1.0,
            doodad_set: 0,
            name_set: 0,
        };
        let composed = wmo::mul4(&building, &local);
        let spawned = interior_placement(&placement, &composed, "t.m2");

        // The doodad's own origin, taken the long way: local space -> WMO space
        // -> world, each step by its own matrix.
        let in_wmo = Mat4::from_cols_array(&local).transform_point3(Vec3::ZERO);
        let in_world = Mat4::from_cols_array(&building).transform_point3(in_wmo);
        let via_composed = Mat4::from_cols_array(&spawned.matrix).transform_point3(Vec3::ZERO);
        assert!(
            (via_composed - in_world).length() < 1e-3,
            "{via_composed:?} != {in_world:?}"
        );

        // And the packaged `position` is that same point, since the doodad pass
        // and the HUD both read it.
        assert!(
            (Vec3::from_array(spawned.position) - in_world).length() < 1e-3,
            "position {:?} is not the matrix's translation",
            spawned.position
        );
    }

    /// The interior test in the building's own frame, checked against points
    /// transformed forward through the placement. This is the same check the
    /// `MODD` matrix gets in the test above, for the same reason: an inverse
    /// applied the wrong way round puts the room at plausible coordinates
    /// elsewhere on the map, and every count stays right.
    #[test]
    fn a_room_holds_the_points_its_own_matrix_says_it_does() {
        // A building 1,000 yards out, turned 60 degrees, with one room ten
        // yards square and five high sitting at its own origin.
        let matrix = vale_assets::world::adt::placement_matrix(
            vale_assets::world::adt::placement_to_world([1000.0, 50.0, 2000.0]),
            [0.0, 60.0, 0.0],
            1.0,
        );
        let room = [[-5.0, -5.0, 0.0], [5.0, 5.0, 5.0]];
        let interior = Interior {
            inverse: Mat4::from_cols_array(&matrix).inverse(),
            bounds: room,
            rooms: vec![room],
            light: RoomLight::new([0.5, 0.5, 0.5]),
            areas: vec![(room, 3558)],
            wmo_id: 208,
            name_set: 0,
        };

        // A point taken forward through the placement from inside the room
        // must test as inside; one from outside it must not. Neither is a
        // coordinate that can be checked by eye, which is why the test works
        // in this direction.
        let world = |local: [f32; 3]| {
            Mat4::from_cols_array(&matrix)
                .transform_point3(Vec3::from(local))
                .to_array()
        };
        assert!(interior.holds(world([0.0, 0.0, 1.0])), "the middle of the room");
        // Not on the floor: a point round-tripped through a rotation lands a
        // few billionths off, and the box's own bottom face is at z = 0. The
        // exact-boundary case is pinned in the assets crate, where there is no
        // matrix in the way.
        assert!(interior.holds(world([4.9, -4.9, 0.5])), "a corner of the room");
        assert!(!interior.holds(world([0.0, 0.0, 6.0])), "above the ceiling");
        assert!(!interior.holds(world([6.0, 0.0, 1.0])), "through the wall");

        // And the world point itself is not the local one: a test that had
        // forgotten the inverse would pass everything above by accident if the
        // building stood at the origin.
        assert!(
            !interior.holds([0.0, 0.0, 1.0]),
            "the building is a thousand yards from the origin"
        );
    }

    /// The area test is the same transform over a different set of boxes, and
    /// answers which group rather than yes or no.
    ///
    /// The two lists differ by design: a city's districts are `EXTERIOR`
    /// groups, so `rooms` is empty where `areas` is not. A building with no
    /// indoor group still names the place a point is in.
    #[test]
    fn a_group_names_the_place_a_point_is_standing_in() {
        let matrix = vale_assets::world::adt::placement_matrix(
            vale_assets::world::adt::placement_to_world([1000.0, 50.0, 2000.0]),
            [0.0, 60.0, 0.0],
            1.0,
        );
        let forge = [[-5.0, -5.0, 0.0], [0.0, 5.0, 5.0]];
        let commons = [[0.0, -5.0, 0.0], [5.0, 5.0, 5.0]];
        let interior = Interior {
            inverse: Mat4::from_cols_array(&matrix).inverse(),
            bounds: [[-5.0, -5.0, 0.0], [5.0, 5.0, 5.0]],
            // No indoor group at all, which is the Stormwind case.
            rooms: Vec::new(),
            light: RoomLight::new([0.5, 0.5, 0.5]),
            areas: vec![(forge, 3558), (commons, 3559)],
            wmo_id: 208,
            name_set: 0,
        };
        let world = |local: [f32; 3]| {
            Mat4::from_cols_array(&matrix)
                .transform_point3(Vec3::from(local))
                .to_array()
        };
        assert_eq!(interior.group_at(world([-2.0, 0.0, 1.0])), Some(3558));
        assert_eq!(interior.group_at(world([2.0, 0.0, 1.0])), Some(3559));
        assert_eq!(interior.group_at(world([0.0, 0.0, 9.0])), None, "over the roof");
        assert!(!interior.holds(world([-2.0, 0.0, 1.0])), "and it is not a room");
        // The world point untransformed lands nowhere, which is the check that
        // the inverse is really being applied.
        assert_eq!(interior.group_at([-2.0, 0.0, 1.0]), None);
    }

    /// A building with no indoor group holds nobody. Most of the world's WMOs
    /// are such buildings (bridges, walls, wall posts).
    #[test]
    fn a_building_with_no_rooms_lights_nothing() {
        let interior = Interior {
            inverse: Mat4::IDENTITY,
            bounds: [[-10.0; 3], [10.0; 3]],
            rooms: Vec::new(),
            light: RoomLight::new([1.0, 1.0, 1.0]),
            areas: Vec::new(),
            wmo_id: 0,
            name_set: 0,
        };
        assert!(!interior.holds([0.0, 0.0, 0.0]), "inside the box, in no room");
    }

    /// A `MODF` names one doodad set and gets that set's spawns, which is how the
    /// same building stands furnished in one place and empty in another. A set the
    /// root does not have is empty rather than a panic, because a patched
    /// archive can name one.
    #[test]
    fn a_placement_gets_only_its_own_doodad_set() {
        let spawn = |name: &str| WmoDoodad {
            path: name.into(),
            position: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            colour: 0,
            matrix: doodad_matrix([0.0; 3], [0.0, 0.0, 0.0, 1.0], 1.0),
            exterior_lit: false,
        };
        let ready = WmoReady {
            draws: Vec::new(),
            ambient: [0.0; 3],
            areas: Vec::new(),
            wmo_id: 0,
            collision: Arc::new(CollisionMesh::default()),
            lights: Vec::new(),
            doodad_sets: vec![
                WmoDoodadSet {
                    name: "Set_$DefaultGlobal".into(),
                    start: 0,
                    count: 1,
                },
                WmoDoodadSet {
                    name: "Set_A".into(),
                    start: 1,
                    count: 2,
                },
                // Overhangs the list, as a patched archive's can.
                WmoDoodadSet {
                    name: "Set_B".into(),
                    start: 2,
                    count: 99,
                },
            ],
            doodads: vec![spawn("global.m2"), spawn("a1.m2"), spawn("a2.m2")],
            interior: Vec::new(),
            bounds: [[0.0; 3]; 2],
        };

        let names = |set: u16| -> Vec<&str> {
            ready
                .doodads_in_set(set)
                .iter()
                .map(|d| d.path.as_str())
                .collect()
        };
        assert_eq!(names(0), ["global.m2"]);
        // Set 1 only. Set 0 is not drawn in addition, matching `Wmos.doodadsOf`.
        assert_eq!(names(1), ["a1.m2", "a2.m2"]);
        assert_eq!(names(2), ["a2.m2"], "an overhanging run is clamped");
        assert!(names(9).is_empty(), "a set that does not exist is empty");
    }
}
