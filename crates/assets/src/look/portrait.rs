//! **Where to stand to take a unit's picture** — the framing behind
//! `SetPortraitTexture`, and the one thing about a portrait that is not a
//! renderer question.
//!
//! ```text
//! M2::cameras -> the one whose kind is 0 -> eye, aim, vertical fov
//!             -> …or, for a model carrying none, a framing off its own box
//! ```
//!
//! ## Two questions, and the file answers both
//!
//! [`framing`] is the head-and-shoulders shot a `<Texture>` region gets from
//! `SetPortraitTexture`. [`body_framing`] is the **full-body** one a
//! `<PlayerModel>` gets from `SetUnit` — the character sheet, the pet paper
//! doll, the dress-up frame and the tabard designer — and it is the same
//! question asked of the model's *other* camera. They are in one file because
//! they are one subject and share every rule below; they are two functions
//! because answering either for both is a well-formed picture of the wrong
//! thing. See [`BODY_KIND`].
//!
//! ## The framing is the file's, and it is a measurement rather than a guess
//!
//! A `<Texture>` region is not a picture of anything until somebody says where
//! the camera goes, and this project has a standing rule about inventing that
//! sort of number: a wrong framing renders *plausibly* — a chest, a pair of
//! knees, the inside of a wolf — rather than failing. So it is not invented.
//! `M2Camera::kind` has been parsed since the login screen landed and its `0`
//! is documented there as the portrait camera; what was never checked is
//! whether the models in the *world* carry one and whether it points at a head.
//! Both are now measured, and `vale portrait` is the census:
//!
//! ```text
//! HumanMale  kind 0  fov 0.785  eye  0.633 -0.388 1.887   aim 0.063 0.034 1.864
//! HumanMale  kind 1  fov 0.980  eye  3.659  0.034 0.923   aim -0.364 0.029 0.987
//! Wolf       kind 0  fov 0.950  eye  1.693  0.724 0.884   aim 1.120 -0.159 0.877
//! Murloc     kind 0  fov 0.950  eye  0.509  0.487 0.735   aim 0.308 0.039 0.704
//! ```
//!
//! **The separator is the distance, and it is not close.** Over the 411 models
//! `CreatureModelData` names, 408 decode and **401 carry a camera of kind 0**;
//! their median eye stands **0.91 yards** from what it looks at, where the 122
//! kind-1 cameras stand four to six. That is a head-and-shoulders view against a
//! full-body one, and no reading of the type code is needed to tell them apart.
//! The kind-0 aim sits at a median **74%** of the model's bind-pose height
//! against the kind-1 camera's **46%** — a real separation, but a weaker one and
//! deliberately not the test: a wolf's head is 47% of the way up because a wolf
//! is long rather than tall, and eight models aim below the waist for exactly
//! that reason. Distance is the measurement; height is corroboration.
//!
//! **What is measured is the geometry; what is interpreted is the name.** That
//! the kind-0 camera is a close three-quarter view of the model's head is a
//! measurement over the whole bestiary. That the reference client uses *this*
//! camera for `SetPortraitTexture` is a reading of the field's own type code and
//! is not confirmed against the client — the two halves are recorded apart
//! here on purpose.
//!
//! ## …and choosing by kind rather than by index is worth the line it costs
//!
//! `cameras[0]` is the portrait camera for all but **five** of the 401 — the
//! four High Elf males and a mounted death knight, which list theirs the other
//! way round. Five is small and the failure is not: those NPCs would have drawn
//! as a full-body shot from four yards in a 37-pixel square, which is a
//! silhouette. See [`camera`], which is where the fallback is deliberately
//! *not* taken.
//!
//! ## A model with no camera is framed from its box, and that half *is* invented
//!
//! [`Source::Derived`] says so in the type. Nothing in any file states a framing
//! for a model that carries no camera, so this composes one from the model's own
//! bind-pose box: aim at the top of it, stand [`THREE_QUARTER`] round and far
//! enough back that a head-sized subject fills the frame. It is marked rather
//! than blended in, so the census can count how much of the game is drawn by a
//! rule the game does not state — and the answer is **seven models**, all of
//! them a taxi mount, a portal, an invisible stalker or a tiger cub. Nothing a
//! unit frame is likely to be pointed at, which is why this half is allowed to
//! be an invention at all.

