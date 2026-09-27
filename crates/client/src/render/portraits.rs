//! **The faces on the unit frames** — `SetPortraitTexture`'s other half.
//!
//! ```text
//! lua::portrait    SetPortraitTexture(TargetPortrait, "target")   a token
//!   -> Units       …which unit that is, this frame
//!   -> assets::dress + ModelCache      …dressed exactly as it is in the world
//!   -> one camera, one model, one render layer, one 128x96 image
//!   -> EguiUserTextures                …and a texture id ui::framexml can draw
//! ```
//!
//! ## This is a render target per portrait, which the glue pass said it was not
//!
//! `render::glue`'s module note has carried the sentence for a dozen rounds:
//! *"a `<Model>` with a rectangle of its own — the fourteen portraits in
//! `Interface\FrameXML\` — would need a render target per frame, which this does
//! not do."* This is that, for the population that actually matters, which turns
//! out not to be the `<Model>` elements at all: **a unit frame's portrait is a
//! plain `<Texture>`**, 64 by 64, filled by a C function rather than by markup.
//! The fourteen `<Model>`s are the dress-up frame and the tabard designer, and
//! they are still not drawn.
//!
//! ## Where the camera stands is the file's and nothing here decides it
//!
//! [`vale_assets::look::portrait`] holds the rule and `vale portrait` is the
//! census: 401 of the 408 creature models that decode carry a camera of kind 0,
//! sitting a median 0.91 yards from what it looks at. This pass converts that
//! into Bevy's axes and points a camera down it. The one number it adds is the
//! **anamorphic squeeze**, which is not this client's invention either:
//! `render::lens::vertical_fov` already records that `M2Camera::fov` is a
//! *diagonal* angle and that the portrait path renders at a fixed 4:3 into a
//! square. So the image is 4:3, egui draws it into a square region, and the
//! vertical field of view is `fov / sqrt((4/3)² + 1)` — the familiar `0.6 · fov`.
//!
//! ## Three deviations, all stated
//!
//! * **The portrait is a still, taken over a settle window rather than baked in
//!   one call.** `SetPortraitTexture` bakes one image and leaves it; this
//!   points a camera at a model, lets it render for [`SETTLE_FRAMES`] while the
//!   meshes allocate and the skin uploads, and then [`shutter`] switches the
//!   camera off — the image persists and egui keeps sampling it. It used to
//!   stay live, and that was the largest single per-player cost in the client:
//!   ~1.5 ms of `Core3d` CPU encode per portrait per frame (Tracy, 2026-08),
//!   paid for the player's own frame every session and again for the target
//!   and every party member. The *picture* is identical either way, because
//!   the pose is never advanced: see [`Portrait::posed`].
//! * **It is lit by its own studio rig rather than by the world.** A portrait
//!   taken under `Light.dbc`'s answer for midnight is a black square, which the
//!   reference's plainly is not. [`STUDIO`] is a fixed ambient and one key lamp,
//!   handed to the material cache through the same [`SceneLighting`] the glue
//!   screens use — so it costs no shader, no light entity and no render-layer
//!   interaction, and it interns the portrait's materials apart from the
//!   world's copy of the same model, which is what keeps the two from fighting.
//! * **It is drawn in the sequence's first frame, not the unit's current one.**
//!   A portrait of a running wolf shows a standing wolf. That is the reference's
//!   behaviour too — its portrait is baked once, long before the wolf ran — but
//!   it is stated because the machinery here *could* animate and deliberately
//!   does not.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::mesh::skinning::SkinnedMesh;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureUsages,
};

use vale_assets::world::m2::M2Skeleton;

use crate::render::axes;
use crate::render::models::material::Materials;
use crate::render::models::{Lookup, ModelAssets, ModelCache, SceneLighting};
use crate::world::entities::{DisplayCache, Joint};

/// The size the picture is taken at: **4:3, for the squeeze**, and big enough
/// that the 64-unit square it lands in is not resampled up.
///
/// 128 wide at a game scale of one screen unit to about 1.4 pixels on a 1080p
/// window is a little over the 90 pixels the square gets, so the sampler is
/// always downscaling — which is the side of 1:1 to be on.
const SIZE: UVec2 = UVec2::new(128, 96);

