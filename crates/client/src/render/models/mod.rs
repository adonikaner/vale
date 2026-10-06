//! M2 models: from an archive path to a list of (mesh, material) draws.
//!
//! ```text
//! mod.rs       the cache, the mesh build, and the types callers use
//! loader.rs    the loader thread: one archive chain, a request channel, and
//!              the M2 and BLP reads that are kept off the frame
//! material.rs  the material a batch is drawn with: the blend table, the two
//!              sidedness flags, the one material every untextured slot
//!              shares, and the shared texture-matrix table
//! ```
//!
//! This module does not know about placement: a model is loaded once, cached
//! by path, and drawn wherever the world asks for it. The doodad pass and the
//! entity pass both use it; the entity pass adds a skeleton and a pose and
//! reads the file the same way.
//!
//! There is no manual instance grouping. The WebGL renderer built per-model
//! instance buffers by hand (`ANGLE_instanced_arrays`, one call per
//! model+batch for the whole loaded world) because one draw call per placement
//! was tens of thousands a frame. Bevy batches by mesh and material
//! automatically, so a placement is an ordinary entity. That also gives
//! per-instance frustum culling, which the hand-built buffers could not do
//! without being rebuilt every frame.
//!
//! ## One mesh per batch
//!
//! An `M2Batch` is a range of the model's index buffer plus a material, and
//! Bevy draws a whole `Mesh3d`, not a range of one. So each batch becomes its
//! own mesh with the vertices it uses remapped into a buffer of its own. The
//! terrain pass does the same, for the same reason: each batch gets its own
//! `Aabb` for culling. Most doodads have one or two batches.
//!
//! ## The loader is a thread, not a task per model
//!
//! Opening the archive chain is ~19 MPQs and takes a moment, and
//! `Assets::read` needs `&mut`. A task per model would either open a chain each
//! (a tile is ~130 distinct models) or queue every model behind the shared lock
//! the session and the tile loads already use. One thread that owns one chain
//! and pulls requests from a channel does neither.
//!
//! ## The cache key includes whether the mesh is skinned
//!
//! 17 of a Darkshire tile's 128 doodad models carry a skeleton (windmills,
//! banners), and a doodad is drawn in its bind pose. So the same M2 builds two
//! different meshes depending on the caller, and the cache is keyed on that as
//! well as on the path.
//!
//! This is required for correctness. Bevy chooses the pipeline from the mesh's
//! attributes (`is_skinned(layout)`) and the bind group from whether the
//! entity has an extracted skin. If the two disagree, wgpu rejects the draw
//! and Bevy exits:
//!
//! ```text
//!   The BindGroupLayout 'mesh_layout' of BindGroup 'model_only_mesh_bind_group'
//!   is not compatible with 'skinned_mesh_layout' of 'opaque_mesh_pipeline'
//! ```
//!
//! A doodad that shared a creature's mesh would therefore stop the renderer as
//! soon as it came into view, and since the geometry is identical, the picture
//! would give no sign of the cause.
//!
//! ## Geometry is cached by path; a dressing by path and skins
//!
//! A dressing is a model together with one set of textures. A creature's skin
//! is not in its M2: the model declares texture type 11 (or type 1 for a
//! character model) with no filename and the client supplies it from
//! `CreatureDisplayInfo`, so one `Wolf.m2` is drawn grey, black and white by
//! three display ids. WebGL bound the texture at draw time. Here a texture is
//! part of the material, so those three need three materials. The geometry is
//! identical and the meshes are the costly part, so [`Geometry`] is keyed by
//! path and only the materials are rebuilt per dressing: the three wolves share
//! one set of vertex buffers.

use crate::assets::GameAssets;
use crate::axes;
// Aliased: `vale_assets::Assets` is the MPQ chain and `bevy::prelude::Assets`
// is the asset store, and both are in scope here.
use vale_assets::{
    world::blp,
    look::character::{Appearance, CharSections, Composite},
    // `CharacterLook` is the asset crate's, because what a player is dressed
    // as is a game rule and not a rendering decision; see `vale_assets::look::dress`.
    // This crate only turns it into a cache key and a texture.
    look::dress::CharacterLook,
    tables::item::ItemDisplays,
    world::m2::{BatchTint, Dress, M2Attachment, M2Batch, M2Skeleton, M2},
    Assets as Archive,
};
use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::camera::primitives::Aabb;
use bevy::ecs::system::SystemParam;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::mesh::VertexAttributeValues;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::platform::collections::{HashMap, HashSet};
use vale_assets::tables::light::LiquidLight;
use vale_assets::world::wmo::{BatchLight, Liquid};
use bevy::prelude::*;
use bevy::render::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, RenderPipelineDescriptor,
    ShaderType, SpecializedMeshPipelineError, TextureFormat,
};
use bevy::shader::ShaderRef;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{mpsc, Arc, Mutex};

/// The alpha-key cutoff for an M2 material (blend mode 1): 224/255, the same
/// value the WMO path uses. It replaces a value of 0.5 that had no source.
///
/// The 1.12.1 client sets the alpha reference from the blend mode whenever it
/// sets the blend state. The values per blend mode are:
///
/// ```text
/// 0, 224, 1, 1, 1, 1, 1, 0, 0, 0, 0
/// ^   ^   ^-------------^
/// |   |   every translucent mode: discard alpha 0 and nothing else
/// |   alpha key
/// opaque
/// ```
///
/// So the engine has one alpha-key cut, 224, and the WMO constant and this one
/// are the same value under two names. The reference for a blend-1 material is
/// also set to `material.alpha * 224.0`, so 224 is the ceiling that a
/// material's own alpha scales.
///
/// This constant is not the cause of the blue squares drawn around an
/// Evocation. `SPELLS\CLOUDS.BLP` decodes with `alphaDepth = 0` and 0%
/// transparent texels, so under a 0.5 cut each of its quads would draw as an
/// opaque tile. But those emitters name a geometry model, and the client draws
/// no quad for such an emitter, so their texture slot is not read and no alpha
/// reference affects them (see `crate::render::particles::model_particles`).
/// Changing the cut hides three of Evocation's five emitters, which masks
/// that separate fault instead of fixing it.
///
/// The value of the constant comes from the table above. Its visible effect on
/// screen is not established.
///
/// The rest of the population is alpha-keyed art (foliage, hair cards, window
/// lattices) whose alpha is near-binary, so a 0.5 cut and a 224/255 cut select
/// the same texels. If a tree looks thin, check that assumption first.
pub const M2_ALPHA_KEY: f32 = 224.0 / 255.0;

/// The alpha cut for every translucent blend mode: 1/255, not zero.
///
/// The same table gives 1 for blend modes 2 through 6 and 0 for opaque: a
/// blended draw discards a fully transparent fragment and nothing else. A
/// particle whose over-life ramp has reached zero is still a quad in the
/// depth-sorted transparent list, and discarding its fragments keeps the tail
/// of every emitter in the world out of the blend.
pub const M2_TRANSLUCENT_CUT: f32 = 1.0 / 255.0;

/// The cut for a material's blend mode, straight off the table above.
pub fn alpha_cut(blend: u16, alpha_key: f32) -> f32 {
    match blend {
        0 => 0.0,
        1 => alpha_key,
        _ => M2_TRANSLUCENT_CUT,
    }
}

/// How many models to turn into GPU assets in one frame.
///
/// A tile's worth arrives in a burst, and building ~130 models' meshes and
/// decoding their textures in a single frame is a visible hitch on every tile
/// crossing. They are already on screen a frame or two later.
const UPLOAD_BUDGET: usize = 8;

/// One batch of one model, ready to be spawned anywhere in the world.
#[derive(Clone)]
pub struct ModelDraw {
    pub mesh: Handle<Mesh>,
    pub material: Handle<M2Material>,
    /// Which of the model's joints this batch's `SkinnedMesh` names; see
    /// [`super::models::loader::RawDraw::bones`]. Empty for scenery and for a
    /// building, which have no skeleton.
    pub bones: Arc<[u16]>,
    /// How many of the model's batches this draw covers: 1 for every batch
    /// the file states, more for a merge; see
    /// [`super::models::loader::MergeSource`]. Used only by the scene tab, to
    /// report what a merge saves without counting entities.
    pub merged: usize,
    /// Which of the model's colour and transparency tracks fade this batch, or
    /// `None` for a batch that does not fade, which is most of the world.
    ///
    /// A tinted batch's `MeshTag` holds its colour, not its room, so a spawner
    /// that draws one must write that tag: see [`tint_tag`], and
    /// `M2Params::particle`'s `.y` for the material side. Set only on the
    /// skinned build of a model, which has an animation clock to sample.
    pub tint: Option<BatchTint>,
    /// The tint value for a batch that does not animate: the tint sampled once
    /// at the start of the timeline, ready to be written as a `MeshTag`.
    ///
    /// `Some` only on the unskinned build (the doodads), which has no clock to
    /// sample and whose track is constant. The skinned build leaves this `None`
    /// and writes the tag per frame from the entity's own animation; see
    /// `world::entities::pose`.
    ///
    /// A spawner that draws a doodad must use this instead of its own
    /// room-light tag when it is set: the material is specialised as tinted
    /// either way, so a tag holding a room light would be read as a colour.
    /// `render::doodads` is the only caller.
    pub baked_tint: Option<u32>,
    /// Whether this batch is a flat rectangle lying in the model's own ground
    /// plane; see [`vale_assets::world::m2::M2::ground_quad`].
    ///
    /// A spawner that draws on the floor passes this to
    /// [`crate::render::decals`] instead of spawning the mesh, and gets a
    /// mesh that follows the ground rather than a plane held at the caster's
    /// feet. `None` for every batch in the world except the 120 spell-effect
    /// batches authored this way.
    pub ground: Option<vale_assets::world::m2::GroundQuad>,
    /// Which liquid this batch is a surface of, or `None`. Only a WMO's `MLIQ`
    /// produces one; an M2 has no liquid block.
    ///
    /// The spawner uses it to mark the entity, for two reasons. The water can
    /// be switched off on its own ([`crate::render::tuning::WorldTuning::water`]),
    /// and a canal inside Stormwind and a lake in Elwynn are the same kind of
    /// surface, so the switch must reach both. And [`crate::render::water`]
    /// animates and colours every one of them, which needs the liquid kind: it
    /// selects the flipbook and the two light bands.
    pub liquid: Option<vale_assets::world::wmo::Liquid>,
}

