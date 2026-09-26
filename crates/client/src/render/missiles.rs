//! The thing a spell **throws**, which hangs off nobody.
//!
//! Every other model this renderer draws belongs to something: a doodad to a
//! tile, a pauldron to a shoulder, a spell glow to a caster's hand. A missile
//! belongs to neither end — it leaves the caster and arrives at the victim, and
//! for the second or so in between it is a model in the world with a velocity
//! and no owner. That is the whole reason this is a module rather than three
//! more fields on `EntityModel`.
//!
//! **It is described one table earlier than the effects and it needs the
//! server's answer to a question nothing else asks.** `SpellVisual` states
//! `hasMissile`, the model, a path type and a destination attachment;
//! `Spell.dbc` states the speed (Fireball: 24 y/s); and *what to fly at* is
//! `SMSG_SPELL_GO`'s hit list, which is the only place in the protocol a cast
//! names its target. See [`vale_assets::tables::spell::Missile`] and
//! [`vale_protocol::play::action::SpellCast::target`].
//!
//! Three systems, in the order a missile happens:
//!
//! * [`launch_missiles`] — a caster's release counter moved and its spell
//!   throws something. Records where it left from and what it is aimed at.
//! * [`spawn_missiles`] — the model has finished loading; build it.
//! * [`fly_missiles`] — move it, pose it, and take it off when it arrives.
//!
//! The load is asynchronous, so the first two are separate for the same reason
//! `spawn_attachments` is separate from the dressing: a missile whose model is
//! still being read must keep its launch position and its clock rather than
//! being launched again next frame.
//!
//! ## What is deliberately not modelled
//!
//! **The arc.** `SpellVisual`'s path type is read and reported and every
//! missile here flies straight, because what the non-zero values mean is not
//! stated anywhere this project can read and a wrong arc is worse than none.
//!
//! **The destination attachment.** Field 9 says which point on the *victim* the
//! missile is aimed at (1, the right hand, for Fireball). Hitting it needs the
//! victim's own posed skeleton at the moment of arrival, which is a second
//! entity's `EntityModel` and its joints; the aim here is the target's
//! mid-height, which is within a foot of it on a humanoid.
//!
//! ## How it ends
//!
//! **A missile ends by bursting, and it times its own arrival.**
//! `SMSG_PLAY_SPELL_IMPACT` is not the mechanism — vmangos sends that opcode
//! from one GM debug command and from nowhere in the gameplay path — so the
//! client decides when a projectile got there and hands the spell id to
//! [`crate::world::entities::PendingImpacts`], which hangs the `impactKit`'s models on
//! the victim. **Only on an actual arrival**: the [`LIFETIME`] ceiling below
//! exists for the projectiles that never got there, and bursting on the way out
//! would explode a fireball on a victim it never reached.

use crate::world::entities::{AttachedPart, EntityModel, EntityPart, Joint};
use crate::render::models::{Lookup, Materials, ModelCache, SceneLighting};
use crate::world::session::WorldEntity;
use bevy::math::Affine3A;
use bevy::prelude::*;

/// How far from its aim point a missile counts as arrived, in yards.
///
/// A projectile is stepped once a frame, so it lands *inside* one step of the
/// target rather than on it; this is the radius that stops the last step
/// overshooting and turning into a missile that orbits. Half a yard is well
/// inside any unit's own footprint.
const ARRIVED: f32 = 0.5;

/// The longest a missile may stay in the world, in seconds.
///
/// **Nothing guarantees an arrival.** The target can be despawned mid-flight
/// (walked out of the streamed set, killed and looted), the speed can be zero
/// on a row that names a missile anyway, and a stationary aim point can be
/// unreachable if the two ends disagree. Without a ceiling each of those leaves
/// a fireball hanging in the air for the rest of the session.
const LIFETIME: f32 = 5.0;

