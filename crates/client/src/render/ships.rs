//! **The boats and the zeppelins** — buildings the server spawns and nothing
//! ever states the position of.
//!
//! Everything else that is drawn as a WMO comes out of a *file*: an `MODF` row
//! in an ADT, or the one a `WDT` names for a map with no ground
//! ([`crate::render::globalwmo`]). Both are placements — a matrix, fixed for the
//! life of the tile — and [`crate::render::wmos::spawn_wmos`] bakes each into a
//! `Transform` once.
//!
//! A continent transport is neither. It is a game object the *server* spawns,
//! it is a `.wmo` rather than an M2, and **it moves 30 yards a second across two
//! continents**. So it needs its own spawner, and this is it.
//!
//! ## What it does not have to do, which is most of it
//!
//! * **The geometry is [`WmoCache`]'s**, path-keyed and shared, exactly as a
//!   building's is. The loader, the material pool, the staging budget and the
//!   eviction are all reused; what is new here is only the placement.
//! * **The placement is [`crate::world::session::place_entities`]'s.** A
//!   transport is a `WorldEntity` like any other, so its `Transform` is already
//!   written from [`crate::world::motion`] every frame — the ship is drawn where
//!   the interpolator says the entity is, and the route it walks is decided one
//!   layer up in `world::entities::transport::ship_placement`.
//! * **The interior, the furniture and the lights are not read.** A `MODD`
//!   spawn's matrix is WMO-local and `crate::render::doodads` takes world-space
//!   placements, so furniture on a moving deck would be nailed to wherever the
//!   ship was when it was spawned. The three shipped transports carry no `MODD`
//!   set worth the ambiguity; this is stated rather than skipped silently.
//!
//! ## The one thing that had to be got right: which way the bow points
//!
//! There is **no half-turn here**, and the first version of this file had one.
//! `axes::body(facing, pitch)` and nothing else is the transform, which is what
//! [`vale_assets::world::collision::object_matrix`] already says for
//! everything the *server* places: the `Rz(pi)` in
//! [`vale_assets::world::adt::placement_matrix`] belongs to an `MDDF`/`MODF`
//! row's own encoding — it is the internal-to-world frame flip a placement
//! taken out of an ADT needs — and a boat's placement never went through that
//! frame. The same M2 is drawn with `Rz(facing)` as a creature and with
//! `Rz(rot.y + pi)` as a doodad, and both are right.
//!
//! What made the mistake plausible is that the server's orientation for a ship
//! is *already* a half-turn from the direction of travel: `ShipTransport::Update`
//! ends in `atan2(dir.y, dir.x) + M_PI`. So the composition has to put the
//! model's **stern** along the heading, and it does, because both shipped
//! transport models are authored with their front at local **−X**:
//! `transportship.wmo` runs −58.6..+44.5 along x with a bowsprit — twelve
//! vertices 0.6 yards wide — off the negative end, and `transport_zeppelin`'s
//! tail fins are the broad end at +x. `vale ships` measures both and states
//! the composed bearing against the direction of travel, because getting this
//! wrong draws a boat sailing smoothly backwards rather than failing.

use bevy::prelude::*;

use crate::render::wmos::{Lookup, WmoCache, WmoPart};
use crate::world::entities::DisplayCache;
use crate::world::session::WorldEntity;
use vale_assets::tables::shiptransport::MoTransport;
use vale_protocol::state::update::ObjectType;

/// A transport whose batches have been spawned, so this pass asks the cache
/// once per entity rather than once per frame.
///
/// Also what keeps [`crate::world::entities::fallback::fallback_shapes`] off it:
/// a `.wmo` game object is given `NoModel` by the M2 spawner, and without this
/// marker every boat in the game would additionally carry a grey box.
#[derive(Component)]
pub struct ShipBody;

/// Is this entity a continent transport that should be drawn as a building?
///
/// The template's type byte and nothing else — `WorldEntity::object_kind` is
/// `GameObjectInfo::type`, which is zero until `CMSG_GAMEOBJECT_QUERY` has come
/// back. Zero reads as `Door`, so an unresolved transport is simply not drawn
/// yet, which is the right answer for the round trip it is waiting on.
pub fn is_transport(entity: &WorldEntity) -> bool {
    entity.kind == ObjectType::GameObject && entity.object_kind == MoTransport::TYPE
}

/// Give every transport in view its batches.
pub(super) fn spawn_ships(
    mut commands: Commands,
    mut cache: ResMut<WmoCache>,
    mut displays: ResMut<DisplayCache>,
    assets: Res<crate::assets::GameAssets>,
    entities: Query<(Entity, &WorldEntity), Without<ShipBody>>,
) {
    for (entity, world) in &entities {
        if !is_transport(world) {
            continue;
        }
        let Some(display_id) = world.display_id else {
            continue;
        };
        let Some(display) = displays.resolve(&assets, world.kind, display_id) else {
            continue;
        };
        if !display.path.to_ascii_lowercase().ends_with(".wmo") {
            continue;
        }
        let ready = match cache.lookup(&display.path) {
            Lookup::Ready(ready) => ready,
            // Asked for; the loader answers in a frame or two.
            Lookup::Loading => continue,
            // Reported once by the loader. Marked so this stops asking — the
            // grey box the M2 spawner already arranged is the right picture for
            // a building that will not read.
            Lookup::Failed => {
                commands.entity(entity).insert(ShipBody);
                continue;
            }
        };

        // **Straight onto the entity**, whose transform is the plain
        // `axes::body(facing, pitch)` everything else reads — the pick box, the
        // blob shadow and the selection ring included. There is no half-turn to
        // insert; see the module note, and `object_matrix`, which is the same
        // statement for every other thing the server places.
        for draw in ready.draws() {
            commands.spawn((
                WmoPart,
                Mesh3d(draw.mesh.clone()),
                MeshMaterial3d(draw.material.clone()),
                ChildOf(entity),
            ));
        }
        commands.entity(entity).insert(ShipBody);
    }
}

/// Registers [`spawn_ships`].
pub struct ShipPlugin;

impl Plugin for ShipPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, spawn_ships);
    }
}
