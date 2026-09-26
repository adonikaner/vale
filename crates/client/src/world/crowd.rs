//! A synthetic crowd: entities the server never sent, for pricing the entity pass.
//!
//! **The one population this project could never measure from a script.** Every
//! other cost in the renderer has a subtraction behind it — `--without` takes a
//! layer out, `--party` takes four frames out, two runs differing in one line
//! price a pass — and the entity pass had none, because the only way to get
//! forty players and forty creatures into one frame is forty people at forty
//! keyboards. "This needs a live group" had been the answer for several
//! rounds, and that is the whole reason the number was never taken.
//!
//! `--crowd <players>[,<mobs>]` fills the frame instead. A crowd member is a
//! **whole cloned `WorldEntity`** — the local player for the player half, and
//! one per distinct display id of whatever *units* are in view for the creature
//! half — so it goes through exactly the pipeline a real entity does:
//! `spawn_models` resolves it, `dress` composes its skin, `animate` poses it,
//! and the blob shadow, the label, the tint and the sound passes all walk it.
//! Nothing here is a stand-in for the thing being measured.
//!
//! **The whole snapshot, and not a field or two lifted out of one**, which is
//! the rule the three faults this instrument shipped with all broke. See
//! [`populate`]: a clone built by pasting a creature's display id over the
//! player's snapshot draws magenta, a template list taken off every entity that
//! has a model clones the *inn* (game objects live in the same table as units),
//! and a list gathered before anything has resolved a model is empty for the
//! whole window the crowd fills in. Every one of the three measured something
//! other than what it said, and every one was found by looking at the picture.
//!
//! Three subtractions ride on the same spec — `still`, `noauras`, `nogear` —
//! and each answered a question no count could. `nogear` is the one that named
//! this round's finding: **the four attached models a geared character wears
//! are 173 of the ~195 draw calls forty players add.**
//!
//! Three properties it deliberately has, because a measurement that flatters is
//! worse than none:
//!
//! * **Each player clone has its own appearance bytes**, so each composes its
//!   own skin rather than sharing one cached texture. Forty copies of one
//!   character would price the draw and not the wardrobe.
//! * **They move**, which is what makes them cost anything: a standing unit's
//!   `Transform` is bit-identical frame to frame and `place_entities` skips the
//!   write, so a still crowd measures neither transform propagation nor the
//!   re-extraction a moving one forces.
//! * **Their health churns**, on the same quarter second a fighting unit's
//!   does, so the `Changed<WorldEntity>` paths are exercised rather than being
//!   permanently quiet. What it does **not** price is
//!   [`super::session::poll_world`], which only ever sees the server's own
//!   entities — that half is priced by the entity count the server supplies.
//!
//! Behind the `diagnostics` feature, like every other instrument in this
//! client: a shipped build has no way to ask for one.
//!
//! What it does **not** reach is a crowd of *different* models — forty distinct
//! display ids rather than forty copies of two — which is what a city is, and
//! which is what would say whether the material pool is doing its job.

use bevy::prelude::*;

use super::session::{LocalPlayer, WorldEntity};

/// How many synthetic entities to hold.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct Crowd {
    /// Clones of the local player, each with its own appearance.
    pub players: usize,
    /// Clones of whatever creature display ids are in view, cycled.
    pub mobs: usize,
    /// **Stand them still instead of running.** Not a knob for looking at —
    /// a subtraction: a still crowd writes no `Transform` and marks no
    /// `GlobalTransform` changed, so the pair of runs separates "what a drawn
    /// entity costs" from "what a *moving* drawn entity costs", which are two
    /// different levers and were being priced as one.
    pub still: bool,
    /// **Take the buffs off**, which is the second subtraction the instrument
    /// needs: an aura with art is a spell-effect model hung on the wearer, and
    /// this client draws every batch of it as its own translucent phase item.
    pub no_auras: bool,
    /// …and the third: **take the gear off**, so the helm, the two shoulders
    /// and the weapon stop being four more models each with batches of their
    /// own. Together the three separate what a *body* costs from what is hung
    /// on it, which no count could otherwise tell apart.
    pub no_gear: bool,
}

