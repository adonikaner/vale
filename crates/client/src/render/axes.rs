//! The one place where WoW's axes become Bevy's.
//!
//! WoW works in +X north, +Y west, +Z up; Bevy in +X right, +Y up,
//! −Z forward. Both are right-handed, so the change of basis is a rotation
//! and nothing here needs a mirror.
//!
//! The data stays in WoW's axes and is converted here, not at the source.
//! Every constant in `adt`, `m2` and `wmo` is expressed in the file's own frame,
//! and the checks that make them trustworthy — `vale wmos` agreeing with the
//! game's own `MODF` boxes to 0.00 yards, the alpha atlas round trip — are
//! expressed there too. Rewriting the parsers into Bevy's frame would invalidate
//! every one of those measurements to save this module.

use bevy::math::{Affine3A, Vec3A};
use bevy::prelude::*;

/// WoW → Bevy: `(x, y, z)` → `(−y, z, −x)`.
///
/// North becomes −Z (Bevy's forward), west becomes −X, up becomes +Y.
pub fn to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(-p[1], p[2], -p[0])
}

/// Bevy → WoW, the exact inverse of [`to_bevy`].
pub fn to_wow(v: Vec3) -> [f32; 3] {
    [-v.z, -v.x, v.y]
}

/// The change of basis as a matrix, for rotating whole transforms rather than
/// points — see [`to_bevy_affine`].
///
/// Orthogonal with determinant +1, which is the formal statement that this
/// preserves handedness. Both facts are asserted in the tests, because getting
/// it wrong mirrors the world rather than failing.
pub fn basis() -> Mat3 {
    // Columns are the images of the WoW basis vectors: north, west, up.
    Mat3::from_cols(
        Vec3::new(0.0, 0.0, -1.0), // +X north  -> −Z
        Vec3::new(-1.0, 0.0, 0.0), // +Y west   -> −X
        Vec3::new(0.0, 1.0, 0.0),  // +Z up     -> +Y
    )
}

/// A WoW-space transform expressed in Bevy space: `B · M · B⁻¹`.
///
/// Used for anything already composed in the file's own frame — most importantly
/// `adt::placement_matrix`, which carries the derived 180° term that drops
/// doodads and buildings onto their `MODF` boxes. Conjugating it is how that
/// derivation survives the move to Bevy instead of being re-eyeballed.
pub fn to_bevy_affine(m: Mat4) -> Mat4 {
    let b = Mat4::from_mat3(basis());
    // Orthogonal, so the inverse is the transpose, and saying so here keeps a
    // general 4x4 inversion (and its failure mode on a singular matrix) out of a
    // function called once per placement.
    let b_inv = Mat4::from_mat3(basis().transpose());
    b * m * b_inv
}

/// A Bevy-space transform expressed in WoW space: `B⁻¹ · M · B`, the exact
/// inverse of [`to_bevy_affine`]. For a delta measured between two entity
/// transforms that has to be applied to data kept in the file's own frame,
/// such as a collision hull.
pub fn from_bevy_affine(m: Mat4) -> Mat4 {
    let b = Mat4::from_mat3(basis());
    let b_inv = Mat4::from_mat3(basis().transpose());
    b_inv * m * b
}

/// One bone's pose — `M2Skeleton::pose`'s row-major 3x4, in the model's own
/// frame — as a Bevy affine.
///
/// An affine and not a `Transform`, because a composed bone matrix is not
/// obliged to be a translation, a rotation and a scale: a bone with non-uniform
/// scale and a rotated child produces shear, and `Transform::from_matrix` would
/// decompose that away silently and pose the model plausibly wrongly. The
/// engine reads joints as `GlobalTransform`, which is an affine and can hold it.
pub fn pose_to_bevy(m: &[f32; 12]) -> Mat4 {
    // Row-major 3x4 -> column-major 4x4, then the same conjugation every other
    // composed-in-WoW-space transform gets.
    to_bevy_affine(Mat4::from_cols_array(&[
        m[0], m[4], m[8], 0.0, //
        m[1], m[5], m[9], 0.0, //
        m[2], m[6], m[10], 0.0, //
        m[3], m[7], m[11], 1.0,
    ]))
}

