//! M2 models: from an archive path to a list of (mesh, material) draws.
//!
//! ```text
//! mod.rs       the cache, the mesh build, and everything a caller sees
//! loader.rs    …and the thread behind it: one archive chain, a channel, and
//!              the M2 and BLP reads that must not happen on a frame
//! material.rs  …and what a batch is *drawn* with: the blend table, the two
//!              sided-ness flags and the one material every untextured slot
//!              shares
//! ```
//!
//! This is the half of the doodad pass that knows nothing about placement — a
//! model is loaded once, cached by path, and drawn wherever the world asks for
//! it. The entity pass reuses all of it; what it adds is a skeleton and a pose,
//! not a different way of reading the file.
//!
//! **The manual instance grouping is gone, not ported.** The WebGL renderer
//! built per-model instance buffers by hand (`ANGLE_instanced_arrays`, one call
//! per model+batch for the whole loaded world) because one draw call per
//! placement was tens of thousands a frame. Bevy batches by mesh and material
//! automatically, so a placement is an ordinary entity — which is also what
//! gives per-instance frustum culling, something the hand-built buffers could
//! not do without rebuilding themselves every frame.
//!
//! ## One mesh per batch
//!
//! An `M2Batch` is a range of the model's index buffer plus a material, and
//! Bevy draws a whole `Mesh3d`, not a range of one. So each batch becomes its
//! own mesh with the vertices it uses remapped into a buffer of its own — the
//! same trade the terrain pass makes, and it buys the same thing: a per-batch
//! `Aabb` for the cull. Most doodads have one or two batches.
//!
//! ## The loader is a thread, not a task per model
//!
//! Opening the archive chain is ~19 MPQs and takes a moment, and
//! `Assets::read` needs `&mut`. A task per model would either open a chain each
//! (a tile is ~130 distinct models) or queue every model behind the shared lock
//! the session and the tile loads are already using. One thread owning one chain
//! and pulling from a channel costs neither.
//!
//! ## The skinning attributes belong to the *use*, not to the file
//!
//! 17 of a Darkshire tile's 128 doodad models carry a skeleton — windmills,
//! banners — and a doodad is drawn in its bind pose by design. So the same M2 is
//! two different meshes depending on who asked for it, and the cache is keyed on
//! that as well as on the path.
//!
//! It is not an optimisation. Bevy chooses the **pipeline** from the mesh's
//! attributes (`is_skinned(layout)`) and the **bind group** from whether the
//! entity has an extracted skin, and if those two disagree wgpu rejects the draw
//! and Bevy quits the app:
//!
//! ```text
//!   The BindGroupLayout 'mesh_layout' of BindGroup 'model_only_mesh_bind_group'
//!   is not compatible with 'skinned_mesh_layout' of 'opaque_mesh_pipeline'
//! ```
//!
//! A doodad sharing a creature's mesh therefore takes the whole renderer down
//! the moment one comes into view — and the geometry is identical, so nothing
//! about the picture says why.
//!
//! ## Geometry is cached by path; a *dressing* is cached by path and skins
//!
//! A creature's skin is not in its M2 — the model declares texture type 11 (or
//! type 1 for a character model) with no filename and the client supplies it
//! from `CreatureDisplayInfo`, so one `Wolf.m2` is drawn grey, black and white by
//! three display ids. In WebGL that cost nothing: the texture was bound at draw
//! time. Here a texture is part of the *material*, so those three need three
//! materials — but the geometry is identical and the meshes are the expensive
//! half, so [`Geometry`] is keyed by path and only the materials are rebuilt per
//! dressing. A wolf and its two cousins are one set of vertex buffers.

use crate::assets::GameAssets;
use crate::axes;
// Aliased: `vale_assets::Assets` is the MPQ chain and `bevy::prelude::Assets`
// is the asset store, and both are in scope here.
use vale_assets::{
    world::blp,
    look::character::{Appearance, CharSections, Composite},
    // `CharacterLook` is the asset crate's, because *what* a player is dressed
    // as is a game rule and not a rendering decision — see `vale_assets::look::dress`.
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

/// The alpha-key cutoff for an M2 material (blend mode 1) — **224/255, the same
/// number the WMO path already used**, and the client's own rather than a guess.
///
/// It was 0.5 for as long as nothing had measured it, and 0.5 is not a number
/// that appears anywhere in the client. The client keeps a flat
/// **alpha-reference table indexed by blend mode**, and whenever the blend
/// state is written it derives the alpha reference from it in the same call:
///
/// ```text
/// 0, 224, 1, 1, 1, 1, 1, 0, 0, 0, 0
/// ^   ^   ^-------------^
/// |   |   every translucent mode: discard alpha 0 and nothing else
/// |   alpha key
/// opaque
/// ```
///
/// So there is one cut in the engine and it is 224, which is why the WMO
/// constant and this one are now the same constant with two names rather than
/// two policies. The second route to the same number corroborates it:
/// the reference for a blend-1 material is set to
/// `material.alpha * 224.0`, so 224 is the ceiling that a
/// material's own alpha scales.
///
/// **The attribution that used to be here is retracted.** This comment claimed
/// the constant was what fixed "the wall of blue squares an Evocation stood
/// in", on the reasoning that `SPELLS\CLOUDS.BLP` decodes with `alphaDepth = 0`
/// and 0% transparent texels, so under an 0.5 cut every one of its quads drew
/// as a hard opaque tile. Every clause of that is true and the conclusion was
/// wrong: those emitters name a **geometry model** and the client never draws
/// a quad for them at all, so their texture slot is not read and no alpha
/// reference could have decided anything about them (see
/// `crate::render::particles::model_particles`). What the cut happened to do
/// was silence three of Evocation's five — which read as progress and was
/// really the bug hiding behind a second one.
///
/// The constant itself stands on the table above, which is a measurement
/// rather than an inference. What it is *worth* on screen is now unclaimed.
///
/// The rest of the population is alpha-keyed *art* — foliage, hair cards, the
/// window lattices — whose alpha is near-binary, so the two cuts pick the same
/// texels. That is the claim to look at if a tree ever reads thin.
pub const M2_ALPHA_KEY: f32 = 224.0 / 255.0;

/// **…and the cut every *translucent* mode takes, which is not zero.**
///
/// The same table puts **1** against blend modes 2 through 6 and 0
/// against opaque: a blended draw discards a fully transparent fragment and
/// nothing else. It costs nothing and it is not decoration — a particle whose
/// over-life ramp has run to zero is still a quad in the depth-sorted
/// transparent list, and discarding it is what keeps the tail of every emitter
/// in the world out of the blend.
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
    /// **Which of the model's joints this batch's `SkinnedMesh` names** — see
    /// [`super::models::loader::RawDraw::bones`]. Empty for scenery and for a
    /// building, which have no skeleton at all.
    pub bones: Arc<[u16]>,
    /// **How many of the model's batches this one draw is**, which is 1 for
    /// everything the file states and more for a merge — see
    /// [`super::models::loader::MergeSource`]. Carried only so the scene tab
    /// can say what the merge is worth without counting entities.
    pub merged: usize,
    /// Which of the model's colour and transparency tracks fade this batch, or
    /// `None` for everything that does not fade — which is the world.
    ///
    /// **A tinted batch's `MeshTag` is its colour, not its room**, so a spawner
    /// that hands one out has to write a tag: see [`tint_tag`], and see
    /// `M2Params::particle`'s `.y` for the material half. Set only on the
    /// *skinned* build of a model, which is the one with a clock to sample on.
    pub tint: Option<BatchTint>,
    /// **…and the value, for a batch that does not animate**: the tint sampled
    /// once at the start of the timeline, ready to be written as a `MeshTag`.
    ///
    /// `Some` only on the **unskinned** build — the doodads — where there is no
    /// clock to sample on and the track is a constant anyway. The skinned build
    /// leaves this `None` and writes the tag per frame from the entity's own
    /// animation; see `world::entities::pose`.
    ///
    /// A spawner that draws a doodad **must** prefer this over its own room-light
    /// tag when it is set: the material is specialised as tinted either way, so a
    /// tag holding a room light would be read as a colour. See
    /// `render::doodads`, which is the one caller.
    pub baked_tint: Option<u32>,
    /// **Whether this batch is a flat rectangle lying in the model's own
    /// ground plane** — see [`vale_assets::world::m2::M2::ground_quad`].
    ///
    /// A spawner that draws on the floor hands this to
    /// [`crate::render::decals`] instead of spawning the mesh, and gets a
    /// mesh that follows the ground rather than a plane held at the caster's
    /// feet. `None` for every batch in the world but the 120 the spell-effect
    /// population authors this way.
    pub ground: Option<vale_assets::world::m2::GroundQuad>,
    /// **Which liquid this batch is a surface of**, or `None` — only a WMO's
    /// `MLIQ` is ever one, since an M2 has no liquid block at all.
    ///
    /// Carried so the spawner can mark the entity, and there are two reasons for
    /// that. The water is switchable on its own
    /// ([`crate::render::tuning::WorldTuning::water`]): a canal inside
    /// Stormwind and a lake in Elwynn are the same surface with the same open
    /// question against them, and a switch that reached only one of the two
    /// would answer half of it. And [`crate::render::water`] keeps every one of
    /// them animated and coloured, which needs the kind rather than the bare
    /// fact — it decides which flipbook and which two light bands.
    pub liquid: Option<vale_assets::world::wmo::Liquid>,
}

