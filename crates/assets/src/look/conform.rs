//! **Which models lean with the ground, and how far** — the rule behind a horse
//! standing square on a hillside instead of upright on it.
//!
//! Nothing on the wire says anything about this and no DBC has a column for it.
//! The one thing in the game that states it is the **M2 header's own
//! `GlobalModelFlags`**, masked with `3`, and the client dispatches on it in
//! three ways:
//!
//! ```text
//! 1        pitch only
//! 0 and 2  level, no conform at all
//! 3        pitch and roll
//! ```
//!
//! **So a player on foot does not tilt and never did.** `HumanMale.m2` reads
//! flag `0`; `Creature\Horse\Horse.m2` and `Creature\Wolf\Wolf.m2` read `1`;
//! `Kodobeast` and `Crab` read `3`. That is the whole shape of the report this
//! module answers — the screenshot filed with it is a *horse* standing bolt
//! upright on a slope, and the character on its back is upright because the
//! horse is. The gate is the model's flag, not whether anybody is riding it, so
//! a wild wolf on a hill leans exactly as the mount does.
//!
//! ## The three stages
//!
//! 1. **A ground normal**: the client averages the normals of its
//!    own collision *contacts*, keeping only those steeper than
//!    [`WALKABLE_Z`] — and writes straight up when none qualifies.
//! 2. **Smoothing**: an exponential decay of the *up vector*
//!    toward that normal at [`SETTLE_PER_SECOND`] — with a **freeze band**
//!    under [`FREEZE_Z`], where the stance is held rather than conformed.
//! 3. **The basis**: the flag-picked rotation, and **there is no
//!    clamp anywhere in it**. A model on a 60° face leans 60°; all of the
//!    softness in the reference is in stage 2 and none of it is a limit.
//!
//! Every constant below is the client's own. What is *not* matched is stage
//! 1's input: this client has no contact
//! list, so it samples the one surface under the feet instead — see
//! [`Stance::settle`], which says what that costs.

/// Which of the client's three conform modes a model's `GlobalModelFlags`
/// names — the client's own dispatch, and the only thing that decides whether a
/// model leans at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Conform {
    /// Upright whatever it is standing on: every character model in the game,
    /// and everything else that authors `0` or `2`.
    #[default]
    Level,
    /// **Pitch only** (flag 1): the nose goes up the hill and the roll is
    /// discarded. 152 mounts and the quadrupeds.
    Pitch,
    /// **Pitch and roll** (flag 3): the model's own up becomes the ground's,
    /// verbatim. The low, wide creatures — kodos, basilisks, crabs, crocolisks,
    /// spiders — where a body that stayed level would sink a flank into the
    /// hill.
    PitchAndRoll,
}

impl Conform {
    /// The mode a model's header names.
    ///
    /// **Flag 2 is inert, and that is the file's doing rather than a
    /// simplification here**: the client has a branch for `1` and for `3` and
    /// falls through for `0` and `2` alike.
    pub fn of(global_flags: u32) -> Conform {
        match global_flags & 3 {
            1 => Conform::Pitch,
            3 => Conform::PitchAndRoll,
            _ => Conform::Level,
        }
    }

    /// Does this model lean at all? The cheap test every caller opens with.
    pub fn leans(self) -> bool {
        self != Conform::Level
    }