/// [`pose_to_bevy`] as an affine, by shuffling rather than multiplying.
///
/// The basis is a signed permutation, so `B * M * B^T` moves every entry of
/// `M` to another slot and flips some signs: entry `(i, j)` of the result is
/// `s_i * s_j * M[p(i)][p(j)]` with `p = (1, 2, 0)` and `s = (-1, +1, -1)`,
/// and the translation is `(-t_y, t_z, -t_x)`, which is [`to_bevy`]. Products
/// by 1 and -1 and sums with 0 are exact in floating point, so this is the
/// same value [`pose_to_bevy`] computes with two 4x4 multiplies — bit for
/// bit, as its test pins — and it is on the per-bone path of the pose pass,
/// where the two multiplies were most of a joint's cost.
pub fn pose_to_bevy_affine(m: &[f32; 12]) -> Affine3A {
    Affine3A::from_cols(
        Vec3A::new(m[5], -m[9], m[1]),
        Vec3A::new(-m[6], m[10], -m[2]),
        Vec3A::new(m[4], -m[8], m[0]),
        Vec3A::new(-m[7], m[11], -m[3]),
    )
}

/// A WoW facing — radians counter-clockwise about +Z, zero pointing north —
/// as a Bevy rotation.
///
/// The angle carries across unchanged: WoW's rotation about up and Bevy's
/// about +Y turn the same direction once north is −Z and west is −X. That is a
/// coincidence of this particular basis change rather than a general rule, so
/// there is a test pinning it.
pub fn facing(orientation: f32) -> Quat {
    Quat::from_rotation_y(orientation)
}