/// A model dressed in one particular set of skins: what a placement or an entity
/// needs in order to draw it.
pub struct ModelAssets {
    /// One per visible batch. Shared by every placement of this dressing.
    pub draws: Vec<ModelDraw>,
    /// `None` for scenery, which is most of the world — a tree has no bones.
    pub skeleton: Option<Arc<M2Skeleton>>,
    /// All identity, one per joint. M2 vertices are already in model space and
    /// `M2Skeleton::pose` handles the pivot itself, so there is nothing for an
    /// inverse bindpose to undo — see the note in `entities.rs`.
    pub inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    /// `bones + 1`. The extra one is the identity joint that weightless vertices
    /// ride on; without it the skinning shader collapses them to the origin.
    pub joint_count: usize,
    /// The model's **own declared** bounding box, in Bevy's axes.
    ///
    /// This is the client's culling volume, authored to cover every frame of
    /// every animation — `vale anim` measures the whole bestiary against it
    /// and 766 of 768 animations stay inside. Using it rather than the bind
    /// pose's is what keeps a running creature from being culled mid-stride.
    pub bounds: Option<Aabb>,
    /// **Where to stand to take this model's picture**, in the *file's* axes —
    /// its own portrait camera, or a framing derived from its bind pose. See
    /// [`vale_assets::look::portrait`] for the rule and the census behind it, and
    /// [`crate::render::portraits`], which is the only reader.
    ///
    /// Resolved at load rather than on demand for the same reason
    /// [`Self::bounds`] is: it wants the model's vertices as well as its camera
    /// block, and by the time a unit frame asks, the `M2` is long gone.
    pub portrait: vale_assets::look::portrait::Framing,
    /// …and **where to stand to draw the whole of it**, in the same axes — its
    /// own character-info camera, or a framing derived from its bind pose. The
    /// counterpart of [`Self::portrait`] and resolved beside it; see
    /// [`vale_assets::look::portrait::body_framing`] for the rule and
    /// [`crate::render::paperdoll`], which is the only reader.
    pub body: vale_assets::look::portrait::Framing,
    /// **Whether this model leans with the ground under it**, off the M2
    /// header's own `GlobalModelFlags` — see [`vale_assets::look::conform`], which
    /// is the whole rule. `Level` for every character model in the game and for
    /// all of the scenery; the mounts and the quadrupeds are what this is for.
    pub conform: vale_assets::look::conform::Conform,
    /// Where other models hang off this one: a bone and a point in its frame.
    /// Empty for scenery; every character model carries shoulders and a helm.
    pub attachments: Arc<Vec<M2Attachment>>,
    /// The sound cues its animations carry — a laugh at the laugh's moment.
    /// See `sound::cues`.
    pub cues: Arc<vale_assets::world::m2::SoundCues>,
    /// **The lights it carries** — a lamppost's glow quad, a sconce's flame.
    /// Empty for almost everything. See [`crate::render::lamps`], which is what
    /// turns one into a light, and `vale_assets::world::glow`, which is what
    /// says a batch is one.
    pub glows: Arc<Vec<crate::render::lamps::ModelGlow>>,
    /// The model's *solid* triangles, in model space — a different set from
    /// [`Self::draws`] and empty for most of the world. Shared by every
    /// placement of the model, exactly as a building's hull is, and transformed
    /// per placement by the doodad pass.
    ///
    /// A property of the file and not of the dressing, so every variant of one
    /// model hands back the same `Arc`: what a creature is wearing does not
    /// change what a stride walks into.
    pub collision: Arc<vale_assets::world::collision::CollisionMesh>,
    /// The model's **drawn** triangles, kept on the CPU in model space, for the
    /// mouse pick's narrow phase — see [`vale_assets::look::pick`].
    ///
    /// A different set from [`Self::collision`] and asked a different question:
    /// the hull is what a *stride* walks into and most of the world has none,
    /// where this is the silhouette a *pointer* has to land on and every model
    /// has one. Shared per path on the same argument the hull is — what a
    /// creature is wearing does not move its outline enough for a click to
    /// notice, and one copy per dressing would be the same arrays several times
    /// over.
    pub pick: Arc<vale_assets::look::pick::PickMesh>,
    /// **The model's own header sphere**, in model yards — what the pick's
    /// broad phase uses when the animation the unit is playing states none, and
    /// the whole answer for anything with no skeleton to ask.
    ///
    /// The file's own `boundingRadius` rather than anything computed from
    /// [`Self::bounds`]: those are two different numbers and only one of them is
    /// what the reference reads.
    pub model_sphere: vale_assets::look::pick::Sphere,
    /// The particle emitters — a torch's flame, a wisp's dust — with their
    /// materials already interned, or `None` for the emitterless world. A
    /// property of the file like the hull above: a particle's texture is the
    /// model's own, so every dressing shares one set.
    pub particles: Option<Arc<crate::render::particles::ParticleSet>>,
    /// The **ribbon** trails — a weapon's streak, a missile's tail — on the
    /// same terms as the emitters above: a property of the file, one set
    /// shared by every dressing.
    pub ribbons: Option<Arc<crate::render::ribbons::RibbonSet>>,
    /// **The model's own cameras** — where to stand to look at it.
    ///
    /// This line used to say "empty for everything in the world" and that was
    /// measured false the moment anything asked: **401 of the 408 creature
    /// models that decode carry two**, a portrait camera and a character-info
    /// one. What was true is that nothing *read* them outside the glue screens.
    /// See [`Self::portrait`], which is the second reader and the reason the
    /// count is now known.
    pub cameras: Arc<Vec<vale_assets::world::m2::M2Camera>>,
    /// …and the model's own **lights**, on the same terms: a property of the
    /// file, shared by every dressing of it, empty for everything in the world.
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
/// Held until nothing has asked for it in [`crate::render::residency::IDLE_SECS`]
/// — see [`ModelCache::evict`]. The meshes and images hanging off it are the
/// GPU memory, and this map is what owns them once the last placement has
/// walked out of range.
struct Geometry {
    /// [`ModelCache::now`] when a dressing last asked for this build.
    used: f32,
    /// One per visible batch, parallel to [`Self::draws`].
    meshes: Vec<Handle<Mesh>>,
    draws: Vec<DrawParams>,
    /// Each batch's own bone subset, parallel to the two above — see
    /// [`super::models::loader::RawDraw::bones`], which is where the argument
    /// for it is. **Not** in [`DrawParams`]: that is the material's interning
    /// key, and two batches with identical materials and different bones must
    /// still share one material.
    bones: Vec<Arc<[u16]>>,
    /// …and each batch's CPU geometry, where it may be merged — see
    /// [`super::models::loader::MergeSource`]. `None` per batch that may not
    /// be, and empty for a build with no skeleton.
    merge: Vec<Option<Arc<crate::render::models::loader::MergeSource>>>,
    /// The texture *type* of each of the model's own texture slots. Type 0 is a
    /// filename inside the model; everything else the client supplies.
    kinds: Vec<u32>,
    /// The model's own textures, magenta where it names one the archive lacks.
    own: Vec<Handle<Image>>,
    skeleton: Option<Arc<M2Skeleton>>,
    inverse_bindposes: Handle<SkinnedMeshInverseBindposes>,
    joint_count: usize,
    bounds: Option<Aabb>,
    /// Where a portrait of it is taken from — see [`ModelAssets::portrait`].
    portrait: vale_assets::look::portrait::Framing,
    /// …and where the whole of it is — see [`ModelAssets::body`].
    body: vale_assets::look::portrait::Framing,
    /// The M2 header's conform mode — see [`ModelAssets::conform`].
    conform: vale_assets::look::conform::Conform,
    attachments: Arc<Vec<M2Attachment>>,
    /// The sound cues, per sequence — shared by every dressing of the file,
    /// like the hull.
    cues: Arc<vale_assets::world::m2::SoundCues>,
    /// The lights it carries — see [`ModelAssets::glows`].
    glows: Arc<Vec<crate::render::lamps::ModelGlow>>,
    collision: Arc<vale_assets::world::collision::CollisionMesh>,
    /// The **drawn** triangles kept on the CPU, for the mouse pick's narrow
    /// phase — see [`ModelAssets::pick`]. Shared per path like the hull above.
    pick: Arc<vale_assets::look::pick::PickMesh>,
    /// …and the header sphere it falls back to — see [`ModelAssets::model_sphere`].
    model_sphere: vale_assets::look::pick::Sphere,
    /// The emitters with their materials already interned, or `None` for the
    /// emitterless world. Built once per geometry: a particle's texture is
    /// always the model's own (the survey counts **0** client-supplied slots
    /// over all 2,626 emitters), so unlike a batch material it cannot depend
    /// on the dressing and every dressing shares this `Arc`.
    particles: Option<Arc<crate::render::particles::ParticleSet>>,
    /// The ribbon trails, interned the same way and for the same reason.
    ribbons: Option<Arc<crate::render::ribbons::RibbonSet>>,
    /// The model's own cameras — a property of the file, shared by every
    /// dressing exactly as the hull is. Empty for all but seven files in the
    /// game; see `crate::render::glue`.
    cameras: Arc<Vec<vale_assets::world::m2::M2Camera>>,
    /// …and its lights, for the same reason.
    lights: Arc<Vec<vale_assets::world::m2::M2Light>>,
    /// The model's **texture matrices**, or `None` when it states none — which
    /// is all but a handful of files. Kept beside the tints for the same
    /// reason: the tracks live once per model however many batches read them.
    uv_anims: Option<Arc<vale_assets::world::m2::M2TextureAnims>>,
    /// Sequence 0's window — the clock the emitters, the trails and a
    /// non-global texture matrix all run on.
    clip: Option<crate::render::particles::ParticleClip>,
    /// The colour and transparency tracks, or `None` when no batch names one.
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
}

/// Everything about a batch except its geometry and its texture: the parameters
/// a material is built from.
pub(crate) struct DrawParams {
    /// The appearance variant this batch belongs to, `group * 100 + variant`.
    /// Which of them are drawn is the *dressing's* decision — see
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
    /// Which of the model's colour and transparency tracks fade this batch —
    /// see [`ModelDraw::tint`]. Only *whether* there is one reaches the
    /// material; which tracks is the instance's business.
    pub tint: Option<BatchTint>,
    /// **The constant tint a doodad batch draws with**, as a `MeshTag` — see
    /// [`ModelDraw::baked_tint`]. Part of the key because the material is
    /// specialised as tinted and the tag beside it is read as a colour, so two
    /// batches with different constant tints must not share a pooled handle.
    pub baked_tint: Option<u32>,
    /// See [`ModelDraw::ground`].
    pub ground: Option<vale_assets::world::m2::GroundQuad>,
    /// See [`super::loader::RawDraw::overlays`] — which of the model's texture
    /// slots the folded environment-map layers read, and with which blend.
    pub overlays: [Option<(u32, u16)>; vale_assets::world::m2::MAX_OVERLAYS],
}

/// A texture asked for by name rather than by the model that uses it — a
/// creature's skin, which lives in a DBC and not in the M2.
#[derive(Clone)]
pub(in crate::render) enum TextureState {
    Ready(Handle<Image>),
    Loading,
    Failed,
}

/// One of those, and when a dressing last wanted it.
///
/// **The clock is the dressing's, not this entry's own.** A skin is only ever
/// looked up while a dressing is being *built*, so a texture behind a hot
/// dressing would otherwise read as untouched for as long as that dressing
/// lasts — and evicting it would leave the live material holding one image
/// while the next build reads a second copy of the same file. [`Variant`]
/// therefore names the skins it was built from and touches them with itself.
struct Texture {
    state: TextureState,
    used: f32,
}

/// A dressing, and the two things it was built out of.
///
/// The geometry key and the skin keys are recorded so that a *hit* on this
/// dressing can touch exactly what it depends on. That is what makes
/// [`ModelCache::evict`] an exact liveness rule rather than a guess: nothing a
/// live dressing needs can age out from under it, and nothing else is kept.
struct Variant {
    assets: Arc<ModelAssets>,
    geometry: String,
    /// The non-empty entries of the `skins` this was dressed with.
    skins: Vec<String>,
    used: f32,
}

/// Every model that has been asked for, by archive path.
///
/// **This is the largest thing the client owns, and until this round it only
/// ever grew.** Every mesh and every image the world has drawn hangs off one of
/// the three maps below, so a session that walked from Elwynn to Stormwind to
/// Tanaris held all three zones' geometry and every texture in them for as long
/// as the process ran. Nothing on the HUD said so — the mesh, batch and doodad
/// counts are all counts of what is *spawned*, which streaming already bounds —
/// and the cost lands in GPU memory, which shows up as a frame rate that decays
/// over an hour rather than as any single slow frame.
///
/// [`Self::evict`] is the answer, and the clock it runs on is
/// [`Self::now`]. See [`crate::render::residency`].
#[derive(Resource, Default)]
pub struct ModelCache {
    /// Decoded geometry, by [`geometry_key`]. One entry however many ways it is
    /// dressed; two if the same model is wanted both skinned and not.
    geometry: HashMap<String, Geometry>,
    /// The paths that have loaded, for the HUD — `geometry` counts a model
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
    /// A resource holding a copy of the time is ordinarily a smell. Here it is
    /// the alternative to threading a `f32` through `lookup`,
    /// `lookup_in_room`, `dressed`, `dressed_as_character`, `attached` and
    /// `dress` — six signatures and every call site of each, to carry a number
    /// that is the same for all of them within a frame.
    now: f32,
    failed: HashSet<String>,
    /// Requested and in flight, so a model shared by 400 placements is read once.
    pending: HashSet<String>,
    /// Decoded, waiting for a frame with upload budget left.
    arrived: Vec<Loaded>,
    /// **Builds forgotten while their read was in flight** — see
    /// [`Self::forget`]. An arrival whose key is here was read before the
    /// bytes changed, so it is dropped and asked for again instead of being
    /// installed. Empty in every session: nothing forgets anything unless a
    /// host makes a path answer differently.
    refetch: HashSet<String>,
    /// The one magenta placeholder, shared by every slot nothing fills.
    missing: Handle<Image>,
    /// The material every unit's shadow is drawn with, interned once — see
    /// [`ModelCache::shadow_material`].
    blob: Option<Handle<M2Material>>,
    /// …and one per **chain-effect texture**, on exactly the same terms: a
    /// bolt of lightning is a strip through a bare BLP that no model names, so
    /// there is no `M2` for the pool to reach it through. Ten textures in the
    /// shipped data — see [`crate::render::lightning`].
    beams: HashMap<String, Handle<M2Material>>,
    loader: Option<Loader>,
}

/// **The shadow a unit stands on, and the whole of what 1.12 has of one.**
///
/// The client uses this file beside two CVars and nothing else:
/// `shadowBias` ("Unit shadow depth bias") and `shadowLOD` ("Unit shadow LOD").
/// The other shadow switch in the client is `mapShadows`, which is the ground's
/// baked `MCSH` this client already draws — so a unit's shadow in this game is a
/// decal, and there is no runtime projection of anything anywhere in the client.
///
/// What to *do* with it is the file's own answer, measured by `vale npc`:
/// 32x32, one-bit alpha, dark in the middle (centre 160/255), white at the rim
/// and **white under the cut-out corners**. That is a `modulate` texture — blend
/// mode 5, `dst * src` — and the last of those three is why it needs no fixup:
/// multiplying by the corners leaves the ground exactly as it was.
pub const SHADOW_BLOB: &str = r"Textures\ShadowBlob.blp";

/// How far above the feet the blob is drawn, in yards — the client's own
/// `shadowBias` by another name.
///
/// The blob is sampled onto the ground it lies on (`crate::render::shadows`), so
/// it is coplanar with the terrain by construction and coplanar geometry
/// z-fights. This is what lifts it clear — a depth bias and nothing more, which
/// is what the CVar's own name says it is.
///
/// **It used to be doing a second job, and that job is gone.** While the blob
/// was one flat horizontal quad, the bias was also the fudge deciding how much
/// of a slope the quad hung over: the uphill half was correctly hidden by the
/// ground (blend mode 5 lands in the sorted phase, which depth-*tests* and does
/// not write) and the downhill half hung in the air with a hard edge. 0.1 is
/// still the reference's own default — `shadowBias` registers as
/// `"0.1"` — and is now only ever a bias.
const SHADOW_BIAS: f32 = 0.10;

/// [`SHADOW_BIAS`], for the entity pass that places the quad.
pub fn shadow_bias() -> f32 {
    SHADOW_BIAS
}

/// The cache key for one build of a model's geometry.
///
/// The path alone when the joint attributes are not wanted, which is every
/// doodad in the world — so scenery is not renamed by a distinction only
/// entities make, and the two builds cannot collide.
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
    /// Dressings — the material lists over that geometry.
    pub dressings: usize,
    /// Client-supplied skins, and with them their images.
    pub textures: usize,
}

