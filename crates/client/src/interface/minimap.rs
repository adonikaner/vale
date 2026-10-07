//! **Where the little round map is looking** — the facts the painter needs
//! and the widget cannot know.
//!
//! `crate::lua::widgets::minimap` holds what a *script* can read and write: the zoom, and
//! the ping. Everything else about the minimap is the world's answer, and the
//! draw walk runs with no borrow of the world at all — so it comes across as a
//! resource rather than through the widget.
//!
//! The position and the facing are the mover's own, already on
//! [`WorldStatus`]; the map *directory* is the one `Map.dbc` name the session
//! resolved at login (`Azeroth`, `Kalimdor`), which is also the key
//! `textures\Minimap\md5translate.trs` is written against; and the indoor flag
//! and the building come from the floor under the feet, below.
//!
//! ## Inside a building
//!
//! The 1.12.1 client changes the whole map when the character stands in a
//! group of a WMO: the terrain is not drawn, the building's own pictures are
//! drawn over a black disc, and the radius and the zoom level come from the
//! indoor table (150 down to 25 yards, against 233 down to 67) and
//! `minimapInsideZoom`. The rule for which pictures, where and in which order
//! is [`vale_assets::tables::minimap::place_pictures`].
//!
//! The client decides "stands in a group" from the group its scene graph has
//! linked the character's model into. This client decides it from the floor
//! under the feet, the test [`super::worldmap::WorldMapState::outdoors`] also
//! makes ([`vale_assets::world::wmo::building_under`]), and then finds the
//! group among the placed buildings by its box and by the floor's flags
//! ([`vale_assets::tables::minimap::InteriorModel::group_at`]). A floor that
//! belongs to an `EXTERIOR` group does not count: the client's group walk
//! refuses such a group, so it could only ever show an empty disc. Stormwind's
//! streets are not `EXTERIOR` groups and carry pictures, so the city shows its
//! own map from the street.
//!
//! [`InteriorModels`] reads each building's root and group headers once, on
//! the task pool, the first time the character stands in it.
//!
//! ## The dots and the two arrows
//!
//! What the map draws over its tiles is the world's answer too, and it comes
//! across the same way: [`MinimapView::blips`] is every unit and object the
//! reference's classifier would list (`vale_assets::look::blips`), each as an offset in yards from the character, and
//! [`MinimapView::markers`] is the two places the map points at — the flag a
//! guard's directions put up, and the body a ghost is walking back to. The
//! offsets are in yards rather than on the disc because the disc's radius is
//! the *widget's* zoom, which lives on the Lua side; the painter projects.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

use vale_assets::look::blips::{self, Kind, Tracking};
use vale_assets::tables::minimap::{self as rule, InteriorModel};

use crate::world::session::{Session, WorldEntity, WorldStatus};

/// One dot: which cell, and where it is relative to the character.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blip {
    pub kind: Kind,
    /// Yards north (`+x`) and west (`+y`) of the character.
    pub north: f32,
    pub west: f32,
}

/// The two places the map points at when they are off it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    /// `SMSG_GOSSIP_POI` — a guard's directions, with the `POIIcons` cell the
    /// packet named. See [`super::worldmap::MapLandmarks`].
    Poi { icon: u32 },
    /// The character's own corpse, while they are a ghost.
    Corpse,
}

/// A marker: inside the disc it is drawn as its icon where it is, beyond it
/// as an arrow at the rim pointing the way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marker {
    pub kind: MarkerKind,
    pub north: f32,
    pub west: f32,
}