/// …and the same with a pitch: how far the body is tipped nose-up, in
/// radians, up-positive.
///
/// Applied after the yaw in the composition and therefore about the model's
/// own right axis, which is the only order that behaves: a pitch taken in
/// world space rolls the character as it turns. In Bevy's basis a model at
/// yaw 0 faces −Z with +Y up, so a positive rotation about +X takes the nose to
/// +Y — nose-up for a positive angle, which is what the sign here means.
///
/// Every unit has a body pitch and only a swimmer has a non-zero one.
/// The wire carries `MovementInfo::pitch` under `MOVEFLAG_SWIMMING` and under
/// nothing else.
///
/// The prediction this comment used to carry is retracted: it said the
/// walking-a-slope lean would be this same term off the ground normal. It is
/// not — see `vale_assets::look::conform`. The reference builds a whole basis, gated
/// on the model's own `GlobalModelFlags`, into the model matrix; this stays
/// what it is, the one angle the wire carries, in the entity's transform.
pub fn body(orientation: f32, pitch: f32) -> Quat {
    Quat::from_rotation_y(orientation) * Quat::from_rotation_x(pitch)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shuffle and the two multiplies are one function: every entry of
    /// a general 3x4 matrix lands in the same slot with the same sign, to the
    /// bit. The matrix is deliberately not a rotation, so that a wrong
    /// transpose could not hide behind orthogonality.
    #[test]
    fn the_pose_shuffle_is_the_conjugation_bit_for_bit() {
        let mut seed = 0x9e37_79b9u32;
        for _ in 0..200 {
            let mut m = [0.0f32; 12];
            for v in &mut m {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                *v = (seed >> 8) as f32 / (1u32 << 24) as f32 * 20.0 - 10.0;
            }
            let long = Affine3A::from_mat4(pose_to_bevy(&m));
            let short = pose_to_bevy_affine(&m);
            assert_eq!(long, short, "{m:?}");
        }
    }

    /// Within a yard-scale epsilon; these are f32 world coordinates.
    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-5
    }

    /// A positive pitch is nose-up, and it is taken about the model's own
    /// right axis.
    ///
    /// Both halves fail silently the wrong way round: a sign error draws a
    /// diver climbing while they sink, and taking the rotation in world space
    /// instead rolls the character onto their side as they turn — which is
    /// invisible facing north and obvious facing east, so it is the yawed case
    /// that is asserted here rather than the identity one.
    #[test]
    fn a_body_pitch_tips_the_nose_up_about_the_models_own_axis() {
        let nose = |q: Quat| q * Vec3::NEG_Z;
        // At yaw 0 the model faces −Z; pitched up a quarter turn it faces +Y.
        assert!(close(nose(body(0.0, std::f32::consts::FRAC_PI_2)), Vec3::Y));
        assert!(close(nose(body(0.0, -std::f32::consts::FRAC_PI_2)), Vec3::NEG_Y));
        // …and a quarter turn of yaw leaves the pitch pointing at the same
        // sky rather than swinging it sideways, which is what a world-space
        // rotation would do.
        let turned = body(std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_4);
        assert!(nose(turned).y > 0.7, "{:?}", nose(turned));
        // …and the up vector stays out of the horizontal plane: no roll.
        let up = turned * Vec3::Y;
        assert!(up.y > 0.7, "the model rolled: {up:?}");
        // A zero pitch is exactly the old rotation, so nothing that does not
        // swim moves a pixel.
        assert_eq!(body(1.234, 0.0), facing(1.234));
    }

    #[test]
    fn the_cardinal_directions_land_where_bevy_expects_them() {
        // North is Bevy's forward, which is −Z.
        assert!(close(to_bevy([1.0, 0.0, 0.0]), Vec3::NEG_Z));
        // West is −X: Bevy's +X is east, because +Y west and +Z up fix the sign.
        assert!(close(to_bevy([0.0, 1.0, 0.0]), Vec3::NEG_X));
        // Up is up. This is the one that would be obvious on screen; the other
        // two are the ones that mirror the world silently.
        assert!(close(to_bevy([0.0, 0.0, 1.0]), Vec3::Y));
    }

    #[test]
    fn the_conversion_round_trips() {
        let p = [1234.5, -678.25, 91.125];
        let back = to_wow(to_bevy(p));
        for i in 0..3 {
            assert!((back[i] - p[i]).abs() < 1e-5, "component {i}: {back:?} != {p:?}");
        }
    }

    /// The whole point of choosing this mapping over the several that also put
    /// up at +Y: a determinant of −1 would mirror the world, which reads as
    /// "the terrain is inside out" rather than as an error.
    #[test]
    fn the_basis_is_a_rotation_and_not_a_mirror() {
        let b = basis();
        assert!((b.determinant() - 1.0).abs() < 1e-6, "det = {}", b.determinant());
        // Orthogonal, which is what lets `to_bevy_affine` use the transpose.
        let should_be_identity = b * b.transpose();
        assert!((should_be_identity - Mat3::IDENTITY).to_cols_array().iter().all(|v| v.abs() < 1e-6));
    }

    /// `to_bevy_affine` must agree with `to_bevy` on points, or a placed doodad
    /// and a loose vertex disagree about where the world is.
    #[test]
    fn conjugating_a_transform_agrees_with_converting_the_point() {
        // An arbitrary WoW-space transform: rotate about up, then translate.
        let wow = Mat4::from_rotation_z(0.7) * Mat4::from_translation(Vec3::new(10.0, 20.0, 30.0));
        let point = [3.0, -4.0, 5.0];

        // Convert the transform, then apply it in Bevy space.
        let via_matrix = to_bevy_affine(wow).transform_point3(to_bevy(point));
        // Apply it in WoW space, then convert the result once. Formally
        // `(B·M·B⁻¹)·(B·p) = B·(M·p)`, which is the whole claim.
        let via_point = to_bevy(wow.transform_point3(Vec3::from_array(point)).to_array());
        assert!(close(via_matrix, via_point), "{via_matrix:?} != {via_point:?}");
    }

    /// Pins the claim in [`facing`]'s doc comment. A creature facing north at
    /// orientation 0 must look along Bevy's forward.
    #[test]
    fn facing_zero_looks_north() {
        let looked = facing(0.0) * Vec3::NEG_Z;
        assert!(close(looked, to_bevy([1.0, 0.0, 0.0])), "{looked:?}");
    }

    /// A pose matrix moves a model-space vertex, and converting the two
    /// separately has to land in the same place — this is the join between
    /// `M2Skeleton::pose`, which is checked against real models in the assets
    /// crate, and the joints the engine skins with.
    #[test]
    fn a_posed_vertex_lands_where_the_pose_put_it() {
        // A quarter turn about the model's own up, then a yard north: a bone
        // that both rotates and translates, so a dropped term shows.
        let (c, s) = (std::f32::consts::FRAC_PI_2.cos(), std::f32::consts::FRAC_PI_2.sin());
        let bone = [
            c, -s, 0.0, 1.0, //
            s, c, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ];
        let vertex = [2.0, 0.0, 3.0];

        // Skinned in the model's frame, exactly as `M2::skin_position` does it,
        // and converted once.
        let wow = [
            bone[0] * vertex[0] + bone[1] * vertex[1] + bone[2] * vertex[2] + bone[3],
            bone[4] * vertex[0] + bone[5] * vertex[1] + bone[6] * vertex[2] + bone[7],
            bone[8] * vertex[0] + bone[9] * vertex[1] + bone[10] * vertex[2] + bone[11],
        ];

        // Converted first, then skinned — which is what the GPU does.
        let via_bevy = pose_to_bevy(&bone).transform_point3(to_bevy(vertex));
        assert!(close(via_bevy, to_bevy(wow)), "{via_bevy:?} != {:?}", to_bevy(wow));
    }

    /// And a quarter turn counter-clockwise from north is west, in both frames.
    #[test]
    fn facing_a_quarter_turn_looks_west() {
        let looked = facing(std::f32::consts::FRAC_PI_2) * Vec3::NEG_Z;
        assert!(close(looked, to_bevy([0.0, 1.0, 0.0])), "{looked:?}");
    }
}
