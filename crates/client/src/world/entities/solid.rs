//! …and which of them you cannot walk through.
//!
//! Every solid thing in the world so far was placed by a *file*: a building out
//! of an `MODF` row, a tree or a fence out of an `MDDF` one, the furniture
//! inside a building out of `MODD`. All three are staged by the passes that
//! draw them ([`crate::render::wmos`], [`crate::render::doodads`]) and all three
//! live and die with the tile that owns them.
//!
//! **A game object is none of those.** A door, a portcullis, a chest and a
//! mailbox are entities the *server* spawns: they arrive with a
//! `GAMEOBJECT_DISPLAYID` and a position, they can be taken away mid-session,
//! and — the part that is not like anything above — **they are solid only in one
//! of their states.** So there was no pass that could have placed them and no
//! store that could have held them, which is the whole of "impassable
//! gameobjects like the Deadmines boss doors do not have any collision".
//!
//! ## The rules are all one layer down, on purpose
//!
//! Three questions decide what happens here and not one of them needs a mesh, a
//! material or a window, so all three are `vale_assets::world::collision`'s:
//! [`game_object_is_solid`] is *whether*, [`object_matrix`] is *where*, and the
//! model's own `BoundingTriangles` block is *what*. This module is the join —
//! it asks the world what it has, asks the cache for the model, and keeps
//! [`Solids`] in step. `vale objects` checks the same three against the real
//! archives with nothing running.
//!
//! ## What it does *not* place, and why each one is deliberate
//!
//! * **A model with no hull.** 678 of the 1,622 game-object models in the game
//!   carry no `BoundingTriangles` at all (`vale objects`), and that is the
//!   game saying *walk through me* — a campfire, a fishing bobber, a light.
//!   Recorded as a finished answer rather than retried, which is what
//!   `CollisionWorld::place_object`'s `None` is for.
//! * **A `.wmo` game object.** The zeppelins and the boats are buildings rather
//!   than M2s, and `spawn::spawn_models` already declines to draw one, so a hull
//!   nailed to its embarkation point would be a wall in the middle of the sea.
//!   **The Deeprun Tram is not one of them** — `SUBWAYCAR.m2`, display 3831,
//!   550 collision triangles — so its car is drawn and hulled here like any
//!   other game object, and the hull moves with it through the same
//!   furthest-moved-first budget the elevators use. The passenger half is
//!   `vale_protocol::state::movement::Ferry` and works off exactly this hull:
//!   `Standing::platform` asks which object's surface is underfoot, and the
//!   answer is the placement recorded below.
//! * **The player's own collision against other units.** Still nobody's.
//!   This is a placed hull, not a moving capsule.

use bevy::prelude::*;
use std::collections::HashSet;
use std::sync::Arc;

/// **What the hull budget was asked for last frame and what it paid**, for the
/// `F4` scene tab.
///
/// Three numbers, because the failure they exist for is invisible from the two
/// that were already there. `solid: N hulls` counts what is *held*, and a hull
/// held at the wrong position counts exactly the same as one held at the right
/// position — which is how a starved elevator went a whole round with no
/// instrument that could see it, while every count on the panel read correctly.
///
/// **Read `worst` first.** It is yards: how far the most-wrong hull in the
/// world is from the object it belongs to, *after* this frame's spend. A door
/// that has just opened puts a large number here for one frame; a number that
/// stays large is a hull nobody is paying for, and whatever is standing on it
/// will fall through.
///
/// Written unconditionally rather than sampled while the panel is open — it is
/// three writes on a system already walking the list, and the rule that nothing
/// accumulates behind a shut panel is about sampling work, of which there is
/// none here.
#[derive(Resource, Default, Clone, Copy)]
pub struct HullWork {
    /// Game-object hulls whose placement no longer matches their object.
    pub wanted: usize,
    /// …and how many of those the frame **settled** — which is not quite the
    /// same as rebuilt, and the difference is deliberate. A `.wmo` game object
    /// (the tram, a boat) and a model the loader gave up on are both recorded
    /// at their new placement with no hull, cost no triangles, and are done
    /// with; counting them as unpaid would leave the panel permanently
    /// reporting a backlog for two populations that are working exactly as
    /// intended. What is *not* counted is a candidate the loop could not decide
    /// yet — no display id, an unresolved display, a model still loading — and
    /// that is right: it will be asked again next frame.
    pub paid: usize,
    /// The largest [`staleness`] left unpaid, in yards. Zero when the budget
    /// covered everything, which is the ordinary case and the one to expect.
    pub worst: f32,
}