impl Crowd {
    /// `--crowd <players>[,<mobs>]`. A bare number is all players, which is the
    /// case the report was about.
    pub fn parse(spec: &str) -> Crowd {
        let word = |name: &str| spec.split(',').any(|p| p.trim().eq_ignore_ascii_case(name));
        let still = word("still");
        let mut parts = spec
            .split(',')
            .filter_map(|p| p.trim().parse::<usize>().ok());
        Crowd {
            players: parts.next().unwrap_or(0),
            mobs: parts.next().unwrap_or(0),
            still,
            no_auras: word("noauras"),
            no_gear: word("nogear"),
        }
    }

    pub fn total(&self) -> usize {
        self.players + self.mobs
    }
}

/// A member of the crowd, and the circle it is walking.
#[derive(Component)]
pub struct CrowdMember {
    /// Where its circle is centred, in Bevy space.
    anchor: Vec3,
    /// Its own place on that circle, so the crowd is not one lockstep ring.
    phase: f32,
    /// …and its own churn phase, so the health writes are spread across the
    /// interval rather than landing on one frame in eight.
    churn: f32,
}

/// How many to add in one frame. The same argument as [`super::entities`]'s own
/// spawn budget: a hundred at once is a hundred model resolutions and a hundred
/// skin composites in one frame, which is a five-second hitch and a misleading
/// first reading.
const BUDGET: usize = 4;

/// The radius of the ring the crowd stands in, in yards.
///
/// Far enough out that the camera is not inside the crowd — a member drawn
/// across the whole screen prices overdraw rather than the pass — and near
/// enough that all of it is in one framing at the default view.
const RING: f32 = 14.0;

/// How fast a member walks its own little circle, in yards a second.
///
/// Above [`super::entities`]' run threshold, so the gait chosen is a run: the
/// clip with the most bone movement in it, and the one a crowd of players is
/// actually doing.
const PACE: f32 = 7.0;

/// The radius of that circle, in yards. Small: the point is that the transform
/// moves every frame, not that the crowd wanders off.
const ORBIT: f32 = 2.5;

/// How often a member's health moves, in seconds.
const CHURN_SECS: f32 = 0.25;

/// The guid space the crowd lives in — high, and nothing on the wire can reach
/// it, so it can never collide with a real one or be mistaken for one in a log.
const CROWD_GUID: u64 = 0x0BAD_C0DE_0000_0000;

/// Whether anything was asked for at all, so a run without `--crowd` pays
/// nothing but the `run_if`.
pub(super) fn is_wanted(crowd: Option<Res<Crowd>>) -> bool {
    crowd.is_some_and(|c| c.total() > 0)
}

