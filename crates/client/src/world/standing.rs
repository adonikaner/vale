//! Ground height for the session thread: the terrain and the buildings on it,
//! joined into the one floor the local simulation walks on.
//!
//! [`Standing`] implements `vale_protocol::socket::session::World`, so the
//! protocol crate asks it for a floor without reading a game file itself.

use std::sync::Arc;

/// The terrain and the buildings on it, as the local simulation sees them.
///
/// Joining the two is all of collision's decision-making, and it is three
/// lines: which of the surfaces over a point is the one being stood on.
///
/// Public because a caller outside this directory needs the same join.
/// [`crate::interface::target`] casts a ray down onto the floor to place a
/// ground-targeted spell, and the floor a Blizzard lands on must be the floor
/// the character would walk on. A second implementation of the join could
/// drift from this one; `vale dress` had such a duplicate, which reported
/// success while covering less.
pub struct Standing {
    pub(super) terrain: Arc<vale_assets::MapTerrain>,
    pub(super) solids: Arc<vale_assets::CollisionWorld>,
}

/// Whether a terrain height may be stood on. This is the one test both halves
/// of the join share. An earlier form of it sent a character in Ironforge to
/// the top of the mountain.
///
/// A free function for the same reason [`crate::glue::loading::reckon`] is
/// one: a [`Standing`] holds a `MapTerrain` and cannot be built without an
/// archive, so as a method this rule would be the one part of the join no test
/// checks, and it is the part that goes visibly wrong in both directions.
///
/// The first condition is the ordinary case. A surface at or below the
/// character's head is one the character is on; a surface above it is a
/// hillside the character is inside, not a floor. Outdoors that is the whole
/// rule, and it costs nothing.
///
/// The second condition is the exception. When nothing else can be stood on,
/// the terrain is taken whatever its height. That puts a character back on the
/// ground after a teleport drops them under the world, and it is what this
/// client did before collision existed.
///
/// No building answering does not mean there is no building. A hull has holes:
/// a doorway, a stair whose group states no `MOPY`, the seam between two
/// groups. So `CollisionWorld` correctly answers nothing at some points well
/// inside a city. Under Ironforge the terrain is the mountain the city is cut
/// into, hundreds of yards up, so the exception applied there and moved the
/// character up through the roof. That was the report "you get teleported to
/// the top of the terrain above you". It looked like a loading race, and the
/// window is widest just after a login or a teleport, but the cause is a hull
/// that is present and has no surface at the point, so it did not stop by
/// itself.
///
/// The exception therefore applies only at a point that no `MODF` box contains.
/// Inside a box, this client has no floor for the point, and the mover reads
/// that as "hold the altitude the server last sent". That is the answer
/// [`vale_assets::world::adt::Adt::awaiting_building`] gives while the building
/// is still being read, and the 1.12.1 client reaches the same result by
/// waiting until the building's groups are loaded.
///
/// `inside_building` is a closure because it must not be called in the
/// ordinary case. It takes the tile lock and walks a dozen `MODF` boxes, and
/// this function runs once per mover step and once per server-driven unit per
/// update. Outdoors the first condition decides and the closure is never
/// called.
fn standable(
    ground: f32,
    ceiling: f32,
    nothing_else_answered: bool,
    inside_building: impl FnOnce() -> bool,
) -> bool {
    ground <= ceiling || (nothing_else_answered && !inside_building())
}

impl Standing {
    /// The same pair the session thread walks on, for a caller that must ask
    /// the same two questions in the same order; see [`crate::world::predict`],
    /// which continues the mover between steps and would otherwise move a
    /// stride into every wall.
    pub(super) fn new(
        terrain: Arc<vale_assets::MapTerrain>,
        solids: Arc<vale_assets::CollisionWorld>,
    ) -> Standing {
        Standing { terrain, solids }
    }

    /// The slope of the surface under the feet, as a unit normal in world
    /// axes: the answer of [`vale_protocol::socket::session::World::floor`]
    /// with the slope kept instead of dropped.
    ///
    /// It uses the same join, in the same order, with the same tie-break:
    /// whichever of the building and the ground is higher wins, and the normal
    /// returned belongs to that surface. It is written beside `floor` rather
    /// than derived from it so the two stay consistent. A model tilted by the
    /// terrain while standing on a bridge is the bug two diverging
    /// implementations of this join would produce.
    ///
    /// `None` where the client has no answer, which the caller reads as no
    /// contact: no tile yet, a hole, a character in the air. This does not
    /// repeat `floor`'s refusal while a building loads, because a missing
    /// stance costs one frame of the previous lean, not a fall through the
    /// world.
    pub fn stance(&self, map_id: u32, x: f32, y: f32, z: f32) -> Option<[f32; 3]> {
        let ceiling = z + vale_assets::world::collision::STEP_UP;
        let building = self.solids.surface(map_id, x, y, ceiling);
        let ground = self.terrain.height_at(map_id, x, y).filter(|g| {
            standable(*g, ceiling, building.is_none(), || {
                self.terrain.inside_building(map_id, x, y, z)
            })
        });
        match (building, ground) {
            (Some((b, n)), Some(g)) if b >= g => Some(n),
            (_, Some(_)) => self.terrain.normal_at(map_id, x, y),
            (Some((_, n)), None) => Some(n),
            (None, None) => None,
        }
    }
}