use super::DisplayCache;
use crate::render::models::{Lookup, ModelCache};
use crate::world::motion::Motion;
use crate::world::session::{Session, Solids, WorldEntity};
use vale_assets::world::collision::{game_object_is_solid, object_matrix, Collider, ObjectPlacement};
use vale_protocol::state::update::ObjectType;

/// How much hull work to do in one frame, **in triangles**.
///
/// `Collider::place` transforms and re-indexes every triangle in the hull, and
/// the heaviest game object in the game is 1,118 of them — so a dungeon door
/// ward opening onto a room of twenty is work worth spreading, on the same
/// argument (and roughly the same size) as `spawn::SPAWN_BUDGET`.
///
/// **It was a count of four hulls, and that was the elevator bug.** A count is
/// the wrong unit the moment anything rebuilds *every* frame: an elevator car
/// is 90 triangles and a portcullis is 1,118, so four of the former is a
/// twelfth of the work four of the latter is, and the same number bought both.
/// With more than four platforms in view the budget was spent in full every
/// frame by whichever four came first in the query — a **stable** order — so
/// the rest were starved for as long as the winners kept moving, and only
/// caught up when one of them parked at a floor and stopped asking. That is
/// exactly what a player sees as *"the collision stays put until the lift
/// reaches the top, then teleports up there"*, and it is what
/// [`spend_order`] is about.
const PLACE_BUDGET_TRIANGLES: usize = 2_400;

