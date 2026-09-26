//! **Standing a model on the hill rather than beside it** — the drawn half of
//! [`vale_assets::look::conform`], which is the rule.
//!
//! The rule says three things and this module supplies the one input it does
//! not: *which surface the unit is on*. What it decides is nothing — the flag
//! dispatch, the two bands and the settle rate are all in `assets`, matching
//! the 1.12.1 client exactly, and are unit-tested with no world running.
//!
//! ## The rotation goes into the model matrix, not the entity
//!
//! An entity's `Transform` stays upright. The conform is composed into
//! `pose::animate`'s one `world_from_entity`, exactly where the saddle already
//! is, so it reaches the joints, the attached pauldrons, the spell glows and the
//! ground decals together — and does *not* reach the blob shadow, the selection
//! ring or the mouse pick box, which read the entity's own transform and which
//! the reference leaves upright too.
//!
//! ## …and on a mounted character it is the animal's flag that decides
//!
//! `HumanMale.m2` authors `GlobalModelFlags = 0` and `Creature\Horse\Horse.m2`
//! authors `1`. The rider does not conform on its own and never did; it tilts
//! because the seat it is sitting on does. So a mounted unit asks the *mount's*
//! flag and the rider rides the answer, which is one line in [`leaning`] and the
//! whole of why the screenshot this fixes has an upright horse in it.

use crate::world::session::{Solids, WorldEntity};
use vale_assets::look::conform::Conform;
use bevy::math::Affine3A;
use bevy::prelude::*;

/// The smoothed ground stance one unit is standing at, as a component so that
/// it dies with the entity.
///
/// Per **unit** and not per model: a rider and its horse are on the same
/// ground, and sampling twice would square the settle.
#[derive(Component, Default)]
pub(crate) struct Stance(vale_assets::look::conform::Stance);

/// Which conform mode actually decides for this unit — the mount's when it is
/// riding one, its own otherwise.
///
/// **Not the larger of the two and not a union.** The rider is drawn off the
/// saddle, so the only flag that can reach it is the animal's; a flag-3 tauren
/// on a flag-1 horse leans exactly as the horse does.
pub(super) fn leaning(own: Conform, mount: Option<Conform>) -> Conform {
    mount.unwrap_or(own)
}

/// The normal of the ground under a unit, or `None` for one that is not on any.
///
/// Swimming and airborne answer `None`, which the rule reads as *no contact* and
/// eases the model level — the same outcome the reference reaches by way of an
/// empty contact list (it writes straight up when nothing qualifies).
/// **This is where the client and the reference differ**: the reference averages
/// the normals of every walkable contact the unit has, and this takes the one
/// surface under its centre. They agree on any uniform slope; where they do not,
/// this one crosses a cell boundary in one step where the average would roll
/// across it.
pub(super) fn under(
    standing: &crate::world::session::Standing,
    map_id: u32,
    world: &WorldEntity,
    position: Vec3,
) -> Option<[f32; 3]> {
    if world.swimming || world.airborne {
        return None;
    }
    let p = crate::render::axes::to_wow(position);
    standing.stance(map_id, p[0], p[1], p[2])
}

/// The conform rotation in the entity's **own** frame, ready to compose after a
/// placement that already carries the facing.
///
/// The normal arrives in world axes and the basis is built in the model's, so
/// the facing is turned back out of it first — which is the same statement as
/// the reference building its basis off a matrix that already has the yaw in it
/// (it reads row 0 rather than a constant). Identity for a level model,
/// bit-exactly, so nothing that does not lean moves a pixel.
pub(super) fn rotation(mode: Conform, up: [f32; 3], facing: Quat) -> Affine3A {
    if !mode.leans() {
        return Affine3A::IDENTITY;
    }
    use crate::render::axes;
    let local = axes::to_wow(facing.inverse() * axes::to_bevy(up));
    let [forward, left, model_up] = mode.basis(local);
    // Columns, because the reference's rows are a row-vector convention's
    // images of the basis axes — see `Conform::basis`.
    let wow = Mat3::from_cols(forward.into(), left.into(), model_up.into());
    // …and the same conjugation every other composed-in-WoW-space transform in
    // this client gets, so a conform and a bone pose cannot disagree about
    // which way is up.
    Affine3A::from_mat3(Mat3::from_mat4(axes::to_bevy_affine(Mat4::from_mat3(wow))))
}

