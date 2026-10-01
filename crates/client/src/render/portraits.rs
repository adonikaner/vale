//! The portraits on the unit frames: the rendering behind `SetPortraitTexture`.
//!
//! ```text
//! lua::portrait    SetPortraitTexture(TargetPortrait, "target")   a unit token
//!   -> Units       the unit the token names this frame
//!   -> assets::dress + ModelCache      the model, dressed as it is in the world
//!   -> one camera, one model, one render layer, one 128x96 image
//!   -> EguiUserTextures                a texture id ui::framexml can draw
//! ```
//!
//! ## Each portrait has its own render target
//!
//! `render::glue`'s module note says that "a `<Model>` with a rectangle of its
//! own — the fourteen portraits in `Interface\FrameXML\` — would need a render
//! target per frame, which this does not do." This module provides a render
//! target per portrait for the unit frames. A unit frame's portrait is not a
//! `<Model>` element: it is a plain 64 by 64 `<Texture>`, filled by a C
//! function rather than by markup. The fourteen `<Model>`s are the dress-up
//! frame and the tabard designer, and they are still not drawn.
//!
//! ## The model file decides where the camera stands
//!
//! [`vale_assets::look::portrait`] holds the rule, and `vale portrait` reports
//! the census: 401 of the 408 creature models that decode carry a camera of
//! kind 0, a median 0.91 yards from the point it looks at. This pass converts
//! that camera into Bevy's axes and points a camera along it. The one number it
//! adds is the anamorphic squeeze, which `render::lens::vertical_fov` already
//! records: `M2Camera::fov` is a diagonal angle, and the portrait is rendered
//! at a fixed 4:3 into a square. So the image is 4:3, egui draws it into a
//! square region, and the vertical field of view is `fov / sqrt((4/3)² + 1)`,
//! which is `0.6 · fov`.
//!
//! ## Three differences from the 1.12.1 client
//!
//! * The portrait is a still image rendered over a settle window, not in one
//!   call. `SetPortraitTexture` in the 1.12.1 client renders one image and
//!   keeps it. This module points a camera at a model, renders for
//!   [`SETTLE_FRAMES`] while the meshes allocate and the skin uploads, and then
//!   [`shutter`] switches the camera off; the image persists and egui keeps
//!   sampling it. The camera used to stay active, and that was the largest
//!   single per-player cost in the client: about 1.5 ms of `Core3d` CPU encode
//!   per portrait per frame (Tracy, 2026-08), paid for the player's own frame
//!   in every session and again for the target and every party member. The
//!   image is the same either way, because the pose is never advanced: see
//!   [`Portrait::posed`].
//! * It is lit by a fixed studio rig, not by the world. A portrait lit by
//!   `Light.dbc`'s values for midnight is a black square, and the 1.12.1
//!   client's portraits are not. [`STUDIO`] is a fixed ambient and one key
//!   lamp, passed to the material cache through the same [`SceneLighting`] the
//!   glue screens use. It needs no shader, no light entity and no render-layer
//!   interaction, and it keys the portrait's materials apart from the world's
//!   copy of the same model, so the two do not overwrite each other.
//! * It is drawn in the first frame of the stand sequence, not the unit's
//!   current frame. A portrait of a running wolf shows a standing wolf. The
//!   1.12.1 client does the same, because it renders its portrait once. This
//!   is stated because this module could animate the portrait and does not.

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

/// The size the image is rendered at: 4:3, for the squeeze, and large enough
/// that the 64-unit square it is drawn into is never upscaled.
///
/// At a game scale of one screen unit to about 1.4 pixels on a 1080p window,
/// the square is about 90 pixels wide. 128 is a little over that, so the
/// sampler always downscales, which looks better than upscaling.
const SIZE: UVec2 = UVec2::new(128, 96);

/// The aspect ratio the field of view is computed at. [`SIZE`] is 4:3 because
/// of this value. See the module note.
const ASPECT: f32 = 4.0 / 3.0;

/// The maximum number of portraits kept at once.
///
/// Nine: the player, the target, target-of-target, four party members, the pet
/// and one spare. Each uses a render layer. The cap exists because the
/// interface, not this client, chooses the tokens: without it, an addon asking
/// for a portrait of `"party8"` would create a new camera each time.
pub(super) const MAX: usize = 9;

/// The first render layer portraits use. Layer 0 is the world's, and every
/// entity in this client that does not set a layer is on it.
pub(super) const FIRST_LAYER: usize = 1;