use crate::world::m2::{M2Camera, M2};

/// The `M2Camera::kind` that frames a portrait. `1` is the character-info
/// camera — the character sheet's full-body shot — and `-1` is anything else.
pub const PORTRAIT_KIND: i32 = 0;

/// How far round from dead ahead a derived framing stands, in radians.
///
/// π/8, a shallow three-quarter view. Chosen to sit inside the spread the
/// *files* use rather than picked for looks: over the models that carry a
/// portrait camera the eye is 20° to 50° off the aim axis, and this is at the
/// low end of that, which is the safe end — a head turned too far shows an ear.
pub const THREE_QUARTER: f32 = std::f32::consts::FRAC_PI_8;

/// The vertical field of view a derived framing uses, in radians.
///
/// **0.95, which is not a taste**: it is the value 1.12's own portrait cameras
/// state almost universally — the wolf's, the murloc's and most of the bestiary
/// carry exactly this, and the human male's 0.785 is the outlier. Deriving a
/// framing at some other angle would put a model with no camera at a visibly
/// different apparent size from its neighbour on the same unit frame.
pub const DERIVED_FOV: f32 = 0.95;

/// How much of a model's height a derived framing treats as its head.
///
/// A quarter. The distance follows from it and the field of view, so this is
/// the only free number in the derivation and it is the one to change if
/// derived portraits come out too tight or too loose.
const HEAD_FRACTION: f32 = 0.25;

/// Where a framing came from, kept so a caller can count the two apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The model's own camera of [`PORTRAIT_KIND`].
    Camera,
    /// …and a model that carries none, framed off its bounding box by the rule
    /// in this module's own comment. **This client's invention**, and the reason
    /// this enum exists at all.
    Derived,
}

/// Where to put the eye to take a model's picture, in **model space** —
/// `+X forward, +Y left, +Z up`, the same frame every position in an `.m2` is
/// in. Turning that into a world matrix is the renderer's business.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Framing {
    pub eye: [f32; 3],
    pub aim: [f32; 3],
    /// **Vertical**, in radians, exactly as the file states it.
    pub fov: f32,
    pub near: f32,
    pub far: f32,
    pub source: Source,
}

/// The clip planes a derived framing takes.
///
/// The file's own, which every camera in the game agrees on: 0.22 and 27.8.
/// Using Bevy's defaults instead would clip the near half of a tauren's face.
const DERIVED_CLIP: (f32, f32) = (0.22, 27.8);

/// **The model's own portrait camera**, or `None` for one that has none.
///
/// Deliberately *not* falling back to `cameras[0]` the way `render::glue`'s
/// `SetCamera(n)` does. That fallback is right there — the interface asked for a
/// framing by index and the file's first is the one it was authored around — and
/// it is wrong here: a model carrying only a kind-1 camera would frame every
/// portrait as a full-body shot from four yards, which is a picture rather than
/// an error. [`framing`] derives instead, and says that it did.
pub fn camera(model: &M2) -> Option<&M2Camera> {
    model.cameras.iter().find(|c| c.kind == PORTRAIT_KIND)
}

/// Where to stand to take this model's picture — the file's camera if it has
/// one, and a framing off its box if it does not.
pub fn framing(model: &M2) -> Framing {
    match camera(model) {
        Some(camera) => Framing {
            eye: camera.position,
            aim: camera.target,
            fov: camera.fov,
            near: camera.near_clip,
            far: camera.far_clip,
            source: Source::Camera,
        },
        None => derive(bind_bounds(model)),
    }
}