/// Sample the ground under one unit and settle its stance, answering the
/// rotation to draw it at.
///
/// Called from [`super::pose::animate`], **after** the frustum gate: a rig the
/// camera cannot see is not posed, so its stance is not worth a terrain lookup
/// either. The consequence is stated rather than hidden — a creature that
/// leaves the frame on a hillside and comes back on the flat eases out of the
/// hillside's stance over the settle's own 158 ms, where the reference, which
/// maintains every unit every frame, would already be level.
#[allow(clippy::too_many_arguments)]
pub(super) fn settle(
    stance: &mut Stance,
    mode: Conform,
    standing: &crate::world::session::Standing,
    map_id: u32,
    world: &WorldEntity,
    placement: &Transform,
    dt: f32,
) -> Affine3A {
    if !mode.leans() {
        return Affine3A::IDENTITY;
    }
    let sampled = under(standing, map_id, world, placement.translation);
    let up = stance.0.settle(sampled, dt);
    rotation(mode, up, placement.rotation)
}

/// A [`Solids`] and a session are what [`under`] needs; bundled so the pose
/// system's parameter list does not grow by two.
///
/// **Both optional, and that is not caution.** `pose::animate` is run by tests
/// in an app with neither resource in it, and by the real client for every
/// frame of the login screen before a session exists — a hard `Res` there is a
/// panic in a system that has plenty to do without a world.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Ground<'w> {
    session: Option<Res<'w, crate::world::session::Session>>,
    solids: Option<Res<'w, Solids>>,
}

impl Ground<'_> {
    /// The join to ask, and the map to ask it about — `None` before there is a
    /// world, which is every frame of the login screen.
    pub(crate) fn of(&self) -> Option<(crate::world::session::Standing, u32)> {
        let active = self.session.as_ref()?.active.as_ref()?;
        Some((active.standing(self.solids.as_ref()?), active.map_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A level model is bit-identical to no conform at all**, which is the
    /// claim that this round moves nothing that does not lean — every character
    /// model in the game included.
    #[test]
    fn a_level_model_is_the_identity() {
        let facing = Quat::from_rotation_y(1.1);
        assert_eq!(
            rotation(Conform::Level, [0.3, 0.1, 0.95], facing),
            Affine3A::IDENTITY
        );
        // …and so is a leaning model on flat ground.
        let flat = rotation(Conform::Pitch, [0.0, 0.0, 1.0], facing);
        assert!((Mat3::from(flat.matrix3) - Mat3::IDENTITY).to_cols_array().iter().all(|v| v.abs() < 1e-5));
    }

    /// **Uphill tips the nose up, whichever way the model is facing.** Taking
    /// the normal in world space instead of turning the facing out of it first
    /// is the failure that reads as a model rolling onto its side as it turns —
    /// invisible facing north, obvious facing east — so it is the turned case
    /// that is asserted.
    #[test]
    fn the_lean_is_about_the_models_own_axis_whichever_way_it_faces() {
        use crate::render::axes;
        for yaw in [0.0, 1.0, std::f32::consts::FRAC_PI_2, 3.0] {
            // Ground rising toward the way the model faces: the normal leans
            // back along the facing, which in WoW's axes is `-cos/-sin` of it.
            let (s, c) = yaw.sin_cos();
            let up = [-0.3 * c, -0.3 * s, 0.95];
            let facing = axes::facing(yaw);
            let conform = rotation(Conform::Pitch, up, facing);
            // The drawn nose, in world terms: the placement then the conform.
            let nose = (facing * Quat::from_mat3(&Mat3::from(conform.matrix3))) * Vec3::NEG_Z;
            assert!(nose.y > 0.1, "yaw {yaw}: nose {nose:?}");
            // …and no roll: the drawn up stays out of the horizontal plane.
            let drawn_up = (facing * Quat::from_mat3(&Mat3::from(conform.matrix3))) * Vec3::Y;
            assert!(drawn_up.y > 0.9, "yaw {yaw}: rolled to {drawn_up:?}");
        }
    }

    /// The mount's flag decides for a mounted unit and its own for everyone
    /// else — which is the whole of why a human on a horse leans and a human on
    /// foot does not. Both real values: `HumanMale.m2` is 0 and `Horse.m2` is 1.
    #[test]
    fn a_mounted_unit_asks_the_animals_flag() {
        let rider = Conform::of(0);
        let horse = Conform::of(1);
        assert_eq!(leaning(rider, None), Conform::Level);
        assert_eq!(leaning(rider, Some(horse)), Conform::Pitch);
        // …and it is the animal's answer even when the rider's is the stronger
        // one, which is the case a `max` would get wrong.
        assert_eq!(
            leaning(Conform::of(3), Some(horse)),
            Conform::Pitch,
            "the rider's own flag must not reach the saddle"
        );
    }
}