impl Evicted {
    pub fn any(self) -> bool {
        self != Evicted::default()
    }
}

/// **The baked light a model standing inside a building is lit by**, packed so
/// that it can ride on the *instance* rather than on the material.
///
/// A `MODD` spawn carries its own colour (see [`vale_assets::world::wmo::WmoDoodad::light`])
/// and that is the right answer physically — a crate in a cellar is darker than
/// the same crate by a window. What it must not be is a material dimension:
/// measured by `vale wmos`, one tile of Stormwind's furniture holds **2,249
/// distinct colours over 6,158 spawns, which is 3,037 (model, light) pairs** —
/// 3,037 materials for the furniture of one tile, against ~1,800 for the entire
/// world before it. For a round the colour *was* a material dimension, rounded
/// to four bits a channel to keep the count survivable, and the HUD's material
/// count still tripled — which matters because the per-frame CPU floor scales
/// with materials and batch sets.
///
/// So the colour goes to the GPU as the entity's [`bevy::mesh::MeshTag`] —
/// the per-instance `u32` Bevy carries in its mesh uniform for exactly this
/// kind of thing — and the material says only *that* the batch is room-lit
/// ([`M2Params::vertex_lit`]), not which colour. Two consequences, both the
/// point: every room-lit build of one model shares one material however many
/// lights the building bakes, and the four-bit rounding is gone — the shader
/// reads the spawn's own bytes.
///
/// Kept in the file's **sRGB** space, not decoded: the shader adds this to the
/// material's ambient and the `MOCV` colour and decodes the sum, because that
/// is the order the client's own arithmetic happens in. See `m2.wgsl`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RoomLight([u8; 3]);