/// The fraction of a unit's head height a missile leaves from and arrives at.
///
/// Neither end is at the feet, which is where an entity's position is. The
/// caster throws from about chest height and the victim is hit in the body;
/// 0.6 of the head height is both, and it is the same number at both ends so
/// that a missile between two identical units flies level.
const CHEST: f32 = 0.6;

/// One projectile in flight.
#[derive(Component)]
pub struct Missile {
    /// What it is flying at, while that entity still exists.
    target: Entity,
    /// The victim's guid, which outlives the `Entity` — what the impact is
    /// addressed to. See [`crate::world::entities::PendingImpacts`].
    victim: u64,
    /// The `Spell.dbc` id that threw this, so the arrival can look up its own
    /// `impactKit`. A property of the *cast* rather than of the missile model,
    /// because one model is thrown by many spells with different impacts.
    spell: u32,
    /// Where it is going, in Bevy world space. Re-read from the target every
    /// frame — a fireball follows a moving victim, which is what the 1.12
    /// client does — and **kept** when the target goes away, so a missile whose
    /// victim is despawned mid-flight still completes its arc instead of
    /// freezing.
    aim: Vec3,
    /// Yards per second, off `Spell.dbc`.
    speed: f32,
    /// When it launched, `Time::elapsed_secs` — the lifetime bound. The
    /// *model's* clock is [`AttachedPart::since`], set when it was built, which
    /// is a frame or two later.
    since: f32,
    /// The model, hanging off this entity exactly as an attachment hangs off a
    /// wearer — which is what lets `entities::animate_attachment` pose and fade
    /// it with no second copy of the billboard derivation.
    model: AttachedPart,
}

/// A missile whose model has been asked for and has not arrived.
///
/// Everything about the launch is captured here rather than re-derived when the
/// model lands: the caster may have moved, or died, in the meantime.
struct Pending {
    target: Entity,
    victim: u64,
    spell: u32,
    from: Vec3,
    aim: Vec3,
    speed: f32,
    since: f32,
    path: String,
    scale: f32,
}

/// The launches waiting on a model.
#[derive(Resource, Default)]
pub struct PendingMissiles(Vec<Pending>);

/// **How many casts this entity had released when the missile pass last
/// looked**, and the latch that stops a missile per frame.
///
/// Its own component rather than a field on `EntityModel` on purpose: a model
/// rebuild — new gear, a change of room, a drawn weapon — drops `EntityModel`
/// and takes its latch with it, which silently swallows the first cast after
/// any of those. This survives all three, because it is a fact about the
/// *entity* and not about what it is currently drawn as.
#[derive(Component)]
struct MissilesSeen(u32);

pub struct MissilePlugin;

impl Plugin for MissilePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingMissiles>().add_systems(
            Update,
            (launch_missiles, spawn_missiles, fly_missiles)
                .chain()
                // The launch reads two entities' placements and the flight
                // writes a world transform, so both want this frame's
                // positions rather than last frame's — the same ordering
                // `entities::animate` is given, and for the same reason.
                .after(crate::world::session::place_entities),
        );
    }
}