/// **The box the model's own vertices occupy**, falling back to the declared one
/// for a file that carries no geometry.
///
/// Not the same box, and the difference is the whole quality of a derived
/// framing. `M2::bounds` is the widest the model ever gets **over all its
/// animations** — `HumanMale.m2` declares a top of 3.0 while its head is at
/// about 1.9, because somewhere in its 500 sequences it throws an arm up. A
/// framing aimed at a tenth below the declared top would look at the air above
/// a human's head. The bind pose is arms-at-side and is what a portrait shows.
pub fn bind_bounds(model: &M2) -> [[f32; 3]; 2] {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in &model.positions {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(p[axis]);
            hi[axis] = hi[axis].max(p[axis]);
        }
    }
    match lo.iter().chain(&hi).all(|v| v.is_finite()) {
        true => [lo, hi],
        false => model.bounds,
    }
}

/// Compose a framing from a declared bounding box.
///
/// The aim goes at the **top** of the box on its own centreline, and the eye
/// stands [`THREE_QUARTER`] round from straight ahead at a distance that makes a
/// [`HEAD_FRACTION`] of the model's height fill the frame vertically.
///
/// A degenerate box — a model that declares nothing, which the parser leaves as
/// zeroes — comes back as a framing one yard in front of the origin rather than
/// as a division by zero: it draws the wrong picture, and so does every other
/// answer available for a model that states no size.
pub fn derive(bounds: [[f32; 3]; 2]) -> Framing {
    let [lo, hi] = bounds;
    let height = (hi[2] - lo[2]).max(f32::EPSILON);
    let aim = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        // Not `hi[2]` itself: the declared box is the widest the model ever gets
        // over all its animations, and for anything that raises an arm or a wing
        // that is well above the head. A tenth of the height down is a
        // compromise and is the second half of what [`Source::Derived`] marks.
        hi[2] - height * 0.1,
    ];
    // The half-angle the frame subtends vertically, and the distance at which a
    // head-sized subject fills it.
    let half = (DERIVED_FOV * 0.5).tan().max(f32::EPSILON);
    let distance = (height * HEAD_FRACTION * 0.5) / half;
    // `+X` is forward, `+Y` is left, so a positive turn about up swings the eye
    // to the model's left. The sign is arbitrary and stated: both shoulders are
    // a portrait, and this is the one the files themselves mostly use — the
    // wolf's eye is at `+Y 0.72` against an aim at `-0.16`, and the murloc's at
    // `+0.49` against `+0.04`.
    let (sin, cos) = THREE_QUARTER.sin_cos();
    Framing {
        eye: [
            aim[0] + distance * cos,
            aim[1] + distance * sin,
            // Very slightly above the aim, which is what the files do: a
            // portrait looks a little down at its subject.
            aim[2] + height * 0.02,
        ],
        aim,
        fov: DERIVED_FOV,
        near: DERIVED_CLIP.0,
        far: DERIVED_CLIP.1,
        source: Source::Derived,
    }
}

/// **The `M2Camera::kind` that frames a character sheet**, against
/// [`PORTRAIT_KIND`]'s head-and-shoulders one.
///
/// `1` is the character-info camera: the full-body shot `<PlayerModel>` frames
/// want — `CharacterModelFrame`, `PetModelFrame`, `DressUpModel`, `TabardModel`
/// and `PetStableModel`, which are five of the nine `<Model>` elements
/// `Interface\FrameXML\` names and the only five that ever call `SetUnit`.
///
/// **What is measured and what is read are different claims here**.
/// Measured, by `vale portrait` over the
/// bestiary: 122 models carry a camera of this kind, they stand four to six
/// yards from what they look at where the kind-0 cameras stand a median 0.91,
/// and they aim at a median 46% of the model's bind-pose height against the
/// kind-0 camera's 74%. That is a full-body view and a head shot, and the
/// separation is not close. **Read**, and not confirmed against the client:
/// that the reference client frames a `<PlayerModel>` through this camera. What
/// corroborates the reading is arithmetic rather than a type code — see
/// [`tests::the_body_camera_frames_a_whole_human_in_the_character_panel`], which
/// puts a human male at 70% of `CharacterModelFrame`'s 233x224 rectangle.
pub const BODY_KIND: i32 = 1;

/// Where a derived full-body framing aims, as a fraction of the model's height.
///
/// **0.46, which is the census's own median** for the 122 files that carry a
/// kind-1 camera rather than a number chosen to look right. The head shot aims
/// at [`derive`]'s `hi[2] - height * 0.1`; this aims at the middle of the
/// subject, which is what a full-body framing does.
const BODY_AIM: f32 = 0.46;