/// Bring the population up to (or down to) what was asked for.
pub(super) fn populate(
    crowd: Res<Crowd>,
    mut commands: Commands,
    player: Query<(&WorldEntity, &Transform), (With<LocalPlayer>, Without<CrowdMember>)>,
    // **Only the units that actually resolved to a model.** A display id that
    // resolves to nothing draws as `fallback_shapes`' red shape, and one that
    // resolves to a *character* model with no baked skin draws magenta — so a
    // crowd cloned off the raw list is a field of instrument artefacts rather
    // than a measurement. `EntityModel` is the filter that says the tables
    // answered.
    units: Query<
        &WorldEntity,
        (
            Without<CrowdMember>,
            With<crate::world::entities::EntityModel>,
        ),
    >,
    members: Query<Entity, With<CrowdMember>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Crowd);
    let Ok((player, placement)) = player.single() else {
        return;
    };
    let have = members.iter().count();
    let want = crowd.total();
    if have > want {
        for entity in members.iter().take(have - want) {
            commands.entity(entity).despawn();
        }
        return;
    }
    if have == want {
        return;
    }
    // The creature half's templates: one per distinct display id of whatever
    // the server has actually put in view, so a crowd of mobs is a crowd of the
    // models this zone really draws. None in view yet is not an error — the
    // creature half simply waits, see below.
    //
    // **The whole snapshot, not a display id lifted out of it.** A first draft
    // overwrote six fields of the *player's* snapshot with a creature's display
    // id, and the crowd came out with magenta bodies and red fallback shapes in
    // it: what a creature is drawn as is its display id crossed with its scale,
    // its kind, its `CreatureDisplayInfoExtra` bake and half a dozen other
    // fields, and a clone that keeps some of the player's is a dressing nothing
    // in the game asks for. Cloning the unit entire cannot be wrong by
    // construction — it dresses exactly as the unit it came from does.
    // …and **only the units**, which is not the tautology it reads as: this
    // client keeps game objects in the same table as units and they carry an
    // `EntityModel` like anything else, so a first draft cloned Goldshire's inn
    // and stood a copy of it fourteen yards from the camera.
    let mut mobs: Vec<WorldEntity> = Vec::new();
    for unit in &units {
        if unit.kind == vale_protocol::state::update::ObjectType::Unit
            && unit.appearance.is_none()
            && unit.display_id.is_some()
            && !mobs.iter().any(|seen| seen.display_id == unit.display_id)
        {
            mobs.push(unit.clone());
        }
    }
    mobs.sort_by_key(|unit| unit.display_id);

    for index in have..(have + BUDGET).min(want) {
        let i = index as u32;
        let angle = index as f32 / want.max(1) as f32 * std::f32::consts::TAU;
        // Two rings once the crowd is bigger than one comfortably holds, so
        // forty of them are not a picket fence one model deep.
        let ring = RING + if index % 2 == 0 { 0.0 } else { 6.0 };
        let anchor =
            placement.translation + Vec3::new(angle.cos() * ring, 0.0, angle.sin() * ring);
        // The player for the player half, and a unit the server really sent for
        // the creature half — see `mobs` above.
        //
        // **The creature half waits for a template rather than standing in as
        // more players.** `EntityModel` is what says a display id resolved, and
        // nothing in the world has one for the first second or two of a
        // session — which is exactly the window this loop fills the crowd in.
        // A first draft took the empty list as "no creatures in this zone" and
        // quietly measured forty players under a `--crowd 20,20`.
        let creature = match index >= crowd.players {
            true if mobs.is_empty() => break,
            true => Some(&mobs[(index - crowd.players) % mobs.len()]),
            false => None,
        };
        let mut snapshot = creature.unwrap_or(player).clone();
        snapshot.guid = CROWD_GUID | u64::from(i);
        snapshot.name = format!("Crowd{i}");
        snapshot.is_self = false;
        // Nothing may take the crowd for a party member, a target or the author
        // of a swing: the three relationships and the three player-only blocks
        // are cleared.
        snapshot.target = None;
        snapshot.pet = None;
        snapshot.summoned_by = None;
        snapshot.stats = None;
        snapshot.explored = None;
        snapshot.skills = None;
        snapshot.main_hand_item = None;
        if crowd.no_auras {
            snapshot.auras = Default::default();
        }
        if crowd.no_gear {
            snapshot.equipment = Vec::new();
            snapshot.weapons = Default::default();
        }
        // Running, which is what makes a pose cost anything — see [`PACE`].
        snapshot.moving = !crowd.still;
        snapshot.speed = if crowd.still { 0.0 } else { PACE };
        snapshot.move_flags = if crowd.still {
            0
        } else {
            vale_protocol::state::movement::move_flags::FORWARD
        };
        if creature.is_none() {
            if let Some(look) = snapshot.appearance.as_mut() {
                // Its own face, so its own composed skin: the wardrobe is
                // priced once per distinct appearance and forty identical
                // clones would price it once for the whole crowd.
                //
                // **Small and wrapped rather than a fresh value**, because
                // `CharSections` does not have every index for every race and a
                // skin the tables cannot answer composes magenta — which is an
                // instrument artefact in the middle of the thing it is
                // measuring. Five of each is inside the narrowest axis any race
                // has.
                look.skin = (look.skin + i as u8) % 5;
                look.face = (look.face + i as u8) % 5;
                look.hair_style = (look.hair_style + i as u8) % 5;
                look.hair_colour = (look.hair_colour + i as u8 * 3) % 5;
            }
        }
        let sheath = snapshot.sheath_state;
        commands.spawn((
            snapshot,
            Transform::from_translation(anchor),
            Visibility::default(),
            super::entities::Sheath::seeded(sheath),
            CrowdMember {
                anchor,
                phase: index as f32 * 0.618_034 * std::f32::consts::TAU,
                churn: index as f32 / want.max(1) as f32 * CHURN_SECS,
            },
        ));
    }
}