/// …and the aspect that field of view is taken at, which is the *reason* for
/// the size above rather than a consequence of it. See the module note.
const ASPECT: f32 = 4.0 / 3.0;

/// **How many pictures may be taken at once.**
///
/// Nine: the player, the target, target-of-target, four party members, the pet
/// and one spare. Each costs a render layer, and the cap exists because the
/// token vocabulary is the interface's rather than this client's — an addon
/// asking for a portrait of `"party8"` would otherwise be a new camera every
/// time it did.
pub(super) const MAX: usize = 9;

/// The first render layer these use. Layer 0 is the world's and everything in
/// this client that does not say otherwise is on it.
pub(super) const FIRST_LAYER: usize = 1;

/// [`crate::ui::report::HudReport`] slot. 32, immediately under the interface's
/// own line at 31: a portrait is a picture the interface asked for, and the
/// number worth reading beside "N quads" is how many cameras that cost.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(32);

/// **The light a portrait is taken under**, and the one number in this file
/// that is neither the game's nor derived from it.
///
/// Shared with [`super::paperdoll`], which is not a convenience: a face on the
/// unit frame and the same character's body on the character sheet are two
/// pictures of one person on one screen, and lighting them differently is
/// visible where the numbers themselves are not.
///
/// A three-quarter fill and one key lamp from the camera's own side, in the
/// *model's* frame — so it follows the subject rather than the world. It is a
/// deviation and the module note says so; what makes it the right one is that
/// the alternative is a face that goes black at dusk and stays black all night,
/// which no reference screenshot of this game has ever shown.
///
/// Written as a literal rather than through [`SceneLighting::resolve`] because
/// there is no `M2Light` to resolve: this is a rig, not a file. The layout is
/// that function's own — `(place.xyz, is_point)` then `(colour.rgb, 0)`, with
/// `w` on the ambient holding the lamp count.
pub(super) const STUDIO: SceneLighting = SceneLighting {
    ambient: Vec4::new(0.42, 0.42, 0.46, 1.0),
    lamps: [
        // Directional (`w = 0`), pointing *toward* the light — up, forward and
        // to the model's left, which is the side the portrait cameras stand on.
        Vec4::new(0.53, 0.53, 0.66, 0.0),
        Vec4::new(0.85, 0.83, 0.78, 0.0),
        Vec4::ZERO,
        Vec4::ZERO,
        Vec4::ZERO,
        Vec4::ZERO,
        Vec4::ZERO,
        Vec4::ZERO,
    ],
};

/// One unit's picture, and everything it took to take it.
struct Portrait {
    /// **What this was built from** — the change detector. A new guid, a
    /// shapeshift, a helmet: any of them and the picture is rebuilt.
    built: Built,
    /// The image the camera draws into, and the id egui knows it by.
    image: Handle<Image>,
    texture: bevy_egui::egui::TextureId,
    camera: Entity,
    /// The model root; despawning it takes the batches and joints with it.
    root: Entity,
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    /// Which render layer this pair is alone on.
    layer: usize,
    /// **Whether the joints have been written yet.** The pose is a still — see
    /// the module note — so it is written once and never advanced, and this is
    /// what says "once".
    posed: bool,
    /// **How many more frames the camera renders before it is switched off.**
    ///
    /// The picture is a still: the pose is written once, the model never moves
    /// and the framing never changes, so re-rendering it every frame bought
    /// nothing and cost what a whole extra `Core3d` camera costs — measured
    /// with Tracy at ~1.5 ms of CPU encode *per portrait per frame*, which with
    /// a player, a target and a party is most of a frame budget on its own and
    /// was the largest single per-player cost in the client. The camera runs
    /// for [`SETTLE_FRAMES`] after the pose lands (meshes allocate and textures
    /// upload over the first few frames) and then [`shutter`] deactivates it;
    /// the rendered image persists and egui goes on sampling it. A retake is a
    /// new camera, so nothing here ever needs waking back up.
    settle: u8,
}

/// What a picture was taken of, compared to decide whether to take it again.
///
/// The display id **and** the look, because a player changing gear keeps the
/// same guid and the same model and is a different picture — and because
/// `dress` is what decides that, this is the same pair the world's own model
/// rebuild keys on.
#[derive(Clone, PartialEq)]
struct Built {
    guid: u64,
    display_id: u32,
    appearance: Option<vale_assets::look::character::Appearance>,
    equipment: Vec<(u32, u32)>,
}