/// [`crate::ui::report::HudReport`] slot 32, immediately under the interface's
/// line at 31. The interface requests portraits, so the number of cameras they
/// cost is shown beside the interface's "N quads".
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(32);

/// The lighting a portrait is rendered under. It is the one value in this file
/// that neither comes from the game nor is derived from it.
///
/// [`super::paperdoll`] uses the same rig. The face on the unit frame and the
/// same character's body on the character sheet are on screen together, and a
/// difference in their lighting would be visible.
///
/// A three-quarter fill and one key lamp from the camera's side, in the model's
/// frame, so the light follows the subject rather than the world. This differs
/// from the 1.12.1 client, as the module note states. World lighting would turn
/// the face black at dusk and keep it black all night, which no screenshot of
/// the 1.12.1 client shows.
///
/// Written as a literal rather than through [`SceneLighting::resolve`] because
/// there is no `M2Light` to resolve: this is a fixed rig, not data from a file.
/// The layout is the one that function produces: `(place.xyz, is_point)` then
/// `(colour.rgb, 0)`, with `w` on the ambient holding the lamp count.
pub(super) const STUDIO: SceneLighting = SceneLighting {
    ambient: Vec4::new(0.42, 0.42, 0.46, 1.0),
    lamps: [
        // Directional (`w = 0`), pointing toward the light: up, forward and to
        // the model's left, which is the side the portrait cameras stand on.
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

/// One unit's portrait, and the entities and assets that render it.
struct Portrait {
    /// The values this portrait was built from, used to detect changes. A new
    /// guid, a shapeshift or a helmet causes a rebuild.
    built: Built,
    /// The image the camera draws into, and the id egui knows it by.
    image: Handle<Image>,
    texture: bevy_egui::egui::TextureId,
    camera: Entity,
    /// The model root; despawning it takes the batches and joints with it.
    root: Entity,
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    /// The render layer this model and camera have to themselves.
    layer: usize,
    /// Whether the joints have been written. The pose is a still (see the
    /// module note), so it is written once and never advanced; this flag
    /// records that it has been written.
    posed: bool,
    /// How many more frames the camera renders before it is switched off.
    ///
    /// The image is a still: the pose is written once, the model never moves
    /// and the framing never changes. Rendering it every frame changed nothing
    /// and cost a whole extra `Core3d` camera, measured with Tracy at about
    /// 1.5 ms of CPU encode per portrait per frame. With a player, a target and
    /// a party, that is most of a frame budget, and it was the largest single
    /// per-player cost in the client. The camera runs for [`SETTLE_FRAMES`]
    /// after the pose is written (meshes allocate and textures upload over the
    /// first few frames), and then [`shutter`] deactivates it; the rendered
    /// image persists and egui keeps sampling it. A rebuild spawns a new
    /// camera, so no camera is ever reactivated.
    settle: u8,
}

/// What a portrait was rendered from, compared to decide whether to render it
/// again.
///
/// It holds the display id and the look, because a player who changes gear
/// keeps the same guid and the same model but needs a new portrait. `dress`
/// decides the look, so this is the same pair the world's model rebuild keys
/// on.
#[derive(Clone, PartialEq)]
struct Built {
    guid: u64,
    display_id: u32,
    appearance: Option<vale_assets::look::character::Appearance>,
    equipment: Vec<(u32, u32)>,
}

/// Every portrait the interface has asked for, and the tokens it asked with.
#[derive(Resource, Default)]
pub struct Portraits {
    /// Every token the interface has named, kept rather than cleared each
    /// frame.
    ///
    /// The set holds the game's unit tokens and stays small. If it were cleared
    /// each frame, a portrait would disappear on every frame the interface did
    /// not ask again. Capped at [`WANTED_CAP`]; the camera cap is [`MAX`].
    wanted: BTreeSet<String>,
    /// The portraits, one per token that names a unit.
    taken: BTreeMap<String, Portrait>,
}

impl Portraits {
    /// The texture id for a token, or `None` while there is no portrait: on the
    /// frame a target is acquired, and on any frame its model is still loading.
    /// The painter then draws whatever the `<Texture>` already had.
    pub fn texture(&self, token: &str) -> Option<bevy_egui::egui::TextureId> {
        Some(self.taken.get(token)?.texture)
    }

    /// The same portrait as the image the camera draws into, for the mesh
    /// painter, which binds textures rather than egui ids. This returns `Some`
    /// exactly when [`Self::texture`] does, because both fields are written
    /// together when a portrait is built.
    pub fn image(&self, token: &str) -> Option<Handle<Image>> {
        Some(self.taken.get(token)?.image.clone())
    }

    /// How many portraits are kept, for the HUD line.
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
                // After the entity pass, which creates the `WorldEntity` that
                // [`follow`] reads and shares the dressing rule with it. Bevy
                // cannot see this dependency, because the two touch different
                // components, so the ordering is stated here, per the
                // convention.
                .after(crate::world::entities::EntitySet),
        );
    }
}