/// Walk the crowd, and churn what a fighting unit's numbers churn.
///
/// **After [`super::session::place_entities`] and before the entity chain**,
/// which is where a real entity's placement is written: a member posed against
/// last frame's position is the one-frame mismatch that pass's own doc is
/// about, and a measurement taken through a different ordering is not the
/// measurement.
pub(super) fn walk(
    time: Res<Time>,
    crowd: Res<Crowd>,
    mut members: Query<(&CrowdMember, &mut Transform, &mut WorldEntity)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Crowd);
    let now = time.elapsed_secs();
    for (member, mut placement, mut snapshot) in &mut members {
        // Standing still is the subtraction, not a pose: the clock is frozen at
        // the member's own phase so each one keeps a distinct heading, and
        // nothing writes the transform at all after the first frame.
        let t = if crowd.still { member.phase } else { now * PACE / ORBIT + member.phase };
        let placed = Transform {
            translation: member.anchor + Vec3::new(t.cos() * ORBIT, 0.0, t.sin() * ORBIT),
            // Facing along the tangent, which is what a unit running a circle
            // is drawn at.
            rotation: Quat::from_rotation_y(-t),
            scale: placement.scale,
        };
        if *placement != placed {
            *placement = placed;
        }
        // …and the churn. The rule `set_if_neq` keeps applies here as
        // everywhere: the write is what marks the component changed, so it
        // happens on the member's own quarter-second edge and not every frame.
        if let Some((health, max)) = snapshot.health_value {
            let tick = ((now + member.churn) / CHURN_SECS) as u32;
            let wanted = max.saturating_sub(tick % max.max(1)).max(1);
            if health != wanted {
                snapshot.health_value = Some((wanted, max));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The spec is a count, an optional second count, and the subtractions**
    /// — see [`Crowd`], where the argument for each of the three is.
    #[test]
    fn the_crowd_spec_is_counts_and_subtractions() {
        assert_eq!(Crowd::parse("40"), Crowd { players: 40, ..Default::default() });
        assert_eq!(
            Crowd::parse("10,30"),
            Crowd { players: 10, mobs: 30, ..Default::default() }
        );
        assert_eq!(
            Crowd::parse("40, 0, still"),
            Crowd { players: 40, still: true, ..Default::default() }
        );
        assert_eq!(
            Crowd::parse("12,0,noauras,nogear"),
            Crowd { players: 12, no_auras: true, no_gear: true, ..Default::default() }
        );
        // A word among the numbers must not be read as one, or `--crowd
        // 40,still` would silently measure forty players and thirty mobs of
        // whatever `still` parsed as.
        assert_eq!(Crowd::parse("40,still").mobs, 0);
        assert_eq!(Crowd::parse("40,still").players, 40);
        // Nothing asked for is nothing run — the `run_if` this feeds.
        assert!(!is_wanted_by(&Crowd::parse("")));
        assert!(!is_wanted_by(&Crowd::parse("0,0")));
        assert!(is_wanted_by(&Crowd::parse("0,1")));
    }

    fn is_wanted_by(crowd: &Crowd) -> bool {
        crowd.total() > 0
    }
}