/// Every picture the interface has asked for, and the tokens it asked by.
#[derive(Resource, Default)]
pub struct Portraits {
    /// **Tokens the interface has ever named**, accumulated rather than
    /// drained.
    ///
    /// The set is the game's own unit vocabulary and never grows past a
    /// handful; what a drained set would cost is a picture that flickers out
    /// every frame the interface happens not to re-ask. Capped at [`MAX`].
    wanted: BTreeSet<String>,
    /// …and the pictures themselves, one per token that resolves to somebody.
    taken: BTreeMap<String, Portrait>,
}

impl Portraits {
    /// The texture id for a token, or `None` while there is no picture — which
    /// is the frame a target is acquired on and any frame its model is still
    /// loading. The painter falls back to whatever the `<Texture>` already had.
    pub fn texture(&self, token: &str) -> Option<bevy_egui::egui::TextureId> {
        Some(self.taken.get(token)?.texture)
    }

    /// The same picture as the image the camera drew it into — the mesh
    /// painter's door, which binds textures rather than egui ids. `Some` and
    /// [`Self::texture`]'s `Some` coincide by construction: both fields are
    /// written together when a picture lands.
    pub fn image(&self, token: &str) -> Option<Handle<Image>> {
        Some(self.taken.get(token)?.image.clone())
    }

    /// How many pictures are being kept, for the HUD line.
    pub fn count(&self) -> usize {
        self.taken.len()
    }

    /// The lowest render layer nobody is using.
    fn free_layer(&self) -> Option<usize> {
        (FIRST_LAYER..FIRST_LAYER + MAX)
            .find(|layer| self.taken.values().all(|p| p.layer != *layer))
    }
}

pub struct PortraitPlugin;

impl Plugin for PortraitPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report);
        app.init_resource::<Portraits>().add_systems(
            Update,
            (collect, follow, pose, shutter)
                .chain()
                // **After the entity pass**, which is what puts a `WorldEntity`
                // where [`follow`] can see it and what the dressing rule is
                // shared with. Not a data dependency Bevy can see — the two
                // touch different components — so it is written rather than
                // inherited, per the convention.
                .after(crate::world::entities::EntitySet),
        );
    }
}

/// **What the faces cost**, on the pass's own HUD line — see
/// [`crate::ui::report`], which is why this is not a line in `hud.rs`.
///
/// Two numbers and they answer different questions. *Asked* is how many tokens
/// the interface has ever named, which is a property of the interface and
/// should sit at three in an ordinary session and nine at a raid frame's worth;
/// *taken* is how many of those resolve to somebody right now, which is how
/// many extra cameras and models the frame is paying for. The two being far
/// apart is the healthy state — an empty party asks for four and takes none.
#[cfg(feature = "diagnostics")]
fn report(portraits: Res<Portraits>, mut hud: ResMut<crate::ui::report::HudReport>) {
    if portraits.wanted.is_empty() {
        hud.clear(SLOT, "portraits");
        return;
    }
    hud.set(
            crate::ui::report::Section::Interface,
        SLOT,
        "portraits",
        format!(
            "portraits: {} asked, {} taken",
            portraits.wanted.len(),
            portraits.taken.len()
        ),
    );
}

/// How many token *names* may accumulate — a bound against a runaway addon
/// inventing tokens, not against cameras. The camera cap is [`MAX`] and it is
/// enforced where the layer is allocated, in [`follow`].
///
/// **These are different bounds and folding them was a bug**: the interface
/// names `pet` and `party1..4` unconditionally at load, none of which this
/// client's vocabulary can resolve — so nine unresolvable names filled the old
/// cap and `"npc"`, asked for the first time you talk to somebody, was refused
/// for the rest of the session. Every conversation portrait was white in any
/// session that had held a party.
const WANTED_CAP: usize = 32;

/// Drain what the interface asked for into the standing set.
fn collect(host: Option<NonSendMut<crate::lua::host::LuaHost>>, mut portraits: ResMut<Portraits>) {
    let Some(mut host) = host else { return };
    for token in host.take_portrait_requests() {
        let token = token.to_ascii_lowercase();
        if portraits.wanted.len() >= WANTED_CAP && !portraits.wanted.contains(&token) {
            continue;
        }
        portraits.wanted.insert(token);
    }
}