impl RoomLight {
    /// The light of one `MODD` spawn, as bytes.
    pub fn new(colour: [f32; 3]) -> RoomLight {
        RoomLight(colour.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8))
    }

    /// The mesh tag the shader unpacks: `0x00RRGGBB`, red high — the same
    /// human order the `Light.dbc` facts use, and pinned by a test because a
    /// swapped channel renders as a plausible room of the wrong hue.
    pub fn tag(self) -> u32 {
        u32::from(self.0[0]) << 16 | u32::from(self.0[1]) << 8 | u32::from(self.0[2])
    }
}

/// The per-instance multiplier on a model's **sun term**, which the 1.12
/// client keeps on every placed M2 and this renderer carried nowhere.
///
/// `Model2.bls` — the game's own model vertex shader — ends its lighting in
/// `MAD result.color, c28[0], R1, c28[1]`: a per-instance scale applied after
/// the light is summed. The *values* are the client's: **2.5 for a terrain
/// doodad on lit ground, 0.5 for one standing in the ground's own baked
/// `MCSH` shadow, 1.0 for everything else** — an exterior WMO prop takes 1.0
/// rather than the boost. That bit is [`vale_assets::world::adt::Adt::shadowed_at`],
/// sampled once at the placement's origin when the tile is read.
///
/// **It scales the sun and never the fill, and it is not pre-clamped.** The
/// scale multiplies the lambert term inside `daylight`'s byte-space sum
/// (`atmosphere.wgsl::daylight_scaled`), so the anti-sun side of a boosted
/// tree keeps the cool fill instead of bleaching with the rest — clamping
/// `sun × 2.5` before the sum would turn a warm dawn white before it is used.
pub mod sun_scale {
    /// A terrain (`MDDF`) doodad whose origin is on lit ground.
    pub const LIT_GROUND: f32 = 2.5;
    /// One standing in the ground's baked `MCSH` shadow: dimmed with the
    /// ground under it, which is the half that was visibly missing — a tree
    /// in a building's shadow took full sun.
    pub const SHADOWED_GROUND: f32 = 0.5;
    /// Everything else: WMO batches, `MODD` props, entities, and the
    /// encoding's absent value.
    pub const NEUTRAL: f32 = 1.0;
}