/// A cast was released, and the spell throws something.
fn launch_missiles(
    mut commands: Commands,
    time: Res<Time>,
    displays: Res<crate::world::entities::DisplayCache>,
    mut pending: ResMut<PendingMissiles>,
    world: Query<(Entity, &WorldEntity, &Transform, Option<&EntityModel>)>,
    mut seen: Query<&mut MissilesSeen>,
) {
    let now = time.elapsed_secs();
    for (entity, caster, from, caster_model) in &world {
        // The latch, adopted on the first look and acted on after: an entity
        // that walks into view mid-cast must not be handed the missile of a
        // cast that went off before it existed.
        let first = match seen.get_mut(entity) {
            Ok(mut seen) => {
                if seen.0 == caster.casts_released {
                    continue;
                }
                seen.0 = caster.casts_released;
                false
            }
            Err(_) => {
                commands
                    .entity(entity)
                    .insert(MissilesSeen(caster.casts_released));
                true
            }
        };
        if first {
            continue;
        }

        let Some(missile) = displays
            .tables()
            .and_then(|tables| tables.cast_missile(caster.last_spell))
        else {
            continue;
        };
        // **A missile with no target is not drawn.** A self-buff and a
        // ground-targeted spell both reach here — `hasMissile` is a property of
        // the visual, and a visual is shared by spells that do and do not aim
        // at a unit — and there is nowhere to fly.
        if caster.last_spell_target == 0 || missile.speed <= 0.0 {
            continue;
        }
        // The victim, by guid. A linear scan of the streamed set, which is
        // affordable because it runs once per *cast* rather than once per
        // frame: the latch above has already decided something happened.
        let Some((target, _, to, target_model)) = world
            .iter()
            .find(|(_, e, _, _)| e.guid == caster.last_spell_target)
        else {
            // Not in the streamed set — out of range, or an object the client
            // did not model. Nothing to aim at.
            continue;
        };
        pending.0.push(Pending {
            target,
            victim: caster.last_spell_target,
            spell: caster.last_spell,
            from: chest(from, caster_model),
            aim: chest(to, target_model),
            speed: missile.speed,
            since: now,
            path: missile.path.clone(),
            scale: missile.scale,
        });
    }
}

/// The point on a unit a missile leaves from and arrives at.
fn chest(placement: &Transform, model: Option<&EntityModel>) -> Vec3 {
    let head = model.map_or(2.0, |m| m.head) * placement.scale.y;
    placement.translation + Vec3::Y * head * CHEST
}

/// Build the launches whose models have finished loading.
fn spawn_missiles(
    mut commands: Commands,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    mut pending: ResMut<PendingMissiles>,
) {
    if pending.0.is_empty() {
        return;
    }
    let mut waiting = Vec::new();
    for launch in std::mem::take(&mut pending.0) {
        // No room: a missile is a spell effect in flight, and it crosses
        // whatever rooms lie between two units — there is no one room to light
        // it by, and it is an unlit glow in any case.
        match cache.attached(
            &launch.path,
            None,
            None,
            SceneLighting::NONE,
            &mut meshes,
            &mut materials,
        ) {
            Lookup::Loading => waiting.push(launch),
            Lookup::Failed => warn!("missile {} will not read", launch.path),
            Lookup::Ready(assets) => {
                // The carrier: a world-space entity with no parent, which is
                // what a missile is. The model hangs off it through the same
                // `hang_model` a pauldron does, so it gets its own joints, its
                // own emitters and its own fade for free — and a missile is
                // mostly emitters, which is why that matters more here than it
                // does on a shoulder.
                let carrier = commands
                    .spawn((
                        Transform::from_translation(launch.from),
                        Visibility::default(),
                    ))
                    .id();
                let model = crate::world::entities::hang_model(
                    &mut commands,
                    &mut meshes,
                    carrier,
                    &assets,
                    // No wearer and no bone: the offset and the bone index are
                    // the attachment's join to a skeleton, and this has none.
                    0,
                    [0.0; 3],
                    launch.scale,
                    // A projectile in flight has no floor to lie on.
                    false,
                    None,
                    crate::render::models::sun_scale::NEUTRAL,
                    launch.since,
                );
                commands.entity(carrier).insert(Missile {
                    target: launch.target,
                    victim: launch.victim,
                    spell: launch.spell,
                    aim: launch.aim,
                    speed: launch.speed,
                    since: launch.since,
                    model,
                });
            }
        }
    }
    pending.0 = waiting;
}