/// Build, rebuild and tear down one model per wanted token.
#[allow(clippy::too_many_arguments)]
fn follow(
    mut commands: Commands,
    mut portraits: ResMut<Portraits>,
    units: crate::interface::api::Units,
    mut cache: ResMut<ModelCache>,
    mut displays: ResMut<DisplayCache>,
    assets: Res<crate::assets::GameAssets>,
    mut materials: Materials,
    // Only so the model cache can build a dressing's merged meshes — see
    // `models::loader::MergeSource`.
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut egui: ResMut<bevy_egui::EguiUserTextures>,
) {
    let tokens: Vec<String> = portraits.wanted.iter().cloned().collect();
    for token in tokens {
        // **Compared before it is built**, which is the whole of what this
        // system does on all but a handful of frames. `Built` carries the
        // wearer's equipment list, so constructing one is a `Vec` allocation —
        // per token, per frame, nine times over, to throw all nine away. See
        // [`unchanged`], which answers the same question by reading.
        if unchanged(&units, &portraits, &token) {
            continue;
        }
        let wanted = subject(&units, &token);
        // **A token naming nobody takes its picture down**, which is what an
        // empty party slot and a dropped target both are. Keeping the last face
        // is the failure this is about: it is indistinguishable from a working
        // portrait of the wrong unit.
        // **A token naming nobody comes down at once**, and so does one naming
        // a *different* somebody — keeping the last face is indistinguishable
        // from a working portrait of the wrong unit. But the **same somebody
        // whose look changed keeps the old picture up until the new one is
        // ready**: a player's equipment arrives piecemeal behind
        // `CMSG_ITEM_QUERY_SINGLE`, and tearing down per arriving template
        // blanked the frame once per item for as long as the wardrobe took.
        let Some(built) = wanted else {
            take_down(&mut commands, &mut portraits, &mut egui, &token);
            continue;
        };
        if portraits
            .taken
            .get(&token)
            .is_some_and(|p| p.built.guid != built.guid)
        {
            take_down(&mut commands, &mut portraits, &mut egui, &token);
        }

        let Some(display) =
            displays.resolve(&assets, unit_kind(&units, &token), built.display_id)
        else {
            continue;
        };
        // A transport is a `.wmo` and has no portrait, exactly as it has no
        // model — see `world::entities::spawn_models`, which declines the same
        // paths for the same reason.
        if display.path.to_ascii_lowercase().ends_with(".wmo") {
            continue;
        }
        let Some(tables) = displays.tables() else {
            continue;
        };
        // **The same dressing rule the world uses**, called through the same
        // door — a second opinion here is what this project avoids, and it would
        // put a face in the portrait wearing different gear from the body three
        // yards away on the screen.
        //
        // **Weapons are deliberately not carried.** A portrait is a head and
        // shoulders; hanging a sword on it is a load of a second model per
        // frame for something outside the frustum. `Wearer::weapons` is empty
        // and the sheath state is the put-away one, which is the shape
        // `dress` reads as "no weapons at all".
        let dressed = vale_assets::look::dress::dress(
            tables,
            &display,
            &vale_assets::look::dress::Wearer {
                appearance: built.appearance,
                equipment: &built.equipment,
                weapons: Default::default(),
                sheath_state: vale_assets::look::sheath::UNARMED,
            },
        );
        let ready = match &dressed.look {
            Some(look) => cache.dressed_as_character(
                &display.path,
                look,
                dressed.dress,
                dressed.cloak.as_deref(),
                None,
                STUDIO,
                &mut meshes,
                &mut materials,
            ),
            None => cache.dressed(
                &display.path,
                &display.skins,
                dressed.hair.as_ref(),
                dressed.dress,
                None,
                &mut meshes,
                &mut materials,
            ),
        };
        let Lookup::Ready(model) = ready else {
            // Loading or unreadable. Nothing is recorded either way, so the
            // next frame asks again — `ModelCache` is idempotent about it, and
            // a `Failed` path costs one map lookup a frame for as long as the
            // unit is targeted. A stale same-guid picture stays up meanwhile —
            // see the teardown note above.
            continue;
        };
        // The replacement is ready: now the old picture comes down, freeing its
        // layer for the rebuild below.
        take_down(&mut commands, &mut portraits, &mut egui, &token);
        let Some(layer) = portraits.free_layer() else {
            continue;
        };
        let portrait = build(
            &mut commands,
            &mut images,
            &mut egui,
            &model,
            built,
            layer,
        );
        portraits.taken.insert(token.clone(), portrait);
    }
}