/// Pack an instance's room light and sun scale into the one `MeshTag`.
///
/// The tag is Bevy's per-instance `u32`: the low 24 bits stay [`RoomLight`]'s
/// `0x00RRGGBB`, and the sun scale rides in the byte the colour never used,
/// as fixed-point 1/32ths (2.5 → 80, 0.5 → 16). Zero means "unspecified",
/// which the shader reads as [`sun_scale::NEUTRAL`] — so every entity and WMO
/// batch that never sets a tag keeps exactly the light it had.
/// Pack a batch's sampled colour into the same `MeshTag`, as `0xAARRGGBB`.
///
/// **The tag is the only per-instance channel there is, and a fade cannot live
/// anywhere else.** A material is interned by value and a batch set cannot span
/// two of them (see [`MaterialPool`]), so a colour that changes every frame
/// would be a new material every frame, per batch, per caster. The tag is
/// Bevy's own per-instance `u32` and it is rewritten in place.
///
/// So the low 24 bits are the same `0xRRGGBB` a [`RoomLight`] writes and the
/// **top byte is opacity rather than a sun scale** — which is why a tinted
/// batch's material says so ([`M2Params::particle`]'s `.y`): the shader has to
/// know which of the two payloads it is holding, and zero is a real value here
/// where it is "unspecified" there. A batch that has faded out packs to
/// `0x00000000`, which is exactly right and is the one tag the sun-scale
/// reading would have misread as neutral.
pub fn tint_tag(rgba: [f32; 4]) -> u32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte(rgba[3]) << 24 | byte(rgba[0]) << 16 | byte(rgba[1]) << 8 | byte(rgba[2])
}

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
/// bake can still differ in hair — the dressing is per *appearance*, which is
/// exactly the granularity the hairstyle needs, so it costs no new dimension.
/// A creature adds nothing to the key at all, which keeps scenery and the
/// bestiary named as they were.
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
    // **Every field of the dress, not just the hair.** A dressing *is* a batch
    // list, and two characters differing only in their boots have different
    // batch lists — so leaving the equipment geosets out of the key hands the
    // second one the first one's gear. It cost nothing while the only NPC gear
    // was in the bake and every player's equipment array came out empty, which
    // is exactly the kind of thing that starts mattering the moment it is
    // fixed elsewhere: the cache would have been silently right until then and
    // silently wrong afterwards.
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
    // **Room-lit is part of the key; the room's colour is not.** The two
    // dressings really do differ — `vertex_lit` is a material parameter — but
    // the colour rides on each instance as its `MeshTag`, so a barrel in a
    // dozen buildings at a dozen brightnesses is *one* dressing and one
    // material. Keying the colour in here is what once made Stormwind's
    // furniture ~1,800 materials for one tile; see [`RoomLight`].
    let room = if room { "|room" } else { "" };
    // **A model drawn as a particle is its own dressing**, for the same reason
    // a room-lit one is: the difference is a material parameter. Every batch
    // of it reads its `MeshTag` as the emitter's over-life colour rather than
    // as a room and a sun scale — see [`ModelCache::as_particle`]. The
    // population is 13 files in the whole game, so this dimension costs
    // essentially nothing and cannot be folded into `room`: the two mean
    // opposite things about the same 32 bits.
    let particle = if as_particle { "|particle" } else { "" };
    // **…and the model's own lights, which really are part of a dressing.**
    // Unlike the room's colour they are *in* the material — see
    // [`SceneLighting`] — so two builds under different lights are two builds,
    // and the character standing on the character-select plinth is otherwise
    // spelled exactly like the same character in the world. Empty for every
    // model the world draws, so nothing there grows a key.
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
    /// The model's own textures and nothing else — a doodad has no skins — and
    /// **without** the skinning attributes, because a doodad is drawn in its
    /// bind pose and a mesh that says otherwise takes the renderer down. See the
    /// module comment.
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

    /// Mark a dressing, and everything it was built from, as wanted **now**.
    ///
    /// The `used == now` guard is what keeps this off the hot path: a doodad
    /// pass that looks the same model up four hundred times in one frame pays
    /// the two string clones once.
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
    /// `None` is [`Self::lookup`] exactly — the outdoor build, which is every
    /// tree in the world and must not pay a hash of the key for a light it does
    /// not have. `Some` is a second dressing of the same *geometry*: the meshes
    /// are shared and only the materials differ, which is the same trade
    /// dressing a wolf in three skins makes.
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

    /// …and the same model **built to be posed**, for the scenery that moves.
    ///
    /// `skinned` is the whole difference and it is a *different build of the
    /// geometry*, not a flag on the same one: the skinning attributes are
    /// emitted at load, so a mesh built without them cannot be posed later and
    /// one built with them pays the vertex attributes whether or not anything
    /// reads them. That is why scenery has taken the unskinned build since this
    /// cache existed and why the animated ones have to ask for the other.
    ///
    /// **Two builds of one model can be resident at once** — a torch standing
    /// in the street near enough to animate and another across the zone in its
    /// bind pose — and that is the point rather than a waste: the geometry key
    /// carries `skinned`, so they are separate entries and each is evicted on
    /// its own use. See [`vale_assets::look::scenery`] for which models this
    /// is asked of and how few they are.
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

    /// The same model as the **body of a particle** — one live particle of a
    /// geometry-model emitter (`M2Particle::geometry_model`).
    ///
    /// It differs from [`Self::lookup`] in one parameter and it is the whole
    /// of what a model particle needs from this cache: every batch's material
    /// reads its `MeshTag` as the emitter's own **over-life colour**
    /// (`0xAARRGGBB`, [`tint_tag`]) rather than as a room light and a sun
    /// scale. Nothing else can carry it — the ramp changes every frame, per
    /// particle, so it cannot be a material, and the tag is the one
    /// per-instance word there is.
    ///
    /// The geometry, the meshes and the model's own textures are shared with
    /// every other use of the file; only the materials are rebuilt, which is
    /// the same trade [`Self::lookup_in_room`] makes.
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

    /// **The same model as a *scene*** — the login screen's backdrop and
    /// character select's.
    ///
    /// It differs from [`Self::lookup`] in one parameter and the parameter is
    /// the whole point: **skinned**. A glue scene wears no skins and stands in
    /// no room, so the obvious call is `lookup` — and `lookup` builds the
    /// *doodad* dressing, which deliberately drops the skeleton and the tints
    /// (see [`Self::lookup`]'s own note and `loader::RawModel`). Measured on
    /// `UI_MainMenu.m2`: 22 batches and **0 bones**, which is a login screen
    /// whose fire does not burn, whose figures do not breathe and whose sky is
    /// drawn at whatever colour its first key happens to be.
    ///
    /// `Dress::Creature` and no skins: the model's own textures are the whole
    /// of it — `vale glue` reports 25 of 25 named textures resolving on this
    /// file, with none of them client-supplied.
    ///
    /// **`scene` is the model's *own* lighting**, which for these two screens is
    /// the whole of the shading — see [`SceneLighting`] and
    /// `vale_assets::world::m2::M2Light`. It is the caller's rather than read off
    /// the model here because the same lights have to reach the *character*
    /// standing in the scene, and that is a different model entirely.
    /// **The lights a file states, before anything has been dressed by them.**
    ///
    /// The chicken-and-egg [`Self::as_scene`] would otherwise have: its `scene`
    /// argument is part of the dressing's identity, and the lights it is built
    /// from are in the same file. They are a property of the *geometry* — like
    /// the cameras and the collision hull — so they can be answered from the
    /// build before any dressing exists, and this requests the model exactly as
    /// [`Self::lookup`] would.
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

    /// **The same model wearing supplied skins, in its bind pose** — the one
    /// corner of this cache's four that [`Self::lookup`], [`Self::lookup_scenery`]
    /// and [`Self::dressed`] leave out.
    ///
    /// `lookup` is the model's own textures unposed, `lookup_scenery` is the
    /// same relit, and `dressed` is supplied skins **posed**, which is what an
    /// entity wants because an entity has a skeleton. This is supplied skins
    /// *unposed*, which is what anything drawing a creature without a skeleton
    /// wants: a still picture of it.
    ///
    /// **It is the skins that make this necessary rather than the pose.**
    /// Measured over the whole table: 10,388 of `CreatureDisplayInfo`'s 10,534
    /// rows take their slot-0 texture from the client — 3,555 from a texture
    /// variation and 6,833 from a baked NPC skin — so a creature drawn through
    /// `lookup`, which supplies none, has its body slot empty and draws
    /// magenta. That is 98.6% of them, which is to say all of them.
    ///
    /// The meshes are shared with every other build of the same geometry; a
    /// dressing is materials and nothing else, which is why this needs the
    /// material store.
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
    /// slot's *position* is the texture type it fills.
    ///
    /// Needs the material store because a dressing is materials and nothing
    /// else; the meshes are already there.
    ///
    /// **`hair` is the one slot a DBC cannot name**, and it is why this takes an
    /// appearance at all. An NPC in a character model has its body *baked* — so
    /// it comes through here rather than through
    /// [`Self::dressed_as_character`] — but its hairstyle is a separate mesh
    /// with a separate texture, which no bake can hold and which
    /// `CreatureDisplayInfoExtra` states as a style and a colour like a
    /// player's. Left empty it draws magenta, which is what every hairy NPC in
    /// the game did; see [`vale_assets::look::dress::Dressing::hair`].
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
        // **`room` is what the entity is standing in**: an entity indoors takes
        // the same interior branch its furniture does, because a room lights
        // whatever is in it and the sun does not reach through a roof. Which
        // room — or whether any — is `crate::world::entities::Indoors`. Only its
        // *presence* names the dressing; the colour is the caller's to put on
        // each part as its `MeshTag` (see `RoomLight`).
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
        // Slot 3 is the hair mesh's, whatever filled slot 0 — the positions are
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
        // the whole entity back — a creature with the wrong face beats no
        // creature at all, and magenta says which.
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

        // Stamped as this dressing is built, not only when one is looked up:
        // otherwise a model loaded a minute ago and dressed just now carries
        // the *load* time, and the next sweep drops the geometry out from
        // under a dressing that has only just been made. Harmless — the
        // dressing holds its own handles and still draws — but it costs a
        // re-read of a file that was in hand a moment earlier.
        let now = self.now;
        let geometry = self.geometry.get_mut(&geometry_key).expect("checked above");
        geometry.used = now;
        let geometry = &*geometry;
        // Which appearance variants this wearer shows. The geometry holds every
        // batch the file has — see `read_model` — so this is where a character
        // stops wearing every hairstyle and every boot at once.
        let all: Vec<u16> = geometry.draws.iter().map(|d| d.geoset).collect();
        let wanted = vale_assets::world::m2::visible_geosets(&all, dress);
        // **Which batches this dressing keeps**, by index rather than as an
        // iterator, because the merge below has to reach each one's CPU
        // geometry as well as its parameters — see [`loader::MergeSource`].
        //
        // A model that numbers its geosets some other way still has to render,
        // which is what the empty case means — the same fallback
        // `M2::visible_batches` makes.
        let kept: Vec<usize> = (0..geometry.draws.len())
            .filter(|&i| wanted.is_empty() || wanted.contains(&geometry.draws[i].geoset))
            .collect();
        let draws: Vec<ModelDraw> = kept
            .iter()
            .map(|&index| {
                let params = &geometry.draws[index];
                let mesh = &geometry.meshes[index];
                let bones = Arc::clone(&geometry.bones[index]);
                // One texture slot resolved: the client's own bake or variation
                // where the model asks for one, the model's own file otherwise.
                let resolve = |slot: usize| -> Option<Handle<Image>> {
                    let kind = *geometry.kinds.get(slot)?;
                    match skin_slot(kind) {
                        // The client supplies this one: a texture variation
                        // or a bake, from the display tables.
                        Some(supplied_slot) => supplied.get(supplied_slot)?.clone(),
                        // Type 0 — the model named the file itself.
                        None => geometry.own.get(slot).cloned(),
                    }
                };
                let texture = params
                    .texture
                    .and_then(resolve)
                    .unwrap_or_else(|| self.missing.clone());
                // **The folded layers go through the same lookup**, and a slot
                // that will not resolve leaves its layer off rather than
                // painting the batch magenta: an environment map is a sheen,
                // and a missing sheen is a duller helmet where a missing *base*
                // is a helmet nobody can identify. See
                // `models::loader::RawDraw::overlays`.
                let overlays = params
                    .overlays
                    .map(|layer| layer.and_then(|(slot, _)| resolve(slot as usize)));
                let mut built = material_for(
                    params,
                    texture,
                    overlays,
                    M2_ALPHA_KEY,
                        // The room's *colour* is not here: it is per spawn and
                        // rides on the instance as its `MeshTag`, or every
                        // distinct light would be a distinct material — see
                        // `RoomLight`. What the material carries is the branch:
                        // `vertex_lit` with no vertex colours to add is exactly
                        // "lit by the one colour the tag names". An M2 has no
                        // `MOCV`, so there is nothing else for it to read.
                    Vec3::ZERO,
                    room.is_some(),
                    scene,
                );
                // **The particle build's one difference.** `.y` is what tells
                // the shader the instance word is a colour rather than a room
                // — and `vertex_lit` has to go with it, because the two read
                // the same 24 bits and a tinted batch is not room-lit (the
                // note in `material_for` says so for the track-driven case;
                // this is the same rule with the ramp coming from an emitter
                // instead of from an `M2Color`).
                if as_particle {
                    built.params.particle.y = 1.0;
                    built.params.vertex_lit = 0.0;
                }
                // **A batch whose texture moves cannot be pooled**: its
                // material is rewritten every frame, and a pooled handle that
                // changed would change some other batch's picture with it. See
                // `Materials::moving`.
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
    /// **Order matters and it is the reverse of the dependency.** A dressing
    /// holds handles into its geometry's meshes and its skins' images, so the
    /// dressings go first; the geometry and the textures under them are then
    /// unreferenced by this cache, and whatever is still spawned in the world
    /// keeps its own handles alive regardless. Nothing here can pull an asset
    /// out from under a drawn entity — Bevy's handles are what own the asset,
    /// and this only drops *this* cache's share of them.
    ///
    /// The failure list is deliberately kept: a model that would not read will
    /// not read the second time either, and re-requesting it on every sweep
    /// would be a loader thread doing nothing but failing.
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
        // `loaded` is the HUD's count of distinct *paths* where `geometry`
        // counts builds — a model wanted both skinned and not is two of the
        // latter and one of the former — so it is rebuilt from what survived
        // rather than decremented alongside it. Anything else double-counts a
        // path whose other build is still resident.
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

    /// **Forget one path, because its bytes have changed.**
    ///
    /// Everything this cache holds is keyed by archive path and kept for the
    /// life of the process, which is right for files that do not change and
    /// wrong the moment a host makes one of those paths answer with different
    /// bytes through `GameAssets::set_overlay`. `GameAssets::forget_tables`
    /// is the same seam one layer up, and the symptom is the same: the file
    /// on disk is new, every fresh reader sees it, and the world goes on
    /// drawing what was read the first time.
    ///
    /// Three things go, and the third is the one that is easy to miss:
    ///
    /// * the **dressings** built from this model, and the **geometry** under
    ///   them, both builds — skinned and not;
    /// * the **texture**, if the path names one rather than a model;
    /// * the mark in `failed`. A path that did not read *before* it existed is
    ///   remembered as unreadable for the life of the process, so a model
    ///   written after something asked for it would never be looked at again.
    ///
    /// **It takes effect on the next lookup.** Anything already spawned keeps
    /// the handles it holds — Bevy's handles own the asset — so a host that
    /// wants what is on screen to change has to have it built again. What a
    /// forget promises is only that the next build reads the file.
    ///
    /// A read already in flight is dropped and re-filed when it lands, so a
    /// forget during a load is not a stale build installed a frame later.
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

    /// …and forget **everything**, for a host that has changed what a whole
    /// namespace answers with — a project switched under the world.
    ///
    /// [`Self::forget`]'s terms in every respect, including that what is
    /// already drawn keeps what it holds.
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

    /// That same clock, for the loader writing a stamp of its own. Private
    /// rather than `pub(crate)` because [`TextureState`] is — the loader is a
    /// child module and sees both.
    fn age(&self) -> f32 {
        self.now
    }

    /// Record a skin the loader has finished with.
    fn set_texture(&mut self, path: String, state: TextureState) {
        let used = self.now;
        self.textures.insert(path, Texture { state, used });
    }

    /// Models, dressings and skins resident right now — the three numbers the
    /// eviction is judged by, and the ones that used to only go up.
    pub fn resident(&self) -> (usize, usize, usize) {
        (self.geometry.len(), self.variants.len(), self.textures.len())
    }

    /// The same model wearing a *player's* skin, composed rather than looked up.
    ///
    /// Display ids 49..57 are the bare race models and have no bake, so the
    /// display tables correctly hand back an empty slot 0 for them and every
    /// player draws magenta. The composition fills that same slot: the skin is
    /// still texture type 1, so nothing below this changes — the key just names
    /// a texture that is built instead of read.
    ///
    /// The key is the appearance, so two humans who look alike share one
    /// composite and one dressing, and one who has changed clothes does not.
    ///
    /// Slot 3 is the hair mesh's own texture. It is the one piece of a
    /// character that is not in the composite — the hairstyle is *geometry*, so
    /// its texture dresses a separate geoset and the M2 asks for it under its
    /// own type. Without it the hair draws magenta, which is how it was found:
    /// the geoset arrived before the texture did, and said so loudly.
    pub fn dressed_as_character(
        &mut self,
        path: &str,
        look: &CharacterLook,
        dress: Dress,
        cloak: Option<&str>,
        room: Option<RoomLight>,
        // …and the lights whatever the character is *standing in* states, which
        // is [`SceneLighting::NONE`] for every player in the world and the
        // backdrop's own rig for the one on the character-select plinth.
        scene: SceneLighting,
        meshes: &mut Assets<Mesh>,
        materials: &mut Materials,
    ) -> Lookup {
        self.as_character(path, true, look, dress, cloak, room, scene, meshes, materials)
    }

    /// **The same body, without the joints** — a still of a dressed character.
    ///
    /// [`Self::dressed_as_character`] is skinned, because the thing that asks
    /// for one is an entity and an entity has a skeleton to bind. A *picture*
    /// of a character has none: a mesh carrying joints with nothing to bind
    /// them to is a mesh the renderer will not draw, which is the same reason
    /// [`Self::worn_unposed`] exists beside [`Self::dressed`].
    ///
    /// What that costs is the pose: the body stands in the bind pose the file
    /// was authored in, arms out. For a picture of what a garment *paints* —
    /// which is the population with no model of its own, and so the population
    /// that has no other picture at all — the pose is not what is being looked
    /// at.
    ///
    /// Lit by nothing in particular ([`SceneLighting::NONE`]) and standing in
    /// no room, for the reason [`Self::worn_unposed`] takes neither: a still
    /// is framed by whoever renders it and lit by the lamp they put beside it.
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
            // Slot 4 is the object skin, which on a *character* is the cloak:
            // the cape is the wearer's own group-15 geoset and the item names
            // only the texture for it.
            cloak.unwrap_or_default().to_string(),
        ];
        self.dress(
            path, skinned, &skins, dress, room, false, scene, meshes, materials,
        )
    }

    /// An item's own model — a pauldron, a helm, a spell's glow — wearing the
    /// skin the item names for it.
    ///
    /// **Skinned**, like an entity: an attached model rides one of the
    /// *wearer's* bones, but it is not rigid — a torch's glow plane sits on a
    /// billboarded bone of the **torch's own** skeleton, and a spell effect's
    /// whole appearance (Arcane Explosion's dome growing out of nothing over
    /// 0.8 s) is its own bone animation. Built unskinned, that skeleton was
    /// dropped at load (`read_model`) and every attached model froze in bind
    /// pose — which drew the glow as a flat static card and the dome at its
    /// full authored extent for its whole lifetime. The spawner binds the
    /// joints and `animate` poses them on the attachment's own clock.
    /// **And lit by the room its wearer is standing in**, for the reason the
    /// wearer's own body is: a helm is on a head the tavern has darkened, and
    /// nothing about the pauldron knows that. `None` is out of doors and is
    /// also what a *spell effect* passes — those are `unlit` glows, so the
    /// branch would change nothing about them and would cost a second dressing
    /// of every effect model per building.
    pub fn attached(
        &mut self,
        path: &str,
        texture: Option<&str>,
        room: Option<RoomLight>,
        // …and by the same rule, the *scene's* lights where there is a scene: a
        // pauldron on the character standing on the character-select plinth is
        // lit by that backdrop's own rig, exactly as its wearer's chest is.
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
    /// A key with the [`CHARACTER_PREFIX`] is a *composed* player skin rather
    /// than an archive path, and goes to the loader as such. Sharing this cache
    /// with the file-backed skins is the point: the dressing, the upload budget
    /// and the Loading/Failed states are all the same machinery, and the only
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

    /// **A material for a texture asked for by name**, with none of a model's
    /// own state behind it — a loose quad drawn with one of the archives'
    /// pictures.
    ///
    /// `None` until the file comes back off the loader thread, which is a frame
    /// or two into the session: the caller's answer to that is to have nothing
    /// yet and try again, the same as [`Self::shadow_blob`]'s. Interned like
    /// every other material, so every caller asking for the same picture and the
    /// same parameters shares one.
    ///
    /// It exists because there is now more than one of these — the blob shadow
    /// built its own inline, and [`crate::render::selection`] wanted the same
    /// three steps with a different blend.
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
            // …nor an environment map: a loose quad is one batch and one layer.
            Default::default(),
            M2_ALPHA_KEY,
            Vec3::ZERO,
            false,
            // A loose quad states nothing about its own lighting.
            SceneLighting::NONE,
        );
        Some(materials.intern(material))
    }

    /// …and the same for a texture this client **made**, rather than one the
    /// archives carry.
    ///
    /// One caller: [`crate::render::labels`], whose textures are rasterised
    /// strings and have no path to be asked for. Everything else about the
    /// material is identical, which is the point — a label goes through the same
    /// pipeline, the same pool and the same sorted transparent phase as every
    /// other blended quad in the world, and therefore takes the same depth test.
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
    /// `None` until the texture has come back off the loader thread, which is a
    /// frame or two into the session — an entity that appears before then simply
    /// has no shadow yet, in the same way it has no model yet.
    ///
    /// **One material and one *mesh* for the whole world.** This used to hand
    /// back a `ModelDraw` — a unit quad plus this material — and every unit was
    /// given a child entity carrying it, scaled by its own footprint. A material
    /// shared by a hundred units is one batch *set*, but the sorted transparent
    /// phase merges only adjacent same-set runs and a hundred blobs at a hundred
    /// depths are never adjacent, so it was one draw call each: measured at a
    /// Northshire framing, **123 of the frame's 238 blended draw calls were blob
    /// shadows**, and subtracting them alone moved the frame from 17.4 ms to
    /// 14.7 ms. [`crate::render::shadows`] builds the quads into one mesh
    /// instead; what is left here is the material they share.
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
                // Nothing lights a shadow: the texel *is* the darkening.
                unlit: true,
                // The quad faces up and is never seen from below — anything
                // under it is under the ground.
                two_sided: false,
                no_depth_write: false,
                light: BatchLight::Sun,
                liquid: None,
                // Not projected — see the `ground` field on the `ModelDraw`
                // this builds.
                ground: None,
                tint: None,
                baked_tint: None,
                uv: None,
                // A quad this client builds has no environment map — see
                // `models::loader::RawDraw::overlays`.
                overlays: Default::default(),
            },
            texture,
            Default::default(),
            M2_ALPHA_KEY,
            Vec3::ZERO,
            false,
            // A shadow is unlit; nothing states anything about it.
            SceneLighting::NONE,
        );
        // **And it is not fogged.** `ambient.w` is the shader's "leave this
        // alone" flag, and this is the one draw in the world that sets it: fog
        // mixes toward a colour, a modulate draw multiplies by one, and the
        // identity for a multiply is white. See `m2.wgsl`.
        material.params.ambient.w = 1.0;
        // **The blob *is* a ground decal in the real client, and it is not one
        // here**: `render::shadows` still lays a flat lifted quad at the feet
        // rather than draping one over the ground. See `render::decals`, which
        // is where it goes when it becomes one.
        let handle = materials.intern(material);
        self.blob = Some(handle.clone());
        Some(handle)
    }

    /// **The material one chain effect's strip draws through** — a bare
    /// `Textures\SpellChainEffects\*.blp` and nothing else.
    ///
    /// [`Self::shadow_material`]'s shape, one subject over, and the reason it
    /// is here rather than in the pass that uses it is the same: the texture
    /// load is asynchronous and the cache is what owns the wait. `None` while
    /// the BLP is still being read, which is a beam that starts a frame or two
    /// late rather than one drawn magenta.
    ///
    /// **Blend 4, not 3.** `SpellChainEffects`' textures carry their shape in
    /// the alpha channel — the bolt is a bright core on a black field with a
    /// soft alpha edge — so an `add` (3) would show the black as a rectangle
    /// where an `alpha-add` (4) does not. Stated as a reading rather than a
    /// measurement: nothing in the tables names a blend mode for these, because
    /// there is no `M2` render flag anywhere in the chain to carry one.
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
                // Seen from both sides — the strip is turned to face the camera
                // every frame and a back face is a bolt that vanishes as the
                // view swings past it.
                two_sided: true,
                no_depth_write: true,
                light: BatchLight::Sun,
                liquid: None,
                ground: None,
                tint: None,
                baked_tint: None,
                uv: None,
                // A quad this client builds has no environment map — see
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
    /// Distinct *paths*, not builds: a model wanted both skinned and not is one
    /// model that happens to be meshed twice.
    pub fn counts(&self) -> (usize, usize) {
        (self.loaded.len(), self.failed.len())
    }

}