/// Move every missile toward its aim point, pose it, and take it off when it
/// arrives.
fn fly_missiles(
    mut commands: Commands,
    time: Res<Time>,
    mut impacts: ResMut<crate::world::entities::PendingImpacts>,
    camera: Query<
        &GlobalTransform,
        (With<crate::world::camera::WorldCamera>, Without<Joint>, Without<Missile>),
    >,
    targets: Query<(&Transform, Option<&EntityModel>), Without<Missile>>,
    mut missiles: Query<(Entity, &mut Missile, &mut Transform)>,
    mut joints: Query<&mut GlobalTransform, With<Joint>>,
    mut tags: Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
) {
    let now = time.elapsed_secs();
    let now_ms = (now * 1000.0) as u32;
    let step = time.delta_secs();
    let camera = camera.iter().next().copied();

    for (entity, mut missile, mut placement) in &mut missiles {
        // A victim that is still in the world is followed; one that is not
        // leaves the last aim point standing, so the missile completes rather
        // than stopping in mid-air.
        if let Ok((target, model)) = targets.get(missile.target) {
            missile.aim = chest(target, model);
        }
        let to = missile.aim - placement.translation;
        let distance = to.length();
        let arrived = distance <= ARRIVED.max(missile.speed * step);
        if arrived || now - missile.since >= LIFETIME {
            // **A missile ends by bursting, not by being taken off** — and only
            // when it actually got there. The ceiling below it is the case
            // nothing guarantees an arrival for, and a fireball that timed out
            // over an empty field should not explode on a victim it never
            // reached.
            if arrived {
                impacts.land(missile.victim, missile.spell);
            }
            commands.entity(entity).despawn();
            continue;
        }
        let direction = to / distance;
        placement.translation += direction * missile.speed * step;
        // A model's forward is WoW's +X, which `axes::to_bevy` puts on Bevy's
        // −Z — exactly what `looking_to` aligns, so the projectile points the
        // way it is going with no correction term.
        placement.rotation = Transform::default()
            .looking_to(direction, Vec3::Y)
            .rotation;

        // The model rides the carrier: its frame is the carrier's placement
        // times the attachment's own scale, and from there it poses and fades
        // exactly as a spell glow on a hand does.
        let mut writes = crate::world::entities::RigWrites::default();
        crate::world::entities::animate_attachment(
            &missile.model,
            placement.compute_affine()
                * Affine3A::from_scale(Vec3::splat(missile.model.scale)),
            camera.as_ref(),
            now,
            now_ms,
            &mut writes,
        );
        writes.apply(&mut joints, &mut tags);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// An app with the flight system and a clock that can be stepped.
    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<Time>();
        app.init_resource::<crate::world::entities::PendingImpacts>();
        app.add_systems(Update, fly_missiles);
        app
    }

    /// What the flight queued for the effect pass — see
    /// [`crate::world::entities::PendingImpacts`].
    fn landings(app: &App) -> Vec<(u64, u32)> {
        app.world()
            .resource::<crate::world::entities::PendingImpacts>()
            .landed()
            .to_vec()
    }

    fn step(app: &mut App, seconds: f32) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(seconds));
        app.update();
    }

    /// A missile with no model behind it: enough to fly, which is what this
    /// tests. `hang_model`'s output needs a `ModelAssets` and therefore an
    /// archive; the flight does not.
    fn missile(target: Entity, aim: Vec3, speed: f32) -> Missile {
        Missile {
            target,
            // Fireball at a victim, so an arrival is checkable by what it
            // queues rather than only by the projectile going away.
            victim: 0xF130_0000_0001_2345,
            spell: 133,
            aim,
            speed,
            since: 0.0,
            model: AttachedPart::rigid(Entity::PLACEHOLDER, 1.0),
        }
    }

    /// **A projectile covers its distance at the speed the spell states and is
    /// taken off when it lands.** Fireball is 24 y/s, so 24 yards is a second
    /// — a missile still in the air after that is one the eye watches hang, and
    /// one that vanishes early never reaches its victim.
    #[test]
    fn a_missile_flies_at_its_own_speed_and_ends_when_it_arrives() {
        let mut app = app();
        // A stationary target 24 yards away. No `EntityModel`, so `chest`
        // takes its two-yard default and both ends sit at the same height.
        let target = app
            .world_mut()
            .spawn(Transform::from_xyz(0.0, 0.0, -24.0))
            .id();
        let flying = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 1.2, 0.0),
                missile(target, Vec3::new(0.0, 1.2, -24.0), 24.0),
            ))
            .id();

        // Half a second in, half the distance — and still in the world.
        step(&mut app, 0.5);
        let placement = *app.world().entity(flying).get::<Transform>().expect("flying");
        assert!(
            (placement.translation.z + 12.0).abs() < 0.1,
            "half a second at 24 y/s put it at {placement:?}"
        );
        // …pointing the way it is going: a model's own forward is WoW's +X,
        // which is Bevy's −Z, so a missile flying that way is unrotated.
        assert!(
            placement.forward().as_vec3().z < -0.99,
            "the projectile is not facing its flight"
        );

        step(&mut app, 0.5);
        assert!(
            app.world().get_entity(flying).is_err(),
            "the missile arrived and was not taken off"
        );
        // **And it ended by bursting.** A projectile that is merely removed is
        // a fireball that reaches its victim and does nothing, which is what
        // this client did before the impact kit was read.
        assert_eq!(
            landings(&app),
            vec![(0xF130_0000_0001_2345, 133)],
            "the arrival queued no impact"
        );
    }

    /// **A missile whose target walks away follows it**, which is what the 1.12
    /// client does with a fireball — and a missile whose target is *despawned*
    /// keeps the last aim point rather than freezing where it is, because a
    /// projectile that stops in mid-air is far more visible than one that
    /// completes into empty space.
    #[test]
    fn a_missile_follows_a_moving_target_and_survives_its_removal() {
        let mut app = app();
        let target = app
            .world_mut()
            .spawn(Transform::from_xyz(0.0, 0.0, -40.0))
            .id();
        let flying = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 1.2, 0.0),
                missile(target, Vec3::new(0.0, 1.2, -40.0), 20.0),
            ))
            .id();

        // The victim sidesteps ten yards while the missile is in the air.
        app.world_mut().entity_mut(target).insert(Transform::from_xyz(10.0, 0.0, -40.0));
        step(&mut app, 0.5);
        assert!(
            app.world().entity(flying).get::<Transform>().unwrap().translation.x > 0.0,
            "the missile ignored the target's new position"
        );

        // …and now the victim is gone.
        app.world_mut().entity_mut(target).despawn();
        step(&mut app, 0.5);
        assert!(
            app.world().get_entity(flying).is_ok(),
            "a missile whose target vanished was dropped in mid-flight"
        );

        // It still completes, on the aim point it had.
        step(&mut app, 2.0);
        assert!(app.world().get_entity(flying).is_err());
    }

    /// **Nothing guarantees an arrival**, so nothing may be left flying for
    /// ever: an unreachable aim point is what a zero-speed row and a target
    /// receding faster than the missile both produce.
    #[test]
    fn a_missile_that_cannot_arrive_is_still_taken_off() {
        let mut app = app();
        let target = app.world_mut().spawn(Transform::default()).id();
        let flying = app
            .world_mut()
            .spawn((
                Transform::default(),
                // Aimed a thousand yards off at walking pace.
                missile(target, Vec3::new(0.0, 0.0, -1000.0), 1.0),
            ))
            .id();
        // The target is at the origin, so the aim is rewritten to it each
        // frame — but it starts *at* the origin, so this is really about the
        // ceiling: step past it in slices too small to arrive in.
        app.world_mut().entity_mut(target).despawn();
        for _ in 0..12 {
            step(&mut app, 0.5);
        }
        assert!(
            app.world().get_entity(flying).is_err(),
            "a missile outlived its {LIFETIME}s ceiling"
        );
        // **A timeout is not an arrival.** The ceiling exists precisely for the
        // missiles that never got there, and bursting on the way out would
        // explode a fireball on a victim it never reached — or, worse, on one
        // that was never in range to be hit at all.
        assert!(
            landings(&app).is_empty(),
            "a missile that timed out burst anyway"
        );
    }
}