/// How far under the floor a game object's own surface may be and still count
/// as the surface being stood on, in yards.
///
/// The two heights come from the same triangles, so they agree exactly when the
/// object is the floor. But `floor` takes the maximum of the building and
/// terrain answers while this takes the object's alone, and a lift whose deck
/// is flush with a landing has two surfaces within a hair of each other. A
/// millimetre absorbs that difference and is far too small to reach the next
/// surface up, which on any object a character can board is a whole step.
const PLATFORM_TOLERANCE: f32 = 0.001;

/// How many tiles out from the character's own the simulation's terrain cache
/// keeps, as a radius: 2 gives a 5x5 block.
///
/// The mover reads the character's tile and, at a border, the next one. The
/// decal projector asks for heights a couple of yards around the feet. The
/// renderer's own block is the 3x3 and is a different cache. Two is one more
/// than any of those needs, so a character pacing along a border does not
/// re-read the tile on the other side of it. See `MapTerrain::retain_near`,
/// which this bounds and which was unbounded before.
const TERRAIN_KEEP: i32 = 2;

/// How many tiles out from the character's own the simulation reads ahead:
/// 1 gives the 3x3, so the tile on the other side of a border is loaded before
/// the character reaches it. It is one less than the keep radius, so a tile
/// read ahead is never dropped by the same step that asked for it. See
/// `MapTerrain::prefetch_near`.
const TERRAIN_AHEAD: i32 = 1;

impl vale_protocol::socket::session::World for Standing {
    /// The local character's position, once a tick: bound the tile cache
    /// around it and read the ring ahead. This is here rather than in `floor`,
    /// which is also called for every dead-reckoned unit; see the trait.
    fn focus(&self, map_id: u32, x: f32, y: f32) {
        self.terrain.retain_near(map_id, x, y, TERRAIN_KEEP);
        self.terrain.prefetch_near(map_id, x, y, TERRAIN_AHEAD);
    }

    fn floor(&self, map_id: u32, x: f32, y: f32, z: f32) -> Option<f32> {
        // A surface more than a step above the feet is not one the character
        // stands on; it is a deck the character walks under. The rule applies
        // to the terrain as much as to a building: a cellar is cut into a
        // hillside, so the ground above the character is as wrong an answer as
        // the roof.
        let ceiling = z + vale_assets::world::collision::STEP_UP;
        let building = self.solids.floor(map_id, x, y, ceiling);
        // A building that has not arrived means no answer, never the ground.
        // `MODF` states each placement's box in the same file the height came
        // from, so the tile that answers the height also knows a building
        // stands over it. Stormwind's terrain is far below Stormwind's streets,
        // so using the terrain in the meantime drops the character through the
        // world.
        //
        // The 1.12.1 client behaves the same way: it waits until the building
        // the point is inside has loaded, and never answers from the terrain
        // instead. This client cannot block a frame, so the mover is told there
        // is no data and holds its altitude. That altitude is the server's last
        // statement of where the character is, so it is a real standing
        // position and not a guess.
        //
        // This is checked only when no staged building answers, which is the
        // ordinary case once a city is loaded: one `MODF` walk of a dozen boxes
        // when the character is over bare ground, and a `settled` lookup only
        // for a box the character is inside.
        if building.is_none()
            && self
                .terrain
                .awaiting_building(map_id, x, y, z, |placement| {
                    !self.solids.settled(map_id, placement)
                })
        {
            return None;
        }
        let ground = self.terrain.height_at(map_id, x, y);
        // The same rule applies to the terrain, with one narrow exception: see
        // [`standable`], which states the rule and why the exception was
        // narrowed. `stance` asks the same question through the same function
        // so the two stay consistent.
        let ground = ground.filter(|g| {
            standable(*g, ceiling, building.is_none(), || {
                self.terrain.inside_building(map_id, x, y, z)
            })
        });
        match (building, ground) {
            (Some(b), Some(g)) => Some(b.max(g)),
            (b, g) => b.or(g),
        }
    }