/// Keep [`Solids`] holding exactly the game objects that are in view and shut.
///
/// **Reconciled rather than evented**, and that is the design decision worth
/// stating. A hull has to be dropped when a door opens, when the object goes out
/// of range, when the server destroys it, when its display id changes under it
/// and when the whole map changes — five different messages, of which this
/// client raises none. Every one of them is the same statement seen from a
/// different side: *this GUID is not solid here any more*. Asking the world what
/// it currently has and making the store agree answers all five with one rule
/// and cannot get one of them wrong on its own.
///
/// The cost of that is a walk of the game objects in view once a frame, which is
/// a `HashSet` insert apiece and a hash lookup apiece — the hull work itself is
/// behind an exact placement comparison and happens only when something actually
/// changed, which for a door is twice in its life.
pub(super) fn solidify_objects(
    mut work: ResMut<HullWork>,
    mut cache: ResMut<ModelCache>,
    // …and the *other* cache, for the one population here that is a building
    // rather than an M2 — see [`ship_hull`].
    mut wmos: ResMut<crate::render::wmos::WmoCache>,
    displays: Res<DisplayCache>,
    motion: Res<Motion>,
    time: Res<Time>,
    session: Res<Session>,
    solids: Res<Solids>,
    objects: Query<&WorldEntity>,
) {
    // No map means no session, and a hull keyed to the wrong map is a door
    // standing in the middle of a field — `render::doodads` opens the same way.
    let Some(map_id) = session.active.as_ref().map(|a| a.map_id) else {
        // **Reset rather than left standing.** A report holding last session's
        // numbers is worse than none: the panel would show a backlog for a
        // world that is not there.
        *work = HullWork::default();
        return;
    };
    let now = time.elapsed_secs();
    // **Where the character is**, for the one population whose hull is worth
    // building only when somebody could be standing on it — see [`ship_hull`].
    // `None` before the player entity exists, which reads as "nowhere near
    // anything" and is right: nothing can board a ship during a login burst.
    let standing = objects
        .iter()
        .find(|e| e.is_self)
        .and_then(|e| motion.position_of(e.guid, now));

    // **The ones that are both in view and shut**, which is one set rather than
    // two: see `CollisionWorld::retain_objects`.
    let mut live: HashSet<u64> = HashSet::new();
    let mut wanted: Vec<(&WorldEntity, ObjectPlacement)> = Vec::new();
    for entity in &objects {
        if entity.kind != ObjectType::GameObject || !game_object_is_solid(entity.object_state) {
            continue;
        }
        // Read from `Motion`, at the same clock and through the same two calls
        // `place_entities` uses — so the hull is where the model is *drawn* and
        // not merely where the last packet said. A game object does not move, so
        // the two cannot fall a frame apart; a door on a lift would, and that is
        // a transport, which is excluded above.
        //
        // **`Motion` is in WoW's axes and it is the renderer that converts.** It
        // is fed `vale_protocol::state::update::Position` verbatim and
        // `place_entities` is the one place a tracked position becomes Bevy's
        // (`axes::to_bevy`), so a hull built from one takes it **unchanged**.
        // Converting here — which this pass did in its first draft — places
        // every door at `[-z, -x, y]` of itself: the hull exists, the counts are
        // right, the door is shut, and you walk straight through it, because the
        // wall is somewhere else in the zone entirely.
        let (Some(position), Some(facing)) = (
            motion.position_of(entity.guid, now),
            motion.facing_of(entity.guid, now),
        ) else {
            continue;
        };
        live.insert(entity.guid);
        wanted.push((
            entity,
            ObjectPlacement {
                position: position.to_array(),
                facing,
                // `OBJECT_FIELD_SCALE_X`, which is what vmangos' own
                // `GameObjectModel::initialize` scales its bounds by. A game
                // object that never stated one is at 1.0 and not at zero.
                scale: entity.scale.filter(|s| *s > 0.01).unwrap_or(1.0),
            },
        ));
    }

    // Anything not in that set has no hull here now — an opened door, a looted
    // chest that despawned, a `SMSG_DESTROY_OBJECT`, or a teleport that took the
    // whole map with it.
    solids.0.retain_objects(&live);

    // **The ones that actually want work, furthest-moved first.**
    //
    // The filter is what keeps this off the hot path: a door that has not moved
    // answers the placement it was given and wants nothing at all, which is
    // every game object in view on all but two frames of its life. The
    // *ordering* is what stops a moving platform being starved — see
    // [`spend_order`], which is the whole argument and is where the elevator
    // report was.
    let mut changed: Vec<(&WorldEntity, ObjectPlacement, f32)> = wanted
        .into_iter()
        .filter_map(|(entity, at)| {
            match solids.0.object_placement(map_id, entity.guid) {
                Some(placed) if placed == at => None,
                // Never placed: it has to be built, and it goes to the front.
                None => Some((entity, at, NEVER_PLACED)),
                Some(placed) => Some((entity, at, staleness(placed.position, at.position))),
            }
        })
        .collect();
    spend_order(&mut changed);
    *work = HullWork {
        wanted: changed.len(),
        paid: 0,
        worst: 0.0,
    };

    let mut budget = PLACE_BUDGET_TRIANGLES;
    for (n, (entity, at, _)) in changed.iter().copied().enumerate() {
        if budget == 0 {
            // The list is sorted descending, so whatever the budget stopped at
            // *is* the worst thing left unpaid.
            work.worst = changed[n].2;
            return;
        }
        let Some(display_id) = entity.display_id else {
            continue;
        };
        let Some(display) = displays.resolved(ObjectType::GameObject, display_id) else {
            // The display cache resolves on the *drawing* pass's `&mut` borrow;
            // a miss here is a game object whose model has not been asked for
            // yet, and it is asked for a frame later. Deliberately not resolved
            // here: two resolvers for one table is how the table's own
            // degradations start disagreeing.
            continue;
        };
        // **A `.wmo` game object is a continent transport, and it does get a
        // hull now** — see [`ship_hull`], which is where the whole of why it is
        // not built the way everything else here is built lives.
        if display.path.to_ascii_lowercase().ends_with(".wmo") {
            let hull = ship_hull(&mut wmos, &display.path, at, standing);
            // **Charged like everything else, and it will usually be the whole
            // frame's budget.** A ship is 3,508 triangles against a 2,400
            // budget, so it saturates — which is right and is the reason the
            // charge is taken *after* the decision to build (see below): a rule
            // that refused to start a hull bigger than the whole budget would
            // never build one at all. What it costs is that a door opening in
            // the same frame waits a frame, and `spend_order` guarantees it is
            // then the most-wrong thing in the world and goes first.
            if let Some(hull) = hull.as_ref() {
                budget = budget.saturating_sub(hull.triangle_count());
            }
            solids.0.place_object(map_id, entity.guid, at, hull);
            work.paid += 1;
            continue;
        }
        let model = match cache.lookup(&display.path) {
            Lookup::Ready(model) => model,
            // Still reading it — try again next frame. Deliberately *not*
            // recorded as placed: that would settle the object on an answer
            // nobody has yet.
            Lookup::Loading => continue,
            // Reported once by the loader. Settled, so it is not re-requested
            // once a frame for the rest of the session.
            Lookup::Failed => {
                solids.0.place_object(map_id, entity.guid, at, None);
                work.paid += 1;
                continue;
            }
        };
        // **Charged in triangles, and always at least one hull a frame.** A
        // 1,118-triangle portcullis is over half the budget on its own; a rule
        // that refused to start one until the whole of it fitted would never
        // build it at all on a frame that had already spent anything. So the
        // charge is taken after the decision to build rather than before it,
        // and the budget saturates at zero.
        budget = budget.saturating_sub(model.collision.triangle_count());
        work.paid += 1;

        // **A hull-less model is an answer, not a failure** — it is most of this
        // table, and it is the game's own way of saying walk through me.
        let hull = (!model.collision.is_empty()).then(|| {
            let matrix = object_matrix(at.position, at.facing, at.scale);
            Arc::new(Collider::place(&model.collision, &matrix))
        });
        solids.0.place_object(map_id, entity.guid, at, hull);
    }
}

