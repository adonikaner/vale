//! **The platforms that move with nothing telling them to** — where an
//! elevator actually is, this frame.
//!
//! ```text
//! wire        UPDATEFLAG_TRANSPORT's trailing word -> Entity::transport_phase_ms
//!   + clock   ObjectManager::advance carries it forward
//!   + table   assets::tables::transport -> an offset in the object's own frame
//!   = here    …rotated into the world and added to the stationary position
//! ```
//!
//! Three lines of arithmetic in a file of its own, because the *thing* is a
//! subject: an elevator is the only object in the world whose position is never
//! stated by anybody, and every layer it crosses had to learn a word for it. See
//! [`vale_assets::tables::transport`], which holds the rule and the whole
//! argument for the phase; this is only the frame conversion and the join.
//!
//! ## Why this is one function and not a system
//!
//! There is exactly one place a tracked position becomes a drawn one —
//! `poll_world` hands `Motion` a `(guid, position, facing)` per entity, and
//! **both** the drawn model and the collision hull read back out of `Motion`
//! (see `world::entities::solid`, whose own comment says why the hull must be
//! where the model is *drawn* rather than where the last packet said). Adding
//! the offset at that one point moves the picture and the wall together and
//! cannot put them a frame apart.
//!
//! A system of its own would have to write somewhere, and the only somewhere is
//! the entity's tracked position — which is the **stationary** point and must
//! stay it. Add the offset there and the next frame adds it again.
//!
//! ## The frame, and the one thing that could be wrong
//!
//! The table's offsets are in the object's own frame. This client places a
//! server-spawned game object with `T · Rz(facing)` and nothing else — see
//! [`vale_assets::world::collision::object_matrix`], which says why the
//! four-float `GAMEOBJECT_ROTATION` quaternion beside the facing is not what
//! either end collides against — so the offset is rotated by the same `Rz` the
//! model is, which is the only self-consistent choice: the platform's travel is
//! authored in the frame its mesh is drawn in.
//!
//! **vmangos rotates by the full quaternion and then negates `y`**
//! (`ElevatorTransport::Update`, with the comment *"magical sign flip but it
//! works — vanilla/tbc only"*). That is a different composition, and it is
//! stated here rather than copied because **every elevator in the game is
//! immune to the difference**: `x` and `y` read `±0.000` for all of 4170, 4171,
//! 11898, 20649 and their siblings, so the offset is a pure `z` and no rotation
//! about the vertical touches it. The population that *would* show a mistake is
//! the tram, whose car travels 2,482 yards along `x` — and it **is** drawn, so
//! the risk is real and lands on something. (An earlier version of this note
//! said the tram was a `.wmo` this client declines to place; it is
//! `SUBWAYCAR.m2`, display 3831, and the round that measured that is in the
//! protocol facts.) What bounds it is that both compositions rotate by the car's
//! own placement facing, so they agree wherever the server spawns a transport
//! axis-aligned — which is where every shipped one sits. **Nobody has watched a
//! ride to check**, which is the cheapest way to settle it and is one login.
//!
//! ## …and it does carry you now, on the deck's own clock
//!
//! The note here used to end *"it does not carry you"*, and that was the
//! Deeprun Tram report: the car travels 2,482 yards along `x` and the character
//! standing on it did not. The relationship is `MOVEFLAG_ONTRANSPORT` and a
//! position in the transport's own frame, and the whole of it is the **client's**
//! — `WorldSession::HandleMoverRelocation` boards a player only when their own
//! movement packet arrives carrying the flag and the guid, so a client that
//! never sets it is never a passenger and `UpdatePassengerPositions` never moves
//! it.
//!
//! Where that lives: `vale_protocol::state::movement::Ferry` holds the
//! offset and the two transforms,
//! `vale_protocol::socket::session::World::platform` is how the session asks
//! what is underfoot, and `crate::world::session::Standing` answers it out of
//! the collision world — the same store `solid.rs` builds the hull into, so the
//! platform reported and the floor being stood on are the same thing by
//! construction.
//!
//! **And the passenger is *drawn* against this function's own answer rather
//! than against the session's.** The offset is stepped ~40 times a second and
//! the deck is drawn at frame rate a step and a half behind it, so composing
//! the two from the session's clock paints the character against a deck that is
//! somewhere else — 0.43 yards of slide forty times a second on the tram, with
//! the camera framed on it. `crate::world::predict::reframe` recomposes them
//! against the `Motion` sample `place_entities` is about to draw the deck at.
//!
//! ## …and the boats and the zeppelins are a second kind entirely
//!
//! Everything above is `GAMEOBJECT_TYPE_TRANSPORT` (11): a lift, a tram car, a
//! platform that shuttles about a **fixed origin** the server states, with the
//! table giving an offset from it. A ship is
//! `GAMEOBJECT_TYPE_MO_TRANSPORT` (15) and shares none of that:
//!
//! ```text
//!            elevator (11)                  ship (15)
//!   origin   the server's stationary point  literally (0, 0, 0)
//!   path     TransportAnimation.dbc         TaxiPathNode.dbc, via the template
//!   shape    an offset to add               an absolute place, on a named map
//!   map      always this one                two, and it crosses between them
//! ```
//!
//! The origin is not an approximation: `GameObject::GetStationaryX` returns a
//! literal `0.f` for this type, and that zero is what the wire carries. So a
//! ship's position is **entirely** [`vale_assets::tables::shiptransport`]'s
//! arithmetic — see [`ship_placement`], and `vale ships` for the check.
//!
//! ## What this still does not do
//!
//! It does not play the platform's own animation — see
//! [`vale_assets::tables::transport::Node::sequence`], which is read, carried
//! and used by nothing.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use std::sync::Arc;