/// The portrait cost, on this pass's own HUD line. [`crate::ui::report`]
/// explains why this is not a line in `hud.rs`.
///
/// It shows two numbers. "Asked" is how many tokens the interface has named,
/// which depends on the interface: about three in an ordinary session and nine
/// with a raid frame. "Taken" is how many of those name a unit now, which is
/// how many extra cameras and models the frame renders. The two are often far
/// apart, and that is expected: an empty party asks for four and takes none.
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

/// How many token names may accumulate. This bounds an addon that invents
/// tokens; it does not bound cameras. The camera cap is [`MAX`], enforced
/// where the layer is allocated, in [`follow`].
///
/// The two bounds must stay separate. When one cap served both, the interface
/// named `pet` and `party1..4` unconditionally at load, and this client could
/// not resolve them. Nine unresolvable names filled the cap, and `"npc"`, first
/// asked for when the player talks to an NPC, was refused for the rest of the
/// session. Every conversation portrait was white in any session that had held
/// a party.
const WANTED_CAP: usize = 32;

/// Move the interface's new portrait requests into the kept set.
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
        // Compare before building. On almost every frame this comparison is
        // all the system does. `Built` carries the wearer's equipment list, so
        // constructing one allocates a `Vec`, per token per frame, only to
        // discard it. [`unchanged`] answers the same question by reading the
        // unit's fields.
        if unchanged(&units, &portraits, &token) {
            continue;
        }
        let wanted = subject(&units, &token);
        // A token that names no unit, such as an empty party slot or a cleared
        // target, has its portrait removed at once. So does a token that now
        // names a different unit. Keeping the last face would look the same as
        // a working portrait of the wrong unit.
        // A token that names the same unit with a changed look keeps the old
        // portrait until the new one is ready. A player's equipment arrives one
        // item at a time behind `CMSG_ITEM_QUERY_SINGLE`; removing the portrait
        // on each arriving item template blanked the frame once per item until
        // all of them had arrived.
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
        // A transport is a `.wmo` and has no portrait, as it has no model.
        // `world::entities::spawn_models` skips the same paths for the same
        // reason.
        if display.path.to_ascii_lowercase().ends_with(".wmo") {
            continue;
        }
        let Some(tables) = displays.tables() else {
            continue;
        };
        // The same dressing rule the world uses, through the same function. A
        // separate rule here could dress the portrait in different gear from
        // the same unit's body on screen.
        //
        // Weapons are left out on purpose. A portrait shows the head and
        // shoulders; attaching a sword would load a second model for something
        // outside the view frustum. `Wearer::weapons` is empty and the sheath
        // state is the sheathed one, which `dress` reads as "no weapons".
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
            // next frame asks again. `ModelCache` handles repeated requests,
            // and a `Failed` path costs one map lookup a frame for as long as
            // the unit is targeted. Meanwhile an older portrait of the same
            // guid stays up; see the removal note above.
            continue;
        };
        // The replacement is ready, so remove the old portrait and free its
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

/// Whether the portrait already shown is the right one, without building a
/// [`Built`] to find out.
///
/// It makes the same comparison [`subject`] would allow, read field by field
/// from the live unit. The answer is yes on almost every frame for almost
/// every token, and building a `Built` first allocated a `Vec` per token per
/// frame. Nine tokens at sixty frames a second is 540 needless allocations a
/// second, in a client whose collector is already one of its larger per-frame
/// costs.
///
/// Returns `false` when the token names no unit or has no portrait. Both mean
/// the caller has work to do and must find out which case applies.
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

/// What a token's portrait should show, or `None` for a token that names no
/// unit.
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

