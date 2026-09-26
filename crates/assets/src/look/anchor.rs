//! **Where the camera looks at a unit** — the point a third-person rig orbits,
//! which is a rule the client keeps entirely to itself.
//!
//! Nothing on the wire carries it and no file states it: the server sends a
//! *ground* position and the archives ship a model. So the height between them
//! is the client's, computed in the camera's own per-frame update.
//!
//! ## The base
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
//! and the result is clamped to `[0.8333, 15.0]` yards.
//!
//! **[`POINT`] is not the head.** Attachment 11 is where a helmet sits — 2.03
//! model yards on `HumanMale`, the top of the skull — and orbiting it puts the
//! middle of the screen above the character. 17 is 1.80 on the same model, the
//! neck, which is the point a camera should hold. That difference is the whole
//! of the "focal point is a little too high" report, and it is 0.13 yards.
//!
//! **And the fallback matters more than it looks**, because it is what a model
//! with no such point gets — every creature that is not a character. It is the
//! model's own *height*, not its centre: `Creature\Wisp\Wisp.m2` declares a
//! symmetric `[-6.4, -6.4, -6.4]..[6.4, 6.4, 6.4]` cube, authored to cover the
//! dust it sprays, whose **centre is the origin**. A rig anchored on that
//! centre orbits the wisp's feet and drags itself into the ground at any
//! shallow pitch; anchored on 0.9 of the height it sits where a body would.
//! (The wisp in fact carries point 17, at 1.84 — so it takes the first branch
//! and never needed the fallback. What it needed was for the first branch to be
//! asked about the right point.)
//!
//! ## What is *not* transcribed, and why
//!
//! The same function then adjusts the height by a **stand state**: a unit that
//! is sitting, asleep, kneeling, dead or mounted is not where a standing one
//! is. The mapping from stand state to clip is recorded in
//! [`stand_state_clip`] — it is the client's own. What is *not*
//! settled is the quantity it differences: the client queries a value for the
//! clip and for `Stand`, and reading that value as "how high the anchor sits
//! in that clip" gives a mounted camera **below the horse's back** — so it is something
//! else, and a number that produces a plausible-looking wrong picture is
//! exactly what this project does not transcribe on a guess. The table is here;
//! the difference term is not, and [`in_clip`] is this client's own answer for
//! the one state it has to draw.

use crate::world::m2::{M2Attachment, M2Skeleton, PoseLayers};

/// The attachment point the camera orbits — **not** [`crate::world::m2::attach::HELM`].
///
/// Pinned by the client rather than by the file: the client looks up
/// attachment 17 by id.
pub const POINT: u32 = 17;

/// Added to [`POINT`]'s own height before scaling.
pub const RISE: f32 = 0.097_222_224;

/// What fraction of a model's height a model with no [`POINT`] is orbited at.
pub const BOX_FRACTION: f32 = 0.9;

/// The floor and ceiling every camera height is clamped between, in world
/// yards.
pub const MIN: f32 = 0.833_333_3;
/// See [`MIN`].
pub const MAX: f32 = 15.0;

/// `cameraHeightSmoothSpeed`'s shipped default, `"1.2"`.
///
/// **The rate is the client's; the curve this client eases on is not.** The
/// height is smoothed rather than snapped, which is the point — mounting moves
/// the anchor the better part of a yard and a rig that jumps there reads as a
/// glitch. See `world::session::follow_player`.
pub const SMOOTH_SPEED: f32 = 1.2;

/// `AnimationData.dbc` id 91, the clip a rider holds while it is on something.
pub const MOUNT_CLIP: u16 = 91;

/// How far above a model's own origin the camera anchor sits, in **model**
/// yards — the caller multiplies by the unit's scale.
///
/// `height` is the model's own declared box measured top to bottom, in model
/// yards, and is used only when the model carries no [`POINT`].
pub fn base(attachments: &[M2Attachment], height: f32) -> f32 {
    match attachments.iter().find(|a| a.id == POINT) {
        Some(point) => point.position[2] + RISE,
        None => height * BOX_FRACTION,
    }
}