/// What a token's picture should be of, or `None` for one naming nobody.
/// **Whether the picture already up is the right one**, without building a
/// [`Built`] to find out.
///
/// The same comparison [`subject`] would produce, read field by field off the
/// live unit — because the answer is *yes* on almost every frame for almost
/// every token, and the version that built one first allocated a `Vec` per token
/// per frame for nothing. Nine tokens at sixty frames a second is 540 wasted
/// allocations a second in a client whose own collector is one of its larger
/// per-frame costs.
///
/// `false` when the token names nobody or has no picture: both mean the caller
/// has work to do and must go and find out which.
fn unchanged(
    units: &crate::interface::api::Units,
    portraits: &Portraits,
    token: &str,
) -> bool {
    let Some(taken) = portraits.taken.get(token) else {
        return false;
    };
    let Some(unit) = crate::interface::api::UnitId::parse(token).and_then(|id| units.get(id)) else {
        return false;
    };
    taken.built.guid == unit.guid
        && Some(taken.built.display_id) == unit.display_id
        && taken.built.appearance == unit.appearance
        && taken.built.equipment == unit.equipment
}

fn subject(units: &crate::interface::api::Units, token: &str) -> Option<Built> {
    let id = crate::interface::api::UnitId::parse(token)?;
    let unit = units.get(id)?;
    Some(Built {
        guid: unit.guid,
        display_id: unit.display_id?,
        appearance: unit.appearance,
        equipment: unit.equipment.clone(),
    })
}

/// …and which display table to resolve it through.
fn unit_kind(
    units: &crate::interface::api::Units,
    token: &str,
) -> vale_protocol::state::update::ObjectType {
    crate::interface::api::UnitId::parse(token)
        .and_then(|id| units.get(id))
        .map(|unit| unit.kind)
        .unwrap_or(vale_protocol::state::update::ObjectType::Unit)
}