/// A model dressed in one particular set of skins: what a placement or an entity
/// needs in order to draw it.
pub struct ModelAssets {
    /// One per visible batch. Shared by every placement of this dressing.
    pub draws: Vec<ModelDraw>,
    /// `None` for scenery, which is most of the world: a tree has no bones.
    pub skeleton: Option<Arc<M2Skeleton>>,
    /// All identity, one per joint. M2 vertices are already in model space and
    /// `M2Skeleton::pose` handles the pivot itself, so an inverse bindpose has
    /// nothing to undo; see the note in `entities.rs`.
    pub inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    /// `bones + 1`. The extra one is the identity joint that weightless vertices
    /// are bound to; without it the skinning shader collapses them to the origin.
    pub joint_count: usize,
    /// The bounding box the model's file declares, in Bevy's axes.
    ///
    /// This is the client's culling volume, authored to cover every frame of
    /// every animation: `vale anim` measures the whole bestiary against it
    /// and 766 of 768 animations stay inside. Using it instead of the bind
    /// pose's box keeps a running creature from being culled mid-stride.
    pub bounds: Option<Aabb>,
    /// Where to place the camera for this model's portrait, in the file's
    /// axes: its own portrait camera, or a framing derived from its bind pose.
    /// See [`vale_assets::look::portrait`] for the rule and the census behind
    /// it, and [`crate::render::portraits`], which is the only reader.
    ///
    /// Resolved at load rather than on demand for the same reason as
    /// [`Self::bounds`]: it needs the model's vertices as well as its camera
    /// block, and the `M2` has been dropped by the time a unit frame asks.
    pub portrait: vale_assets::look::portrait::Framing,
    /// Where to place the camera to show the whole model, in the same axes:
    /// its own character-info camera, or a framing derived from its bind pose.
    /// The counterpart of [`Self::portrait`], resolved beside it; see
    /// [`vale_assets::look::portrait::body_framing`] for the rule and
    /// [`crate::render::paperdoll`], which is the only reader.
    pub body: vale_assets::look::portrait::Framing,
    /// Whether this model tilts with the ground under it, from the M2
    /// header's `GlobalModelFlags`; see [`vale_assets::look::conform`] for the
    /// rule. `Level` for every character model in the game and for all of the
    /// scenery; the other modes are for mounts and quadrupeds.
    pub conform: vale_assets::look::conform::Conform,
    /// Where other models hang off this one: a bone and a point in its frame.
    /// Empty for scenery; every character model carries shoulders and a helm.
    pub attachments: Arc<Vec<M2Attachment>>,
    /// The sound cues its animations carry, such as a laugh at the laugh's
    /// keyframe. See `sound::cues`.
    pub cues: Arc<vale_assets::world::m2::SoundCues>,
    /// The two points a weapon's trail runs between, `None` for every model
    /// that is not a melee weapon. See [`vale_assets::look::weapon_trail`].
    pub trail: Option<vale_assets::look::weapon_trail::TrailPoints>,
    /// The lights it carries, such as a lamppost's glow quad or a sconce's
    /// flame. Empty for almost everything. See [`crate::render::lamps`], which
    /// turns one into a light, and `vale_assets::world::glow`, which decides
    /// whether a batch is one.
    pub glows: Arc<Vec<crate::render::lamps::ModelGlow>>,
    /// The model's solid triangles, in model space: a different set from
    /// [`Self::draws`], and empty for most of the world. Shared by every
    /// placement of the model, as a building's hull is, and transformed
    /// per placement by the doodad pass.
    ///
    /// A property of the file and not of the dressing, so every variant of one
    /// model returns the same `Arc`: what a creature is wearing does not
    /// change what a character walks into.
    pub collision: Arc<vale_assets::world::collision::CollisionMesh>,
    /// The model's drawn triangles, kept on the CPU in model space, for the
    /// mouse pick's narrow phase; see [`vale_assets::look::pick`].
    ///
    /// A different set from [`Self::collision`], used for a different test:
    /// the hull is what a character walks into and most of the world has none,
    /// while this is the silhouette a pointer has to land on, and every model
    /// has one. Shared per path for the same reason as the hull: what a
    /// creature is wearing does not move its outline enough to change a click,
    /// and one copy per dressing would store the same arrays several times.
    pub pick: Arc<vale_assets::look::pick::PickMesh>,
    /// The sphere in the model's header, in model yards. The pick's broad
    /// phase uses it when the animation the unit is playing states no sphere,
    /// and it is the only sphere for a model with no skeleton.
    ///
    /// The file's `boundingRadius`, not a value computed from
    /// [`Self::bounds`]: the two are different numbers, and the client uses
    /// `boundingRadius`.
    pub model_sphere: vale_assets::look::pick::Sphere,
    /// The particle emitters (a torch's flame, a wisp's dust) with their
    /// materials already interned, or `None` for a model with no emitters. A
    /// property of the file like the hull above: a particle's texture is the
    /// model's own, so every dressing shares one set.
    pub particles: Option<Arc<crate::render::particles::ParticleSet>>,
    /// The ribbon trails (a weapon's streak, a missile's tail), on the same
    /// terms as the emitters above: a property of the file, one set shared by
    /// every dressing.
    pub ribbons: Option<Arc<crate::render::ribbons::RibbonSet>>,
    /// The model's own cameras: where to place a camera to look at it.
    ///
    /// 401 of the 408 creature models that decode carry two, a portrait
    /// camera and a character-info camera. Besides the glue screens,
    /// [`Self::portrait`] is resolved from them.
    pub cameras: Arc<Vec<vale_assets::world::m2::M2Camera>>,
    /// The model's own lights, on the same terms: a property of the file,
    /// shared by every dressing of it, empty for everything in the world.
    /// See [`SceneLighting`], which is what they become.
    pub lights: Arc<Vec<vale_assets::world::m2::M2Light>>,
    /// The model's colour and transparency tracks, or `None` when no batch of
    /// this build is tinted. Shared like the hull and the emitters: a fade is a
    /// property of the file, and what differs per instance is only the clock.
    pub tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
}

/// What the cache knows about a path.
pub enum Lookup {
    /// Loaded; this is everything needed to draw it.
    Ready(Arc<ModelAssets>),
    /// Asked for, not back yet.
    Loading,
    /// Will not read. Reported once, not once per placement.
    Failed,
}

/// One decoded model's geometry and its own textures, shared by every dressing.
///
/// Held until nothing has asked for it in [`crate::render::residency::IDLE_SECS`];
/// see [`ModelCache::evict`]. The meshes and images it holds are the GPU
/// memory, and this map owns them after the last placement has gone out of
/// range.
struct Geometry {
    /// [`ModelCache::now`] when a dressing last asked for this build.
    used: f32,
    /// One per visible batch, parallel to [`Self::draws`].
    meshes: Vec<Handle<Mesh>>,
    draws: Vec<DrawParams>,
    /// Each batch's own bone subset, parallel to the two above; see
    /// [`super::models::loader::RawDraw::bones`] for why it exists. Not in
    /// [`DrawParams`]: that is the material's interning key, and two batches
    /// with identical materials and different bones must still share one
    /// material.
    bones: Vec<Arc<[u16]>>,
    /// Each batch's CPU geometry, where the batch may be merged; see
    /// [`super::models::loader::MergeSource`]. `None` for a batch that may not
    /// be merged, and empty for a build with no skeleton.
    merge: Vec<Option<Arc<crate::render::models::loader::MergeSource>>>,
    /// The texture type of each of the model's own texture slots. Type 0 is a
    /// filename inside the model; the client supplies every other type.
    kinds: Vec<u32>,
    /// The model's own textures, magenta where it names one the archive lacks.
    own: Vec<Handle<Image>>,
    skeleton: Option<Arc<M2Skeleton>>,
    inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    joint_count: usize,
    bounds: Option<Aabb>,
    /// Where a portrait of it is taken from; see [`ModelAssets::portrait`].
    portrait: vale_assets::look::portrait::Framing,
    /// Where the whole model is framed from; see [`ModelAssets::body`].
    body: vale_assets::look::portrait::Framing,
    /// The M2 header's conform mode; see [`ModelAssets::conform`].
    conform: vale_assets::look::conform::Conform,
    attachments: Arc<Vec<M2Attachment>>,
    /// The sound cues, per sequence, shared by every dressing of the file,
    /// like the hull.
    cues: Arc<vale_assets::world::m2::SoundCues>,
    /// The weapon-trail points; see [`ModelAssets::trail`].
    trail: Option<vale_assets::look::weapon_trail::TrailPoints>,
    /// The lights it carries; see [`ModelAssets::glows`].
    glows: Arc<Vec<crate::render::lamps::ModelGlow>>,
    collision: Arc<vale_assets::world::collision::CollisionMesh>,
    /// The drawn triangles kept on the CPU, for the mouse pick's narrow
    /// phase; see [`ModelAssets::pick`]. Shared per path like the hull above.
    pick: Arc<vale_assets::look::pick::PickMesh>,
    /// The header sphere the pick falls back to; see [`ModelAssets::model_sphere`].
    model_sphere: vale_assets::look::pick::Sphere,
    /// The emitters with their materials already interned, or `None` for a
    /// model with no emitters. Built once per geometry: a particle's texture is
    /// always the model's own (the survey counts 0 client-supplied slots
    /// over all 2,626 emitters), so unlike a batch material it cannot depend
    /// on the dressing, and every dressing shares this `Arc`.
    particles: Option<Arc<crate::render::particles::ParticleSet>>,
    /// The ribbon trails, interned the same way and for the same reason.
    ribbons: Option<Arc<crate::render::ribbons::RibbonSet>>,
    /// The model's own cameras: a property of the file, shared by every
    /// dressing as the hull is. Empty for all but seven files in the
    /// game; see `crate::render::glue`.
    cameras: Arc<Vec<vale_assets::world::m2::M2Camera>>,
    /// The model's own lights, shared for the same reason.
    lights: Arc<Vec<vale_assets::world::m2::M2Light>>,
    /// The model's texture matrices, or `None` when it states none, which
    /// is true of all but a handful of files. Kept beside the tints for the
    /// same reason: the tracks are stored once per model however many batches
    /// read them.
    uv_anims: Option<Arc<vale_assets::world::m2::M2TextureAnims>>,
    /// Sequence 0's window: the clock the emitters, the trails and a
    /// non-global texture matrix all run on.
    clip: Option<crate::render::particles::ParticleClip>,
    /// The colour and transparency tracks, or `None` when no batch names one.
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
}

/// Everything about a batch except its geometry and its texture: the parameters
/// a material is built from.
pub(crate) struct DrawParams {
    /// The appearance variant this batch belongs to, `group * 100 + variant`.
    /// The dressing decides which of them are drawn; see
    /// [`vale_assets::world::m2::visible_geosets`].
    pub geoset: u16,
    /// Index into the model's texture list.
    pub texture: Option<usize>,
    pub blend: u16,
    pub unlit: bool,
    pub two_sided: bool,
    pub no_depth_write: bool,
    /// See [`super::loader::RawDraw::light`].
    pub light: BatchLight,
    /// See [`RawDraw::uv`].
    pub uv: Option<u16>,
    /// The colour and opacity of this liquid, when it is one. See
    /// [`RawDraw::liquid`].
    pub liquid: Option<LiquidLight>,
    /// Which of the model's colour and transparency tracks fade this batch;
    /// see [`ModelDraw::tint`]. Only whether there is one reaches the
    /// material; which tracks is handled per instance.
    pub tint: Option<BatchTint>,
    /// The constant tint a doodad batch draws with, as a `MeshTag`; see
    /// [`ModelDraw::baked_tint`]. Part of the key because the material is
    /// specialised as tinted and the tag beside it is read as a colour, so two
    /// batches with different constant tints must not share a pooled handle.
    pub baked_tint: Option<u32>,
    /// See [`ModelDraw::ground`].
    pub ground: Option<vale_assets::world::m2::GroundQuad>,
    /// See [`super::loader::RawDraw::overlays`]: which of the model's texture
    /// slots the folded environment-map layers read, and with which blend.
    pub overlays: [Option<(u32, u16)>; vale_assets::world::m2::MAX_OVERLAYS],
}

