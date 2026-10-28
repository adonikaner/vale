//! The camera anchor: the point above a unit's feet that a third-person camera
//! orbits.
//!
//! Nothing on the wire carries this height and no file states it. The server
//! sends a ground position and the archives ship a model, and the 1.12.1
//! client computes the height between them in the camera's per-frame update.
//!
//! ## The base height
//!
//! ```text
//! has attachment 17?   yes -> that point's z
//!                              + 0.0972222
//!                              * the unit's scale
//!                      no  -> the model's own box: max.z - min.z
//!                              * the unit's scale
//!                              * 0.9
//! ```
//!
//! The result is clamped to `[0.8333, 15.0]` yards.
//!
//! [`POINT`] is not the head. Attachment 11 is where a helmet sits: 2.03 model
//! yards on `HumanMale`, the top of the skull. Orbiting it puts the middle of
//! the screen above the character. Attachment 17 is 1.80 on the same model, the
//! neck. The 0.13 yards between them made the camera's focal point too high
//! when this client orbited attachment 11.
//!
//! The fallback applies to every model without attachment 17, which is every
//! creature that is not a character. It uses the model's height, not the
//! centre of its box. `Creature\Wisp\Wisp.m2` declares a symmetric
//! `[-6.4, -6.4, -6.4]..[6.4, 6.4, 6.4]` box, sized to cover the dust it emits,
//! whose centre is the origin. A camera anchored on that centre orbits the
//! wisp's feet and goes into the ground at any shallow pitch; anchored at 0.9
//! of the height it sits where a body would. The wisp does carry attachment 17,
//! at 1.84, so it takes the first branch.
//!
//! ## What is not implemented
//!
//! The 1.12.1 client then adjusts the height by the unit's stand state: a unit
//! that is sitting, asleep, kneeling, dead or mounted is not where a standing
//! one is. The mapping from stand state to clip is [`stand_state_clip`]. The
//! quantity the adjustment takes the difference of is not known: the client
//! reads one value for the clip and one for `Stand`, and reading that value as
//! the anchor's height in the clip puts a mounted camera below the horse's
//! back, so it is something else. The adjustment is therefore not applied, and
//! [`in_clip`] is this client's own rule for the one state it draws, mounted.

use crate::world::m2::{M2Attachment, M2Skeleton, PoseLayers};

/// The attachment point the camera orbits. Not
/// [`crate::world::m2::attach::HELM`].
///
/// The 1.12.1 client looks up attachment 17 by id; the model file does not mark
/// it as the camera's.
pub const POINT: u32 = 17;

/// Added to [`POINT`]'s own height before scaling.
pub(crate) const RISE: f32 = 0.097_222_224;

/// What fraction of a model's height a model with no [`POINT`] is orbited at.
pub const BOX_FRACTION: f32 = 0.9;

/// The floor and ceiling every camera height is clamped between, in world
/// yards.
pub const MIN: f32 = 0.833_333_3;
/// See [`MIN`].
pub const MAX: f32 = 15.0;

/// `cameraHeightSmoothSpeed`'s registered default, `"1.2"` yards a second.
///
/// The camera moves to a new anchor height at this speed on the camera's eased
/// curve rather than in one step: mounting moves the anchor most of a yard, and
/// a camera that jumps there looks like a fault. The value is the fallback for
/// the setting of that name; see `world::camera::follow_player` and
/// `world::camera_ease` in the client.
pub const SMOOTH_SPEED: f32 = 1.2;

/// `AnimationData.dbc` id 91, the clip a rider holds while it is on something.
pub const MOUNT_CLIP: u16 = 91;

/// How far above a model's own origin the camera anchor sits, in model yards.
/// The caller multiplies by the unit's scale.
///
/// `height` is the model's own declared box measured top to bottom, in model
/// yards, and is used only when the model carries no [`POINT`].
pub fn base(attachments: &[M2Attachment], height: f32) -> f32 {
    match attachments.iter().find(|a| a.id == POINT) {
        Some(point) => point.position[2] + RISE,
        None => height * BOX_FRACTION,
    }
}

/// Where the anchor sits once a clip has moved the body, in model yards.
///
/// `None` when the model has no such clip or no bone to hang the point on,
/// which is every creature that is not a character for [`MOUNT_CLIP`].
///
/// A constant per model, sampled at the clip's first frame. A mount's gait
/// pitches its spine, so an anchor taken from the live pose would rise and fall
/// with each stride and move the whole view. The 1.12.1 client moves the camera
/// with the stride only when `cameraBobbing` is set, which it is not by
/// default.
pub fn in_clip(attachments: &[M2Attachment], skeleton: &M2Skeleton, clip: u16) -> Option<f32> {
    Some(point_in_clip(attachments, skeleton, clip, POINT)? + RISE)
}