/// Spawn the image, the camera and the model for one portrait.
/// **No emitters and no trails**, unlike `render::glue::spawn`, which is the
/// other function of this shape. A portrait is a head and shoulders held for as
/// long as a unit is targeted; a wisp's dust and a weapon's streak are a
/// simulation running every frame for something 90 pixels across, and neither is
/// in shot at this framing anyway. Stated rather than merely omitted, because
/// the omission is invisible next to the pass it was copied from.
fn build(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    egui: &mut bevy_egui::EguiUserTextures,
    model: &ModelAssets,
    built: Built,
    layer: usize,
) -> Portrait {
    let image = images.add(target_image());
    let texture = egui.add_image(bevy_egui::EguiTextureHandle::Strong(image.clone()));
    let layers = RenderLayers::layer(layer);

    // --- the model, alone on its layer at the origin ---

    let root = commands
        .spawn((Transform::default(), Visibility::default()))
        .id();
    let mut joints = Vec::new();
    if model.skeleton.is_some() {
        joints.extend((0..model.joint_count).map(|_| {
            commands
                .spawn((GlobalTransform::default(), Joint, ChildOf(root)))
                .id()
        }));
    }
    for draw in &model.draws {
        let mut part = commands.spawn((
            Mesh3d(draw.mesh.clone()),
            MeshMaterial3d(draw.material.clone()),
            Transform::default(),
            ChildOf(root),
            // **On every drawn entity**, because Bevy reads `RenderLayers` per
            // entity and does not propagate it down a hierarchy. A part left
            // off the layer is a part the world camera draws — a floating head
            // at the origin of the map, which is exactly the artefact this
            // costs one component to avoid.
            layers.clone(),
            // The portrait camera sits inside the model's own bounding box, so
            // the box is useless as a culling volume here and actively harmful:
            // a subject whose declared box does not contain the camera can be
            // culled out of its own portrait.
            NoFrustumCulling,
        ));
        // **The batch's own bones, not the model's whole skeleton** — see
        // `models::skin_for`, which is the one place that decides it. This was
        // the *fourth* spawner and the one the subset round missed: the three
        // it converted are named in that function's own doc, this one is not,
        // and a portrait went on binding the whole skeleton to meshes whose
        // joint indices had become subset-local. Every vertex posed off the
        // wrong bone — which is a face stretched across the frame, and which no
        // headless check looks at.
        if let Some(joints) = crate::render::models::skin_for(draw, &joints) {
            part.insert(SkinnedMesh {
                inverse_bindposes: model.inverse_bindposes.clone(),
                joints,
            });
        }
    }

    // --- and the camera, down the file's own axis ---

    let framing = model.portrait;
    let eye = axes::to_bevy(framing.eye);
    // **A degenerate framing is nudged rather than aimed at itself.** An eye and
    // an aim in the same place make `looking_at` produce a NaN basis, and a NaN
    // view matrix takes the whole frame's culling with it — every mesh in the
    // world, not only this one. The rule's own guards make it unreachable and
    // the check costs one comparison; `render::glue::aim_camera` makes the same
    // one and returns instead, which is not available here because the camera
    // has to exist.
    let aim = match axes::to_bevy(framing.aim) {
        target if eye.distance_squared(target) < 1e-6 => eye - Vec3::Z,
        target => target,
    };
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                // **Before the world's**, which is order 0 — a camera drawing
                // into an image that something later in the frame samples has
                // to have finished.
                order: -1 - layer as isize,
                // **Transparent**, which is the whole reason the portrait is
                // its own layer: the unit frame's art is drawn *under* this
                // square and shows through wherever the model is not.
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            // The target is a component of its own in Bevy 0.19, exactly as
            // `world::camera::spawn` writes `WorldFrame::target()`.
            RenderTarget::Image(ImageRenderTarget {
                handle: image.clone(),
                scale_factor: 1.0,
            }),
            // The same declaration the world camera makes, and for the same
            // reason: every model shader in this client ends in
            // `atmosphere::to_frame` and writes the game's own bytes, so a
            // camera left in the default space would encode them a second time
            // and hand the interface a washed-out face. See
            // `crate::render::present`.
            bevy::camera::CompositingSpace::Srgb,
            Projection::Perspective(PerspectiveProjection {
                fov: crate::render::lens::vertical_fov(framing.fov, ASPECT),
                near: framing.near.max(0.01),
                far: framing.far.max(framing.near.max(0.01) * 10.0),
                aspect_ratio: ASPECT,
                ..default()
            }),
            // Nothing here is edge-sampled and nothing is graded: this is a
            // 128x96 square of one model. Both are named rather than defaulted
            // because `Camera3d`'s defaults are multisampled and tonemapped.
            Msaa::Off,
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            layers.clone(),
            Transform::from_translation(eye).looking_at(aim, Vec3::Y),
            Name::new("portrait"),
        ))
        .id();

    Portrait {
        built,
        image,
        texture,
        camera,
        root,
        joints,
        skeleton: model.skeleton.clone(),
        layer,
        posed: false,
        settle: SETTLE_FRAMES,
    }
}

/// How many frames a portrait camera keeps rendering once its pose is written.
///
/// The content is final the moment the joints land, but the render world takes
/// a few frames to allocate the meshes and upload the composed skin, and the
/// camera has to be live for the frame that draws the finished picture into
/// the target. Half a second at 60 fps is far past any of that and still
/// removes ~97% of the steady-state cost; the number is margin, not tuning.
const SETTLE_FRAMES: u8 = 30;

/// Switch a settled portrait's camera off.
///
/// The picture is a still (see [`pose`]) drawn into a persistent image, so
/// once it has been rendered there is nothing left for the camera to do — and
/// an active `Camera3d` is a whole `Core3d` graph run per frame, ~1.5 ms of
/// CPU encode on the measured machine, per portrait. The countdown starts when
/// the content is final — the pose written, or the model rigid — and the
/// camera comes off at zero. It never goes back on: a retake ([`follow`]) is a
/// new camera. The one thing this freezes that the live camera did not is a
/// UV-animated material on the subject, which a baked-once portrait freezes in
/// the reference too.
fn shutter(mut portraits: ResMut<Portraits>, mut cameras: Query<&mut Camera>) {
    for portrait in portraits.taken.values_mut() {
        // Content not final yet: the pose system is still waiting for the
        // joint entities to exist. Keep rendering.
        if !(portrait.posed || portrait.joints.is_empty()) {
            continue;
        }
        if portrait.settle > 0 {
            portrait.settle -= 1;
            continue;
        }
        if let Ok(mut camera) = cameras.get_mut(portrait.camera) {
            if camera.is_active {
                camera.is_active = false;
            }
        }
    }
}