/// How near the character a continent transport has to be before its hull is
/// built, in yards.
///
/// **A gate rather than a budget, and it is the difference between the feature
/// costing nothing and costing a frame.** The server sends every transport on
/// the map at *login*, whatever the distance — `Map::SendInitTransports` walks
/// them all and its own comment calls it a hack — so a session on Azeroth holds
/// four or five ships from the moment it starts, each 3,508 solid triangles, and
/// each somewhere new every frame. `Collider::place` transforms and re-indexes
/// every one of those triangles: **~0.16 ms a placement, measured by `vale
/// ships`**, so hulling them unconditionally is most of a millisecond a frame
/// spent on decks nobody is within a quarter of a mile of.
///
/// 150 yards is comfortably more than the longest of them and enough to have
/// the hull already standing by the time anybody walks up a gangplank. A docked
/// ship is not moving, so its hull is built once and then costs nothing at all —
/// the expensive case is a ship under way, which is exactly when somebody is on
/// it.
const TRANSPORT_HULL_RANGE: f32 = 150.0;

/// **The hull of a building the server spawns** — a boat, a zeppelin.
///
/// Two things separate it from every other hull this pass builds, and both are
/// stated rather than worked around:
///
/// * **The geometry is `WmoCache`'s rather than `ModelCache`'s.** A transport is
///   a `.wmo`, so the M2 loader has nothing to say about it; the WMO cache is
///   path-keyed and already holds it for `render::ships` to draw.
/// * **The matrix is [`object_matrix`], exactly as every other hull here.** It
///   was `adt::placement_matrix` for one round, on the reasoning that WMO
///   vertices are authored a half-turn round from the yaw a placement states —
///   but that `Rz(pi)` is the internal-to-world flip an `MDDF`/`MODF` row's own
///   encoding needs, not a property of the model, and a boat's placement never
///   went through that frame. `object_matrix`'s own doc says so. Getting it
///   wrong is a hull a half-turn round from the picture: a boat you can walk
///   through from the bow and not from the stern. See `render::ships`, which is
///   the same statement on the drawing side and has to agree with this line.
///
/// `None` while the building is still being read, which the caller records as a
/// settled *placement* with no hull — and then re-asks next frame, because the
/// placement of a moving ship differs every frame and it re-enters the work list
/// on its own.
fn ship_hull(
    wmos: &mut crate::render::wmos::WmoCache,
    path: &str,
    at: ObjectPlacement,
    standing: Option<Vec3>,
) -> Option<Arc<Collider>> {
    let standing = standing?;
    if standing.distance(Vec3::from_array(at.position)) > TRANSPORT_HULL_RANGE {
        return None;
    }
    let crate::render::wmos::Lookup::Ready(ready) = wmos.lookup(path) else {
        return None;
    };
    if ready.collision().is_empty() {
        return None;
    }
    let matrix = object_matrix(at.position, at.facing, at.scale);
    Some(Arc::new(Collider::place(ready.collision(), &matrix)))
}

