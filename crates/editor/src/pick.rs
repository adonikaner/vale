//! What the pointer is over: a ray onto the edited ground.
//!
//! ## It is marched against the edited heights, not against the drawn mesh
//!
//! The ground on screen is a mesh the streamer built when the tile was last
//! read, and while a stroke is being held the heights have moved ahead of it.
//! Picking against the mesh would put the brush where the ground used to be,
//! which during a stroke that raises a hill means the brush walks down the side
//! of its own hill.
//!
//! So the ray is marched against [`vale_edit::adt::heights::height_at`],
//! which reads the open tile — the same bytes the tool is writing. The interval
//! the ray crosses the ground in is found by stepping, then narrowed by
//! bisection; the height inside a cell comes from `vale_assets`' own wedge
//! rule and not from a second one, so the point under the cursor is the point a
//! character would stand on.
//!
//! A tile that is not open answers nothing rather than falling back to the
//! archives. That is deliberate: the caller opens the tile it is about to edit,
//! and a pick that quietly answered from unedited bytes would be a pick that
//! disagreed with the tool by however much had been painted.
//!
//! ## The ground is not the only surface, and two sets of tools want the other
//! one
//!
//! A tool whose subject is the **terrain** — raise, paint, hole, water, area —
//! wants the ground and only the ground: there is nothing to paint on the roof
//! of a building. A tool whose subject is a **thing standing in the world** —
//! a creature spawn, a game object spawn, a path node, a model being dropped —
//! wants whatever is under the pointer, which over a bridge, a dock, an inn's
//! roof or a crate is not the ground at all. Until [`Cursor::surface`] existed
//! every one of them used [`Cursor::ground`], so clicking a pier placed the
//! creature in the water beneath it.
//!
//! [`Cursor::surface`] is the nearer of two answers: the ground march above, and
//! a ray against the collision hulls the renderer has already placed
//! (`vale_assets::world::collision::CollisionWorld::ray_of`). Three things
//! follow from it being the **collision** hull rather than the drawn geometry,
//! and all three are wanted:
//!
//! * it is the surface a character would stand on, which is the same geometry
//!   vmangos' vmaps are built from — so a spawn placed on a floor here is on
//!   that floor on the server;
//! * a model with no `BoundingTriangles` — grass, a flame, a bird — is not a
//!   surface, so the pointer falls through it to the ground, which is where
//!   anything standing "on" a tuft of grass belongs;
//! * a batch the file marks as drawn-but-not-solid — a banner, a rail — is
//!   likewise not a surface.
//!
//! **It answers only from populations the view bar is showing.** The switches
//! are `WorldTuning`'s, so buildings off means roofs stop being surfaces and
//! doodads off means crates do. A hidden surface is one the person placing
//! cannot see, and standing a creature on one is how a spawn ends up somewhere
//! nobody can find it.
//!
//! **One thing it is not right about**, and it is the collision world's own
//! limit rather than this module's: a doodad or building *moved* in this
//! session keeps its hull where it was until the tile is read again — see
//! `tools::wmos`, where that is written up. Placing on top of something just
//! dragged aims at where it used to be.

use crate::session::EditSession;
use vale_assets::world::adt::TILE_SIZE;
use vale_assets::world::collision::Surfaces;
use vale_client::render::axes;
use vale_client::render::tuning::WorldTuning;
use vale_client::world::camera::WorldCamera;
use vale_client::world::session::Solids;
use bevy::prelude::*;

/// Where the pointer is in the world.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct Cursor {
    /// The point on the ground under the pointer, in the world's own axes.
    pub ground: Option<Vec3>,
    /// …and the point on **whatever is under the pointer**: the ground, or the
    /// building, doodad or game object standing on it, whichever the ray meets
    /// first. Also in the world's own axes.
    ///
    /// The field a tool placing something in the world reads. See the module
    /// comment for which populations answer and why the ground alone will not
    /// do.
    pub surface: Option<Vec3>,
    /// …and which tile that is, whether or not the ground answered: a pointer
    /// over a tile the map does not have still names a coordinate.
    pub tile: Option<(u32, u32)>,
    /// …and which of that tile's 256 map chunks, when the ground answered.
    ///
    /// A chunk and not only a tile because that is the unit most of what is left
    /// to edit is stored in: the textures it is painted with, its area id, its
    /// holes, its water. `None` wherever `ground` is, since it is found from the
    /// point the ray met.
    pub chunk: Option<usize>,
}