/// The object type of a token's unit, which selects the display table its
/// display id is resolved through.
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
///
/// This spawns no particle emitters and no ribbon trails, unlike
/// `render::glue::spawn`, the other function with this structure. A portrait
/// shows a head and shoulders for as long as a unit is targeted. A wisp's dust
/// and a weapon's trail would be a simulation running every frame for an image
/// 90 pixels across, and neither is in view at this framing. This is stated
/// because the omission is not visible when comparing the code with the glue
/// pass it was based on.
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

    // --- The model, alone on its render layer at the origin ---

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
            // Set on every drawn entity, because Bevy reads `RenderLayers` per
            // entity and does not propagate it down a hierarchy. A part without
            // the layer is drawn by the world camera, as a floating head at the
            // origin of the map.
            layers.clone(),
            // The portrait camera can sit inside the model's bounding box, so
            // frustum culling against that box is wrong here: a model whose
            // declared box does not contain the camera can be culled out of its
            // own portrait.
            NoFrustumCulling,
        ));
        // Bind the batch's own bones, not the model's whole skeleton.
        // `models::skin_for` is the one place that decides this. When mesh
        // joint indices became subset-local, three spawners were converted and
        // are named in that function's doc; this fourth one was missed. It kept
        // binding the whole skeleton, every vertex was posed by the wrong bone,
        // and the face was stretched across the frame. No headless check
        // covers this.
        if let Some(joints) = crate::render::models::skin_for(draw, &joints) {
            part.insert(SkinnedMesh {
                inverse_bindposes: model.inverse_bindposes.clone(),
                joints,
            });
        }
    }

    // --- The camera, along the model file's portrait camera ---

    let framing = model.portrait;
    let eye = axes::to_bevy(framing.eye);
    // A degenerate framing has its aim point moved rather than aimed at the
    // eye. An eye and an aim at the same place make `looking_at` produce a NaN
    // basis, and a NaN view matrix breaks culling for the whole frame, for
    // every mesh in the world. The guards in the framing rule should prevent
    // this case, and the check costs one comparison. `render::glue::aim_camera`
    // makes the same check and returns instead; that is not possible here
    // because the camera must exist.
    let aim = match axes::to_bevy(framing.aim) {
        target if eye.distance_squared(target) < 1e-6 => eye - Vec3::Z,
        target => target,
    };
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                // Before the world camera, which is order 0. A camera drawing
                // into an image that is sampled later in the frame must finish
                // first.
                order: -1 - layer as isize,
                // Opaque black. The 1.12.1 client draws a portrait over a
                // black ground, so the disc behind an NPC's face is black
                // rather than the unit frame's art. The painter's disc cuts
                // the square to the frame's round hole.
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            // The render target is its own component in Bevy 0.19, as in
            // `world::camera::spawn`, which writes `WorldFrame::target()`.
            RenderTarget::Image(ImageRenderTarget {
                handle: image.clone(),
                scale_factor: 1.0,
            }),
            // The world camera sets the same space for the same reason. Every
            // model shader in this client ends in `atmosphere::to_frame` and
            // writes the game's own bytes, so a camera in the default space
            // would encode them a second time and give the interface a
            // washed-out face. See `crate::render::present`.
            bevy::camera::CompositingSpace::Srgb,
            Projection::Perspective(PerspectiveProjection {
                fov: crate::render::lens::vertical_fov(framing.fov, ASPECT),
                near: framing.near.max(0.01),
                far: framing.far.max(framing.near.max(0.01) * 10.0),
                aspect_ratio: ASPECT,
                ..default()
            }),
            // No multisampling and no tonemapping: this is a 128x96 image of
            // one model. Both are set explicitly because `Camera3d` defaults to
            // multisampled and tonemapped.
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
/// The content is final once the joints are written, but the render world
/// takes a few frames to allocate the meshes and upload the composed skin, and
/// the camera must be active on the frame that draws the finished image into
/// the target. Half a second at 60 fps is well past that and still removes
/// about 97% of the steady-state cost. The value is a safety margin, not a
/// tuned number.
const SETTLE_FRAMES: u8 = 30;