    /// The conform rotation, as the three **columns** of a 3x3 in the model's
    /// own frame — forward, left, up, which is `+X`, `+Y`, `+Z` in WoW's axes.
    ///
    /// `up` is the smoothed ground normal expressed in that same frame, i.e.
    /// the world normal turned back through the model's own facing. Expressing
    /// it locally is what makes this a rotation a caller can compose *after* a
    /// facing it already has, rather than a whole world matrix to replace it
    /// with; the reference builds the world matrix directly because it has the
    /// yaw to hand there, and the two are the same rotation.
    ///
    /// The difference between the two branches is one vector:
    ///
    /// * **pitch only** forces the left axis *horizontal* — the reference
    ///   writes a literal `0` into its z — so the model's up
    ///   comes out as the part of the normal that is square to it and the roll
    ///   is dropped;
    /// * **pitch and roll** takes left from `up x forward` instead, and then
    ///   copies the normal into the up row **verbatim**, which is
    ///   the whole of what makes a crab follow a camber.
    pub fn basis(self, up: [f32; 3]) -> [[f32; 3]; 3] {
        // The model's own forward before any of this, which in its own frame is
        // the x axis by definition. The reference reads it out of the matrix it
        // has already built the yaw into; here it is a constant, and that is
        // the same statement.
        const FORWARD: [f32; 3] = [1.0, 0.0, 0.0];
        const LEVEL: [[f32; 3]; 3] = [FORWARD, [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

        let Some(up) = normalize(up) else {
            return LEVEL;
        };
        let left = match self {
            Conform::Level => return LEVEL,
            // `(-forward.y, forward.x, 0)`, which for a local forward of +X is
            // exactly +Y. Left horizontal is the whole of the pitch-only rule.
            Conform::Pitch => [0.0, 1.0, 0.0],
            Conform::PitchAndRoll => match normalize(cross(up, FORWARD)) {
                Some(left) => left,
                // The normal lies along the facing — impossible for anything
                // the freeze band lets through, and level rather than inverted
                // if it ever happens.
                None => return LEVEL,
            },
        };
        let Some(forward) = normalize(cross(left, up)) else {
            return LEVEL;
        };
        match self {
            // The recomputed up: square to both, so it is the normal with its
            // roll about the forward axis taken out.
            Conform::Pitch => [forward, left, cross(forward, left)],
            // …and the normal itself, which is what the reference copies.
            Conform::PitchAndRoll => [forward, left, up],
            Conform::Level => LEVEL,
        }
    }
}

/// The steepest face that still counts as ground, as the `z` of its unit
/// normal: **0.0174524** — `cos 89°` to seven places.
///
/// A contact steeper than this is not walked on and does not vote. With no
/// votes at all the reference writes straight up, which is what stands a
/// model back up when it leaves the ground.
pub const WALKABLE_Z: f32 = 0.017_452_406;

/// …and the shallower cutoff under which the stance is **frozen** rather than
/// conformed: **0.3572124** — `cos 69.07°`.
///
/// The client compares the sampled normal's `z` against it and skips
/// the whole smoothing step when it is under, so the model holds whatever
/// stance it had. The band between the two — 69° to 89° — is therefore ground
/// that is walkable and *not* leaned into, which is why running up a cliff face
/// does not lay a horse on its back.
pub const FREEZE_Z: f32 = 0.357_212_36;

/// How much of the gap to the target survives one second: **0.0018**, a
/// double.
///
/// The reference raises it to the frame's delta (`pow`) and scales
/// the *residual* by the result, so the settle is frame-rate independent:
/// `up = n + (up - n) * 0.0018^dt`. That is a time constant of about 158 ms —
/// fast enough that a mount seats itself on a slope within a stride, slow
/// enough that a cell boundary is a lean rather than a snap.
pub const SETTLE_PER_SECOND: f64 = 0.0018;

/// The smoothed up-vector one unit is standing at — stage 2, and the only state
/// this rule has.
///
/// In the world's own axes, not the model's: the sample is a world normal and
/// the facing turns under it while the stance does not.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stance {
    /// `None` until the first sample, which is **seeded at the slope rather
    /// than eased onto it**: a mount that has just been summoned, or a creature
    /// that has just streamed in, is already standing where it stands. Easing
    /// from upright would tip every new arrival in the world for a sixth of a
    /// second.
    up: Option<[f32; 3]>,
}

impl Stance {
    /// Take one sample and settle toward it, answering the stance to draw at.
    ///
    /// `sampled` is the normal of the surface under the feet, or `None` for a
    /// unit that is not on one — swimming, or in the air. That is this client's
    /// stand-in for the reference's contact list, and it is a **named
    /// approximation**: the reference averages every walkable contact a unit has,
    /// so a horse straddling a ridge stands on the mean of two faces where this
    /// takes the one under its centre. The two agree on any uniform slope,
    /// which is nearly all of the outdoors, and where they differ this one is
    /// twitchier by one cell boundary — under the same 158 ms settle.
    pub fn settle(&mut self, sampled: Option<[f32; 3]>, dt: f32) -> [f32; 3] {
        let up = match self.up {
            Some(up) => up,
            // First sight: adopt the target outright.
            None => {
                let seeded = target(sampled).unwrap_or(UP);
                self.up = Some(seeded);
                return seeded;
            }
        };
        let Some(target) = target(sampled) else {
            // The freeze band: hold what we had.
            return up;
        };
        let keep = (SETTLE_PER_SECOND.powf(dt as f64)) as f32;
        let settled = [
            target[0] + (up[0] - target[0]) * keep,
            target[1] + (up[1] - target[1]) * keep,
            target[2] + (up[2] - target[2]) * keep,
        ];
        self.up = Some(settled);
        settled
    }