/// A texture requested by name rather than through the model that uses it,
/// such as a creature's skin, which is named in a DBC and not in the M2.
#[derive(Clone)]
pub(in crate::render) enum TextureState {
    Ready(Handle<Image>),
    Loading,
    Failed,
}

/// A [`TextureState`], and when a dressing last used it.
///
/// `used` is updated when a dressing that uses the texture is used, not only
/// when the texture itself is looked up. A skin is looked up only while a
/// dressing is being built, so a texture behind a dressing in constant use
/// would otherwise look unused for as long as that dressing lasts. Evicting it
/// would leave the live material holding one image while the next build reads
/// a second copy of the same file. [`Variant`] therefore lists the skins it was
/// built from and touches them when it is touched.
struct Texture {
    state: TextureState,
    used: f32,
}

/// A dressing, and the two things it was built out of.
///
/// The geometry key and the skin keys are recorded so that a cache hit on this
/// dressing can touch exactly what it depends on. This makes
/// [`ModelCache::evict`] an exact liveness rule: nothing a live dressing needs
/// can be evicted while it is in use, and nothing else is kept.
struct Variant {
    assets: Arc<ModelAssets>,
    geometry: String,
    /// The non-empty entries of the `skins` this was dressed with.
    skins: Vec<String>,
    used: f32,
}

/// Every model that has been asked for, by archive path.
///
/// This is the largest store this program keeps. Every mesh and every image
/// the world has drawn is held by one of the three maps below, so without
/// eviction a session that walked from Elwynn to Stormwind to Tanaris would
/// hold all three zones' geometry and every texture in them for as long as the
/// process ran. The HUD does not show this: its mesh, batch and doodad counts
/// count what is spawned, which streaming already bounds. The cost is GPU
/// memory, which appears as a frame rate that decays over an hour rather than
/// as any single slow frame.
///
/// [`Self::evict`] releases unused entries, using the clock in
/// [`Self::now`]. See [`crate::render::residency`].
#[derive(Resource, Default)]
pub struct ModelCache {
    /// Decoded geometry, by [`geometry_key`]. One entry however many ways it is
    /// dressed; two if the same model is wanted both skinned and not.
    geometry: HashMap<String, Geometry>,
    /// The paths that have loaded, for the HUD. `geometry` counts a model
    /// wanted both ways twice.
    loaded: HashSet<String>,
    /// Dressed models, by [`dressing_key`]. The undressed one is built as each
    /// model arrives; the rest on demand.
    variants: HashMap<String, Variant>,
    /// Client-supplied skins, by archive path.
    textures: HashMap<String, Texture>,
    /// The renderer's clock, written once a frame by
    /// [`crate::render::residency::tick`], so that every path through this
    /// cache can stamp what it touched without taking `Res<Time>`.
    ///
    /// A copy of the time in a resource avoids passing an `f32` through
    /// `lookup`, `lookup_in_room`, `dressed`, `dressed_as_character`,
    /// `attached` and `dress`: six signatures and every call site of each, for
    /// a number that is the same for all of them within a frame.
    now: f32,
    failed: HashSet<String>,
    /// Requested and in flight, so a model shared by 400 placements is read once.
    pending: HashSet<String>,
    /// Decoded, waiting for a frame with upload budget left.
    arrived: Vec<Loaded>,
    /// Builds forgotten while their read was in flight; see
    /// [`Self::forget`]. An arrival whose key is here was read before the
    /// bytes changed, so it is dropped and requested again instead of being
    /// installed. Empty in a normal session: nothing is forgotten unless a
    /// host changes what a path returns.
    refetch: HashSet<String>,
    /// The one magenta placeholder, shared by every slot nothing fills.
    missing: Handle<Image>,
    /// The material every unit's shadow is drawn with, interned once; see
    /// [`ModelCache::shadow_material`].
    blob: Option<Handle<M2Material>>,
    /// One material per chain-effect texture, on the same terms as `blob`: a
    /// bolt of lightning is a strip textured with a bare BLP that no model
    /// names, so there is no `M2` through which the pool could reach it. The
    /// shipped data has ten such textures; see [`crate::render::lightning`].
    beams: HashMap<String, Handle<M2Material>>,
    loader: Option<Loader>,
}

/// The texture of the shadow a unit stands on. This is the only unit shadow
/// the 1.12.1 client draws.
///
/// The client's unit shadow uses this file and two CVars:
/// `shadowBias` ("Unit shadow depth bias") and `shadowLOD` ("Unit shadow LOD").
/// The client's other shadow setting is `mapShadows`, the ground's baked
/// `MCSH`, which this project already draws. So a unit's shadow is a decal, and
/// the client projects no shadows at run time.
///
/// The file's content, measured by `vale npc`, says how to draw it: 32x32,
/// one-bit alpha, dark in the middle (centre 160/255), white at the rim and
/// white under the cut-out corners. That is a modulate texture (blend mode 5,
/// `dst * src`), and because the corners are white it needs no fixup:
/// multiplying by them leaves the ground unchanged.
pub const SHADOW_BLOB: &str = r"Textures\ShadowBlob.blp";

/// How far above the feet the blob is drawn, in yards: the client's
/// `shadowBias`.
///
/// The blob is sampled onto the ground it lies on (`crate::render::shadows`), so
/// it is coplanar with the terrain, and coplanar geometry z-fights. This offset
/// lifts it clear. It is a depth bias only, as the CVar's name says.
///
/// The blob follows the ground, so the bias has no other effect. On one flat
/// horizontal quad the bias would also decide how much of a slope the quad
/// hung over: the uphill half is hidden by the ground (blend mode 5 is drawn
/// in the sorted phase, which depth-tests and does not write depth) and the
/// downhill half hangs in the air with a hard edge. 0.1 is the client's
/// default for `shadowBias`.
const SHADOW_BIAS: f32 = 0.10;

/// [`SHADOW_BIAS`], for the entity pass that places the quad.
pub fn shadow_bias() -> f32 {
    SHADOW_BIAS
}

/// The cache key for one build of a model's geometry.
///
/// The path alone when the joint attributes are not wanted, which is true of
/// every doodad in the world, so scenery keys do not change for a distinction
/// only entities make, and the two builds cannot collide.
fn geometry_key(path: &str, skinned: bool) -> String {
    if skinned {
        format!("{path}{SKINNED_SUFFIX}")
    } else {
        path.to_string()
    }
}

/// What [`geometry_key`] appends for the build carrying joint attributes. A NUL
/// cannot appear in an MPQ path, so the two namespaces cannot collide.
const SKINNED_SUFFIX: &str = "\u{0}skinned";

/// The inverse: the archive path a geometry key was built from.
fn path_of(key: &str) -> &str {
    key.strip_suffix(SKINNED_SUFFIX).unwrap_or(key)
}

/// What one [`ModelCache::evict`] let go of.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Evicted {
    /// Builds of a model's geometry, and with them their meshes.
    pub models: usize,
    /// Dressings: the material lists over that geometry.
    pub dressings: usize,
    /// Client-supplied skins, and with them their images.
    pub textures: usize,
}

impl Evicted {
    pub fn any(self) -> bool {
        self != Evicted::default()
    }
}

/// The baked light that lights a model standing inside a building, packed so
/// that it is stored per instance rather than in the material.
///
/// A `MODD` spawn carries its own colour (see
/// [`vale_assets::world::wmo::WmoDoodad::light`]), so a crate in a cellar is
/// darker than the same crate by a window. The colour must not be a material
/// parameter: measured by `vale wmos`, one tile of Stormwind's furniture holds
/// 2,249 distinct colours over 6,158 spawns, which is 3,037 (model, light)
/// pairs, so 3,037 materials for the furniture of one tile, against ~1,800 for
/// the entire world without them. When the colour was a material parameter,
/// even rounded to four bits a channel, the HUD's material count tripled, and
/// the per-frame CPU cost scales with the number of materials and batch sets.
///
/// So the colour goes to the GPU as the entity's [`bevy::mesh::MeshTag`], the
/// per-instance `u32` Bevy carries in its mesh uniform, and the material states
/// only that the batch is room-lit ([`M2Params::vertex_lit`]), not the colour.
/// As a result every room-lit build of one model shares one material however
/// many lights the building bakes, and the colour is not rounded: the shader
/// reads the spawn's own bytes.
///
/// Kept in the file's sRGB space, not decoded: the shader adds this to the
/// material's ambient and the `MOCV` colour and decodes the sum, because the
/// client does the arithmetic in that order. See `m2.wgsl`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RoomLight([u8; 3]);

impl RoomLight {
    /// The light of one `MODD` spawn, as bytes.
    pub fn new(colour: [f32; 3]) -> RoomLight {
        RoomLight(colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
    }

    /// The mesh tag the shader unpacks: `0x00RRGGBB`, red in the high byte,
    /// the same order the `Light.dbc` facts use. A test checks the order,
    /// because a swapped channel renders as a plausible room of the wrong hue.
    pub fn tag(self) -> u32 {
        u32::from(self.0[0]) << 16 | u32::from(self.0[1]) << 8 | u32::from(self.0[2])
    }
}

/// The per-instance multiplier the 1.12.1 client applies to a model's sun
/// colour.
///
/// | object | lit ground | in the ground's `MCSH` shadow |
/// |---|---|---|
/// | a terrain (`MDDF`) doodad | 1.0 | 0.5 |
/// | a WMO's `MODD` prop in an exterior group | 1.0 | 0.5 |
/// | an entity: unit, player, game object | 2.5 | 0.5 |
/// | an entity standing on a building outdoors | 2.5 | 2.5, not tested |
///
/// A prop in an interior group is lit by its room's colour and takes no
/// scale. A doodad's value is decided once, from the shadow bit under its
/// origin ([`vale_assets::world::adt::Adt::shadowed_at`]); an entity's moves
/// with the entity, toward its target at a fixed rate (see
/// [`crate::world::entities::SunScale`]).
///
/// The scale multiplies the sun colour and never the ambient fill. It is
/// applied inside `daylight`'s byte-space sum
/// (`atmosphere.wgsl::daylight_scaled`) and not clamped first, so the side of
/// a unit facing away from the sun keeps the cool fill, and a warm low sun is
/// not turned white by clamping `sun × 2.5` on its own.
pub mod sun_scale {
    /// An entity on lit ground, or standing on a building outdoors.
    pub const LIT_GROUND: f32 = 2.5;
    /// A doodad or an entity whose position is in the ground's baked `MCSH`
    /// shadow.
    pub const SHADOWED_GROUND: f32 = 0.5;
    /// A doodad on lit ground, WMO batches, an interior prop, and the
    /// encoding's absent value.
    pub const NEUTRAL: f32 = 1.0;
}

/// Pack a batch's sampled colour into the same `MeshTag` as [`instance_tag`],
/// as `0xAARRGGBB`.
///
/// The tag is the only per-instance channel, so a fade has to be stored in it.
/// A material is interned by value and a batch set cannot span two materials
/// (see [`MaterialPool`]), so a colour that changes every frame would be a new
/// material every frame, per batch, per caster. The tag is Bevy's per-instance
/// `u32` and is rewritten in place.
///
/// The low 24 bits are the same `0xRRGGBB` a [`RoomLight`] writes, and the top
/// byte is opacity rather than a sun scale. A tinted batch's material records
/// this ([`M2Params::particle`]'s `.y`) because the shader has to know which of
/// the two payloads it holds: zero is a real opacity here, where it means
/// "unspecified" for a sun scale. A batch that has faded out packs to
/// `0x00000000`, which the sun-scale reading would treat as neutral.
pub fn tint_tag(rgba: [f32; 4]) -> u32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(rgba[3]) << 24 | byte(rgba[0]) << 16 | byte(rgba[1]) << 8 | byte(rgba[2])
}