/// Switch a settled portrait's camera off.
///
/// The portrait is a still (see [`pose`]) drawn into a persistent image, so
/// once it has been rendered the camera has nothing left to do. An active
/// `Camera3d` runs a whole `Core3d` graph per frame, about 1.5 ms of CPU encode
/// per portrait on the measured machine. The countdown starts when the content
/// is final (the pose is written, or the model has no skeleton), and the
/// camera is switched off at zero. It is never switched back on: a rebuild
/// ([`follow`]) spawns a new camera. The one thing this freezes that an active
/// camera would animate is a UV-animated material on the model; the 1.12.1
/// client renders its portrait once, so it freezes that too.
fn shutter(mut portraits: ResMut<Portraits>, mut cameras: Query<&mut Camera>) {
    for portrait in portraits.taken.values_mut() {
        // The content is not final: the pose system is still waiting for the
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
/// The pose is `Stand` at time zero and is never advanced; the module note
/// explains why the portrait is a still. The system runs every frame and does
/// nothing after the first pass for each portrait. That costs one `bool`, which
/// is cheaper than the alternatives (a one-shot schedule, or a marker component
/// removed after the first pass).
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
        // The identity joint at the end, which vertices with no bone weights
        // use. Without it the skinning shader collapses them to the origin.
        // `world::entities` and `render::glue` spawn the same extra joint.
        if let Some(&last) = portrait.joints.last() {
            if let Ok(mut transform) = joints.get_mut(last) {
                *transform = GlobalTransform::default();
            }
        }
        portrait.posed = true;
    }
}

/// Remove one portrait: the model, the camera, the image and egui's handle to
/// it.
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
    // Remove egui's handle as well. Otherwise the id stays registered against
    // an image asset nothing else holds, which leaks one 48 KB image per
    // target change; over a session of fighting many creatures that adds up
    // to a large, unexplained memory growth.
    egui.remove_image(&portrait.image);
}

/// The image a portrait is drawn into.
///
/// The format is `Rgba8UnormSrgb`, for the same reason
/// `render::present::WorldFrame` uses it. The camera's main texture holds raw
/// bytes (`CompositingSpace::Srgb`), and Bevy's upscaling blit applies
/// `SRGB_TO_LINEAR` on output because it assumes the destination re-encodes;
/// this image is that destination. With a linear format the linear value would
/// be stored as is, and the face would render as dark as dusk in daylight, the
/// same artefact that note describes for the world frame.
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
        // Render world only: nothing on the CPU reads this image, and egui
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

    /// The squeeze as arithmetic. `M2Camera::fov` is a diagonal angle at 4:3
    /// (`render::lens::vertical_fov` records the evidence), so a human male's
    /// authored 0.785 becomes 0.6 of itself vertically. Read as a vertical
    /// angle instead, every face in the interface is framed 67% too wide and
    /// the head fills a third of the square.
    #[test]
    fn the_field_of_view_is_the_diagonal_squeezed_to_four_three() {
        let vertical = |diagonal: f32| diagonal / (ASPECT * ASPECT + 1.0).sqrt();
        assert!((vertical(1.0) - 0.6).abs() < 1e-6);
        // The human male's own camera, and the wolf's.
        assert!((vertical(0.785) - 0.471).abs() < 1e-3);
        assert!((vertical(0.950) - 0.570).abs() < 1e-3);
    }

    /// Every portrait has a render layer to itself; that is how one model is
    /// drawn without the world behind it. Two portraits on one layer would put
    /// two units in one square.
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

    /// The camera orders are distinct and all before the world camera's, which
    /// is order 0. If two cameras shared an order, a target could be sampled
    /// before it was drawn into, which shows a portrait one frame old, or an
    /// empty square on the first frame.
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

    /// The material cache treats the studio rig as a lit scene, and keys it
    /// apart from the world's build of the same model. Without both, a portrait
    /// would either draw unlit or give the world its studio lighting.
    #[test]
    fn the_studio_rig_is_lit_and_keys_apart_from_the_world() {
        assert!(STUDIO.is_lit());
        assert!(!SceneLighting::NONE.is_lit());
        assert_ne!(STUDIO.key(), SceneLighting::NONE.key());
    }

    /// A settled portrait's camera is switched off, and an unposed one's stays
    /// on. The first half is the optimization: rendering a still image every
    /// frame cost about 1.5 ms of encode per portrait per frame. The second half
    /// prevents a regression: deactivating before the pose is written leaves a
    /// bind-pose face, or an empty square, in the frame.
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
            // Skinned and not yet posed: the joints exist but the pose has not
            // been written, so the countdown must not start.
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
        // The unposed portrait still has its full settle count.
        let portraits = app.world().resource::<Portraits>();
        assert_eq!(portraits.taken.get("waiting").unwrap().settle, 2);
    }
}