/// What marks a skin key as a *composed* player skin rather than an archive
/// path. A leading NUL cannot appear in an MPQ path, so the two namespaces
/// cannot collide however the game data is patched.
const CHARACTER_PREFIX: &str = "\u{0}character:";

/// The same, for the hair mesh's own texture — a separate key because it is a
/// separate image filling a separate slot, and because a change of hairstyle
/// must not reuse the body composite.
const CHARACTER_HAIR_PREFIX: &str = "\u{0}characterhair:";

/// The cache key for one player's composed body texture.
///
/// The appearance *and the wardrobe*, so two players who look alike and are
/// dressed alike share one composite and one dressing — which on a populated
/// realm is a great many of them, since a race has a handful of faces and a
/// starting zone has one set of gear. One who changes clothes gets a new key,
/// which is exactly what should happen.
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
/// Equipment cannot change it — a helmet hides hair rather than recolouring it
/// — so the key deliberately drops the wardrobe and two players in the same
/// hairstyle share one texture however differently they are dressed.
fn character_hair_key(look: &CharacterLook) -> String {
    hair_key(&look.appearance)
}

/// …and the same from the appearance alone, which is all it ever needed.
///
/// A character-model **NPC** has no [`CharacterLook`] — its body is baked rather
/// than composed — and still wears a hair mesh, so this is the door both kinds
/// go through. See [`ModelCache::dressed`].
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