/// Yards between samples along the ray. A cell is 4.17 yards across, so a step
/// of two cannot walk over a ridge narrow enough to matter at editing distance
/// and costs 500 height lookups over the whole range.
const STEP: f32 = 2.0;

/// How far the ray is followed. Two tiles, which is past the far edge of the 3x3
/// at any pitch a person edits at.
const RANGE: f32 = 2.0 * TILE_SIZE;

/// How many times the crossing interval is halved. Ten takes a two-yard interval
/// to two millimetres, which is well under the resolution of anything that reads
/// this.
const BISECTIONS: usize = 10;

pub struct PickPlugin;

impl Plugin for PickPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Cursor>().add_systems(
            Update,
            // **After the camera has been placed**, because the ray is built
            // from the camera's transform: reading it before `place` would aim
            // this frame's pointer through last frame's view, which while the
            // camera is being flown is the whole of the movement.
            aim.after(vale_client::world::camera::place),
        );
    }
}

pub fn aim(
    mut cursor: ResMut<Cursor>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    session: Option<Res<EditSession>>,
    state: Res<crate::playtest::Playtest>,
    solids: Res<Solids>,
    tuning: Res<WorldTuning>,
) {
    cursor.ground = None;
    cursor.surface = None;
    cursor.tile = None;
    cursor.chunk = None;
    // Nothing to aim while a playtest has the pointer: the ray is 500 height
    // lookups and what it would answer is where a brush nobody may use would
    // land. The `None` it leaves is also what stops the brush ring being drawn
    // over the game's own interface.
    if !state.editing() {
        return;
    }
    let Some(session) = session else { return };
    let Ok(window) = windows.single() else { return };
    let Some(at) = window.cursor_position() else {
        return;
    };
    let Ok((camera, transform)) = camera.single() else {
        return;
    };
    // **Physical pixels, not logical.** `cursor_position` is logical and the
    // world camera draws into `render::present`'s frame image, which is created
    // at the window's *physical* size with `scale_factor: 1.0` — so its viewport
    // is measured in physical pixels. The client's own two picks
    // (`game::combat::target::hover` and `ui::debug::inspect`) both make this
    // conversion and say so; this one did not, and on a 125% display the brush
    // landed up and left of the pointer by a quarter of the distance from the
    // screen's centre. That is an *angular* error, so what it comes to in yards
    // grows with how far the ray travels — which is why it read as "the cursor
    // and the brush move apart the higher the camera is".
    let Ok(ray) = camera.viewport_to_world(transform, at * window.scale_factor()) else {
        return;
    };

    // The ray in the world's own axes, which is what the height field is in.
    let origin = Vec3::from(axes::to_wow(ray.origin));
    let direction = Vec3::from(axes::to_wow(*ray.direction)).normalize_or_zero();
    if direction == Vec3::ZERO {
        return;
    }

    // How far the ray is above the ground at a distance, or `None` where there
    // is no ground: off the map, over a hole, or over a tile that is not open.
    let gap = |distance: f32| {
        let at = origin + direction * distance;
        let coord = vale_assets::tile_for_position(at.x, at.y);
        let tile = session.tiles.get(&coord)?;
        vale_edit::adt::heights::height_at(tile, at.x, at.y).map(|ground| at.z - ground)
    };

    // **The buildings and the scenery, cast before the ground is marched**, so
    // that a miss on the ground still leaves a surface: the ray hits a roof
    // whose tile is not open, and over open water there is a dock and no
    // terrain at all. It is one query against a uniform grid — the same one the
    // client's camera makes every frame — where the march below is five hundred
    // height lookups, so the order costs nothing either way.
    let hull = hull_hit(&solids, &tuning, session.map_id, origin, direction);

    let mut previous: Option<(f32, f32)> = None;
    let mut distance = STEP;
    let mut ground: Option<f32> = None;
    while distance < RANGE {
        if let Some(above) = gap(distance) {
            if above <= 0.0 {
                // The first sample under the ground. The crossing is between
                // this one and the last one that was above it; without a
                // previous sample the ray began underground and there is no
                // interval to narrow.
                if let Some((was, _)) = previous.filter(|(_, above)| *above > 0.0) {
                    let hit = bisect(&gap, was, distance);
                    let at = origin + direction * hit;
                    let coord = vale_assets::tile_for_position(at.x, at.y);
                    ground = Some(hit);
                    cursor.ground = Some(at);
                    cursor.tile = Some(coord);
                    cursor.chunk = session
                        .tiles
                        .get(&coord)
                        .and_then(|tile| vale_edit::adt::heights::chunk_at(tile, at.x, at.y));
                }
                break;
            }
            previous = Some((distance, above));
        }
        distance += STEP;
    }

    // **Whichever of the two the ray reached first**, which is the whole of the
    // rule: a roof stands over the ground it is on, so a pointer on the roof
    // meets the hull first and a pointer on the grass beside the building meets
    // the ground first. `min` and not a preference, so no case has to be
    // enumerated.
    cursor.surface = nearest(ground, hull).map(|hit| origin + direction * hit);
    if cursor.ground.is_some() {
        return;
    }

    // Nothing was hit, but the pointer still names a tile: the one the ray is
    // over when it reaches the height the camera is orbiting, which is what a
    // panel needs to say "open this tile".
    let flat = (origin.z - session_ground(&session, origin)) / -direction.z;
    if flat.is_finite() && flat > 0.0 && flat < RANGE {
        let at = origin + direction * flat;
        cursor.tile = Some(vale_assets::tile_for_position(at.x, at.y));
    }
}