/// What the minimap is centred on this frame — see the module comment.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct MinimapView {
    /// `false` before the character is in the world, which is what makes the
    /// widget draw nothing rather than the middle of the map grid. The two glue
    /// screens and `--audit` are both this state.
    pub in_world: bool,
    /// The `Map.dbc` directory — `Azeroth`, `Kalimdor`, `PVPZone01`. The key
    /// `md5translate.trs` is written against, and empty before there is a map.
    pub directory: String,
    /// The character's world position: north (+x) and west (+y).
    pub position: (f32, f32),
    /// …and which way they are looking, in the server's own radians.
    pub facing: f32,
    /// …and the height of their feet, which the building pictures are chosen
    /// and ordered by.
    pub elevation: f32,
    /// Whether the character stands in a building group, which chooses the
    /// indoor radius table and zoom level and replaces the terrain with the
    /// building's pictures. See the module comment.
    pub indoors: bool,
    /// The building, once its headers are read; `None` outdoors, and while
    /// they load, when the map is an empty disc.
    pub interior: Option<InteriorView>,
    /// The dots — see the module comment. Rebuilt every frame there is a world,
    /// as the reference rebuilds its five lists on every update.
    pub blips: Vec<Blip>,
    /// …and the two places the map points at.
    pub markers: Vec<Marker>,
}

/// The building the character stands in, as the painter needs it.
#[derive(Debug, Clone)]
pub struct InteriorView {
    pub model: Arc<InteriorModel>,
    /// The `MODF` placement's id, which tells two copies of one model apart.
    pub unique_id: u32,
    /// The group the character stands in, where the group walk starts.
    pub group: u32,
    /// The placement and its inverse, column-major, WoW axes.
    pub local_to_world: [f32; 16],
    pub world_to_local: [f32; 16],
}

impl InteriorView {
    /// The pictures to draw at a radius; see [`rule::place_pictures`].
    pub fn pictures(&self, position: [f32; 3], radius: f32) -> Vec<rule::PlacedPicture> {
        rule::place_pictures(
            &self.model,
            self.group,
            &self.local_to_world,
            &self.world_to_local,
            position,
            radius,
        )
    }
}

/// Compared by identity of the model rather than by its contents, because
/// [`MinimapView`] is compared every frame to decide whether the minimap is
/// rebuilt.
impl PartialEq for InteriorView {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.model, &other.model)
            && self.unique_id == other.unique_id
            && self.group == other.group
            && self.local_to_world == other.local_to_world
    }
}

/// Every building whose minimap headers have been asked for, by lower-case
/// archive path: read once on the task pool and kept for the session. One
/// building's entry is a few kilobytes.
#[derive(Resource, Default)]
pub struct InteriorModels {
    ready: HashMap<String, Option<Arc<InteriorModel>>>,
    pending: HashMap<String, Task<Option<InteriorModel>>>,
}

impl InteriorModels {
    /// The model for `path`, or `None` while it loads or when it could not be
    /// read. The first ask starts the read.
    fn get(&mut self, path: &str, assets: &crate::assets::GameAssets) -> Option<Arc<InteriorModel>> {
        let key = path.to_ascii_lowercase();
        if let Some(done) = self.ready.get(&key) {
            return done.clone();
        }
        if let Some(task) = self.pending.get_mut(&key) {
            let finished = block_on(future::poll_once(task))?;
            self.pending.remove(&key);
            let model = finished.map(Arc::new);
            if model.is_none() {
                warn!("minimap: {path} could not be read; no pictures inside it");
            }
            self.ready.insert(key, model.clone());
            return model;
        }
        let read = assets.shared_reader();
        let owned = path.to_string();
        let task = AsyncComputeTaskPool::get()
            .spawn(async move { InteriorModel::load(&owned, |p| read(p)) });
        self.pending.insert(key, task);
        None
    }
}

pub struct MinimapPlugin;

impl Plugin for MinimapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MinimapView>()
            .init_resource::<InteriorModels>()
            .add_systems(Update, (track, collect).chain().in_set(super::GameSet))
            // After the indoor flag is known, so that the interface's zoom
            // level and `MINIMAP_UPDATE_ZOOM` follow it on the same frame.
            .add_systems(
                Update,
                crate::lua::widgets::minimap::announce_inside.after(super::GameSet),
            );
    }
}