/// Which client-supplied slot an M2 texture type fills.
///
/// Types 1 and 11 share slot 0 deliberately: a creature declares its body as
/// type 11 and a *character* model declares it as type 1, and no model is ever
/// both. 12 and 13 are creature texture variations 2 and 3. Type 6 is the hair
/// mesh's own texture, which is the one part of a character's appearance that is
/// **not** in the body composite — it dresses separate geometry, so it needs a
/// separate slot rather than a region. `None` means the model supplies this
/// texture itself (type 0) or the client has nothing for it.

/// **The `SkinnedMesh` one batch of a spawned model takes**, or `None` for a
/// batch with no bones — a tree, or a model with no skeleton at all.
///
/// `joints` is the whole model's joint list in bone order with the identity
/// joint last, exactly as every spawner builds it; what this picks out of it is
/// [`ModelDraw::bones`], the batch's own subset.
///
/// **It is a function because there are three spawners and there was very
/// nearly a fourth mistake.** `world::entities::spawn`, `entities::effects` and
/// `render::glue` each build a model's parts, each had its own copy of these
/// four lines, and the round that introduced the subset updated two of them:
/// the login and character-select screens went on binding the model's *whole*
/// skeleton to meshes whose joint indices had become subset-local, which poses
/// every vertex off the wrong bone. It draws as splayed shoulders, a twisted
/// weapon and a body that clips through itself — plausibly wrong rather than
/// obviously so, and it survived a full test suite, six interface probes and
/// two in-world screenshots because none of them looks at the glue screens.
///
/// **And it was four, not three.** `render::portraits` is a spawner too — it
/// builds a model's parts on its own render layer for the unit-frame faces —
/// and it was not in the list above, so it went on binding the whole skeleton
/// for two more rounds. On a 64-pixel face a wrong bone is a smear rather than
/// a recognisable mistake, and no headless check looks at a portrait, so it was
/// reported by somebody looking at the screen. The sentence that used to end
/// this paragraph — *"one function, three callers, and a fourth cannot get it
/// wrong"* — was the claim, not the mechanism. The mechanism is
/// `every_skinned_mesh_is_built_through_skin_for` in this module's tests, which
/// reads the call sites off the source and fails the build for a fifth.
pub fn skin_for(draw: &ModelDraw, joints: &[Entity]) -> Option<SkinnedMeshFor> {
    let bones: Vec<Entity> = draw
        .bones
        .iter()
        .filter_map(|&bone| joints.get(usize::from(bone)).copied())
        .collect();
    (!bones.is_empty()).then_some(bones)
}