/// …and how much of the model's height it fills the frame with.
///
/// Slightly more than the whole of it, so a subject is not flush against the
/// top and bottom edges of its rectangle. This is the one free number in the
/// derivation, the counterpart of [`HEAD_FRACTION`], and the one to change if
/// derived paper dolls come out too tight or too loose.
const BODY_FRACTION: f32 = 1.3;

/// **The frame shape a derived body framing sizes itself for.**
///
/// `CharacterModelFrame`'s own 233x224, which is the panel this is drawn into
/// most often and is very nearly square. It is here because of the next
/// paragraph, which is the one thing in this file that is easy to get wrong
/// twice.
///
/// **[`Framing::fov`] is a *diagonal* angle** — that is what the `M2Camera`
/// field is (see `render::glue::vertical_fov` for the address that settles it),
/// so every reader divides by `sqrt(aspect² + 1)` before handing it to a
/// projection. A derivation that sizes its distance from the raw number is
/// therefore sizing it from an angle 44% wider than the one that will actually
/// be used, and the picture comes out that much too tight. `vale portrait`
/// caught exactly that: the first draft of [`derive_body`] reported a median
/// coverage of 0.86 model heights when it intended [`BODY_FRACTION`], and 310
/// of 405 models would not have fitted in the panel.
const BODY_ASPECT: f32 = 233.0 / 224.0;

/// **The model's own character-info camera**, or `None` for one that has none.
///
/// Chosen by kind and never by index, for [`camera`]'s reason turned round: a
/// model that lists only a portrait camera would frame a whole paper doll as a
/// close-up of a face, which is a picture rather than an error.
pub fn body_camera(model: &M2) -> Option<&M2Camera> {
    model.cameras.iter().find(|c| c.kind == BODY_KIND)
}

/// Where to stand to draw this model's whole body — the file's character-info
/// camera if it has one, and a framing off its bind-pose box if it does not.
///
/// The counterpart of [`framing`], and it takes the *bind* box for that
/// function's reason: `M2::bounds` is the widest the model ever gets over all
/// its animations, so a human male declares a top of 3.0 while standing 1.9,
/// and a framing that fits the declared box puts the character two thirds of the
/// way down the panel with air above their head.
pub fn body_framing(model: &M2) -> Framing {
    match body_camera(model) {
        Some(camera) => Framing {
            eye: camera.position,
            aim: camera.target,
            fov: camera.fov,
            near: camera.near_clip,
            far: camera.far_clip,
            source: Source::Camera,
        },
        None => derive_body(bind_bounds(model)),
    }
}

/// Compose a full-body framing from a bounding box.
///
/// The aim goes at [`BODY_AIM`] of the way up the box on its own centreline and
/// the eye stands [`THREE_QUARTER`] round at a distance that fits
/// [`BODY_FRACTION`] of the model's height in the frame. **This half is this
/// client's invention** and [`Source::Derived`] marks it, exactly as [`derive`]
/// marks its own.
pub fn derive_body(bounds: [[f32; 3]; 2]) -> Framing {
    let [lo, hi] = bounds;
    let height = (hi[2] - lo[2]).max(f32::EPSILON);
    let aim = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        lo[2] + height * BODY_AIM,
    ];
    // **The vertical angle, not the stated one** — see [`BODY_ASPECT`]. The fov
    // this framing reports is a diagonal, like every camera in the game, so the
    // distance has to be sized from what a reader will make of it.
    let vertical = DERIVED_FOV / (BODY_ASPECT * BODY_ASPECT + 1.0).sqrt();
    let half = (vertical * 0.5).tan().max(f32::EPSILON);
    let distance = (height * BODY_FRACTION * 0.5) / half;
    let (sin, cos) = THREE_QUARTER.sin_cos();
    Framing {
        eye: [
            aim[0] + distance * cos,
            aim[1] + distance * sin,
            // Level with the aim rather than above it. A portrait looks a
            // little down at its subject; a paper doll is drawn straight on,
            // which is what keeps a tauren's feet in the rectangle.
            aim[2],
        ],
        aim,
        fov: DERIVED_FOV,
        near: DERIVED_CLIP.0,
        // Far enough for the distance this stands at, which is four to six
        // yards rather than the portrait camera's one — the shared 27.8 is
        // ample, and taking the max is what keeps a model whose own box is
        // enormous from being clipped in half.
        far: DERIVED_CLIP.1.max(distance * 4.0),
        source: Source::Derived,
    }
}