/// How far above the feet the floor under the character is looked for: the
/// server's own `z + 1`, as in [`super::worldmap`].
const FLOOR_PROBE: f32 = 1.0;

/// Copy the facts across, and find the building the character stands in.
///
/// **Written unconditionally rather than on a change**, unlike every other
/// resource in this directory: the position moves every frame a character
/// walks, so a change test here would be a comparison that almost always
/// says yes for the cost of a comparison. The `String` is only cloned when
/// the map id moves.
#[allow(clippy::too_many_arguments)]
fn track(
    session: Res<Session>,
    status: Res<WorldStatus>,
    assets: Res<crate::assets::GameAssets>,
    solids: Res<crate::world::session::Solids>,
    buildings: Query<(&crate::render::wmos::WmoPlacement, &crate::render::wmos::Interior)>,
    mut models: ResMut<InteriorModels>,
    mut view: ResMut<MinimapView>,
) {
    let Some(active) = session.active.as_ref().filter(|_| status.in_world) else {
        if view.in_world {
            *view = MinimapView::default();
        }
        return;
    };
    if view.directory != status.map_name {
        view.directory = status.map_name.clone();
    }
    view.in_world = true;
    let position = status.position;
    view.position = (position.x, position.y);
    view.elevation = position.z;
    view.facing = status.orientation;

    // The floor under the feet, and whether it counts as a building's.
    let map = active.map_id;
    let probe_z = position.z + FLOOR_PROBE;
    let floor = vale_assets::world::wmo::building_under(
        solids.0.building_floor(map, position.x, position.y, probe_z),
        active.terrain_height(map, position.x, position.y),
        probe_z,
    )
    .filter(|flags| flags & vale_assets::world::wmo::group_flags::EXTERIOR == 0);
    view.indoors = floor.is_some();
    let Some(flags) = floor else {
        view.interior = None;
        return;
    };

    // Which building and group: the smallest group box that holds the probe,
    // among the groups whose flags are the floor's, over every placement whose
    // own box holds it.
    let probe = [position.x, position.y, probe_z];
    let mut best: Option<(f32, InteriorView)> = None;
    for (placement, interior) in &buildings {
        if !interior.near(probe, 0.0) {
            continue;
        }
        let Some(model) = models.get(&placement.path, &assets) else {
            continue;
        };
        let local = interior.inverse.transform_point3(Vec3::from(probe)).to_array();
        let Some(group) = model.group_at(local, flags) else {
            continue;
        };
        let Some(Some(header)) = model.groups.get(group as usize) else {
            continue;
        };
        let volume: f32 = (0..3).map(|a| header.bounds[1][a] - header.bounds[0][a]).product();
        if best.as_ref().is_some_and(|(least, _)| *least <= volume) {
            continue;
        }
        best = Some((
            volume,
            InteriorView {
                model,
                unique_id: placement.unique_id,
                group,
                local_to_world: interior.inverse.inverse().to_cols_array(),
                world_to_local: interior.inverse.to_cols_array(),
            },
        ));
    }
    let found = best.map(|(_, found)| found);
    if view.interior != found {
        view.interior = found;
    }
}

/// A cap on the dots, so a city under Track Humanoids costs a bounded walk in
/// the painter. The reference's lists grow without one; two hundred is more
/// than fit on the disc.
const MOST_BLIPS: usize = 200;