/// Pack an instance's room light and sun scale into the one `MeshTag`.
///
/// The tag is Bevy's per-instance `u32`: the low 24 bits are [`RoomLight`]'s
/// `0x00RRGGBB`, and the sun scale is stored in the byte the colour does not
/// use, as fixed-point 1/32ths (2.5 → 80, 0.5 → 16). Zero means "unspecified",
/// which the shader reads as [`sun_scale::NEUTRAL`], so every WMO batch and
/// every other instance that never sets a tag is lit at a sun scale of 1.0.
pub fn instance_tag(room: Option<RoomLight>, sun: f32) -> u32 {
    let scale = if sun == sun_scale::NEUTRAL {
        0
    } else {
        ((sun * 32.0).round() as u32).clamp(1, 255)
    };
    scale << 24 | room.map_or(0, RoomLight::tag)
}

/// The cache key for a model dressed in a particular set of skins, showing a
/// particular set of geosets.
///
/// The geosets are part of it because two NPCs sharing `HumanMale.m2` and one
/// bake can still differ in hair. The dressing is per appearance, which is the
/// granularity the hairstyle needs, so it adds no new dimension. A creature
/// adds nothing to the key, so scenery and creature keys are unchanged.
fn dressing_key(
    path: &str,
    skinned: bool,
    skins: &[String],
    dress: Dress,
    room: bool,
    as_particle: bool,
    scene: SceneLighting,
) -> String {
    let base = geometry_key(path, skinned);
    // Every field of the dress is in the key, not only the hair. A dressing is
    // a batch list, and two characters differing only in their boots have
    // different batch lists, so leaving the equipment geosets out of the key
    // would give the second one the first one's gear. The fault is invisible
    // while the only NPC gear is in the bake and every player's equipment
    // array is empty, and visible once equipment is filled in.
    let look = match dress {
        Dress::Creature => String::new(),
        Dress::Character(g) => {
            let mut s = format!(
                "|hair{},{},{},{}",
                g.hair, g.facial[0], g.facial[1], g.facial[2]
            );
            for geoset in g.equipment.iter().filter(|g| **g != 0) {
                s.push_str(&format!(",{geoset}"));
            }
            if g.hide_ears {
                s.push_str(",noears");
            }
            s
        }
    };
    // Room-lit is part of the key; the room's colour is not. The two
    // dressings differ because `vertex_lit` is a material parameter, but the
    // colour is stored on each instance as its `MeshTag`, so a barrel in a
    // dozen buildings at a dozen brightnesses is one dressing and one
    // material. Keying the colour here made Stormwind's furniture ~1,800
    // materials for one tile; see [`RoomLight`].
    let room = if room { "|room" } else { "" };
    // A model drawn as a particle is its own dressing, for the same reason as
    // a room-lit one: the difference is a material parameter. Every batch of
    // it reads its `MeshTag` as the emitter's over-life colour rather than as
    // a room and a sun scale; see [`ModelCache::as_particle`]. The population
    // is 13 files in the whole game, so this dimension costs almost nothing.
    // It cannot be folded into `room`, because the two give the same 32 bits
    // opposite meanings.
    let particle = if as_particle { "|particle" } else { "" };
    // The model's own lights are part of a dressing. Unlike the room's colour
    // they are in the material (see [`SceneLighting`]), so two builds under
    // different lights are two builds; without this, the character on the
    // character-select plinth would have the same key as the same character
    // in the world. Empty for every model the world draws, so no world key
    // grows.
    let lit = scene.key();
    if skins.iter().all(|s| s.is_empty())
        && look.is_empty()
        && room.is_empty()
        && particle.is_empty()
        && lit.is_empty()
    {
        return base;
    }
    format!("{base}|{}{look}{room}{particle}{lit}", skins.join("|"))
}

impl ModelCache {
    /// Where `path` has got to, requesting it if this is the first time it has
    /// been asked for.
    ///
    /// The model's own textures only (a doodad has no skins), and without the
    /// skinning attributes, because a doodad is drawn in its bind pose and a
    /// skinned mesh on an unskinned entity stops the renderer. See the module
    /// comment.
    pub fn lookup(&mut self, path: &str) -> Lookup {
        if self.variants.contains_key(path) {
            self.touch(path);
            let variant = self.variants.get(path).expect("touched above");
            return Lookup::Ready(Arc::clone(&variant.assets));
        }
        if self.failed.contains(path) {
            return Lookup::Failed;
        }
        self.request_model(path, false);
        Lookup::Loading
    }

    /// Mark a dressing, and everything it was built from, as used at
    /// [`Self::now`].
    ///
    /// The `used == now` guard keeps this cheap: a doodad pass that looks the
    /// same model up four hundred times in one frame pays for the two string
    /// clones once.
    fn touch(&mut self, key: &str) {
        let now = self.now;
        let Some(variant) = self.variants.get_mut(key) else {
            return;
        };
        if variant.used == now {
            return;
        }
        variant.used = now;
        let geometry = variant.geometry.clone();
        let skins = variant.skins.clone();
        if let Some(built) = self.geometry.get_mut(&geometry) {
            built.used = now;
        }
        for skin in &skins {
            if let Some(texture) = self.textures.get_mut(skin) {
                texture.used = now;
            }
        }
    }

    /// The same model standing inside a building, lit by the room instead of by
    /// the sun.
    ///
    /// `None` is the same as [`Self::lookup`]: the outdoor build, used by every
    /// tree in the world, which should not pay for hashing a key for a light it
    /// does not have. `Some` is a second dressing of the same geometry: the
    /// meshes are shared and only the materials differ, as with a wolf dressed
    /// in three skins.
    ///
    /// Needs the material store for that reason, where `lookup` does not.
    pub fn lookup_in_room(
        &mut self,
        path: &str,
        room: Option<RoomLight>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        self.lookup_scenery(path, room, false, meshes, materials)
    }

    /// Scenery lookup with an optional room light and an optional skinned
    /// build, for scenery that animates.
    ///
    /// `skinned` selects a different build of the geometry, not a flag on the
    /// same one: the skinning attributes are emitted at load, so a mesh built
    /// without them cannot be posed later, and one built with them carries the
    /// vertex attributes whether or not anything reads them. So scenery takes
    /// the unskinned build and animated scenery asks for the skinned one.
    ///
    /// Two builds of one model can be resident at once, for example a torch in
    /// the street near enough to animate and another across the zone in its
    /// bind pose. The geometry key includes `skinned`, so they are separate
    /// entries and each is evicted according to its own use. See
    /// [`vale_assets::look::scenery`] for which models ask for this and how
    /// few they are.
    pub fn lookup_scenery(
        &mut self,
        path: &str,
        room: Option<RoomLight>,
        skinned: bool,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        match (room, skinned) {
            (None, false) => self.lookup(path),
            (room, skinned) => self.dress(
                path,
                skinned,
                &[],
                Dress::Creature,
                room,
                false,
                SceneLighting::NONE,
                meshes,
                materials,
            ),
        }
    }

    /// The same model as the body of a particle: one live particle of a
    /// geometry-model emitter (`M2Particle::geometry_model`).
    ///
    /// It differs from [`Self::lookup`] in one parameter, which is all a model
    /// particle needs from this cache: every batch's material reads its
    /// `MeshTag` as the emitter's over-life colour (`0xAARRGGBB`,
    /// [`tint_tag`]) rather than as a room light and a sun scale. The colour
    /// ramp changes every frame, per particle, so it cannot be in a material,
    /// and the tag is the only per-instance word.
    ///
    /// The geometry, the meshes and the model's own textures are shared with
    /// every other use of the file; only the materials are rebuilt, as in
    /// [`Self::lookup_in_room`].
    pub fn as_particle(
        &mut self,
        path: &str,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        self.dress(
            path,
            false,
            &[],
            Dress::Creature,
            None,
            true,
            SceneLighting::NONE,
            meshes,
            materials,
        )
    }

    /// The lights a file states, available before any dressing is built with
    /// them.
    ///
    /// [`Self::as_scene`] needs this: its `scene` argument is part of the
    /// dressing's identity, and the lights it is built from are in the same
    /// file. They are a property of the geometry, like the cameras and the
    /// collision hull, so they can be read from the build before any dressing
    /// exists. If the model is not loaded, this requests it as [`Self::lookup`]
    /// does, but asks for the skinned build that [`Self::as_scene`] uses.
    ///
    /// `None` while the file is still loading; an empty list for one that
    /// carries none, which is everything in the world.
    pub fn model_lights(&mut self, path: &str) -> Option<Arc<Vec<vale_assets::world::m2::M2Light>>> {
        let key = geometry_key(path, true);
        match self.geometry.get(&key) {
            Some(built) => Some(Arc::clone(&built.lights)),
            None if self.failed.contains(path) => Some(Arc::default()),
            None => {
                self.request_model(path, true);
                None
            }
        }
    }

    /// The same model as a scene: the login screen's backdrop and character
    /// select's.
    ///
    /// It differs from [`Self::lookup`] in one parameter: it is skinned. A glue
    /// scene wears no skins and stands in no room, so `lookup` looks like the
    /// right call, but `lookup` builds the doodad dressing, which drops the
    /// skeleton and the tints (see [`Self::lookup`] and `loader::RawModel`).
    /// Measured on `UI_MainMenu.m2`, that build has 22 batches and 0 bones: the
    /// login screen's fire and figures would not animate, and its sky would be
    /// drawn at the colour of its first key.
    ///
    /// `Dress::Creature` and no skins: the model's own textures are all it
    /// uses. `vale glue` reports 25 of 25 named textures resolving on this
    /// file, none of them client-supplied.
    ///
    /// `scene` is the model's own lighting, which for these two screens is all
    /// of the shading; see [`SceneLighting`] and
    /// `vale_assets::world::m2::M2Light`. The caller passes it rather than this
    /// function reading it from the model, because the same lights have to
    /// reach the character standing in the scene, which is a different model.
    /// See [`Self::model_lights`].
    pub fn as_scene(
        &mut self,
        path: &str,
        scene: SceneLighting,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        self.dress(
            path,
            true, &[], Dress::Creature, None, false, scene, meshes, materials)
    }