impl Framing {
    /// How far the eye stands from what it is looking at.
    pub fn distance(&self) -> f32 {
        let d = [
            self.eye[0] - self.aim[0],
            self.eye[1] - self.aim[1],
            self.eye[2] - self.aim[2],
        ];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    }

    /// **Whether this framing is aimed at the top of the model** — the check
    /// that says a kind-0 camera really is a portrait camera, expressed as a
    /// fraction of the declared box's height.
    ///
    /// `1.0` is the top of the box and `0.0` the bottom. A head shot lands near
    /// the top; the character-info camera lands near the middle. `None` for a
    /// model that declares no height, where the question has no answer.
    ///
    /// This is what `vale portrait` counts over the bestiary, and it is the
    /// reason the interpretation in this module's comment is a measurement
    /// rather than a hope.
    pub fn aim_height(&self, bounds: [[f32; 3]; 2]) -> Option<f32> {
        let [lo, hi] = bounds;
        let height = hi[2] - lo[2];
        (height > f32::EPSILON).then(|| (self.aim[2] - lo[2]) / height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(kind: i32, z: f32) -> M2Camera {
        M2Camera {
            kind,
            fov: 0.95,
            far_clip: 27.8,
            near_clip: 0.22,
            position: [0.5, 0.4, z],
            target: [0.0, 0.0, z],
        }
    }

    fn model(cameras: Vec<M2Camera>, bounds: [[f32; 3]; 2]) -> M2 {
        M2 {
            cameras,
            bounds,
            ..M2::default()
        }
    }

    /// The kind is what picks, and **not the index** — a model whose first
    /// camera is the character-info one must not be framed by it. That is the
    /// whole of the difference between a face and a pair of knees, and it is
    /// the case `cameras[0]` would have got wrong on every model that lists them
    /// the other way round.
    #[test]
    fn the_portrait_camera_is_chosen_by_kind_and_never_by_index() {
        let m2 = model(vec![cam(1, 0.9), cam(0, 1.9)], [[0.0; 3], [1.0, 1.0, 2.0]]);
        let framing = framing(&m2);
        assert_eq!(framing.source, Source::Camera);
        assert_eq!(framing.aim[2], 1.9);
    }

    /// …and a model carrying only the *other* camera is derived rather than
    /// framed by it. The failure this pins is silent: taking the kind-1 camera
    /// would produce a perfectly well-formed picture of the wrong thing.
    #[test]
    fn a_model_with_only_a_character_camera_is_derived() {
        let m2 = model(vec![cam(1, 0.9)], [[0.0; 3], [1.0, 1.0, 2.0]]);
        assert_eq!(framing(&m2).source, Source::Derived);
        assert!(camera(&m2).is_none());
    }

    #[test]
    fn a_model_with_no_camera_at_all_is_derived() {
        let m2 = model(Vec::new(), [[0.0; 3], [1.0, 1.0, 2.0]]);
        assert_eq!(framing(&m2).source, Source::Derived);
    }

    /// A derived framing is a head shot: it aims near the top of the box and
    /// stands close enough to fill the frame with a quarter of the height.
    #[test]
    fn a_derived_framing_aims_at_the_head_and_stands_close() {
        let bounds = [[-1.0, -1.0, 0.0], [1.0, 1.0, 2.0]];
        let framing = derive(bounds);
        let height = framing.aim_height(bounds).expect("a box with height");
        assert!(height > 0.85, "aimed at {height} of the way up, not the head");
        // …and the distance is not a taste either: it is whatever makes a
        // [`HEAD_FRACTION`] of the height exactly fill the frame, so the check
        // is that arithmetic rather than a range somebody liked the look of.
        let filled = 2.0 * framing.distance() * (framing.fov * 0.5).tan();
        let head = (bounds[1][2] - bounds[0][2]) * HEAD_FRACTION;
        assert!(
            (filled - head).abs() < 0.15,
            "the frame holds {filled} yards where a head is {head}"
        );
    }

    /// **The bind pose, not the declared box** — a model that throws an arm up
    /// in some sequence declares a box far above its own head, and a framing
    /// aimed at a tenth below *that* looks at empty air. This is the one thing
    /// that makes a derived portrait of a humanoid a face.
    #[test]
    fn a_derived_framing_is_measured_from_the_bind_pose_and_not_the_declared_box() {
        // A one-yard body standing in a box declared three yards tall.
        let m2 = M2 {
            positions: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            bounds: [[-1.0, -1.0, 0.0], [1.0, 1.0, 3.0]],
            ..M2::default()
        };
        assert_eq!(bind_bounds(&m2), [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
        assert!(
            framing(&m2).aim[2] < 1.0,
            "aimed above a body that is one yard tall"
        );
    }

    /// …and a file with no geometry at all falls back to what it declares,
    /// rather than to an infinite box.
    #[test]
    fn a_model_with_no_vertices_falls_back_to_its_declared_box() {
        let m2 = model(Vec::new(), [[-1.0, -1.0, 0.0], [1.0, 1.0, 2.0]]);
        assert_eq!(bind_bounds(&m2), [[-1.0, -1.0, 0.0], [1.0, 1.0, 2.0]]);
        assert!(framing(&m2).aim[2].is_finite());
    }

    /// **The eye is off the axis**, which is what makes it a three-quarter view
    /// rather than a passport photo — and it is off it to the model's *left*,
    /// which is the side the files themselves stand on.
    #[test]
    fn a_derived_eye_stands_off_the_models_own_axis() {
        let framing = derive([[-1.0, -1.0, 0.0], [1.0, 1.0, 2.0]]);
        assert!(framing.eye[0] > framing.aim[0], "in front of the face");
        assert!(framing.eye[1] > framing.aim[1], "and round to its left");
    }

    /// A model that declares no size at all answers a framing rather than a
    /// NaN. Every candidate answer here is wrong; the one that must not happen
    /// is a divide by zero reaching a camera transform.
    #[test]
    fn a_model_with_no_declared_size_still_answers_a_finite_framing() {
        let framing = derive([[0.0; 3], [0.0; 3]]);
        for value in framing.eye.iter().chain(&framing.aim) {
            assert!(value.is_finite(), "{value} is not a number");
        }
        assert!(framing.distance().is_finite());
    }

    /// The height fraction is a *fraction*, and a model with no height has no
    /// answer rather than an infinite one — which is what the census divides by.
    #[test]
    fn aim_height_is_none_for_a_flat_model() {
        let framing = derive([[0.0; 3], [1.0, 1.0, 0.0]]);
        assert_eq!(framing.aim_height([[0.0; 3], [1.0, 1.0, 0.0]]), None);
    }

    // --- the full-body framing, which is the other question this file answers

    /// **The two framings pick different cameras off the same model**, which is
    /// the whole of what [`BODY_KIND`] is for. A model carrying both must give
    /// a head shot to [`framing`] and a full-body shot to [`body_framing`];
    /// answering either one for both is a well-formed picture of the wrong
    /// thing, which is this file's standing failure mode.
    #[test]
    fn the_head_and_the_body_are_two_cameras_and_never_one() {
        let m2 = model(vec![cam(1, 0.9), cam(0, 1.9)], [[0.0; 3], [1.0, 1.0, 2.0]]);
        assert_eq!(framing(&m2).aim[2], 1.9, "the head shot is the kind-0 one");
        assert_eq!(body_framing(&m2).aim[2], 0.9, "and the body the kind-1");
        assert_eq!(body_framing(&m2).source, Source::Camera);
    }

    /// …and a model carrying only the *portrait* camera derives its body
    /// framing rather than borrowing it. This is [`a_model_with_only_a_character_camera_is_derived`]
    /// turned round, and it is the case that matters more: 401 of the bestiary's
    /// 408 carry a kind-0 camera and only 122 carry a kind-1, so this is the
    /// arm most creatures in the game take.
    #[test]
    fn a_model_with_only_a_portrait_camera_derives_its_body_framing() {
        let m2 = model(vec![cam(0, 1.9)], [[0.0; 3], [1.0, 1.0, 2.0]]);
        assert_eq!(body_framing(&m2).source, Source::Derived);
        assert!(body_camera(&m2).is_none());
    }

    /// **A derived body framing holds the whole model, measured the way the
    /// renderer will measure it.**
    ///
    /// The span is taken through the diagonal-to-vertical conversion at
    /// [`BODY_ASPECT`], which is the whole point of the test: the first draft
    /// sized its distance from the raw fov, every derived doll came out at 0.86
    /// of the height it intended, and a test that did the same arithmetic as
    /// the code passed. `vale portrait` caught it; this is that check as an
    /// assertion.
    #[test]
    fn a_derived_body_framing_holds_the_whole_model_at_the_panels_own_shape() {
        let bounds = [[-1.0, -1.0, 0.0], [1.0, 1.0, 2.0]];
        // Exactly what `render::paperdoll` does with a `Framing`.
        let coverage = |f: &Framing| {
            let vertical = f.fov / (BODY_ASPECT * BODY_ASPECT + 1.0).sqrt();
            2.0 * f.distance() * (vertical * 0.5).tan() / 2.0
        };
        let body = coverage(&derive_body(bounds));
        assert!(
            (body - BODY_FRACTION).abs() < 0.01,
            "a derived doll must span what it says it does: {body} against {BODY_FRACTION}"
        );
        assert!(body > 1.0, "which is more than the whole model");
        // …and the head shot deliberately does not, which is what makes these
        // two functions rather than one.
        assert!(coverage(&derive(bounds)) < 0.5);
        // …and it aims at the middle rather than the top.
        assert!((derive_body(bounds).aim_height(bounds).unwrap() - BODY_AIM).abs() < 1e-5);
    }

    /// **The reading, checked as arithmetic**: a human male framed through his
    /// own character-info camera fills about 70% of `CharacterModelFrame`'s
    /// 233x224 rectangle.
    ///
    /// That the kind-1 camera is the one `<PlayerModel>` uses is a *reading* of
    /// the field's type code and is not confirmed — see [`BODY_KIND`]. What
    /// this pins is that the reading produces a character sheet rather than a
    /// close-up or a speck: at the numbers `vale portrait` measures off
    /// `HumanMale.m2` (kind 1, fov 0.980, eye 3.659 0.034 0.923, aim -0.364
    /// 0.029 0.987) the frustum spans about 2.9 yards where the model stands
    /// about 2.0. A wrong reading is not off by 10% here; it is off by four
    /// times, which is what makes one test enough.
    #[test]
    fn the_body_camera_frames_a_whole_human_in_the_character_panel() {
        let human = M2Camera {
            kind: BODY_KIND,
            fov: 0.980,
            far_clip: 27.8,
            near_clip: 0.22,
            position: [3.659, 0.034, 0.923],
            target: [-0.364, 0.029, 0.987],
        };
        let m2 = model(vec![human], [[0.0; 3], [1.0, 1.0, 2.0]]);
        let framing = body_framing(&m2);
        // `M2Camera::fov` is a **diagonal** angle — see `render::glue`'s
        // `vertical_fov` for the address that settles it — so the vertical
        // opening at the panel's own 233:224 has to be taken the same way the
        // portrait path takes it at 4:3.
        let aspect: f32 = 233.0 / 224.0;
        let vertical = framing.fov / (aspect * aspect + 1.0).sqrt();
        let span = 2.0 * framing.distance() * (vertical * 0.5).tan();
        assert!(
            (2.5..3.4).contains(&span),
            "a human male's panel spans {span} yards; he is about 2.0 tall"
        );
    }
}