/// Write the joints, once.
///
/// **`Stand` at time zero and never advanced** — see the module note on why a
/// live camera still produces a still. The system runs every frame and does
/// nothing on all but the first, which is cheaper than the alternative shapes
/// (a one-shot schedule, or a marker component removed after the first pass)
/// and is one `bool`.
fn pose(mut portraits: ResMut<Portraits>, mut joints: Query<&mut GlobalTransform, With<Joint>>) {
    for portrait in portraits.taken.values_mut() {
        if portrait.posed || portrait.joints.is_empty() {
            continue;
        }
        let Some(skeleton) = &portrait.skeleton else {
            portrait.posed = true;
            continue;
        };
        let sequence = skeleton
            .best_sequence(&[vale_assets::world::m2::anim::STAND])
            .unwrap_or(0);
        let pose = skeleton.pose(sequence, 0, 0, None, Default::default());
        for (bone, &joint) in pose.iter().zip(portrait.joints.iter()) {
            if let Ok(mut transform) = joints.get_mut(joint) {
                *transform = GlobalTransform::from(Affine3A::from_mat4(axes::pose_to_bevy(bone)));
            }
        }
        // The identity joint on the end, which weightless vertices ride —
        // without it the skinning shader collapses them to the origin. The same
        // arrangement `world::entities` and `render::glue` both spawn.
        if let Some(&last) = portrait.joints.last() {
            if let Ok(mut transform) = joints.get_mut(last) {
                *transform = GlobalTransform::default();
            }
        }
        portrait.posed = true;
    }
}

/// Take one picture down: the model, the camera, the image and egui's handle
/// on it.
fn take_down(
    commands: &mut Commands,
    portraits: &mut Portraits,
    egui: &mut bevy_egui::EguiUserTextures,
    token: &str,
) {
    let Some(portrait) = portraits.taken.remove(token) else {
        return;
    };
    commands.entity(portrait.root).despawn();
    commands.entity(portrait.camera).despawn();
    // **Egui's handle goes with it**, or the id stays registered against an
    // image asset nobody holds: a texture leak of one 48 KB image per target
    // change, which over a session of pulling mobs is the whole of a memory
    // report nobody could explain.
    egui.remove_image(&portrait.image);
}