/// **Where the pointer meets the ground as though it had no holes**, in the
/// world's own axes.
///
/// [`Cursor::ground`] is the ground that is *drawn*, which is what a brush
/// wants: over a hole there is nothing to paint and the answer is rightly
/// nothing. Two tools want the other surface. A tool that **cuts** holes has to
/// be able to point at one it has already cut; a tool that assigns an **area
/// id** has to be able to point into a cave mouth, which is a hole with a
/// building behind it. Both are asking *where was the ground before somebody
/// took it away*, and `vale_edit::adt::heights::solid_height_at` answers it
/// — a hole takes cells out of the mesh and leaves `MCVT` exactly as it was.
///
/// **It is a function rather than a second field on [`Cursor`]** because the
/// march is five hundred height lookups and neither of its two callers is open
/// most of the time. It is also not a flag on [`aim`]: a pick whose meaning
/// depends on who is asking is a pick nobody can reason about.
pub fn solid_under(
    session: &EditSession,
    windows: &Query<&Window>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<Vec3> {
    let (origin, direction) = ray(windows, camera)?;
    let gap = |distance: f32| {
        let at = origin + direction * distance;
        let coord = vale_assets::tile_for_position(at.x, at.y);
        let tile = session.tiles.get(&coord)?;
        vale_edit::adt::heights::solid_height_at(tile, at.x, at.y).map(|ground| at.z - ground)
    };

    let mut previous: Option<(f32, f32)> = None;
    let mut distance = STEP;
    while distance < RANGE {
        if let Some(above) = gap(distance) {
            if above <= 0.0 {
                let (was, _) = previous.filter(|(_, above)| *above > 0.0)?;
                return Some(origin + direction * bisect(&gap, was, distance));
            }
            previous = Some((distance, above));
        }
        distance += STEP;
    }
    None
}

/// **Where the pointer meets a level surface at `z`**, in the world's own axes.
///
/// The third of the three answers here, and the one for a tool whose subject is
/// not the ground at all. The water tool paints a *pool*, which is flat by
/// definition and stands at a height it was told rather than at the height of
/// anything under it — so what the pointer is over, for that tool, is the plane
/// it is painting on.
///
/// **Aiming such a tool at the ground is what makes it uncontrollable**, and it
/// was reported from the window before it was understood here. Over a lake the
/// ray goes straight through the surface and lands on the *bed*, which at
/// editing pitch is met at a grazing angle: a pointer moved thirteen pixels
/// moved the aim a hundred and forty yards and dropped it thirty-three. The
/// plane has neither property — the intersection moves smoothly with the pointer,
/// and it is where the thing being drawn actually is, so the preview sits under
/// the cursor instead of hanging above it.
///
/// `None` when the plane is behind the eye, when the ray is parallel to it, or
/// when it is met beyond [`RANGE`] — the last is the grazing case that remains,
/// and answering nothing there is better than answering a mile away.
pub fn on_plane(
    z: f32,
    windows: &Query<&Window>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<Vec3> {
    let (origin, direction) = ray(windows, camera)?;
    meets_plane(origin, direction, z)
}

/// The arithmetic of [`on_plane`], with no window in it so that it can be
/// tested. All four refusals are here rather than at the call site.
///
/// Public because a caller that has already built the ray should not build a
/// second one: the lights tool casts once and asks it about several planes,
/// one per light height, and going back through [`on_plane`] would re-read the
/// window and the camera for each.
pub fn meets_plane(origin: Vec3, direction: Vec3, z: f32) -> Option<Vec3> {
    let distance = (z - origin.z) / direction.z;
    (distance.is_finite() && distance > 0.0 && distance < RANGE)
        .then(|| origin + direction * distance)
}

/// The pointer's ray, in the world's own axes.
///
/// Physical pixels, for the reason [`aim`] gives beside its own copy of this
/// conversion: `cursor_position` is logical and the world camera draws into a
/// target measured in physical pixels.
pub fn ray(
    windows: &Query<&Window>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<(Vec3, Vec3)> {
    let window = windows.single().ok()?;
    let at = window.cursor_position()?;
    let (camera, eye) = camera.single().ok()?;
    let ray = camera
        .viewport_to_world(eye, at * window.scale_factor())
        .ok()?;
    let origin = Vec3::from(axes::to_wow(ray.origin));
    let direction = Vec3::from(axes::to_wow(*ray.direction)).normalize_or_zero();
    (direction != Vec3::ZERO).then_some((origin, direction))
}

/// **How far along the ray the first collision hull is**, or `None` when it
/// meets none.
///
/// The populations asked are the ones the view bar is showing, and that mapping
/// is the whole of the configurability: `buildings` answers for `MODF`
/// buildings, `doodads` for `MDDF` scenery and a building's own `MODD`
/// furniture, `entities` for the game objects the server spawned. Switch a
/// layer off and what it draws stops being something to put a spawn on, which
/// is what makes "place on the terrain under this tree" expressible at all —
/// turn the doodads off, place, turn them back on.
///
/// A free function rather than a closure inside [`aim`] so that the mapping
/// from switches to populations is one statement with a name on it.
fn hull_hit(
    solids: &Solids,
    tuning: &WorldTuning,
    map_id: u32,
    origin: Vec3,
    direction: Vec3,
) -> Option<f32> {
    let want = Surfaces {
        buildings: tuning.buildings,
        doodads: tuning.doodads,
        objects: tuning.entities,
    };
    let to = origin + direction * RANGE;
    // A fraction of the segment, which is what the camera's question wanted;
    // the segment is `RANGE` long, so this is a distance in yards.
    let hit = solids
        .0
        .ray_of(map_id, origin.to_array(), to.to_array(), want)?;
    Some(hit * RANGE)
}

/// The nearer of two distances along the ray, where either may be absent.
fn nearest(a: Option<f32>, b: Option<f32>) -> Option<f32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (found, None) | (None, found) => found,
    }
}

/// Narrow a crossing that is known to lie between `above` and `below`.
fn bisect(gap: &impl Fn(f32) -> Option<f32>, mut above: f32, mut below: f32) -> f32 {
    for _ in 0..BISECTIONS {
        let middle = (above + below) * 0.5;
        match gap(middle) {
            Some(v) if v > 0.0 => above = middle,
            Some(_) => below = middle,
            // A hole in the middle of the interval: keep the half that is known
            // to be over ground rather than guessing across it.
            None => below = middle,
        }
    }
    (above + below) * 0.5
}

/// A height to aim the fallback plane at: the ground under the tile at the
/// origin, or zero.
fn session_ground(session: &EditSession, origin: Vec3) -> f32 {
    let coord = vale_assets::tile_for_position(origin.x, origin.y);
    session
        .tiles
        .get(&coord)
        .and_then(|tile| vale_edit::adt::heights::height_at(tile, origin.x, origin.y))
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::collision::{
        Collider, CollisionMesh, CollisionWorld, Solid, SolidId,
    };
    use std::sync::Arc;

    /// A flat ten-yard square at `z`, offset along x, as two triangles.
    fn slab(z: f32, x: f32) -> CollisionMesh {
        CollisionMesh {
            positions: vec![
                [x, 0.0, z],
                [x + 10.0, 0.0, z],
                [x + 10.0, 10.0, z],
                [x, 10.0, z],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..CollisionMesh::default()
        }
    }

    /// The identity `MODF` matrix, column-major.
    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];

    /// **A building answers only while the buildings are shown, and a doodad
    /// only while the doodads are.**
    ///
    /// This is the configurable half of [`Cursor::surface`] and the half that
    /// cannot be seen in a screenshot: with a layer hidden and its hull still
    /// answering, a creature is placed on a roof nobody can see, and the picture
    /// of the place it was meant to go looks exactly as it did before.
    #[test]
    fn a_hidden_layer_is_not_a_surface() {
        let world = CollisionWorld::new();
        world.insert(
            0,
            (32, 32),
            SolidId::placement(1),
            Solid::Building,
            Arc::new(Collider::place(&slab(30.0, 0.0), &IDENTITY)),
        );
        world.insert(
            0,
            (32, 32),
            SolidId::placement(2),
            Solid::Doodad,
            Arc::new(Collider::place(&slab(30.0, 20.0), &IDENTITY)),
        );
        let solids = Solids(Arc::new(world));

        let down = Vec3::new(0.0, 0.0, -1.0);
        let over_building = Vec3::new(5.0, 5.0, 100.0);
        let over_doodad = Vec3::new(25.0, 5.0, 100.0);

        let mut tuning = WorldTuning::default();
        tuning.buildings = true;
        tuning.doodads = true;
        // Both shown: each slab answers, 70 yards down from the eye.
        for at in [over_building, over_doodad] {
            let hit = hull_hit(&solids, &tuning, 0, at, down).expect("a shown hull did not answer");
            assert!((hit - 70.0).abs() < 0.1, "met the slab at {hit}");
        }

        tuning.buildings = false;
        assert!(
            hull_hit(&solids, &tuning, 0, over_building, down).is_none(),
            "a hidden building was still a surface"
        );
        assert!(
            hull_hit(&solids, &tuning, 0, over_doodad, down).is_some(),
            "hiding the buildings took the doodads with them"
        );

        tuning.buildings = true;
        tuning.doodads = false;
        assert!(
            hull_hit(&solids, &tuning, 0, over_doodad, down).is_none(),
            "a hidden doodad was still a surface"
        );
        assert!(
            hull_hit(&solids, &tuning, 0, over_building, down).is_some(),
            "hiding the doodads took the buildings with them"
        );

        // …and a hull on another map is never a surface, whatever is shown.
        assert!(
            hull_hit(&solids, &tuning, 1, over_building, down).is_none(),
            "another map's hull answered"
        );
    }

    /// The surface is the **nearer** of the ground and the hull, with either
    /// absent, which is the whole of the rule [`aim`] applies.
    ///
    /// A roof stands over the ground it is on, so over a building the hull is
    /// nearer; beside it the ground is the only answer. A dock over open water
    /// is the case with a hull and no ground, and it is the one that used to
    /// leave the pointer answering nothing at all.
    #[test]
    fn the_surface_is_whichever_was_reached_first() {
        assert_eq!(nearest(Some(90.0), Some(70.0)), Some(70.0));
        assert_eq!(nearest(Some(50.0), Some(70.0)), Some(50.0));
        assert_eq!(nearest(Some(50.0), None), Some(50.0));
        assert_eq!(nearest(None, Some(70.0)), Some(70.0));
        assert_eq!(nearest(None, None), None);
    }

    /// A ray meets a level surface where the geometry says, and refuses the
    /// three cases that have no answer.
    ///
    /// **The last of those is what a reported fault came down to.** The water
    /// tool aimed at the ground, so over a lake the ray passed through the
    /// surface and met the bed at a grazing angle; a pointer moved thirteen
    /// pixels moved the aim a hundred and forty yards. A plane has the same
    /// failure mode only as the ray approaches horizontal, and there the honest
    /// answer is nothing rather than a point past the loaded world.
    #[test]
    fn a_ray_meets_a_level_surface_or_says_it_does_not() {
        let down = Vec3::new(1.0, 0.0, -1.0).normalize();
        let at = meets_plane(Vec3::new(0.0, 0.0, 100.0), down, 40.0).expect("looking down");
        assert!((at.z - 40.0).abs() < 1e-3, "{at:?} is not on the plane");
        assert!(
            (at.x - 60.0).abs() < 1e-3,
            "{at:?} travelled the wrong way out"
        );

        // Behind the eye: the plane is above a ray going down.
        assert!(meets_plane(Vec3::new(0.0, 0.0, 100.0), down, 180.0).is_none());
        // Parallel: a horizontal ray never reaches it, however far it is followed.
        let flat = Vec3::new(1.0, 0.0, 0.0);
        assert!(meets_plane(Vec3::new(0.0, 0.0, 100.0), flat, 40.0).is_none());
        // …and the grazing case, which is the one worth refusing: a ray a
        // thousandth off horizontal meets the plane sixty thousand yards out.
        let graze = Vec3::new(1.0, 0.0, -0.001).normalize();
        assert!(meets_plane(Vec3::new(0.0, 0.0, 100.0), graze, 40.0).is_none());
        // The same ray does answer for a plane it reaches inside the range.
        assert!(meets_plane(Vec3::new(0.0, 0.0, 100.0), graze, 99.5).is_some());
    }
}