/// The height of attachment `id` at a clip's first frame, in model yards:
/// [`in_clip`] for any point. `None` when the model has no such point or
/// clip.
pub fn point_in_clip(
    attachments: &[M2Attachment],
    skeleton: &M2Skeleton,
    clip: u16,
    id: u32,
) -> Option<f32> {
    let point = attachments.iter().find(|a| a.id == id)?;
    let sequence = skeleton.find_sequence(clip)?;
    let pose = skeleton.pose(sequence, 0, 0, None, PoseLayers::default());
    let m = pose.get(point.bone as usize)?;
    let p = point.position;
    // Row-major 3x4 in the model's own frame, as `M2::skin_position` applies
    // it. Only the third row is wanted: the point's height.
    Some(m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11])
}

/// The clip a unit's stand state puts it in, for the camera's purposes, or
/// `None` for a state the camera does not adjust for.
///
/// The 1.12.1 client's mapping: nine entries indexed by `UNIT_FIELD_BYTES_1`'s
/// stand-state byte, which uses the same numbering as vmangos'
/// `UnitStandStateType`:
///
/// ```text
///   0 STAND             -> (none)      4 SIT_LOW_CHAIR    -> 102 SitChairLow
///   1 SIT               -> 97  SitGround  5 SIT_MEDIUM_CHAIR -> 103 SitChairMed
///   2 SIT_CHAIR         -> (none)      6 SIT_HIGH_CHAIR   -> 104 SitChairHigh
///   3 SLEEP             -> 100 Sleep      7 DEAD             ->   6 Dead
///                                       8 KNEEL            -> 115 KneelLoop
/// ```
///
/// States 0 and 2 map to nothing, although 4, 5 and 6 each name a chair clip.
/// A mount overrides all nine with [`MOUNT_CLIP`].
///
/// Not used outside the tests: the module doc says what is missing before the
/// adjustment can be applied.
#[cfg(test)]
pub(crate) fn stand_state_clip(stand_state: u8) -> Option<u16> {
    match stand_state {
        1 => Some(97),
        3 => Some(100),
        4 => Some(102),
        5 => Some(103),
        6 => Some(104),
        7 => Some(6),
        8 => Some(115),
        _ => None,
    }
}

/// The clamp every camera height passes through, in world yards.
pub fn clamp(height: f32) -> f32 {
    height.clamp(MIN, MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(id: u32, z: f32) -> M2Attachment {
        M2Attachment {
            id,
            bone: 0,
            position: [0.0, 0.0, z],
        }
    }

    /// `HumanMale.m2`'s own numbers: point 17 at 1.80, point 11 at 2.03.
    ///
    /// The second assertion states the rule: the anchor is below the helm. No
    /// other test in this module fails for an implementation that reads point
    /// 11.
    #[test]
    fn the_anchor_is_point_seventeen_and_not_the_helm() {
        let human = [point(11, 2.03), point(17, 1.80)];
        let anchor = base(&human, 2.2);
        assert!((anchor - 1.897).abs() < 0.001, "{anchor}");
        assert!(
            anchor < 2.03,
            "the anchor must sit below the helm, not on it"
        );
    }

    /// A model with no point 17 is orbited at 0.9 of its own height. The case
    /// is `Creature\Wisp\Wisp.m2`'s shape, a box centred on the origin, whose
    /// centre is zero.
    #[test]
    fn a_model_with_no_point_is_orbited_by_its_height_not_its_centre() {
        let box_centred_on_the_origin = 12.8;
        let anchor = base(&[point(11, 1.0)], box_centred_on_the_origin);
        assert!((anchor - 11.52).abs() < 0.001, "{anchor}");
    }

    /// Both ends of the 1.12.1 client's clamp, which keeps a model with an
    /// unusable box from putting the eye far above it or on the floor.
    #[test]
    fn the_height_is_clamped_at_both_ends() {
        assert_eq!(clamp(0.0), MIN);
        assert_eq!(clamp(400.0), MAX);
        assert_eq!(clamp(1.9), 1.9);
    }

    /// The two states the table maps to nothing, and two that it maps.
    #[test]
    fn standing_and_a_plain_chair_move_the_camera_for_nothing() {
        assert_eq!(stand_state_clip(0), None);
        assert_eq!(stand_state_clip(2), None);
        assert_eq!(stand_state_clip(1), Some(97));
        assert_eq!(stand_state_clip(8), Some(115));
        assert_eq!(stand_state_clip(200), None);
    }
}