    /// The same model wearing supplied skins, in its bind pose: the one
    /// combination of the four that [`Self::lookup`], [`Self::lookup_scenery`]
    /// and [`Self::dressed`] do not cover.
    ///
    /// `lookup` is the model's own textures unposed, `lookup_scenery` is the
    /// same relit, and `dressed` is supplied skins posed, which an entity
    /// needs because it has a skeleton. This is supplied skins unposed, for
    /// anything that draws a creature without a skeleton: a still picture of
    /// it.
    ///
    /// The skins are the reason this exists, not the pose. Measured over the
    /// whole table: 10,388 of `CreatureDisplayInfo`'s 10,534 rows take their
    /// slot-0 texture from the client (3,555 from a texture variation and
    /// 6,833 from a baked NPC skin), so a creature drawn through `lookup`,
    /// which supplies none, has an empty body slot and draws magenta. That is
    /// 98.6% of creatures.
    ///
    /// The meshes are shared with every other build of the same geometry; a
    /// dressing is only materials, which is why this needs the material store.
    pub fn worn_unposed(
        &mut self,
        path: &str,
        skins: &[String],
        hair: Option<&Appearance>,
        dress: Dress,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        self.dressed_inner(path, false, skins, hair, dress, None, meshes, materials)
    }

    /// The same model wearing the skins a DBC named for it.
    ///
    /// `skins` is [`DisplayModel::skins`]: slot 0 the body (texture type 11 on a
    /// creature, type 1 on a character model), slots 1 and 2 the second and
    /// third creature variations. An empty entry keeps its slot, because a
    /// slot's position is the texture type it fills.
    ///
    /// Needs the material store because a dressing is only materials; the
    /// meshes already exist.
    ///
    /// `hair` is the one slot a DBC cannot name, and is why this takes an
    /// appearance. An NPC in a character model has its body baked, so it comes
    /// through here rather than through [`Self::dressed_as_character`], but its
    /// hairstyle is a separate mesh with a separate texture. No bake can hold
    /// it, and `CreatureDisplayInfoExtra` states it as a style and a colour, as
    /// for a player. Left empty, the hair slot draws magenta; see
    /// [`vale_assets::look::dress::Dressing::hair`].
    ///
    /// [`DisplayModel::skins`]: vale_assets::tables::dbc::DisplayModel::skins
    pub fn dressed(
        &mut self,
        path: &str,
        skins: &[String],
        hair: Option<&Appearance>,
        dress: Dress,
        room: Option<RoomLight>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        // Skinned: an entity is the only thing that asks to be dressed, and an
        // entity is the only thing with joints to bind.
        //
        // `room` is the room the entity is standing in: an entity indoors takes
        // the same interior branch as its furniture, because a room lights
        // whatever is in it and the sun does not reach through a roof.
        // `crate::world::entities::Indoors` decides which room, if any. Only
        // whether there is a room is part of the dressing; the caller puts the
        // colour on each part as its `MeshTag` (see `RoomLight`).
        self.dressed_inner(path, true, skins, hair, dress, room, meshes, materials)
    }