/// **The dots and the markers**, off the world the frame is drawing.
///
/// The reference's classifier is walked over every entity in view;
/// the rule itself is [`vale_assets::look::blips`], and what is here is only
/// the gathering of its inputs: the character's own tracking fields, each unit's
/// flags and cached creature type, each object's lock, and the group roster.
///
/// **A group member's dot comes off the roster, not off the object** — the
/// reference appends its party list separately and skips a grouped player in
/// the object walk. Where the member is in view
/// their drawn position is used; where they are not,
/// `SMSG_PARTY_MEMBER_STATS`' two `int16`s are, if the zone puts them on this
/// map.
fn collect(
    mut view: ResMut<MinimapView>,
    units: Query<(&WorldEntity, Option<&Transform>)>,
    party: Res<crate::interface::party::Party>,
    landmarks: Res<super::worldmap::MapLandmarks>,
    dying: Res<crate::interface::death::Dying>,
    status: Res<WorldStatus>,
    assets: Res<crate::assets::GameAssets>,
) {
    view.blips.clear();
    view.markers.clear();
    if !view.in_world {
        return;
    }
    let here = view.position;
    let Some((me, tracking)) = units
        .iter()
        .find(|(unit, _)| unit.is_self)
        .map(|(unit, _)| (unit.guid, unit.tracking))
    else {
        return;
    };
    let tracking = tracking
        .map(|(creatures, resources, stealthed)| Tracking { creatures, resources, stealthed })
        .unwrap_or_default();
    let tables = assets.display_tables().ok();
    let offset = |transform: &Transform| {
        let at = crate::render::axes::to_wow(transform.translation);
        (at[0] - here.0, at[1] - here.1)
    };

    for (unit, transform) in units.iter() {
        if view.blips.len() >= MOST_BLIPS {
            break;
        }
        if unit.is_self || party.holds(unit.guid) {
            continue;
        }
        let Some(transform) = transform else {
            continue;
        };
        let kind = if unit.object_state.is_some() {
            // A game object — `GAMEOBJECT_STATE` is the one field nothing else
            // carries — and its lock against the resource mask.
            let Some(tables) = tables.as_ref() else {
                continue;
            };
            blips::resource_kind(&tracking, tables.locks().keys(unit.object_lock))
        } else {
            let owner = unit.charmed_by.or(unit.summoned_by);
            blips::unit_kind(
                &tracking,
                &blips::Unit {
                    creature_type: unit.creature_type,
                    vis_flags: unit.vis_flags,
                    hunters_marked: unit.hunters_marked,
                    alive: !unit.dead,
                    mine: owner == Some(me),
                    quest_turn_in: unit.quest_turn_in,
                },
            )
        };
        if let Some(kind) = kind {
            let (north, west) = offset(transform);
            view.blips.push(Blip { kind, north, west });
        }
    }

    // The group, off the roster. Each member once: the drawn position when
    // they are in view, the stats packet's when they are not.
    for index in 0..party.count() {
        let Some(member) = party.member(index) else {
            continue;
        };
        if member.guid == me {
            continue;
        }
        let seen = units
            .iter()
            .find(|(unit, transform)| unit.guid == member.guid && transform.is_some())
            .and_then(|(_, transform)| transform.map(offset));
        let far = || {
            let stats = member.stats.as_ref()?;
            let (x, y) = stats.position?;
            let zone = u32::from(stats.zone?);
            let map = tables.as_ref()?.areas()?.get(zone)?.map;
            (map == status.map_id).then(|| (f32::from(x) - here.0, f32::from(y) - here.1))
        };
        if let Some((north, west)) = seen.or_else(far) {
            view.blips.push(Blip { kind: Kind::Party, north, west });
        }
    }

    // The two markers: the guard's flag, on the map it was named on, and the
    // body, while there is one to walk back to.
    if let Some(poi) = landmarks.gossip.as_ref().filter(|_| landmarks.map == status.map_id) {
        view.markers.push(Marker {
            kind: MarkerKind::Poi { icon: poi.icon },
            north: poi.position.0 - here.0,
            west: poi.position.1 - here.1,
        });
    }
    if dying.ghost {
        if let Some(corpse) = dying.corpse.as_ref().filter(|c| c.corpse_map_id == status.map_id) {
            view.markers.push(Marker {
                kind: MarkerKind::Corpse,
                north: corpse.x - here.0,
                west: corpse.y - here.1,
            });
        }
    }
}