    fn step(&self, map_id: u32, from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
        self.solids.step(map_id, from, to)
    }

    /// Which game object is underfoot, and where it is now: the question
    /// behind `MOVEFLAG_ONTRANSPORT`. See
    /// [`vale_protocol::state::movement::Ferry`].
    ///
    /// The placement comes from the same store the hull was built into, which
    /// keeps the answer consistent with the floor. `solid.rs` builds an
    /// object's hull from `Motion` (where the model is drawn, transport offset
    /// included) and records the placement it used. So the platform this
    /// reports is the one the character stands on, not the position the last
    /// packet mentioned.
    ///
    /// It uses the same `STEP_UP` ceiling as the floor query, so a deck the
    /// character walks under does not count.
    fn platform(
        &self,
        map_id: u32,
        x: f32,
        y: f32,
        z: f32,
    ) -> Option<vale_protocol::state::movement::Platform> {
        let ceiling = z + vale_assets::world::collision::STEP_UP;
        let (guid, surface) = self.solids.object_under(map_id, x, y, ceiling)?;
        // A hull below the floor is not the floor. An object can answer here
        // while a building's deck or the terrain lies above it (a crate under a
        // pier, a chest inside a room whose floor the character is on), and
        // boarding it would carry the character off with an object nowhere
        // near them. The height comparison is the whole test, and it is made
        // against the same combined answer the mover stands on.
        let floor = <Self as vale_protocol::socket::session::World>::floor(self, map_id, x, y, z)?;
        if surface + PLATFORM_TOLERANCE < floor {
            return None;
        }
        self.platform_of(map_id, guid)
    }

    /// Where a named platform is now, for a passenger whose spline is stated
    /// in the platform's frame; see
    /// [`vale_protocol::state::movement::Spline::in_world`].
    ///
    /// It reads the same store as [`Self::platform`], by guid: the placement
    /// `solid.rs` last built the object's hull at, which is where the model is
    /// drawn.
    fn platform_of(
        &self,
        map_id: u32,
        guid: u64,
    ) -> Option<vale_protocol::state::movement::Platform> {
        let at = self.solids.object_placement(map_id, guid)?;
        Some(vale_protocol::state::movement::Platform {
            guid,
            position: at.position,
            facing: at.facing,
        })
    }

    /// Whether the world holds a hull for this object at all; see
    /// [`vale_assets::CollisionWorld::object_hulled`], which describes how this
    /// differs from [`Self::platform_of`].
    ///
    /// The two differ for the continent transports:
    /// `world::entities::solid::ship_hull` builds a boat's 3,508 triangles only
    /// while the character is within 150 yards of it, so a boat that has just
    /// teleported has its new placement on record and no hull under it.
    fn platform_hulled(&self, map_id: u32, guid: u64) -> bool {
        self.solids.object_hulled(map_id, guid)
    }

    /// The terrain's `MCLQ` and nothing else; see
    /// [`vale_assets::world::terrain::Terrain::liquid_at`], which says what that
    /// leaves out and why. It uses the same tile cache as the height, so a
    /// character walking into a lake costs no lookup the ground did not already
    /// make.
    fn liquid(&self, map_id: u32, x: f32, y: f32) -> Option<f32> {
        self.terrain.liquid_at(map_id, x, y).map(|(_, z)| z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character inside a building's box is never placed on the ground
    /// above them, which is the Ironforge report, and the three other cases
    /// around it all keep working.
    ///
    /// The case that used to be wrong is the third: the hull is loaded, it
    /// answers nothing at this exact point (a doorway, a seam, a stair with no
    /// `MOPY`), and the terrain overhead is the mountain the city is cut into.
    /// Taking the terrain there moves the character hundreds of yards up,
    /// through the roof.
    #[test]
    fn the_ground_over_a_characters_head_is_a_floor_only_outside_a_building() {
        // Standing on it: the ordinary case, everywhere in the world.
        assert!(standable(10.0, 12.0, true, || panic!("not asked outdoors")));
        assert!(standable(10.0, 12.0, false, || panic!("not asked outdoors")));

        // Under it, with nothing over the point: taken anyway, which puts a
        // character back on the ground after a teleport drops them beneath the
        // world.
        assert!(standable(300.0, 12.0, true, || false));

        // Under it, inside a building whose hull has no surface here: refused,
        // so the mover holds the altitude the server gave it.
        assert!(!standable(300.0, 12.0, true, || true));

        // Under it with something else already answering (a floor, a bridge,
        // an elevator): the terrain overhead is wrong whatever is over the
        // point. The closure must not decide this case.
        assert!(!standable(300.0, 12.0, false, || panic!("already answered")));
    }
}