/// **How wrong a hull's position is**, in yards — the key
/// [`spend_order`] ranks by.
///
/// Squared would do for an ordering and is not used, because this is also the
/// number a reader wants when asking *why* something was rebuilt, and it is
/// what the debug panel prints.
///
/// **Never negative and never `NaN`.** A position the server has not stated
/// properly would otherwise sort *above* [`NEVER_PLACED`] — `f32::total_cmp`
/// ranks a positive `NaN` above every finite value and above infinity — and an
/// object whose placement can never compare equal would then hold the whole
/// budget every frame, for ever. That is the same starvation this function
/// exists to end, arriving through the fix rather than through the bug, so it
/// is closed here rather than in the sort: a coordinate with no number in it is
/// not a reason to rebuild anything.
fn staleness(placed: [f32; 3], now: [f32; 3]) -> f32 {
    let (dx, dy, dz) = (now[0] - placed[0], now[1] - placed[1], now[2] - placed[2]);
    let moved = (dx * dx + dy * dy + dz * dz).sqrt();
    if moved.is_finite() {
        moved
    } else {
        0.0
    }
}

/// The staleness of a hull that does not exist yet, which outranks any amount
/// of being in the wrong place. Finite on purpose — see [`staleness`].
const NEVER_PLACED: f32 = f32::MAX;