    /// What [`Self::dressed`] and [`Self::worn_unposed`] both are, differing
    /// only in whether the geometry carries its joints.
    #[allow(clippy::too_many_arguments)]
    fn dressed_inner(
        &mut self,
        path: &str,
        skinned: bool,
        skins: &[String],
        hair: Option<&Appearance>,
        dress: Dress,
        room: Option<RoomLight>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        // Slot 3 is the hair mesh's, whatever filled slot 0. The positions are
        // the texture types, so the array has to be long enough to reach it.
        let mut skins = skins.to_vec();
        if let Some(appearance) = hair {
            skins.resize(SKIN_SLOTS.max(skins.len()), String::new());
            skins[3] = hair_key(appearance);
        }
        self.dress(
            path,
            skinned,
            &skins[..],
            dress,
            room,
            false,
            SceneLighting::NONE,
            meshes,
            materials,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn dress(
        &mut self,
        path: &str,
        skinned: bool,
        skins: &[String],
        dress: Dress,
        room: Option<RoomLight>,
        as_particle: bool,
        scene: SceneLighting,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        let key = dressing_key(path, skinned, skins, dress, room.is_some(), as_particle, scene);
        if self.variants.contains_key(&key) {
            self.touch(&key);
            let variant = self.variants.get(&key).expect("touched above");
            return Lookup::Ready(Arc::clone(&variant.assets));
        }
        // A model that will not read fails on the path, whichever build asked
        // for it first.
        if self.failed.contains(path) {
            return Lookup::Failed;
        }
        let geometry_key = geometry_key(path, skinned);
        if !self.geometry.contains_key(&geometry_key) {
            self.request_model(path, skinned);
            return Lookup::Loading;
        }

        // Every skin has to be decoded before the dressing can be built, and a
        // skin that will not read leaves its slot magenta rather than holding
        // the whole entity back. The creature is still drawn, and the magenta
        // slot shows which texture failed.
        let mut supplied: Vec<Option<Handle<Image>>> = Vec::with_capacity(skins.len());
        for skin in skins {
            if skin.is_empty() {
                supplied.push(None);
                continue;
            }
            match self.texture(skin) {
                TextureState::Ready(handle) => supplied.push(Some(handle)),
                TextureState::Loading => return Lookup::Loading,
                TextureState::Failed => supplied.push(None),
            }
        }

        // Stamped as this dressing is built, not only when one is looked up.
        // Otherwise a model loaded a minute ago and dressed just now carries
        // the load time, and the next sweep drops the geometry under a
        // dressing that has just been made. The dressing holds its own handles
        // and still draws, but the next build has to re-read a file that was
        // loaded a moment earlier.
        let now = self.now;
        let geometry = self.geometry.get_mut(&geometry_key).expect("checked above");
        geometry.used = now;
        let geometry = &*geometry;
        // Which appearance variants this wearer shows. The geometry holds every
        // batch the file has (see `read_model`), so this selection is what
        // keeps a character from wearing every hairstyle and every boot at
        // once.
        let all: Vec<u16> = geometry.draws.iter().map(|d| d.geoset).collect();
        let wanted = vale_assets::world::m2::visible_geosets(&all, dress);
        // Which batches this dressing keeps, by index rather than as an
        // iterator, because the merge below needs each one's CPU geometry as
        // well as its parameters; see [`loader::MergeSource`].
        //
        // An empty `wanted` keeps every batch, so a model that numbers its
        // geosets some other way still renders. `M2::visible_batches` uses the
        // same fallback.
        let kept: Vec<usize> = (0..geometry.draws.len())
            .filter(|&i| wanted.is_empty() || wanted.contains(&geometry.draws[i].geoset))
            .collect();
        let draws: Vec<ModelDraw> = kept
            .iter()
            .map(|&index| {
                let params = &geometry.draws[index];
                let mesh = &geometry.meshes[index];
                let bones = Arc::clone(&geometry.bones[index]);
                // Resolve one texture slot: the client-supplied bake or
                // variation where the model asks for one, the model's own file
                // otherwise.
                let resolve = |slot: usize| -> Option<Handle<Image>> {
                    let kind = *geometry.kinds.get(slot)?;
                    match skin_slot(kind) {
                        // The client supplies this one: a texture variation
                        // or a bake, from the display tables.
                        Some(supplied_slot) => supplied.get(supplied_slot)?.clone(),
                        // Type 0: the model names the file itself.
                        None => geometry.own.get(slot).cloned(),
                    }
                };
                let texture = params
                    .texture
                    .and_then(resolve)
                    .unwrap_or_else(|| self.missing.clone());
                // The folded layers use the same lookup. A slot that will not
                // resolve leaves its layer off rather than painting the batch
                // magenta: an environment map is a sheen, and a missing sheen
                // only makes a helmet duller, whereas a missing base texture
                // makes it unrecognisable. See
                // `models::loader::RawDraw::overlays`.
                let overlays = params
                    .overlays
                    .map(|layer| layer.and_then(|(slot, _)| resolve(slot as usize)));
                let mut built = material_for(
                    params,
                    texture,
                    overlays,
                    M2_ALPHA_KEY,
                        // The room's colour is not here: it is per spawn and
                        // is stored on the instance as its `MeshTag`, or every
                        // distinct light would be a distinct material; see
                        // `RoomLight`. The material carries only the branch:
                        // `vertex_lit` with no vertex colours to add means
                        // "lit by the one colour the tag names". An M2 has no
                        // `MOCV`, so there is nothing else for it to read.
                    Vec3::ZERO,
                    room.is_some(),
                    scene,
                );
                // The one difference in the particle build. `.y` tells the
                // shader the instance word is a colour rather than a room, and
                // `vertex_lit` is cleared with it, because the two read the
                // same 24 bits and a tinted batch is not room-lit. The note in
                // `material_for` states this for the track-driven case; this
                // is the same rule with the ramp coming from an emitter
                // instead of from an `M2Color`.
                if as_particle {
                    built.params.particle.y = 1.0;
                    built.params.vertex_lit = 0.0;
                }
                // A batch whose texture moves is not pooled: its material
                // owns one row of `UV_TABLE`, which `follow_uv_animations`
                // writes every frame (or, when the table is full, its own
                // params are rewritten), so a pooled handle would give
                // another batch its matrix. See `Materials::moving`.
                let material = match (params.uv, &geometry.uv_anims) {
                    (Some(index), Some(anims)) => {
                        materials.moving(built, Arc::clone(anims), index, geometry.clip.clone())
                    }
                    _ => materials.intern(built),
                };
                ModelDraw {
                    mesh: mesh.clone(),
                    material,
                    bones,
                    merged: 1,
                    tint: params.tint,
                    baked_tint: params.baked_tint,
                    ground: params.ground,
                    // An M2 has no liquid block.
                    liquid: None,
                }
            })
            .collect();
        let draws = merge_draws(draws, &kept, &geometry.merge, meshes);

        let assets = Arc::new(ModelAssets {
            draws,
            skeleton: geometry.skeleton.clone(),
            inverse_bindposes: geometry.inverse_bindposes.clone(),
            joint_count: geometry.joint_count,
            bounds: geometry.bounds,
            portrait: geometry.portrait,
            body: geometry.body,
            conform: geometry.conform,
            attachments: Arc::clone(&geometry.attachments),
            cues: Arc::clone(&geometry.cues),
            trail: geometry.trail,
            glows: Arc::clone(&geometry.glows),
            collision: Arc::clone(&geometry.collision),
            pick: Arc::clone(&geometry.pick),
            model_sphere: geometry.model_sphere,
            particles: geometry.particles.clone(),
            ribbons: geometry.ribbons.clone(),
            cameras: Arc::clone(&geometry.cameras),
            lights: Arc::clone(&geometry.lights),
            tints: geometry.tints.clone(),
        });
        self.variants.insert(
            key,
            Variant {
                assets: Arc::clone(&assets),
                geometry: geometry_key,
                // The empty slots are not recorded: an empty skin names no
                // texture, so there is nothing for a touch to keep alive.
                skins: skins.iter().filter(|s| !s.is_empty()).cloned().collect(),
                used: self.now,
            },
        );
        Lookup::Ready(assets)
    }

    /// Drop every cached model, dressing and skin that nothing has asked for
    /// since `before`, and answer what went.
    ///
    /// The order is the reverse of the dependency. A dressing holds handles
    /// into its geometry's meshes and its skins' images, so the dressings go
    /// first; the geometry and the textures under them are then unreferenced
    /// by this cache, and whatever is still spawned in the world keeps its own
    /// handles alive. Nothing here can remove an asset from a drawn entity:
    /// Bevy's handles own the asset, and this drops only this cache's handles.
    ///
    /// The failure list is kept: a model that would not read will not read
    /// the second time either, and re-requesting it on every sweep would keep
    /// the loader thread busy failing.
    pub fn evict(&mut self, before: f32) -> Evicted {
        let mut evicted = Evicted::default();
        self.variants.retain(|_, variant| {
            let keep = variant.used >= before;
            evicted.dressings += usize::from(!keep);
            keep
        });
        self.geometry.retain(|_, built| {
            let keep = built.used >= before;
            evicted.models += usize::from(!keep);
            keep
        });
        // `loaded` is the HUD's count of distinct paths, where `geometry`
        // counts builds: a model wanted both skinned and unskinned is two
        // builds and one path. So `loaded` is rebuilt from what survived
        // rather than decremented; decrementing would double-count a path
        // whose other build is still resident.
        if evicted.models > 0 {
            self.loaded = self.geometry.keys().map(|k| path_of(k).to_string()).collect();
        }
        self.textures.retain(|_, texture| {
            // A texture still in flight has a request outstanding on the
            // loader thread; dropping the entry would let the next asker file
            // a second one and then be handed the first one's answer.
            let keep = texture.used >= before || matches!(texture.state, TextureState::Loading);
            evicted.textures += usize::from(!keep);
            keep
        });
        evicted
    }

    /// Forget one path, because its bytes have changed.
    ///
    /// Everything this cache holds is keyed by archive path and kept for the
    /// life of the process. That is correct for files that do not change, and
    /// wrong once a host makes one of those paths return different bytes
    /// through `GameAssets::set_overlay`. `GameAssets::forget_tables` does the
    /// same for tables. Without it, the file on disk is new and every fresh
    /// reader sees it, but the world keeps drawing what was read the first
    /// time.
    ///
    /// Three things are removed:
    ///
    /// * the dressings built from this model, and the geometry under them,
    ///   both the skinned and the unskinned build;
    /// * the texture, if the path names one rather than a model;
    /// * the mark in `failed`. A path requested before its file existed is
    ///   recorded as unreadable for the life of the process, so a model
    ///   written after something asked for it would otherwise never be read.
    ///
    /// It takes effect on the next lookup. Anything already spawned keeps the
    /// handles it holds (Bevy's handles own the asset), so a host that wants
    /// what is on screen to change has to have it built again. A forget
    /// guarantees only that the next build reads the file.
    ///
    /// A read already in flight is dropped and requested again when it
    /// arrives, so a forget during a load does not install a stale build a
    /// frame later.
    ///
    /// `true` when anything was held. Costs a handful of hash lookups.
    pub fn forget(&mut self, path: &str) -> bool {
        let keys = [geometry_key(path, false), geometry_key(path, true)];
        let mut held = false;
        for key in &keys {
            held |= self.geometry.remove(key).is_some();
            self.variants.retain(|_, variant| variant.geometry != *key);
            if self.pending.remove(key) {
                // Still being read, so what comes back is the old file.
                self.refetch.insert(key.clone());
                held = true;
            }
        }
        held |= self.loaded.remove(path);
        held |= self.failed.remove(path);
        held |= self.textures.remove(path).is_some();
        held
    }

    /// Forget everything, for a host that has changed what a whole namespace
    /// returns, such as a project switched while the world is loaded.
    ///
    /// Behaves as [`Self::forget`] in every other respect, including that
    /// what is already drawn keeps what it holds.
    pub fn forget_all(&mut self) -> usize {
        let held = self.geometry.len() + self.textures.len() + self.failed.len();
        self.geometry.clear();
        self.variants.clear();
        self.textures.clear();
        self.failed.clear();
        self.loaded.clear();
        self.refetch.extend(self.pending.drain());
        held
    }

    /// The renderer's clock, for the stamps [`Self::evict`] reads. See
    /// [`Self::now`].
    pub fn tick(&mut self, now: f32) {
        self.now = now;
    }

    /// The same clock, for the loader to write a stamp of its own. Private
    /// rather than `pub(crate)` because [`TextureState`] is visible only
    /// inside `render`; the loader is a child module and sees both.
    fn age(&self) -> f32 {
        self.now
    }

    /// Record a skin the loader has finished with.
    fn set_texture(&mut self, path: String, state: TextureState) {
        let used = self.now;
        self.textures.insert(path, Texture { state, used });
    }

    /// Models, dressings and skins resident now: the three numbers that show
    /// whether eviction is working. Without eviction they only increase.
    pub fn resident(&self) -> (usize, usize, usize) {
        (self.geometry.len(), self.variants.len(), self.textures.len())
    }

    /// The same model wearing a player's skin, composed rather than looked up.
    ///
    /// Display ids 49..57 are the bare race models and have no bake, so the
    /// display tables return an empty slot 0 for them, and without this every
    /// player draws magenta. The composition fills that slot: the skin is
    /// still texture type 1, so nothing below this changes; the key names a
    /// texture that is built instead of read.
    ///
    /// The key is the appearance, so two humans who look alike share one
    /// composite and one dressing, and one who has changed clothes does not.
    ///
    /// Slot 3 is the hair mesh's own texture. It is the one piece of a
    /// character that is not in the composite: the hairstyle is geometry, so
    /// its texture dresses a separate geoset and the M2 asks for it under its
    /// own type. Without it the hair draws magenta.
    pub fn dressed_as_character(
        &mut self,
        path: &str,
        look: &CharacterLook,
        dress: Dress,
        cloak: Option<&str>,
        room: Option<RoomLight>,
        // The lights of the scene the character is standing in:
        // [`SceneLighting::NONE`] for every player in the world, and the
        // backdrop's own lights for the one on the character-select plinth.
        scene: SceneLighting,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        self.as_character(path, true, look, dress, cloak, room, scene, meshes, materials)
    }

    /// The same body without the joints: a still image of a dressed character.
    ///
    /// [`Self::dressed_as_character`] is skinned, because its caller is an
    /// entity and an entity has a skeleton to bind. A picture of a character
    /// has none, and the renderer will not draw a mesh carrying joints with
    /// nothing to bind them to. [`Self::worn_unposed`] exists beside
    /// [`Self::dressed`] for the same reason.
    ///
    /// The cost is the pose: the body stands in the bind pose the file was
    /// authored in, arms out. This is used for pictures of what a garment
    /// paints onto the body; those garments have no model of their own and so
    /// no other picture, and the pose does not matter for them.
    ///
    /// Uses no scene lighting ([`SceneLighting::NONE`]) and no room, for the
    /// same reason as [`Self::worn_unposed`]: whoever renders a still frames it
    /// and lights it.
    pub fn worn_as_character_unposed(
        &mut self,
        path: &str,
        look: &CharacterLook,
        dress: Dress,
        cloak: Option<&str>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        self.as_character(path, false, look, dress, cloak, None, SceneLighting::NONE, meshes, materials)
    }

    /// What [`Self::dressed_as_character`] and [`Self::worn_as_character_unposed`]
    /// both are, differing only in whether the geometry carries its joints.
    #[allow(clippy::too_many_arguments)]
    fn as_character(
        &mut self,
        path: &str,
        skinned: bool,
        look: &CharacterLook,
        dress: Dress,
        cloak: Option<&str>,
        room: Option<RoomLight>,
        scene: SceneLighting,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        let skins = [
            character_key(look),
            String::new(),
            String::new(),
            character_hair_key(look),
            // Slot 4 is the object skin, which on a character is the cloak:
            // the cape is the wearer's own group-15 geoset and the item names
            // only the texture for it.
            cloak.unwrap_or_default().to_string(),
        ];
        self.dress(
            path, skinned, &skins, dress, room, false, scene, meshes, materials,
        )
    }

    /// An item's own model (a pauldron, a helm, a spell's glow) wearing the
    /// skin the item names for it.
    ///
    /// Skinned, like an entity. An attached model follows one of the wearer's
    /// bones, but it is not rigid: a torch's glow plane sits on a billboarded
    /// bone of the torch's own skeleton, and a spell effect's whole appearance
    /// (Arcane Explosion's dome growing from nothing over 0.8 s) is its own
    /// bone animation. An unskinned build drops that skeleton at load
    /// (`read_model`), so every attached model would be frozen in bind pose:
    /// the glow a flat static card, and the dome at its full authored extent
    /// for its whole lifetime. The spawner binds the joints and `animate`
    /// poses them on the attachment's own clock.
    ///
    /// Lit by the room its wearer is standing in, for the same reason as the
    /// wearer's body: a helm is on a head the tavern has darkened, and the
    /// helm's model has no other way to know that. `None` is outdoors, and is
    /// also what a spell effect passes: those are `unlit` glows, so the room
    /// branch would change nothing about them and would cost a second dressing
    /// of every effect model per building.
    pub fn attached(
        &mut self,
        path: &str,
        texture: Option<&str>,
        room: Option<RoomLight>,
        // By the same rule, the scene's lights where there is a scene: a
        // pauldron on the character standing on the character-select plinth is
        // lit by that backdrop's own lights, as its wearer's chest is.
        scene: SceneLighting,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        let mut skins: [String; SKIN_SLOTS] = Default::default();
        skins[4] = texture.unwrap_or_default().to_string();
        self.dress(
            path,
            true,
            &skins,
            Dress::Creature,
            room,
            false,
            scene,
            meshes,
            materials,
        )
    }

    /// Where one client-supplied texture has got to, requesting it if new.
    ///
    /// A key with the [`CHARACTER_PREFIX`] is a composed player skin rather
    /// than an archive path, and goes to the loader as such. It shares this
    /// cache with the file-backed skins so that the dressing, the upload budget
    /// and the Loading/Failed states are the same code for both; the only
    /// difference is which request produces the pixels.
    pub(in crate::render) fn texture(&mut self, path: &str) -> TextureState {
        if let Some(texture) = self.textures.get_mut(path) {
            texture.used = self.now;
            return texture.state.clone();
        }
        let request = match parse_character_key(path) {
            Some(look) if path.starts_with(CHARACTER_HAIR_PREFIX) => Request::CharacterHair {
                key: path.to_string(),
                look,
            },
            Some(look) => Request::Character {
                key: path.to_string(),
                look,
            },
            None => Request::Texture(path.to_string()),
        };
        let state = match &self.loader {
            Some(loader) if loader.request(request) => TextureState::Loading,
            _ => TextureState::Failed,
        };
        self.textures.insert(
            path.to_string(),
            Texture {
                state: state.clone(),
                used: self.now,
            },
        );
        state
    }

    /// Ask the loader for one build of a model, once however many placements
    /// want it.
    fn request_model(&mut self, path: &str, skinned: bool) {
        let key = geometry_key(path, skinned);
        if self.pending.contains(&key) {
            return;
        }
        match &self.loader {
            Some(loader) if loader.request(Request::Model {
                path: path.to_string(),
                skinned,
            }) =>
            {
                self.pending.insert(key);
            }
            // No loader (no game data) or the thread is gone: fail it here
            // rather than leaving every placement of it waiting forever.
            _ => {
                self.failed.insert(path.to_string());
            }
        }
    }

    /// A material for a texture requested by name, with no model behind it:
    /// a loose quad drawn with one of the archives' images.
    ///
    /// `None` until the file comes back from the loader thread, a frame or two
    /// into the session; the caller draws nothing yet and tries again, as with
    /// [`Self::shadow_material`]. Interned like every other material, so every
    /// caller asking for the same image and the same parameters shares one.
    ///
    /// The blob shadow builds its material inline; [`crate::render::selection`]
    /// and `render::reticle` need the same three steps with a different blend,
    /// and share them here.
    pub(crate) fn simple_material(
        &mut self,
        materials: &mut Materials,
        path: &str,
        params: &DrawParams,
    ) -> Option<Handle<M2Material>> {
        let TextureState::Ready(texture) = self.texture(path) else {
            return None;
        };
        let material = material_for(
            params,
            texture,
            // No environment map: a loose quad is one batch and one layer.
            Default::default(),
            M2_ALPHA_KEY,
            Vec3::ZERO,
            false,
            // A loose quad states nothing about its own lighting.
            SceneLighting::NONE,
        );
        Some(materials.intern(material))
    }

    /// The same as [`Self::simple_material`] for a texture this program made,
    /// rather than one the archives carry.
    ///
    /// One caller: [`crate::render::labels`], whose textures are rasterised
    /// strings and have no path. Everything else about the material is
    /// identical, so a label goes through the same pipeline, the same pool and
    /// the same sorted transparent phase as every other blended quad in the
    /// world, and therefore takes the same depth test.
    pub(crate) fn quad_material(
        &mut self,
        materials: &mut Materials,
        texture: Handle<Image>,
        params: &DrawParams,
    ) -> Handle<M2Material> {
        let material = material_for(
            params,
            texture,
            // A rasterised string has no environment map either.
            Default::default(),
            M2_ALPHA_KEY,
            Vec3::ZERO,
            false,
            SceneLighting::NONE,
        );
        materials.intern(material)
    }

    /// The material every unit's shadow is drawn with.
    ///
    /// `None` until the texture has come back from the loader thread, a frame
    /// or two into the session. An entity that appears before then has no
    /// shadow yet, in the same way it has no model yet.
    ///
    /// One material and one mesh for the whole world. When every unit had a
    /// child entity carrying a unit quad with this material, scaled by its
    /// footprint, a hundred units shared one batch set, but the sorted
    /// transparent phase merges only adjacent same-set runs, and a hundred
    /// blobs at a hundred depths are never adjacent, so each was one draw
    /// call. Measured at a Northshire framing, 123 of the frame's 238 blended
    /// draw calls were blob shadows, and removing them alone moved the frame
    /// from 17.4 ms to 14.7 ms. [`crate::render::shadows`] therefore builds the
    /// quads into one mesh, and this function provides the material they
    /// share.
    pub fn shadow_material(&mut self, materials: &mut Materials) -> Option<Handle<M2Material>> {
        if let Some(handle) = &self.blob {
            return Some(handle.clone());
        }
        let TextureState::Ready(texture) = self.texture(SHADOW_BLOB) else {
            return None;
        };
        let mut material = material_for(
            &DrawParams {
                geoset: 0,
                texture: None,
                // Modulate. See `SHADOW_BLOB` for what measured it.
                blend: 5,
                // Nothing lights a shadow: the texel is the darkening.
                unlit: true,
                // The quad faces up and is never seen from below: anything
                // under it is under the ground.
                two_sided: false,
                no_depth_write: false,
                light: BatchLight::Sun,
                liquid: None,
                // Not a `GroundQuad`: `render::shadows` fits the blob to the
                // ground itself; see the `ground` field on `ModelDraw`.
                ground: None,
                tint: None,
                baked_tint: None,
                uv: None,
                // A quad this program builds has no environment map; see
                // `models::loader::RawDraw::overlays`.
                overlays: Default::default(),
            },
            texture,
            Default::default(),
            M2_ALPHA_KEY,
            Vec3::ZERO,
            false,
            // A shadow is unlit and has no scene lighting.
            SceneLighting::NONE,
        );
        // Not fogged. `ambient.w` is the shader's "no fog" flag, and this is
        // the one draw in the world that sets it: fog mixes toward a colour,
        // a modulate draw multiplies by its colour, and the identity for a
        // multiply is white. See `m2.wgsl`.
        material.params.ambient.w = 1.0;
        // The client draws the blob as a ground decal. `render::shadows`
        // samples it onto the ground in one mesh, using the same heightfield
        // sample as `render::decals`, rather than spawning a `GroundDecal`.
        let handle = materials.intern(material);
        self.blob = Some(handle.clone());
        Some(handle)
    }

    /// The material one chain effect's strip is drawn with: a bare
    /// `Textures\SpellChainEffects\*.blp` and nothing else.
    ///
    /// Built like [`Self::shadow_material`], and kept in the cache rather than
    /// in the pass that uses it for the same reason: the texture load is
    /// asynchronous and the cache handles the wait. `None` while the BLP is
    /// still being read, so a beam starts a frame or two late rather than
    /// being drawn magenta.
    ///
    /// Blend 4, not 3. `SpellChainEffects`' textures carry their shape in the
    /// alpha channel (the bolt is a bright core on a black field with a soft
    /// alpha edge), so an `add` (3) would show the black as a rectangle where
    /// an `alpha-add` (4) does not. This is inferred from the textures, not
    /// measured: no table names a blend mode for these, because no `M2` render
    /// flag is involved.
    pub fn beam_material(
        &mut self,
        path: &str,
        materials: &mut Materials,
    ) -> Option<Handle<M2Material>> {
        if let Some(handle) = self.beams.get(path) {
            return Some(handle.clone());
        }
        let TextureState::Ready(texture) = self.texture(path) else {
            return None;
        };
        let material = material_for(
            &DrawParams {
                geoset: 0,
                texture: None,
                blend: 4,
                // A bolt is its own light; nothing in the world dims it.
                unlit: true,
                // Drawn from both sides: the strip is turned to face the
                // camera every frame, and with back faces culled the bolt
                // would vanish as the view swings past it.
                two_sided: true,
                no_depth_write: true,
                light: BatchLight::Sun,
                liquid: None,
                ground: None,
                tint: None,
                baked_tint: None,
                uv: None,
                // A quad this program builds has no environment map; see
                // `models::loader::RawDraw::overlays`.
                overlays: Default::default(),
            },
            texture,
            Default::default(),
            M2_ALPHA_KEY,
            Vec3::ZERO,
            false,
            SceneLighting::NONE,
        );
        let handle = materials.intern(material);
        self.beams.insert(path.to_string(), handle.clone());
        Some(handle)
    }

    /// How many models are loaded and how many would not read, for the HUD.
    ///
    /// Distinct paths, not builds: a model wanted both skinned and unskinned
    /// is one model meshed twice.
    pub fn counts(&self) -> (usize, usize) {
        (self.loaded.len(), self.failed.len())
    }

}

/// What marks a skin key as a composed player skin rather than an archive
/// path. A leading NUL cannot appear in an MPQ path, so the two namespaces
/// cannot collide however the game data is patched.
const CHARACTER_PREFIX: &str = "\u{0}character:";

/// The same, for the hair mesh's own texture: a separate key because it is a
/// separate image filling a separate slot, and because a change of hairstyle
/// must not reuse the body composite.
const CHARACTER_HAIR_PREFIX: &str = "\u{0}characterhair:";

/// The cache key for one player's composed body texture.
///
/// The appearance and the equipment, so two players who look alike and are
/// dressed alike share one composite and one dressing. On a populated realm
/// that is many players, since a race has a handful of faces and a starting
/// zone has one set of gear. A player who changes clothes gets a new key.
fn character_key(look: &CharacterLook) -> String {
    let a = &look.appearance;
    let mut key = format!(
        "{CHARACTER_PREFIX}{},{},{},{},{},{},{}",
        a.race, a.gender, a.skin, a.face, a.hair_style, a.hair_colour, a.facial_hair
    );
    for (display_id, inventory_type) in &look.equipment {
        key.push_str(&format!(";{display_id}.{inventory_type}"));
    }
    key
}

/// The same look, naming the hair mesh's texture instead of the body's.
///
/// Equipment cannot change it (a helmet hides hair rather than recolouring
/// it), so the key leaves out the equipment, and two players with the same
/// hairstyle share one texture however differently they are dressed.
fn character_hair_key(look: &CharacterLook) -> String {
    hair_key(&look.appearance)
}

/// The hair texture key from the appearance alone, which is all it depends
/// on.
///
/// A character-model NPC has no [`CharacterLook`] (its body is baked rather
/// than composed) and still wears a hair mesh, so both players and NPCs use
/// this function. See [`ModelCache::dressed`].
fn hair_key(appearance: &Appearance) -> String {
    character_key(&CharacterLook {
        appearance: *appearance,
        equipment: Vec::new(),
    })
    .replace(CHARACTER_PREFIX, CHARACTER_HAIR_PREFIX)
}

/// The inverse, so the loader needs no second channel to be told what to build.
fn parse_character_key(key: &str) -> Option<CharacterLook> {
    let body = key
        .strip_prefix(CHARACTER_PREFIX)
        .or_else(|| key.strip_prefix(CHARACTER_HAIR_PREFIX))?;
    let mut parts = body.split(';');
    let n: Vec<u8> = parts
        .next()?
        .split(',')
        .map(|s| s.parse().ok())
        .collect::<Option<Vec<u8>>>()?;
    let [race, gender, skin, face, hair_style, hair_colour, facial_hair] = n[..] else {
        return None;
    };
    let mut equipment = Vec::new();
    for item in parts {
        let (display_id, inventory_type) = item.split_once('.')?;
        equipment.push((display_id.parse().ok()?, inventory_type.parse().ok()?));
    }
    Some(CharacterLook {
        appearance: Appearance {
            race,
            gender,
            skin,
            face,
            hair_style,
            hair_colour,
            facial_hair,
        },
        equipment,
    })
}

/// The joints of the `SkinnedMesh` for one batch of a spawned model, or `None`
/// for a batch with no bones (a tree, or a model with no skeleton).
///
/// `joints` is the whole model's joint list in bone order with the identity
/// joint last, as every spawner builds it; this selects
/// [`ModelDraw::bones`], the batch's own subset, from it.
///
/// It is one function because four spawners build a model's parts:
/// `world::entities::spawn`, `entities::effects`, `render::glue` and
/// `render::portraits` (which builds a model's parts on its own render layer
/// for the unit-frame faces). When each had its own copy of these four lines,
/// the change that introduced bone subsets updated only two of them. The login
/// and character-select screens, and the portraits, then bound the model's
/// whole skeleton to meshes whose joint indices were subset-local, which poses
/// every vertex off the wrong bone: splayed shoulders, a twisted weapon and a
/// body that clips through itself. No test or headless check renders the glue
/// screens or a portrait, so none of them detected it.
///
/// `every_skinned_mesh_is_built_through_skin_for` in this module's tests reads
/// the call sites from the source and fails if a further spawner builds a
/// skinned mesh without this function.
pub fn skin_for(draw: &ModelDraw, joints: &[Entity]) -> Option<SkinnedMeshFor> {
    let bones: Vec<Entity> = draw
        .bones
        .iter()
        .filter_map(|&bone| joints.get(usize::from(bone)).copied())
        .collect();
    (!bones.is_empty()).then_some(bones)
}

/// Whether a batch names only the bones it is bound to; see
/// [`loader::RawDraw::bones`], and `VALE_NO_MERGE` for why an optimisation
/// like this has an off switch (`VALE_NO_BONE_SUBSET`).
///
/// Read on the load side, not at spawn. Branching in [`skin_for`] alone gives
/// a mesh whose vertex indices are already subset-local the model's whole
/// skeleton, so every vertex is posed off the wrong bone. A switch that
/// renders wrongly makes the A/B comparison it exists for meaningless.
///
/// Read once and cached: this is called on the loader thread, once per batch
/// per model.
pub(crate) fn subsetting() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VALE_NO_BONE_SUBSET").is_none())
}

/// What [`skin_for`] returns: the joint entities one batch is bound to, in the
/// order its vertex indices expect them.
pub type SkinnedMeshFor = Vec<Entity>;

/// Concatenate a dressing's batches that share a material into one mesh.
///
/// In this renderer, only moving models cost per mesh instance: a static
/// doodad is extracted once and then costs nothing (615 of them account for
/// 0.1 ms of frame time), while a skinned instance under a moving entity is
/// re-propagated, re-extracted and re-skinned every frame. A dressed level-60
/// character draws about twenty of those, and most share a material: the
/// body's opaque batches all use the one composed skin and its alpha-keyed
/// ones all use the second texture, because the composite combines them.
///
/// So the batches that may be merged ([`loader::MergeSource`] states which,
/// and why each exclusion is needed for correct rendering rather than for
/// cost) are grouped by their interned material and concatenated. The merged
/// draw takes the position of the group's first member, so the file's batch
/// order is unchanged for everything that was not merged. This matters for
/// the translucent batches, none of which are ever in a group.
///
/// Three things must be right, and each would otherwise draw a wrong picture
/// rather than a slow one:
///
/// * the vertex indices are per batch, so each member's are offset by the
///   vertices already written;
/// * the joint indices are per batch too (they index that batch's own bone
///   subset, [`loader::RawDraw::bones`], not the skeleton), so the merged mesh
///   takes the union of the members' subsets and every member's indices are
///   remapped into it;
/// * a group of one is not merged. A second copy of a mesh the geometry
///   already holds would cost memory and save nothing, so a lone batch keeps
///   the shared handle.
fn merge_draws(
    draws: Vec<ModelDraw>,
    kept: &[usize],
    merge: &[Option<Arc<loader::MergeSource>>],
    meshes: &mut Assets<Mesh>,
) -> Vec<ModelDraw> {
    // Group by material, in first-appearance order. A linear scan rather than a
    // map: a dressing is a few dozen batches and this runs once per dressing.
    let mut groups: Vec<(AssetId<M2Material>, Vec<usize>)> = Vec::new();
    for (slot, &index) in kept.iter().enumerate() {
        if merge.get(index).and_then(Option::as_ref).is_none() {
            continue;
        }
        let id = draws[slot].material.id();
        match groups.iter_mut().find(|(key, _)| *key == id) {
            Some((_, members)) => members.push(slot),
            None => groups.push((id, vec![slot])),
        }
    }
    groups.retain(|(_, members)| members.len() > 1);
    // `VALE_NO_MERGE` turns the merge off, for the same reason
    // `VALE_NO_WIREFRAME` exists: an optimisation that is on by default must
    // be possible to switch off. Two runs differing in this variable are the
    // only way to measure what the merge saves, and without it a dressed
    // character is about twenty moving mesh instances.
    if groups.is_empty() || std::env::var_os("VALE_NO_MERGE").is_some() {
        return draws;
    }

    // Which merged draw, if any, each slot has become: `None` for a batch that
    // was not merged, `Some(group)` for the first member of a group; the other
    // members are dropped.
    let mut merged_at: Vec<Option<usize>> = vec![None; draws.len()];
    let mut dropped: Vec<bool> = vec![false; draws.len()];
    let mut built: Vec<ModelDraw> = Vec::with_capacity(groups.len());
    for (group, (_, members)) in groups.iter().enumerate() {
        let sources: Vec<&loader::MergeSource> = members
            .iter()
            .map(|&slot| {
                merge[kept[slot]]
                    .as_deref()
                    .expect("only mergeable slots are grouped")
            })
            .collect();
        // The union of the members' bone subsets, and the map into it.
        let mut union: Vec<u16> = Vec::new();
        for source in &sources {
            for &bone in &source.bones {
                if !union.contains(&bone) {
                    union.push(bone);
                }
            }
        }
        union.sort_unstable();
        let slot_of = |bone: u16| union.binary_search(&bone).unwrap_or(0) as u16;

        let vertices: usize = sources.iter().map(|s| s.positions.len()).sum();
        let triangles: usize = sources.iter().map(|s| s.indices.len()).sum();
        let mut positions = Vec::with_capacity(vertices);
        let mut normals = Vec::with_capacity(vertices);
        let mut uvs = Vec::with_capacity(vertices);
        let mut joints = Vec::with_capacity(vertices);
        let mut weights = Vec::with_capacity(vertices);
        let mut indices = Vec::with_capacity(triangles);
        for source in &sources {
            let base = positions.len() as u32;
            positions.extend_from_slice(&source.positions);
            normals.extend_from_slice(&source.normals);
            uvs.extend_from_slice(&source.uvs);
            weights.extend_from_slice(&source.weights);
            for vertex in &source.joints {
                joints.push(vertex.map(|local| {
                    // A local index past the batch's own subset cannot occur,
                    // because `batch_draw` builds the two together. If one did,
                    // this falls back to the first bone rather than panicking
                    // on a malformed file.
                    slot_of(source.bones.get(usize::from(local)).copied().unwrap_or(0))
                }));
            }
            indices.extend(source.indices.iter().map(|i| i + base));
        }
        let mut mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            bevy::mesh::VertexAttributeValues::Uint16x4(joints),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
        mesh.insert_indices(bevy::mesh::Indices::U32(indices));

        let first = members[0];
        built.push(ModelDraw {
            mesh: meshes.add(mesh),
            material: draws[first].material.clone(),
            bones: union.as_slice().into(),
            merged: members.len(),
            // Every one of these is `None` on a mergeable batch by
            // construction; see [`loader::MergeSource`].
            tint: None,
            baked_tint: None,
            ground: None,
            liquid: None,
        });
        merged_at[first] = Some(group);
        for &slot in &members[1..] {
            dropped[slot] = true;
        }
    }

    let mut out = Vec::with_capacity(draws.len());
    for (slot, draw) in draws.into_iter().enumerate() {
        if dropped[slot] {
            continue;
        }
        match merged_at[slot] {
            Some(group) => out.push(built[group].clone()),
            None => out.push(draw),
        }
    }
    out
}

/// Which client-supplied slot an M2 texture type fills.
///
/// Types 1 and 11 share slot 0: a creature declares its body as type 11 and a
/// character model declares it as type 1, and no model is both. 12 and 13 are
/// creature texture variations 2 and 3. Type 6 is the hair mesh's own texture,
/// the one part of a character's appearance that is not in the body
/// composite: it dresses separate geometry, so it needs a separate slot
/// rather than a region. `None` means the model supplies this texture itself
/// (type 0) or the client has nothing for it.
pub fn skin_slot(kind: u32) -> Option<usize> {
    match kind {
        1 | 11 => Some(0),
        12 => Some(1),
        13 => Some(2),
        6 => Some(3),
        // Type 2 is the object skin, and it is filled from an item rather
        // than from the model: a pauldron's M2 declares it with no filename and
        // `ItemDisplayInfo::modelTexture` names the file, in the same
        // `Item\ObjectComponents\<slot>\` directory the model came from. The
        // wearer's own cape geoset asks for it too: a cloak is geometry the
        // character already has and a texture the item supplies.
        2 => Some(4),
        _ => None,
    }
}

/// How many client-supplied skin slots there are; see [`skin_slot`].
pub const SKIN_SLOTS: usize = 5;

pub struct ModelPlugin;

impl Plugin for ModelPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/m2.wgsl");
        embedded_asset!(app, "shaders/m2_prepass.wgsl");
        // The vertex stages this crate owns, and the one function they share.
        //
        // `wind.wgsl` is loaded with `load_shader_library!`, not
        // `embedded_asset!`. Nothing names it as a `ShaderRef`: it is only
        // `#import`ed, by the two vertex stages below. An `embedded_asset!` is
        // registered and then loaded on demand, so a module nobody asks for by
        // name is never loaded and never registers its `#define_import_path`,
        // and every shader that imports it waits for it indefinitely.
        //
        // That failure is silent: no compile error and no missing-asset
        // warning. The pipeline never becomes ready, so the phase draws
        // nothing. When this happened, every M2 in the world (the trees, the
        // buildings' batches and every character) vanished, with a clean log
        // and a plausible mesh count. `render::sky` describes the same issue
        // for `atmosphere.wgsl`.
        bevy::shader::load_shader_library!(app, "shaders/wind.wgsl");
        embedded_asset!(app, "shaders/m2_vertex.wgsl");
        embedded_asset!(app, "shaders/m2_prepass_vertex.wgsl");
        app.add_plugins(MaterialPlugin::<M2Material>::default())
            .init_resource::<ModelCache>()
            .init_resource::<MaterialPool>()
            .init_resource::<UvAnimations>()
            // The CVar is read before the first texture is built: the sampler
            // is made on the loader thread, which has no world to ask.
            .add_systems(Startup, (loader::read_anisotropy, start_loader).chain())
            .add_systems(Startup, insert_uv_table)
            // `follow_uv_animations` writes the texture matrices into
            // `UV_TABLE` after the loads land, so a batch dressed this frame
            // is never drawn before its matrix is written: for a scroll that
            // would be one frame of the texture at rest, and for a rotation a
            // jump.
            .add_systems(Update, (receive_models, follow_uv_animations).chain());
    }
}

/// Start the loader thread, once, with the archive directory the app was given.
fn start_loader(
    mut cache: ResMut<ModelCache>,
    assets: Res<GameAssets>,
    mut images: ResMut<Assets<Image>>,
) {
    cache.loader = Some(Loader::start(assets.gamedata_dir.clone(), assets.overlay_cell()));
    // One magenta image for the whole world rather than one per unfilled slot:
    // a creature model declares two or three textures it does not carry, and
    // there are ten thousand display ids.
    cache.missing = images.add(missing_image());
}


pub(crate) mod loader;
pub mod material;

// Re-exported flat, so every `render::models::M2Material` elsewhere in the
// crate keeps working: this split is internal organisation, not a new API.
pub(crate) use loader::*;
pub use material::*;
// The types the cache uses directly, which stay internal to `models`.
use loader::{material_for, receive_models, Loaded, Loader, Request};

#[cfg(test)]
mod tests;