/// The image a portrait is drawn into.
///
/// **`Rgba8UnormSrgb`, for the reason `render::present::WorldFrame` is**: the
/// camera's main texture is raw bytes (`CompositingSpace::Srgb`), Bevy's
/// upscaling blit applies `SRGB_TO_LINEAR` on the way out because it assumes the
/// destination re-encodes, and this is the destination. Getting it backwards
/// stores the linear value raw and the face comes out at dusk in the middle of
/// the afternoon — which is the artefact that note describes, one texture over.
fn target_image() -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: SIZE.x,
            height: SIZE.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        // Transparent black, so a portrait whose camera has not run yet shows
        // the frame's own art rather than a coloured square.
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        // The render world only: nothing on the CPU reads this, and egui
        // samples it on the GPU like any other texture.
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The squeeze, stated as arithmetic.** `M2Camera::fov` is a diagonal
    /// angle at 4:3 — see `render::lens::vertical_fov` for what
    /// settles it — so a human male's authored 0.785 has to come out at 0.6 of
    /// itself vertically. Taken as vertical instead, every face in the
    /// interface is framed 67% too wide and the head fills a third of the
    /// square.
    #[test]
    fn the_field_of_view_is_the_diagonal_squeezed_to_four_three() {
        let vertical = |diagonal: f32| diagonal / (ASPECT * ASPECT + 1.0).sqrt();
        assert!((vertical(1.0) - 0.6).abs() < 1e-6);
        // The human male's own camera, and the wolf's.
        assert!((vertical(0.785) - 0.471).abs() < 1e-3);
        assert!((vertical(0.950) - 0.570).abs() < 1e-3);
    }

    /// **Every portrait is alone on its own layer**, which is the whole of how
    /// one model is drawn without the world behind it. A collision here is two
    /// units in one square.
    #[test]
    fn layers_are_handed_out_one_apiece_and_run_out_rather_than_repeating() {
        let mut portraits = Portraits::default();
        let mut used = Vec::new();
        for i in 0..MAX {
            let layer = portraits.free_layer().expect("a free layer");
            assert!(!used.contains(&layer), "layer {layer} handed out twice");
            used.push(layer);
            portraits.taken.insert(
                format!("unit{i}"),
                Portrait {
                    built: Built {
                        guid: i as u64,
                        display_id: 0,
                        appearance: None,
                        equipment: Vec::new(),
                    },
                    image: Handle::default(),
                    texture: bevy_egui::egui::TextureId::Managed(0),
                    camera: Entity::from_raw_u32(1).expect("an entity id"),
                    root: Entity::from_raw_u32(2).expect("an entity id"),
                    joints: Vec::new(),
                    skeleton: None,
                    layer,
                    posed: true,
                    settle: 0,
                },
            );
        }
        assert_eq!(portraits.free_layer(), None, "and they run out");
        assert!(!used.contains(&0), "layer 0 is the world's");
    }

    /// The camera orders are distinct and all **before** the world's, which is
    /// order 0. Two cameras sharing an order is a target sampled before it has
    /// been drawn into, which reads as a portrait one frame stale — or, on the
    /// first frame, as an empty square.
    #[test]
    fn every_portrait_camera_draws_before_the_world() {
        let orders: Vec<isize> = (FIRST_LAYER..FIRST_LAYER + MAX)
            .map(|layer| -1 - layer as isize)
            .collect();
        assert!(orders.iter().all(|o| *o < 0));
        let mut sorted = orders.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), orders.len(), "no two share an order");
    }

    /// The studio rig is a *lit* scene as far as the material cache is
    /// concerned, and it keys apart from the world's build of the same model.
    /// Without both of those a portrait would either draw unlit or would hand
    /// the world its studio lighting.
    #[test]
    fn the_studio_rig_is_lit_and_keys_apart_from_the_world() {
        assert!(STUDIO.is_lit());
        assert!(!SceneLighting::NONE.is_lit());
        assert_ne!(STUDIO.key(), SceneLighting::NONE.key());
    }

    /// **A settled portrait's camera goes off, and an unposed one's stays on.**
    /// The first half is the whole optimization — a still re-rendered forever
    /// was ~1.5 ms of encode per portrait per frame — and the second half is
    /// what keeps it from being a regression: deactivating before the pose has
    /// landed bakes a bind-pose face, or an empty square, into the frame.
    #[test]
    fn the_shutter_closes_after_settling_and_never_early() {
        let mut app = App::new();
        app.init_resource::<Portraits>()
            .add_systems(Update, shutter);
        let settled = app.world_mut().spawn(Camera::default()).id();
        let waiting = app.world_mut().spawn(Camera::default()).id();
        let portrait = |camera, posed, joints: Vec<Entity>, settle| Portrait {
            built: Built {
                guid: 0,
                display_id: 0,
                appearance: None,
                equipment: Vec::new(),
            },
            image: Handle::default(),
            texture: bevy_egui::egui::TextureId::Managed(0),
            camera,
            root: Entity::from_raw_u32(9).expect("an entity id"),
            joints,
            skeleton: None,
            layer: FIRST_LAYER,
            posed,
            settle,
        };
        {
            let mut portraits = app.world_mut().resource_mut::<Portraits>();
            portraits
                .taken
                .insert("posed".into(), portrait(settled, true, Vec::new(), 2));
            // Skinned and not yet posed: the joints exist, the pose has not
            // landed, so the countdown must not even start.
            portraits.taken.insert(
                "waiting".into(),
                portrait(waiting, false, vec![settled], 2),
            );
        }
        // Two settle frames pass, then the third run closes the shutter.
        for _ in 0..3 {
            app.update();
        }
        assert!(
            !app.world().entity(settled).get::<Camera>().unwrap().is_active,
            "a settled portrait must stop rendering"
        );
        assert!(
            app.world().entity(waiting).get::<Camera>().unwrap().is_active,
            "an unposed portrait must keep its camera live"
        );
        // …and the un-posed one still holds its full settle budget.
        let portraits = app.world().resource::<Portraits>();
        assert_eq!(portraits.taken.get("waiting").unwrap().settle, 2);
    }
}