/// **Spend the frame's hull budget on whatever is most wrong**, furthest-moved
/// first.
///
/// A budget with a fixed spend order is a budget that starves everything past
/// the cut, because the ECS query order is **stable**: the same candidates come
/// first every frame, so once demand exceeds supply the tail is never reached
/// again — not "later", *never*, for as long as the head keeps asking. That is
/// not a hypothetical. Before this existed, four moving platforms could hold the
/// whole budget, and any beyond them kept the hull they were last built with
/// until one of the winners reached a floor and stopped changing. A player
/// standing on a starved one falls through a platform they can see under their
/// feet, and the hull appears to teleport up the shaft whenever a *different*
/// lift parks.
///
/// Ordering by displacement fixes it for a reason worth stating rather than by
/// being fairer in general: **the object that has moved furthest from its hull
/// is by definition the one whose hull is most wrong**, and a platform carrying
/// a character is moving metres a second while a door that jiggled is moving
/// millimetres. It is also a total order with no memory, so there is no cursor
/// to keep in step with a candidate list that changes every frame.
///
/// A never-placed object sorts first (`f32::INFINITY`): it has no hull at all,
/// which is worse than any amount of staleness.
fn spend_order<A, B>(changed: &mut [(A, B, f32)]) {
    // Descending, and `total_cmp` rather than `partial_cmp` so an infinity — or
    // a NaN from a position the server has not stated properly — has a defined
    // place instead of panicking the sort.
    changed.sort_by(|a, b| b.2.total_cmp(&a.2));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::axes;
    use bevy::math::Mat4;

    /// **The furthest-moved hull is rebuilt first, and nothing is starved.**
    ///
    /// This is the elevator report: *"the collision mesh stays in place until
    /// the elevator reaches the top, then it teleports up there."* The cause
    /// was a budget of four **hulls** spent in ECS query order, which is
    /// **stable** — so once more than four objects wanted a rebuild every
    /// frame, the same four won every frame and the rest were never reached
    /// again for as long as the winners kept moving. A platform past the cut
    /// kept the hull it was last built with; a player standing on it fell
    /// through a floor they could see. It caught up only when one of the
    /// winners parked at a dwell and stopped asking, which is the "teleport".
    ///
    /// The first assertion is the direct one. The second is the property that
    /// makes the ordering a *fix* rather than a different arbitrary order: with
    /// staleness as the key, an object skipped this frame is more stale next
    /// frame, so it climbs, so it is reached. There is no cursor to keep and no
    /// order to preserve across frames.
    ///
    /// **A model of the loop rather than the loop**, deliberately: the real one
    /// cannot run without a session, a display cache and an archive chain, and
    /// what is being asserted about it is entirely in `spend_order` and the fact
    /// that the budget is charged in triangles. Both of those are here.
    #[test]
    fn the_furthest_moved_hull_is_rebuilt_first_and_nothing_starves() {
        /// A platform's own hull: `THUNDERBLUFFELEVATOR\\ELEVATORCAR.m2`.
        const CAR_TRIANGLES: usize = 90;
        /// …against the heaviest game object in the game, a portcullis.
        const PORTCULLIS_TRIANGLES: usize = 1_118;

        // Eight lifts moving at a yard a frame, and one portcullis that is
        // opening — more work than a single frame's budget can pay for.
        let lifts = 8usize;
        let mut placed_at: Vec<f32> = vec![0.0; lifts + 1];
        let mut position: Vec<f32> = vec![0.0; lifts + 1];
        let mut rebuilt_on: Vec<Option<usize>> = vec![None; lifts + 1];
        let cost = |i: usize| if i == lifts { PORTCULLIS_TRIANGLES } else { CAR_TRIANGLES };

        for frame in 0..4 {
            for (i, p) in position.iter_mut().enumerate() {
                *p += if i == lifts { 0.05 } else { 1.0 };
            }
            // The candidate list in **query order**, which is what the old spend
            // consumed and what made it starve: index 0 first, every frame.
            let mut changed: Vec<(usize, (), f32)> = (0..=lifts)
                .filter(|i| placed_at[*i] != position[*i])
                .map(|i| (i, (), (position[i] - placed_at[i]).abs()))
                .collect();
            spend_order(&mut changed);

            let mut budget = PLACE_BUDGET_TRIANGLES;
            for (i, (), _) in changed {
                if budget == 0 {
                    break;
                }
                budget = budget.saturating_sub(cost(i));
                placed_at[i] = position[i];
                rebuilt_on[i] = Some(frame);
            }
        }

        assert!(
            rebuilt_on.iter().all(Option::is_some),
            "every hull was reached: {rebuilt_on:?}",
        );
        assert!(
            placed_at
                .iter()
                .zip(&position)
                .all(|(placed, now)| (placed - now).abs() < 1e-6),
            "and every hull is where its object is, not where it used to be",
        );
    }

    /// …and the ordering itself: a hull that does not exist yet outranks any
    /// amount of staleness, and **a coordinate with no number in it ranks
    /// last**.
    ///
    /// The second half is the one that would have been got wrong for free.
    /// `f32::total_cmp` puts a positive `NaN` *above* infinity, so a `NaN`
    /// staleness would win the budget every frame — and an object whose
    /// placement can never compare equal (`NaN != NaN`) would ask again the
    /// next frame, and the next. That is the very starvation this ordering
    /// exists to end, re-entering through the fix. [`staleness`] closes it at
    /// the source rather than in the sort.
    #[test]
    fn a_hull_that_does_not_exist_yet_outranks_a_stale_one() {
        let nowhere = [f32::NAN, 0.0, 0.0];
        let mut changed: Vec<(&str, (), f32)> = vec![
            ("a jiggling door", (), staleness([0.0; 3], [0.002, 0.0, 0.0])),
            ("a lift under your feet", (), staleness([0.0; 3], [0.0, 0.0, 6.1])),
            ("never placed", (), NEVER_PLACED),
            ("a position with no number in it", (), staleness([0.0; 3], nowhere)),
        ];
        spend_order(&mut changed);
        let order: Vec<&str> = changed.iter().map(|(name, _, _)| *name).collect();
        assert_eq!(
            order,
            vec![
                "never placed",
                "a lift under your feet",
                "a jiggling door",
                "a position with no number in it",
            ],
        );
    }

    /// **A hull is placed where its door actually stands**, in the world's own
    /// coordinates.
    ///
    /// This is the absolute half and it exists because the relative half below
    /// **shipped green while the feature did not work at all.** The first draft
    /// of this pass ran `axes::to_wow` over a position that was already in WoW's
    /// axes — `Motion` is fed `vale_protocol::state::update::Position` verbatim and
    /// it is `place_entities` that converts — so every door's hull went to
    /// `[-z, -x, y]` of itself. Nothing reported it: the object was found, the
    /// state was read, the hull was built, the counts were right, and the
    /// character walked through a shut door because the wall was four hundred
    /// yards away. The agreement test passed because **both of its sides were
    /// built from the same wrong premise**, which is the one thing an
    /// agreement test cannot catch.
    ///
    /// So this one names a number. A door tracked at a known world position has
    /// its hull *there*, checked in the frame the server speaks — no conversion
    /// on either side of the assertion, and nothing to agree with.
    #[test]
    fn a_hull_is_placed_at_the_world_position_the_server_gave() {
        // A wall in the plane x = 5 of its own model space, four yards high.
        let mesh = vale_assets::world::collision::CollisionMesh {
            positions: vec![
                [5.0, 0.0, 0.0],
                [5.0, 10.0, 0.0],
                [5.0, 10.0, 4.0],
                [5.0, 0.0, 4.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            declared_triangles: 2,
            ..Default::default()
        };
        // What `Motion` answers for a game object: the server's own position,
        // unconverted. All three distinct and none of them zero, so a
        // transposed or negated axis cannot pass by coincidence.
        let tracked = [-9470.0, 60.0, 56.0];
        let matrix = object_matrix(tracked, 0.0, 1.0);
        let placed = Collider::place(&mesh, &matrix);
        let [lo, hi] = placed.bounds();

        // The hull's box has to contain the point the server named — this is
        // the assertion the axis bug fails, by four hundred yards.
        for axis in 0..3 {
            assert!(
                lo[axis] <= tracked[axis] + 10.5 && hi[axis] >= tracked[axis] - 0.5,
                "axis {axis}: door at {tracked:?}, hull spans {lo:?}..{hi:?}"
            );
        }
        // …and specifically: the wall stands five yards north of the door's
        // own origin, ten yards wide in y, four high, exactly as authored.
        assert!((lo[0] - (tracked[0] + 5.0)).abs() < 1e-3, "{lo:?}");
        assert!((lo[2] - tracked[2]).abs() < 1e-3, "{lo:?}");
        assert!((hi[2] - (tracked[2] + 4.0)).abs() < 1e-3, "{hi:?}");
    }

    /// …and the relative half: **the hull lands exactly where the model is
    /// drawn**, at any facing and any scale.
    ///
    /// Kept, because it is the check for a *composition* error — a stray 180°,
    /// a rotation taken the wrong way, a scale applied to the translation — and
    /// it covers those well. What it cannot cover is the frame the position
    /// arrives in, which is what the test above is for. Note that both sides
    /// start from the **tracked** position now rather than from a drawn one, so
    /// the conversion appears exactly once and on the drawn side, which is where
    /// it really is.
    #[test]
    fn a_hull_is_placed_exactly_where_the_model_is_drawn() {
        for (facing, scale) in [(0.0, 1.0), (0.9, 1.0), (-2.4, 2.5), (3.1, 0.4)] {
            let tracked = [-1234.5, 90.25, -678.75];

            // What `place_entities` writes for a game object: `to_bevy` over the
            // tracked position, and no pitch — the wire carries one only under
            // `MOVEFLAG_SWIMMING`.
            let placed = Transform {
                translation: axes::to_bevy(tracked),
                rotation: axes::body(facing, 0.0),
                scale: Vec3::splat(scale),
            }
            .to_matrix();

            // …and what this pass builds: the same position, unconverted.
            let hull = object_matrix(tracked, facing, scale);
            let hull = axes::to_bevy_affine(Mat4::from_cols_array(&hull));

            // A model-space vertex off the model's own axis, so the rotation
            // and the scale are both exercised rather than only the origin.
            let vertex = Vec3::new(0.5, 1.75, -2.25);
            let (a, b) = (placed.transform_point3(vertex), hull.transform_point3(vertex));
            assert!(
                (a - b).length() < 1e-3,
                "facing {facing}, scale {scale}: drawn at {a:?}, solid at {b:?}"
            );
        }
    }
}