use vale_assets::tables::shiptransport::{MoTransport, Placement, Route};
use vale_assets::tables::taxi::TaxiTables;
use vale_assets::tables::transport::Transports;
use vale_protocol::state::objects::Entity;
use vale_protocol::state::query::GameObjectInfo;
use vale_protocol::state::update::ObjectType;

/// **Where this object is now, relative to where it was spawned** — in world
/// axes, or `None` for everything that is not a moving platform.
///
/// `None` is the answer for every object in the world but a few dozen: one that
/// is not a game object, one the server never flagged `UPDATEFLAG_TRANSPORT`,
/// and one whose entry `TransportAnimation.dbc` has no rows for — which
/// includes both Ironforge lifts, and is the file's own answer rather than a
/// gap.
///
/// `facing` is the server's orientation in radians, the same one
/// [`vale_assets::world::collision::object_matrix`] takes.
pub fn offset_of(entity: &Entity, facing: f32, transports: &Transports) -> Option<Vec3> {
    if entity.object_type != Some(ObjectType::GameObject) {
        return None;
    }
    let phase = entity.transport_phase_ms?;
    let entry = entity.entry()?;
    let [x, y, z] = transports.offset_at(entry, phase)?;
    // `Rz(facing)` — the object's own placement rotation, and see the module
    // note for what is being taken on trust here and why it is bounded.
    let (sin, cos) = facing.sin_cos();
    Some(Vec3::new(x * cos - y * sin, x * sin + y * cos, z))
}

/// **The routes, built once per template and kept.**
///
/// A [`Route`] is a Catmull-Rom curve and a schedule over thirty-odd waypoints;
/// it is cheap to build and not cheap enough to build sixty times a second, and
/// there are nine of them in the game. Keyed by the game object's **entry**,
/// because that is what the template arrived under and what every copy of the
/// same boat shares.
///
/// A `None` value is a settled negative — a template that is not a transport, a
/// `taxiPathId` with no waypoints — so it is asked once rather than every frame.
#[derive(Resource, Default)]
pub struct ShipRoutes(HashMap<u32, Option<Arc<Route>>>);

impl ShipRoutes {
    /// The route for this template, building it the first time it is asked for.
    ///
    /// `taxi` is [`vale_assets::tables::dbc::DisplayTables::taxi`], which the
    /// flight map already loads — a boat and a gryphon read the same three
    /// tables, which is the whole reason `TaxiPath.dbc` has thirteen rows no
    /// flight master offers.
    pub fn of(&mut self, info: &GameObjectInfo, taxi: &TaxiTables) -> Option<Arc<Route>> {
        if let Some(known) = self.0.get(&info.entry) {
            return known.clone();
        }
        let route = MoTransport::read(info.object_type, &info.data)
            .and_then(|template| Some((taxi.waypoints(template.taxi_path)?, template)))
            .and_then(|(path, template)| Route::build(path, template))
            .map(Arc::new);
        self.0.insert(info.entry, route.clone());
        route
    }