    /// What this stance is now, without advancing it — for a caller that has to
    /// draw a frame it did not sample on.
    pub fn up(&self) -> [f32; 3] {
        self.up.unwrap_or(UP)
    }
}

/// Straight up in the world's axes, which is where a model with nothing under
/// it stands.
pub const UP: [f32; 3] = [0.0, 0.0, 1.0];

/// What one sample asks the stance to settle toward — the two gates composed.
///
/// * a face too steep to walk on, or no surface at all, targets **straight
///   up**;
/// * a face in the 69°..89° band answers **`None`**, which is the freeze
///   (the whole settle is skipped);
/// * anything else tracks its own normal.
pub fn target(sampled: Option<[f32; 3]>) -> Option<[f32; 3]> {
    match sampled {
        Some(n) if n[2] < WALKABLE_Z => Some(UP),
        Some(n) if n[2] < FREEZE_Z => None,
        Some(n) => Some(n),
        None => Some(UP),
    }
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    // The client's own "too short to normalise" floor, which it
    // applies at every one of these three cross products.
    (len > 2.384_185_8e-7).then(|| [v[0] / len, v[1] / len, v[2] / len])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    /// The flag dispatch, and the two live values a shipped model
    /// actually authors. **2 is inert** — the reference falls through it the
    /// same way it falls through 0 — and the high bits are not part of the
    /// question.
    #[test]
    fn the_flag_dispatch_is_the_bottom_two_bits() {
        assert_eq!(Conform::of(0), Conform::Level);
        assert_eq!(Conform::of(1), Conform::Pitch);
        assert_eq!(Conform::of(2), Conform::Level);
        assert_eq!(Conform::of(3), Conform::PitchAndRoll);
        // Horse.m2 reads 0x1 and Kodobeast.m2 0x3; a model with other bits set
        // must still answer on the bottom two alone.
        assert_eq!(Conform::of(0xffff_fffd), Conform::Pitch);
        assert!(!Conform::of(0).leans());
        assert!(Conform::of(3).leans());
    }

    /// Level ground is the identity in every mode, which is the claim that
    /// nothing not standing on a slope moves a pixel.
    #[test]
    fn level_ground_is_the_identity() {
        for mode in [Conform::Level, Conform::Pitch, Conform::PitchAndRoll] {
            let b = mode.basis(UP);
            assert!(close(b[0], [1.0, 0.0, 0.0]), "{mode:?}: {b:?}");
            assert!(close(b[1], [0.0, 1.0, 0.0]), "{mode:?}: {b:?}");
            assert!(close(b[2], [0.0, 0.0, 1.0]), "{mode:?}: {b:?}");
        }
    }

    /// **Uphill is nose-up, and downhill is nose-down.** The sign is the half
    /// of this that fails silently: a model tipped the wrong way looks
    /// deliberate.
    ///
    /// Ground rising toward the model's own forward (`+X`) has a normal leaning
    /// *backwards*, so `n.x` is negative — and the forward axis has to come out
    /// with a positive `z`.
    #[test]
    fn a_slope_under_the_nose_tips_it_up() {
        let uphill = normalize([-0.3, 0.0, 0.95]).unwrap();
        for mode in [Conform::Pitch, Conform::PitchAndRoll] {
            let forward = mode.basis(uphill)[0];
            assert!(forward[2] > 0.1, "{mode:?} did not tip up: {forward:?}");
        }
        let downhill = normalize([0.3, 0.0, 0.95]).unwrap();
        for mode in [Conform::Pitch, Conform::PitchAndRoll] {
            let forward = mode.basis(downhill)[0];
            assert!(forward[2] < -0.1, "{mode:?} did not tip down: {forward:?}");
        }
    }

    /// The one difference between the two live modes: a **camber** — ground
    /// falling away to one side with none of it under the nose — rolls a flag-3
    /// model and leaves a flag-1 model level.
    #[test]
    fn only_pitch_and_roll_follows_a_camber() {
        let camber = normalize([0.0, 0.3, 0.95]).unwrap();
        let pitched = Conform::Pitch.basis(camber);
        assert!(close(pitched[2], [0.0, 0.0, 1.0]), "{pitched:?}");
        assert!(close(pitched[0], [1.0, 0.0, 0.0]), "{pitched:?}");
        // …where flag 3 carries the normal into its up row verbatim.
        let rolled = Conform::PitchAndRoll.basis(camber);
        assert!(close(rolled[2], camber), "{rolled:?}");
    }

    /// Whatever the slope, the answer has to be a rotation: three unit columns,
    /// mutually square, right-handed. A basis that quietly loses its
    /// orthogonality shears the model instead of turning it.
    #[test]
    fn every_answer_is_a_rotation() {
        let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        for n in [[-0.4, 0.2, 0.9], [0.5, -0.5, 0.7], [0.0, 0.0, 1.0], [0.6, 0.1, 0.79]] {
            let n = normalize(n).unwrap();
            for mode in [Conform::Pitch, Conform::PitchAndRoll] {
                let b = mode.basis(n);
                for col in b {
                    assert!((dot(col, col) - 1.0).abs() < 1e-4, "{mode:?} {n:?}: {b:?}");
                }
                assert!(dot(b[0], b[1]).abs() < 1e-4, "{mode:?} {n:?}: {b:?}");
                assert!(dot(b[1], b[2]).abs() < 1e-4, "{mode:?} {n:?}: {b:?}");
                assert!(dot(b[0], b[2]).abs() < 1e-4, "{mode:?} {n:?}: {b:?}");
                // Right-handed: forward x left = up, or the model is mirrored.
                assert!(close(cross(b[0], b[1]), b[2]), "{mode:?} {n:?}: {b:?}");
            }
        }
    }

    /// The two gates, each of which fails in its own direction: a
    /// missed walkable cutoff lays a model against a cliff, and a missed freeze
    /// band is the same thing one step less extreme.
    #[test]
    fn the_two_bands_are_where_the_bytes_say() {
        assert_eq!(target(None), Some(UP));
        // Steeper than 89°: not ground, stand up.
        assert_eq!(target(Some([1.0, 0.0, 0.01])), Some(UP));
        // Between 69° and 89°: freeze.
        assert_eq!(target(Some([0.9, 0.0, 0.2])), None);
        // Walkable: track it.
        let n = normalize([0.0, 0.3, 0.95]).unwrap();
        assert_eq!(target(Some(n)), Some(n));
        // …and the two constants really are the cosines the addresses hold.
        assert!((WALKABLE_Z.acos().to_degrees() - 89.0).abs() < 1e-4);
        assert!((FREEZE_Z.acos().to_degrees() - 69.07).abs() < 0.01);
    }

    /// The settle is frame-rate independent and seeds warm — the two things a
    /// hand-rolled lerp gets wrong. A sixtieth of a second keeps ~90% of the
    /// gap; a whole second keeps 0.18% of it.
    #[test]
    fn the_stance_seeds_at_the_slope_and_settles_at_the_measured_rate() {
        let slope = normalize([-0.3, 0.0, 0.95]).unwrap();
        let mut stance = Stance::default();
        // Warm: the first frame is already at the slope, not easing onto it.
        assert!(close(stance.settle(Some(slope), 1.0 / 60.0), slope));

        // …and from level, one frame covers about a tenth of the gap.
        let mut stance = Stance::default();
        stance.settle(Some(UP), 1.0 / 60.0);
        let after = stance.settle(Some(slope), 1.0 / 60.0);
        let covered = (after[0] - UP[0]) / (slope[0] - UP[0]);
        assert!((covered - 0.1).abs() < 0.01, "one frame covered {covered}");

        // Two half-frames must equal one whole one, which is the whole point of
        // raising the constant to `dt` rather than scaling by it.
        let mut halves = Stance::default();
        halves.settle(Some(UP), 1.0 / 60.0);
        halves.settle(Some(slope), 1.0 / 120.0);
        let split = halves.settle(Some(slope), 1.0 / 120.0);
        assert!(close(split, after), "{split:?} != {after:?}");
    }

    /// The freeze holds the *previous* stance rather than reverting to level,
    /// which is the difference between a model that pauses on a cliff and one
    /// that snaps upright halfway up it.
    #[test]
    fn the_freeze_band_holds_the_stance_it_had() {
        let slope = normalize([-0.5, 0.0, 0.86]).unwrap();
        let mut stance = Stance::default();
        stance.settle(Some(slope), 1.0 / 60.0);
        let held = stance.settle(Some([0.9, 0.0, 0.2]), 1.0 / 60.0);
        assert!(close(held, slope), "{held:?}");
        // …and leaving the ground altogether does not freeze: it stands up.
        let after = stance.settle(None, 1.0);
        assert!(after[2] > slope[2], "{after:?}");
    }
}