/// Whether a batch names only the bones it rides — see
/// [`loader::RawDraw::bones`], and `VALE_NO_MERGE` for why a saving like
/// this has to stay subtractable.
///
/// **Read on the *load* side rather than at spawn**, which is a correction: a
/// first draft branched in [`skin_for`] alone, so the switch handed a mesh
/// whose vertex indices were already subset-local the model's whole skeleton —
/// every vertex posed off the wrong bone. A switch that renders wrongly is
/// worse than no switch, because the A/B it exists for is then between a
/// picture and a different picture.
///
/// Read once and cached: this is on the loader thread, once per batch per
/// model.
pub(crate) fn subsetting() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VALE_NO_BONE_SUBSET").is_none())
}

/// What [`skin_for`] answers: the joint entities one batch rides, in the order
/// its vertex indices expect them.
pub type SkinnedMeshFor = Vec<Entity>;

/// **Concatenate a dressing's batches that share a material into one mesh.**
///
/// The only population in this renderer whose cost is *per mesh instance* is
/// the one that moves: a static doodad is extracted once and then free — 615 of
/// them subtract to 0.1 ms of frame time — while a skinned instance under a
/// moving entity is re-propagated, re-extracted and re-skinned every frame. A
/// dressed level-60 character draws about twenty of those, and most of them are
/// the same material over and over: the body's opaque batches all wear the one
/// composed skin and its alpha-keyed ones all wear the second texture, because
/// that is what a composite *is*.
///
/// So the batches that may be merged ([`loader::MergeSource`] states which, and
/// why each exclusion is about the picture rather than the cost) are grouped by
/// their interned material and concatenated. **The merged draw takes the
/// position of the group's first member**, so what is left of the file's batch
/// order is unchanged for everything that was not merged — which matters for
/// the translucent batches, none of which are ever in a group.
///
/// Three things it has to get right, and each is a way to draw a wrong picture
/// rather than a slow one:
///
/// * **the vertex indices are per batch**, so each member's are offset by the
///   vertices already written;
/// * **the joint indices are per batch too** — they index that batch's own bone
///   subset ([`loader::RawDraw::bones`]), not the skeleton — so the merged mesh
///   takes the union of the members' subsets and every member's indices are
///   remapped into it;
/// * **a group of one is not a merge.** Building a second copy of a mesh the
///   geometry already holds would cost memory and save nothing, so a lone batch
///   keeps the shared handle.
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
    // **The kill switch, and it is here for the reason `VALE_NO_WIREFRAME`
    // is**: a cost paid by a switch nobody has touched is one that has to stay
    // subtractable. Two runs differing in this variable is the only way to
    // price the merge, and the merge is the only thing standing between a
    // dressed character and twenty moving mesh instances.
    if groups.is_empty() || std::env::var_os("VALE_NO_MERGE").is_some() {
        return draws;
    }

    // Which merged draw, if any, each slot has become — `None` for a batch that
    // was not merged, `Some(group)` for the first member of one, and dropped
    // entirely for the rest.
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
                    // A local index past the batch's own subset cannot happen —
                    // `batch_draw` builds the two together — but reading one
                    // would silently pose the vertex off some other bone, so it
                    // falls back to the first rather than panicking on a
                    // twenty-year-old file.
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
            // Every one of these is `None` on a mergeable batch by construction
            // — see [`loader::MergeSource`].
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

pub fn skin_slot(kind: u32) -> Option<usize> {
    match kind {
        1 | 11 => Some(0),
        12 => Some(1),
        13 => Some(2),
        6 => Some(3),
        // Type 2 is the *object* skin, and it is filled from an item rather
        // than from the model: a pauldron's M2 declares it with no filename and
        // `ItemDisplayInfo::modelTexture` names the file, out of the same
        // `Item\ObjectComponents\<slot>\` directory the model came from. The
        // wearer's own cape geoset asks for it too — a cloak is geometry the
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
        // **`wind.wgsl` is a `load_shader_library!` and not an
        // `embedded_asset!`, and the difference is not cosmetic.** Nothing ever
        // names it as a `ShaderRef`: it is only ever `#import`ed, by the two
        // vertex stages below. An `embedded_asset!` is registered and then
        // loaded *on demand*, so a module nobody asks for by name is never
        // loaded, never registers its `#define_import_path`, and every shader
        // that imports it sits waiting for a module that will never arrive.
        //
        // That failure is **silent**. It is not a compile error and not a
        // missing-asset warning — the pipeline simply never becomes ready, so
        // the phase draws nothing. It cost a round here: every M2 in the
        // world — the trees, the buildings' batches and every character —
        // vanished, with a clean log and a plausible mesh count, and the
        // screenshot looked like a camera bug. `render::sky` says the same
        // thing about `atmosphere.wgsl` two files over, which is where the
        // answer was.
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
            // The moving textures are rewritten after the loads land, so a
            // batch dressed this frame is never drawn holding the identity
            // matrix its material was built with — which for a scroll is one
            // frame of the texture at rest and for a *rotation* is a jump.
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
// …and the three the cache holds directly, which stay internal to `models`.
use loader::{material_for, receive_models, Loaded, Loader, Request};

#[cfg(test)]
mod tests;