    /// Forget everything — a logout, or a map this client is no longer on. The
    /// routes themselves do not go stale, but the map does: see
    /// [`crate::world::residency`].
    pub fn reset(&mut self) {
        self.0.clear();
    }
}

/// **Where a continent transport is right now**, in world axes, or `None` for
/// everything that is not one.
///
/// Unlike [`offset_of`] this is a *place* rather than a displacement, and it
/// carries the map: a route crosses an ocean, so the same boat is on Azeroth
/// for half its cycle and Kalimdor for the other half, and the caller has to
/// drop it while it is elsewhere. The server says the same thing a moment later
/// with an out-of-range block; drawing a Kalimdor boat off Menethil in the
/// meantime is a ship in the wrong sea.
///
/// `facing` is the direction of travel and nothing else —
/// `ShipTransport::Update` ends in `atan2(dir.y, dir.x) + PI` off the spline's
/// own derivative, with no pitch and no roll.
pub fn ship_placement(entity: &Entity, route: &Route) -> Option<Placement> {
    if entity.object_type != Some(ObjectType::GameObject) {
        return None;
    }
    let phase = entity.transport_phase_ms?;
    route.at((phase % u64::from(route.period().max(1))) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::state::objects::ObjectManager;

    /// A lift that drops 60 yards over ten seconds and comes straight back —
    /// the shape the shipped file has, with the cycle being the last node's own
    /// time.
    fn table() -> Transports {
        let mut raw = Vec::new();
        raw.extend_from_slice(b"WDBC");
        raw.extend_from_slice(&3u32.to_le_bytes());
        raw.extend_from_slice(&7u32.to_le_bytes());
        raw.extend_from_slice(&28u32.to_le_bytes());
        raw.extend_from_slice(&1u32.to_le_bytes());
        for (id, time, offset) in [
            (0u32, 0u32, [0.0f32, 0.0, 0.0]),
            (1, 10_000, [0.0, 0.0, -60.0]),
            (2, 20_000, [0.0, 0.0, 0.0]),
        ] {
            raw.extend_from_slice(&id.to_le_bytes());
            raw.extend_from_slice(&ENTRY.to_le_bytes());
            raw.extend_from_slice(&time.to_le_bytes());
            for axis in offset {
                raw.extend_from_slice(&axis.to_le_bytes());
            }
            raw.extend_from_slice(&162u32.to_le_bytes());
        }
        raw.push(0);
        Transports::parse(&raw)
    }

    /// The Mesa Elevator's own entry, so the numbers below are recognisable.
    const ENTRY: u32 = 4170;
    const GUID: u64 = 0xF110_0000_0000_0007;

    /// **One `SMSG_UPDATE_OBJECT` creating a transport**, byte for byte as
    /// `Object::BuildMovementUpdate` writes one: `HAS_POSITION` carries the
    /// *stationary* point, `TRANSPORT` appends the path progress, and the
    /// values block names the entry.
    fn create(progress: u32) -> Vec<u8> {
        use vale_protocol::state::update::update_flags;
        let mut w = vale_protocol::bytes::Writer::new();
        w.u32(1); // one block
        w.u8(0); // no transport guid header
        w.u8(2); // CREATE_OBJECT2
        // A packed guid: a mask byte, then the non-zero bytes, low first.
        let mut mask = 0u8;
        let mut bytes = Vec::new();
        for i in 0..8 {
            let byte = (GUID >> (i * 8)) as u8;
            if byte != 0 {
                mask |= 1 << i;
                bytes.push(byte);
            }
        }
        w.u8(mask);
        for byte in bytes {
            w.u8(byte);
        }
        w.u8(5); // ObjectType::GameObject
        w.u8(update_flags::HAS_POSITION | update_flags::TRANSPORT);
        for f in [100.0f32, 200.0, 300.0, 0.0] {
            w.u32(f.to_bits());
        }
        w.u32(progress);
        // …and the values block: one mask word, `object::ENTRY` set.
        let entry = vale_protocol::state::fields::object::ENTRY;
        w.u8(1);
        w.u32(1 << entry);
        w.u32(ENTRY);
        w.buf
    }

    fn world(progress: u32) -> ObjectManager {
        let mut world = ObjectManager::new();
        world.apply(&vale_protocol::state::update::parse(&create(progress)));
        world
    }

    /// **The whole chain, from the one word on the wire to a place in the
    /// world**: `UPDATEFLAG_TRANSPORT`'s trailing `u32` becomes a phase, the
    /// world's clock carries it forward, and the table turns it into an offset.
    ///
    /// Written end to end rather than as three unit tests because every layer
    /// it crosses is a place this could silently answer nothing — a parser that
    /// discards the word (which is what it did), a phase nothing advances, an
    /// entry read off the wrong field. Each of those draws the platform at its
    /// spawn point, which is the report, and none of them fails anything.
    #[test]
    fn the_wires_one_word_becomes_a_place_in_the_world() {
        let table = table();
        let mut world = world(0);
        let entity = world.get(GUID).expect("the create block made one");
        assert_eq!(entity.transport_phase_ms, Some(0), "the word was read");
        assert_eq!(entity.entry(), Some(ENTRY));
        let at = offset_of(entity, 0.0, &table).expect("it animates");
        assert!(at.z.abs() < 1e-4, "at the top: {at:?}");

        // Five seconds of world time — in the steps the session thread really
        // takes, because `advance` is what carries the phase.
        for _ in 0..200 {
            world.advance(25, None);
        }
        let at = offset_of(world.get(GUID).expect("still there"), 0.0, &table)
            .expect("it animates");
        assert!((at.z + 30.0).abs() < 1e-3, "halfway down: {at:?}");
    }

    /// **The server's own reading is where the cycle starts**, which is the
    /// half that cannot be guessed: two clients that logged in hours apart see
    /// the same lift at the same height because each is told where in the cycle
    /// it already is.
    #[test]
    fn the_phase_starts_where_the_server_says_and_not_at_zero() {
        let table = table();
        let world = world(15_000);
        let at = offset_of(world.get(GUID).expect("created"), 0.0, &table).expect("animates");
        assert!((at.z + 30.0).abs() < 1e-3, "halfway back up: {at:?}");
    }

    /// **A lift moves straight down whatever it is facing** — the property the
    /// whole frame argument in the module note rests on, and the reason the
    /// difference from vmangos' composition costs nothing on this population.
    #[test]
    fn a_lift_moves_straight_down_whatever_it_is_facing() {
        let table = table();
        let mut world = world(0);
        for _ in 0..200 {
            world.advance(25, None);
        }
        let entity = world.get(GUID).expect("created");
        for facing in [0.0, 1.2, std::f32::consts::PI, -2.7] {
            let at = offset_of(entity, facing, &table).expect("it animates");
            assert!(at.x.abs() < 1e-4 && at.y.abs() < 1e-4, "facing {facing}: {at:?}");
            assert!((at.z + 30.0).abs() < 1e-3, "facing {facing}: {at:?}");
        }
    }

    /// **The three ways an object is not a platform**, each of which answers
    /// `None` rather than the origin — the caller reads `None` as "leave the
    /// tracked position alone", and the origin is a real position a lift can
    /// be at.
    #[test]
    fn everything_that_is_not_a_platform_answers_nothing() {
        let table = table();
        let mut world = world(0);

        // A game object the server never flagged: no phase, no motion.
        let unflagged = world.get_mut(GUID).expect("created");
        unflagged.transport_phase_ms = None;
        assert_eq!(offset_of(world.get(GUID).unwrap(), 0.0, &table), None);

        // A unit that somehow carries one.
        let unit = world.get_mut(GUID).expect("created");
        unit.transport_phase_ms = Some(0);
        unit.object_type = Some(ObjectType::Unit);
        assert_eq!(offset_of(world.get(GUID).unwrap(), 0.0, &table), None);

        // …and an entry the table has no rows for: a door, a chest, and both
        // Ironforge lifts, which this file simply does not describe.
        let other = world.get_mut(GUID).expect("created");
        other.object_type = Some(ObjectType::GameObject);
        other.fields.insert(vale_protocol::state::fields::object::ENTRY, 32056);
        assert_eq!(offset_of(world.get(GUID).unwrap(), 0.0, &table), None);
    }
}