/// Where the anchor sits once a clip has moved the body, in **model** yards.
///
/// `None` when the model has no such clip or no bone to hang the point on,
/// which is every creature that is not a character for [`MOUNT_CLIP`].
///
/// **A per-model constant, sampled at the clip's first frame** — and that is
/// the load-bearing half of it rather than an optimisation. A mount's gait
/// pitches its spine, so an anchor taken from the *live* pose rises and falls
/// with the hooves and the whole view bobs with the animation. The reference
/// bobs a camera only where `cameraBobbing*` says to, which is not here.
pub fn in_clip(attachments: &[M2Attachment], skeleton: &M2Skeleton, clip: u16) -> Option<f32> {
    let point = attachments.iter().find(|a| a.id == POINT)?;
    let sequence = skeleton.find_sequence(clip)?;
    let pose = skeleton.pose(sequence, 0, 0, None, PoseLayers::default());
    let m = pose.get(point.bone as usize)?;
    let p = point.position;
    // Row-major 3x4 in the model's own frame, as `M2::skin_position` applies
    // it. Only the third row is wanted: the anchor's height.
    Some(m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11] + RISE)
}

/// The clip a unit's **stand state** puts it in, as far as the camera is
/// concerned, or `None` for one the camera does not move for.
///
/// The client's mapping — nine entries indexed by
/// `UNIT_FIELD_BYTES_1`'s stand-state byte, which is the same numbering
/// vmangos' `UnitStandStateType` uses:
///
/// ```text
///   0 STAND             -> (none)      4 SIT_LOW_CHAIR    -> 102 SitChairLow
///   1 SIT               -> 97  SitGround  5 SIT_MEDIUM_CHAIR -> 103 SitChairMed
///   2 SIT_CHAIR         -> (none)      6 SIT_HIGH_CHAIR   -> 104 SitChairHigh
///   3 SLEEP             -> 100 Sleep      7 DEAD             ->   6 Dead
///                                       8 KNEEL            -> 115 KneelLoop
/// ```
///
/// Two of the nine answer nothing, and state 2 answering nothing while 4, 5 and
/// 6 each name a chair clip is the shape that says this is a transcription
/// rather than a reconstruction — nobody would invent that gap. A mount
/// overrides all nine with [`MOUNT_CLIP`].
///
/// **Recorded and not yet acted on**: see the module doc for what is missing
/// under it.
pub fn stand_state_clip(stand_state: u8) -> Option<u16> {
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
    /// The assertion that matters is the second one — that the anchor is
    /// **below** the helm — because that difference is the whole report and an
    /// implementation that read point 11 would still pass every other check
    /// here.
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

    /// A model with no point 17 is orbited at 0.9 of its own height — and the
    /// case is `Creature\Wisp\Wisp.m2`'s shape, a box centred on the origin,
    /// where the *centre* this client used to read is zero.
    #[test]
    fn a_model_with_no_point_is_orbited_by_its_height_not_its_centre() {
        let box_centred_on_the_origin = 12.8;
        let anchor = base(&[point(11, 1.0)], box_centred_on_the_origin);
        assert!((anchor - 11.52).abs() < 0.001, "{anchor}");
    }

    /// Both ends of the client's own clamp, which is what keeps a model with a
    /// nonsense box from putting the eye in orbit or on the floor.
    #[test]
    fn the_height_is_clamped_at_both_ends() {
        assert_eq!(clamp(0.0), MIN);
        assert_eq!(clamp(400.0), MAX);
        assert_eq!(clamp(1.9), 1.9);
    }

    /// The two states the table deliberately answers nothing for, which is the
    /// half a reconstruction would have got wrong.
    #[test]
    fn standing_and_a_plain_chair_move_the_camera_for_nothing() {
        assert_eq!(stand_state_clip(0), None);
        assert_eq!(stand_state_clip(2), None);
        assert_eq!(stand_state_clip(1), Some(97));
        assert_eq!(stand_state_clip(8), Some(115));
        assert_eq!(stand_state_clip(200), None);
    }
}
