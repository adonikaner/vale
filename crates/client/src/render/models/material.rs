//! The Bevy material an M2 batch is drawn through.
//!
//! It is separate from [`super`] because it faces the renderer rather than the
//! archives: a blend mode, a bind group and a specialisation key. Nothing here
//! reads a file — [`super::loader`] does that — and nothing here knows what a
//! model is.

use super::*;

// ---------------------------------------------------------------------------
// The material
// ---------------------------------------------------------------------------

/// One M2 batch's material: a texture and a blend mode.
///
/// The blend mode is not a uniform. Modes 0..6 are pipeline state — blend
/// factors, depth write, back-face culling — so they live in the
/// specialisation key and cost a pipeline variant each rather than a branch in
/// every fragment. Bevy's `AlphaMode` covers opaque, mask and blend; additive,
/// add-alpha, modulate and modulate2x have no `AlphaMode` and are set as
/// explicit blend state in [`M2Material::specialize`].
///
/// The material is bindless where the machine supports it. `#[bindless]` packs
/// many materials' textures and samplers into shared binding arrays and their
/// params into one storage-buffer array (`#[data]`), so a slab of materials —
/// 2048 resources on native, per `AUTO_BINDLESS_SLAB_RESOURCE_LIMIT` — shares
/// one bind group index. The batch-set key is
/// `(pipeline, draw function, material bind group index, mesh slab)`, so under
/// multidraw the batch sets collapse from one per material (~1,239 in a loaded
/// town) to roughly one per pipeline variant. That also removes most of the
/// per-frame binned-phase rebuild (`write_binned_instance_buffers`,
/// `prepare_preprocess_bind_groups`), which the trace named as the cause of
/// the CPU-bound frame. On a machine without bindless support the `BINDLESS`
/// shader def is absent and the derive falls back to plain per-material bind
/// groups, with the same behaviour as the non-bindless material.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
#[data(0, M2Params, binding_array(10))]
#[bindless]
#[bind_group_data(M2MaterialKey)]
pub struct M2Material {
    pub params: M2Params,
    #[texture(1)]
    #[sampler(2)]
    pub texture: Handle<Image>,
    /// The environment-map layers folded onto this batch, and the reason they
    /// are here rather than drawn separately — see
    /// [`vale_assets::world::m2::overlay_layers`].
    ///
    /// Nearly every piece of armour and every weapon in the game is a base
    /// batch plus one or two blended layers over the same triangles, not
    /// writing depth, drawn straight after it. Drawn separately, each layer is
    /// an item in the sorted phase, and a sorted item is a draw call at about
    /// 18 µs on this machine: four worn models on a geared character came to
    /// 173 of the ~195 draw calls forty players added. Folded, they are two
    /// more texture fetches in a fragment that was already running.
    ///
    /// `params.overlay.x` and `.y` are the blend modes, zero meaning "no
    /// layer". A slot with no layer still holds a handle — the base's own —
    /// because a binding cannot be empty. That costs nothing: bevy's bindless
    /// allocator ref-counts resources by id, so every unlayered material in a
    /// slab shares the one slot its base texture already occupies.
    #[texture(3)]
    #[sampler(4)]
    pub overlay_a: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub overlay_b: Handle<Image>,
    /// `M2Material::blending_mode`: 0 opaque, 1 alpha-key, 2 alpha blend,
    /// 3 additive, 4 add-alpha, 5 modulate, 6 modulate2x.
    pub blend: u16,
    /// Material flag 0x04 — do not cull back faces. Set on most foliage.
    pub two_sided: bool,
    /// Material flag 0x10 — do not write depth.
    pub no_depth_write: bool,
    /// Whether this batch's vertices sway. Only the ground foliage sets it.
    ///
    /// It is a pipeline distinction, not a uniform or a branch, in the same way
    /// as [`M2MaterialKey::scene_lit`]. It becomes the `WIND` shader def, and
    /// outside the foliage the compiled vertex stage is identical to upstream's
    /// — see `m2_vertex.wgsl`. That matters more here than for a fragment def
    /// because every skinned character in the game is drawn through this
    /// material's vertex stage.
    ///
    /// 5875 does not sway foliage. See `wind.wgsl`, which carries the
    /// measurement.
    pub wind: bool,
}

/// A batch that states no texture matrix: leave the UV alone.
///
/// The `w` of the first row is the switch, and it has to be a switch rather
/// than the identity matrix — a zero-initialised pair would map every UV in the
/// world to one texel.
pub const UV_STILL: (Vec4, Vec4) = (Vec4::ZERO, Vec4::ZERO);

#[derive(Clone, Copy, ShaderType)]
pub struct M2Params {
    /// The building's own `MOHD` ambient, which the shader adds back to the
    /// vertex colours because [`WmoGroup::shaded_colours`] subtracted it out.
    /// The two are one round trip and dropping either half leaves every room too
    /// dark by exactly the ambient. Zero for an M2, which has no such thing.
    ///
    /// [`WmoGroup::shaded_colours`]: vale_assets::world::wmo::WmoGroup::shaded_colours
    pub ambient: Vec4,
    pub alpha_cutoff: f32,
    pub unlit: f32,
    /// Light this batch from its vertex colours plus [`Self::ambient`] instead of
    /// from the outdoor sun. 1.0 or 0.0; see the note in `m2.wgsl` for why a
    /// room lit by the sun is lit through its own roof.
    pub vertex_lit: f32,
    /// This batch is an `MLIQ` surface, so its colour and its alpha come from
    /// the two fields below rather than from the texel. 1.0 or 0.0. This was the
    /// `_pad` the struct needed anyway to reach its 16-byte alignment.
    pub liquid: f32,
    /// The liquid's near colour with its shallow opacity in `w`, and its far
    /// colour with the deep opacity in `w`, taken from
    /// [`vale_assets::tables::light::LiquidLight`].
    ///
    /// Both are here because the texture has neither. `lake_a` peaks at
    /// channel 41 of 255 across the whole image and `ocean_h` at 82, greyscale;
    /// they are foam masks, and the colour of water in this game is a property of
    /// the zone and the hour, not of the water. Zero on every other batch, where
    /// `liquid` is 0 and nothing reads them.
    pub liquid_close: Vec4,
    pub liquid_far: Vec4,
    /// The batch's texture matrix, as the 2x3 affine `M2TextureTransform`
    /// resolves to: `x`, `y`, `z` of the first are `a`, `b`, `tx` and of the
    /// second are `c`, `d`, `ty`, so `u' = a*u + b*v + tx`. `w` of the first is
    /// the switch — 0 leaves the UV alone, which is every batch in the world
    /// but the two dozen that state one.
    ///
    /// It is in the material rather than in a `MeshTag`, unlike the animated
    /// colour beside it, because there is only one tag and the tint already
    /// uses it, and because the value is shared more widely than a tint: a
    /// scroll on a global sequence is the same number for every instance of the
    /// model at a given moment. The cost is that the batches carrying one are
    /// not interned (see [`Materials::moving`]): they are mutated in place every
    /// frame, and a pooled handle that mutates would change some other batch's
    /// picture.
    pub uv_row0: Vec4,
    pub uv_row1: Vec4,
    /// `.x`: 0 for everything that is not a particle quad, 1 for one, 2 for an
    /// additive one, which fogs toward black rather than toward the air —
    /// the client's own per-blend fog policy. The
    /// particle branch multiplies the texel by the quad's own over-life colour
    /// in byte space; see `m2.wgsl`. `.yzw` unused — the struct's size is the
    /// bindless stride, so growing it is a coordinated change with
    /// `m2_prepass.wgsl` and space is claimed a vec4 at a time.
    pub particle: Vec4,
    /// How opaque this batch's body is drawn, and the blend mode it had before
    /// it was made translucent.
    ///
    /// `.x` is a flat opacity multiplied into the fragment's alpha after the
    /// alpha test — 1.0 on every batch in the world, and less on one wearing
    /// `UNIT_VIS_FLAGS_CREEP` or `..._GHOST`. It is applied after the test
    /// because the cutoff is a property of the blend mode (224/255 for an alpha
    /// key), so scaling the alpha first would push a cut-out's own texels under
    /// their own threshold and cut holes in the model.
    ///
    /// `.y` is the batch's real [`M2Material::blend`] while `.x` is under 1.0.
    /// An opaque or alpha-keyed batch has to be switched to mode 2 to be
    /// translucent at all — the blend is in the pipeline key — and the pool is
    /// content-keyed, so there is nowhere else to keep the original mode.
    ///
    /// `.y` stores the mode plus one. Zero has to mean "nothing is stashed",
    /// and blend mode zero is `Opaque`, the mode almost every batch of a
    /// character's body is in. When the mode itself was stored, an opaque batch
    /// stashed `0.0`, the restore's `> 0.0` test read that as "never forced",
    /// and the batch stayed in mode 2 for the rest of its life: drawn in the
    /// transparent phase, which does not write depth and is sorted per batch, so
    /// a character that had been a ghost or stealthed came back with their limbs
    /// drawing through and behind each other. See [`Materials::with_opacity`]
    /// and [`M2Params::stashed_blend`].
    ///
    /// `.zw` unused. See [`Materials::with_opacity`], and see
    /// `crate::world::entities::tint`, which is the only writer and which states
    /// that the rule is this client's own and only the flag is measured.
    pub body: Vec4,
    /// The two folded environment-map layers' blend modes, `.x` then `.y`,
    /// as the file's own `M2Material::blending_mode` — 2 alpha, 3 additive, 4
    /// add-alpha, 5 modulate, 6 modulate2x — and 0 for "no layer", which is
    /// every batch in the world that is not a piece of armour. `.zw` unused.
    ///
    /// See [`M2Material::overlay_a`] for what a layer is and why it is a
    /// texture fetch here rather than a draw call of its own.
    pub overlay: Vec4,
    /// The ambient the model states for itself, rgb, with the number of lamps
    /// in [`Self::scene_lamps`] in `w`. Zero for everything in the world.
    ///
    /// See [`SceneLighting`] for why this is in the material.
    pub scene_ambient: Vec4,
    /// The model's own lamps, two `vec4`s each. The first is
    /// `(place.xyz, is_point)`: for a point light `place` is a position in the
    /// space the fragment's own is in, and for a directional one it is a unit
    /// vector pointing toward the light. The second is `(colour.rgb, 0)`. See
    /// `vale_assets::world::m2::M2Light::kind`, which holds the measurement
    /// that the two kinds are different.
    pub scene_lamps: [Vec4; MAX_SCENE_LAMPS * 2],
}

impl M2Params {
    /// The value of [`M2Params::body`]'s `.y` when no mode is stashed.
    ///
    /// Zero rather than a separate sentinel because every material in the world
    /// is already built with zero there. The stashed value is offset instead:
    /// see [`stash_blend`].
    pub const NO_STASH: f32 = 0.0;

    /// Remember the blend mode a batch really has, while it is forced to 2 to
    /// be blendable at all.
    fn stash_blend(&mut self, blend: u16) {
        self.body.y = stash_blend(blend);
    }

    /// Read the stashed blend mode back, or `None` for a batch that never had
    /// one forced.
    fn stashed_blend(&self) -> Option<u16> {
        stashed_blend(self.body.y)
    }
}

/// A blend mode as [`M2Params::body`]'s `.y` carries it: the mode plus one.
///
/// Free functions as well as the two methods above so that the rule can be
/// tested without an `M2Params`, which has no meaningful zero value (see
/// [`UV_STILL`]: a zeroed material is a wrong material, not a blank one).
///
/// The plus one fixes a reported bug. Zero has to mean "nothing is stashed",
/// and blend mode zero is `Opaque` — the mode nearly every batch of a
/// character's body is in.
fn stash_blend(blend: u16) -> f32 {
    f32::from(blend) + 1.0
}

/// The inverse of [`stash_blend`], or `None` for a batch whose mode was never
/// forced.
fn stashed_blend(y: f32) -> Option<u16> {
    (y > M2Params::NO_STASH).then(|| y as u16 - 1)
}

/// How many of a model's own lights the shader carries.
///
/// Four, because that is how many every one of the game's glue scenes states —
/// `vale glue` counts 23 over 13 models and the busiest has four. A model
/// stating more would have the rest dropped, which is a documented loss rather
/// than a wrong picture; nothing in 1.12 does.
pub const MAX_SCENE_LAMPS: usize = 4;

/// The lighting a model states for itself, resolved into what the shader
/// reads — see `vale_assets::world::m2::M2Light` for the block and the measurement.
///
/// Nothing in the world uses one: outdoors a model is lit by `Light.dbc`'s sun
/// and fill, indoors by its room's baked colour, and neither uses a lamp inside
/// an `.m2`. The screens before the world do. `UI_MainMenu` states a near-black
/// warm ambient and three short-range lamps, and its stone is authored to be
/// dark except where the brazier reaches it. Lit by the world's daylight
/// instead, the same stone draws at nearly its own texture value, which gives a
/// flat, pale login screen.
///
/// These values are in the material rather than in the view bindings. The sun
/// and the fill are in Bevy's own light uniform because they belong to the
/// view — one sun, every surface. These belong to the model, they are point
/// lights, and `m2.wgsl` has no clustered path to read them through. In the
/// material, the pool interns two otherwise-identical batches separately when
/// their scenes differ, which is correct, and the world pays nothing because
/// every one of these is zero there.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SceneLighting {
    /// rgb, and `w` is how many of [`Self::lamps`] are real.
    pub ambient: Vec4,
    pub lamps: [Vec4; MAX_SCENE_LAMPS * 2],
}

impl SceneLighting {
    /// What every model in the world states, which is nothing: outdoors the
    /// sun and the fill light it, indoors the room's own colour, and no lamp in
    /// either.
    pub const NONE: SceneLighting = SceneLighting {
        ambient: Vec4::ZERO,
        lamps: [Vec4::ZERO; MAX_SCENE_LAMPS * 2],
    };

    /// Resolve a model's own light block into what the shader reads.
    ///
    /// `place` maps a light's `(bone, position)` into the space the fragment's
    /// world position is in — for a glue scene that is the backdrop's own pose,
    /// composed by the caller, because only the caller knows where the scene
    /// stands.
    ///
    /// Every light's ambient is summed into [`Self::ambient`] and each one
    /// that throws something becomes a lamp; every glue scene has one entry that
    /// is only a fill and two to three that are only lamps. A model stating more
    /// than [`MAX_SCENE_LAMPS`] lamps keeps the first four, which is a documented
    /// loss and one nothing in 1.12 reaches.
    ///
    /// `place` maps a point light's position; a directional one's aim comes
    /// off the light itself and needs no mapping but the axis change, which is
    /// the same `place` closure applied to a direction.
    pub fn resolve(
        lights: &[vale_assets::world::m2::M2Light],
        mut place: impl FnMut([f32; 3]) -> Vec3,
    ) -> SceneLighting {
        let mut scene = SceneLighting::NONE;
        let mut lamps = 0usize;
        for light in lights {
            let [ar, ag, ab] = light.ambient_colour();
            scene.ambient += Vec4::new(ar, ag, ab, 0.0);
            if !light.is_lamp() || lamps == MAX_SCENE_LAMPS {
                continue;
            }
            let [dr, dg, db] = light.diffuse_colour();
            // A position for a point light, a direction for a directional one,
            // and `w` says which — see `M2Light::kind` for the arithmetic.
            // Swapping the two puts `UI_Human`'s two key lights 92 and 127
            // yards from the character they are aimed at, which under any
            // falloff leaves the character lit by its ambient alone.
            let (place, kind) = if light.is_point() {
                (place(light.position), 1.0)
            } else {
                (place(light.direction).normalize_or_zero(), 0.0)
            };
            scene.lamps[lamps * 2] = place.extend(kind);
            scene.lamps[lamps * 2 + 1] = Vec4::new(dr, dg, db, 0.0);
            lamps += 1;
        }
        // The lamp count is also the shader's switch. A scene with an ambient
        // and no lamps must still take this branch rather than the sun's —
        // `UI_Human`'s fill is most of what lights it.
        scene.ambient.w = lamps as f32;
        if lamps == 0 && scene.ambient.truncate() == Vec3::ZERO {
            return SceneLighting::NONE;
        }
        // A model whose only statement is a fill still has to select the scene
        // branch, and the shader picks on `w`. Half a lamp is the smallest thing above the
        // `> 0.5` test and rounds to zero lamps in the loop.
        scene.ambient.w = scene.ambient.w.max(0.6);
        scene
    }

    /// Whether anything is stated at all. False for every model in the world.
    pub fn is_lit(&self) -> bool {
        self.ambient.w > 0.0 || self.ambient.truncate() != Vec3::ZERO
    }

    /// A short, stable spelling for [`super::dressing_key`].
    ///
    /// The lights are part of a dressing's identity and cannot be left out:
    /// the character standing on the character-select plinth is the same body,
    /// the same skins and the same geosets as the one in the world, and only
    /// this tells the two apart. A key that ignored it would hand the plinth
    /// the world's build, or hand the world the plinth's.
    pub fn key(&self) -> String {
        if !self.is_lit() {
            return String::new();
        }
        let mut key = String::from("|lit");
        for value in std::iter::once(&self.ambient).chain(&self.lamps) {
            for c in [value.x, value.y, value.z, value.w] {
                key.push_str(&format!(",{:x}", c.to_bits()));
            }
        }
        key
    }
}

/// What the `#[data]` attribute ships to the GPU: the whole material converts
/// to its uniform half, and in bindless mode every material in a slab lands in
/// one storage-buffer array of these.
impl From<&M2Material> for M2Params {
    fn from(material: &M2Material) -> M2Params {
        material.params
    }
}

/// The part of a material that changes the pipeline rather than the bindings.
///
/// Small, cheap to hash, and it decides the pipeline variant — so two batches
/// that differ only in texture share one.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct M2MaterialKey {
    blend: u16,
    two_sided: bool,
    no_depth_write: bool,
    /// Whether this batch's model states its own lighting. This is a pipeline
    /// distinction, not a branch.
    ///
    /// The scene path is a loop over a dynamically indexed uniform array and a
    /// second `srgb_to_linear`, and only two screens use it, so the world must
    /// not compile it at all, let alone step over it per fragment. A `select`
    /// in the fragment costs every M2 pixel in the game the array indexing
    /// whether or not the branch is taken, and a driver answers that shape by
    /// spilling the whole uniform into indexable temporaries. As a shader def,
    /// the pipeline outside those two screens is identical to the one compiled
    /// without the scene path.
    scene_lit: bool,
    /// Whether this batch's vertices sway — see [`M2Material::wind`]. One
    /// more pipeline variant, used by the foliage's materials only.
    wind: bool,
}

impl From<&M2Material> for M2MaterialKey {
    fn from(material: &M2Material) -> Self {
        M2MaterialKey {
            blend: material.blend,
            two_sided: material.two_sided,
            no_depth_write: material.no_depth_write,
            scene_lit: material.params.scene_ambient.w > 0.0,
            wind: material.wind,
        }
    }
}

/// Every distinct material in the world, so that two batches wanting the same
/// one get the same handle.
///
/// This is not a memory optimisation. Bevy's batch-set key is
/// `(pipeline, draw function, material bind group index, mesh slab)`, and on a
/// machine where [`M2Material`]'s `#[bindless]` falls back to plain bind
/// groups, `MaterialBindGroupAllocator` gives every `M2Material` asset its
/// own bind group index. So two batches built by separate `materials.add`
/// calls sit in separate batch sets even when every field of the material is
/// identical, and separate batch sets are separate draw calls: multi-draw
/// indirect merges within a set and never across one. Where bindless is
/// supported a whole slab of materials shares one index, so the batch-set
/// argument is weaker, but every duplicate asset still costs a slab slot, an
/// extraction, and a per-frame allocator walk. The interning is needed on both
/// kinds of machine.
///
/// `vale wmos` measures the cost: `stormwind.wmo` is one building of 3,042
/// batches naming 155 textures, so without interning it is 3,042 unmergeable
/// draw calls where 186 distinct materials exist. Every one of them is opaque
/// or alpha-keyed — none is translucent — so nothing about the geometry forces
/// them apart. Interning reduces the set count by 16x and changes no pixel.
///
/// This does not merge the geometry. Batches sharing a material could be
/// concatenated into one mesh, and the same measurement says that would leave
/// 186 meshes rather than 186 batch sets, but each would span the whole city,
/// which is the granularity frustum culling works at. `vale wmos` prints the
/// per-group figure beside it (2,581, a 15% saving) so the trade stays
/// checkable: merging saves almost nothing here and loses what the per-batch
/// entity provides.
///
/// ## Why the pool holds asset ids rather than handles
///
/// A pool of strong handles never releases anything: every material ever
/// built stays in `Assets<M2Material>`, and with it the `Handle<Image>` it
/// binds. Walking or teleporting across zones then accumulates every texture of
/// every zone visited for the life of the process, with nothing on the HUD
/// moving except the material count. That matches the "it is fast when I start
/// and slower an hour later" report: the growth is in GPU memory, which no
/// frame-time number shows until the driver starts paging.
///
/// So the entry is an [`AssetId`], which owns nothing, and [`Materials::intern`]
/// upgrades it with `Assets::get_strong_handle`, which answers `None` once the
/// last real holder (a cached dressing, a spawned batch) has dropped it. The
/// asset is then gone and the next intern rebuilds it. The identity is
/// unchanged: two batches meaning the same thing still share one handle and
/// therefore one batch set, for as long as either of them exists.
///
/// An id whose asset has been dropped leaves a dead key behind — 32 bytes, and
/// [`MaterialPool::prune`] sweeps them. A dead key cannot resolve to the wrong
/// asset: an `AssetId::Index` carries a generation, so a recycled slot does not
/// answer to the old id.
#[derive(Resource, Default)]
pub struct MaterialPool {
    by_key: HashMap<MaterialKey, AssetId<M2Material>>,
}

impl MaterialPool {
    /// How many distinct materials the world holds.
    ///
    /// This is the draw-call floor for everything drawn through
    /// [`M2Material`] — terrain excepted, which has its own — because a batch
    /// set cannot span two of them. Worth having on the HUD beside the batch
    /// counts: those say how much geometry is in the world and this says how
    /// little of it the GPU has to be told about separately.
    ///
    /// Counts the keys, dead ones included, which is why [`Self::prune`] runs
    /// on the residency sweep: the number on the HUD is the one the report
    /// "the material count only ever goes up" would be read off.
    pub fn distinct(&self) -> usize {
        self.by_key.len()
    }

    /// Forget the keys whose material has been dropped by everything that held
    /// it. Answers how many went.
    ///
    /// Nothing depends on this having run — an intern of a dead key rebuilds
    /// the material and overwrites the entry — so it is bookkeeping, and it
    /// keeps [`Self::distinct`] an accurate count of what is resident.
    pub fn prune(&mut self, assets: &Assets<M2Material>) -> usize {
        let before = self.by_key.len();
        self.by_key.retain(|_, id| assets.contains(*id));
        before - self.by_key.len()
    }
}

/// A material's identity: every field of [`M2Material`], hashably.
///
/// The floats go in by their bits rather than by value because that is the only
/// total equality a float has, and it is the correct one here, since these
/// numbers are copied from a file rather than computed, so two batches meaning
/// the same thing carry the same bits.
#[derive(PartialEq, Eq, Hash)]
pub(super) struct MaterialKey {
    texture: AssetId<Image>,
    /// The two folded layers' textures — see [`M2Material::overlay_a`]. Their
    /// blend modes are in `params.overlay` and reach the key through
    /// [`Self::params`] with everything else.
    overlays: [AssetId<Image>; 2],
    blend: u16,
    two_sided: bool,
    no_depth_write: bool,
    /// See [`M2Material::wind`]. In the key because two materials differing
    /// only in it are two pipelines, and sharing a handle between them would
    /// draw the foliage through the still program or the world through the
    /// swaying one.
    wind: bool,
    /// `ambient` (4), `alpha_cutoff`, `unlit`, `vertex_lit`, `liquid`,
    /// `liquid_close` (4), `liquid_far` (4), `uv_row0` (4), `uv_row1` (4),
    /// `particle` (4), `body` (4), `overlay` (4), `scene_ambient` (4) and
    /// `scene_lamps` (8 x 4).
    params: [u32; 72],
}

impl MaterialKey {
    pub(super) fn of(material: &M2Material) -> MaterialKey {
        // Destructured exhaustively. A field added to `M2Material` and not
        // added here would make two materials that differ in it hash the same,
        // so the second batch would be drawn with the first one's handle, which
        // is the first one's texture and the first one's uniform, with no
        // warning. Binding every field by name makes forgetting one a compile
        // error instead.
        let M2Material {
            params,
            texture,
            overlay_a,
            overlay_b,
            blend,
            two_sided,
            no_depth_write,
            wind,
        } = material;
        let M2Params {
            ambient,
            alpha_cutoff,
            unlit,
            vertex_lit,
            liquid,
            liquid_close,
            liquid_far,
            uv_row0,
            uv_row1,
            particle,
            body,
            overlay,
            scene_ambient,
            scene_lamps,
        } = params;
        let mut key = MaterialKey {
            texture: texture.id(),
            overlays: [overlay_a.id(), overlay_b.id()],
            blend: *blend,
            two_sided: *two_sided,
            no_depth_write: *no_depth_write,
            wind: *wind,
            params: [
                ambient.x.to_bits(),
                ambient.y.to_bits(),
                ambient.z.to_bits(),
                ambient.w.to_bits(),
                alpha_cutoff.to_bits(),
                unlit.to_bits(),
                vertex_lit.to_bits(),
                liquid.to_bits(),
                liquid_close.x.to_bits(),
                liquid_close.y.to_bits(),
                liquid_close.z.to_bits(),
                liquid_close.w.to_bits(),
                liquid_far.x.to_bits(),
                liquid_far.y.to_bits(),
                liquid_far.z.to_bits(),
                liquid_far.w.to_bits(),
                uv_row0.x.to_bits(),
                uv_row0.y.to_bits(),
                uv_row0.z.to_bits(),
                uv_row0.w.to_bits(),
                uv_row1.x.to_bits(),
                uv_row1.y.to_bits(),
                uv_row1.z.to_bits(),
                uv_row1.w.to_bits(),
                particle.x.to_bits(),
                particle.y.to_bits(),
                particle.z.to_bits(),
                particle.w.to_bits(),
                body.x.to_bits(),
                body.y.to_bits(),
                body.z.to_bits(),
                body.w.to_bits(),
                overlay.x.to_bits(),
                overlay.y.to_bits(),
                overlay.z.to_bits(),
                overlay.w.to_bits(),
                scene_ambient.x.to_bits(),
                scene_ambient.y.to_bits(),
                scene_ambient.z.to_bits(),
                scene_ambient.w.to_bits(),
                // The eight lamp vectors, flattened. Written as a loop below
                // rather than 32 more lines; the array is fixed-size, so the
                // arithmetic cannot run short of the slots it fills.
                0, 0, 0, 0, 0, 0, 0, 0, //
                0, 0, 0, 0, 0, 0, 0, 0, //
                0, 0, 0, 0, 0, 0, 0, 0, //
                0, 0, 0, 0, 0, 0, 0, 0, //
            ],
        };
        for (i, lamp) in scene_lamps.iter().enumerate() {
            let at = 36 + i * 4;
            key.params[at] = lamp.x.to_bits();
            key.params[at + 1] = lamp.y.to_bits();
            key.params[at + 2] = lamp.z.to_bits();
            key.params[at + 3] = lamp.w.to_bits();
        }
        key
    }
}

/// The batches whose texture is on a moving matrix, and the matrix that moves
/// it.
///
/// This is a registry rather than a per-instance channel. That is a deviation,
/// so the trade is stated here. A texture matrix changes
/// every frame, so it cannot be an interned material, but the one per-instance
/// word a batch has (`MeshTag`) already carries the animated colour, and the
/// animated colour is what makes an effect fade out, which matters more. So
/// the matrix lives in the material and the material is owned by this list,
/// mutated in place once a frame.
///
/// The cost: two instances of one model at different points in their own
/// animation share a phase. It is exact for a global-sequence scroll, which is
/// free-running wall-clock and the same number for every instance anyway. The
/// population it is inexact for is small and short-lived: `vale model`
/// counts 24 batches over 10 spell-effect models in the whole game, all of
/// them one-shot bursts. Two arcane explosions overlapping within the same
/// 800 ms would swirl in step.
#[derive(Resource, Default)]
pub struct UvAnimations {
    entries: Vec<UvAnimated>,
}

struct UvAnimated {
    /// An id, not a handle, for the reason [`MaterialPool`] holds one: a
    /// strong handle here would keep every moving-texture material ever built
    /// alive. This list is walked and written to every frame, so a dead entry
    /// costs per-frame work as well as memory. The dressing that asked for the
    /// material keeps it alive; when the dressing is dropped,
    /// [`follow_uv_animations`] drops the entry on its next pass.
    material: AssetId<M2Material>,
    anims: Arc<vale_assets::world::m2::M2TextureAnims>,
    index: u16,
    /// The model's sequence-0 window, which is the clock a non-global track
    /// runs on — the same clip the emitters and the trails use.
    clip: Option<crate::render::particles::ParticleClip>,
}

impl UvAnimations {
    /// How many batches in the world have a moving texture — for the HUD and
    /// for the check that the plumbing reaches the shader at all.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Is this material one of the ones rewritten every frame?
    ///
    /// Asked by anything that wants to copy a material and hand the copy to
    /// the same batch — [`crate::render::selection`]'s highlight is the one
    /// caller. A moving material is not pooled ([`Materials::moving`] says
    /// why), so interning a copy of one both breaks the sharing rule and stops
    /// the texture scrolling. Such a batch is left unchanged.
    pub fn is_moving(&self, material: AssetId<M2Material>) -> bool {
        self.entries.iter().any(|entry| entry.material == material)
    }
}

/// Write this frame's matrix into every batch that states one, and forget the
/// ones whose material nothing holds any more.
///
/// It is cheap because the list is short: two dozen entries, and a `get_mut`
/// on an asset is what tells the render world to re-upload that one material.
///
/// The forgetting happens here rather than on the residency sweep because
/// this pass already walks the list, and the test is the same lookup it has
/// to make anyway: a material `Assets` no longer holds is a dressing that has
/// been evicted, and an entry for one is a `get_mut` that misses sixty times a
/// second forever.
///
/// Only the materials a camera saw are written. A `get_mut` is a re-upload,
/// not only a write: the render world rebuilds the changed material's bind
/// group and re-specializes its meshes, and Tracy priced the steady stream of
/// them at most of `prepare_material_bind_groups`' ~0.76 ms/frame while nearly
/// every animated batch in the loaded tiles was off screen. Skipping a hidden
/// batch is exact, not approximate: the matrix is a pure function of
/// wall-clock time (`now_ms` below), so the frame a batch comes back into view
/// it takes the same matrix it would have carried anyway. The visibility read
/// is last frame's, which is the same one-frame staleness the particle rebuild
/// gate accepts — a mesh entering the frustum scrolls from its second visible
/// frame.
pub fn follow_uv_animations(
    time: Res<Time>,
    mut registry: ResMut<UvAnimations>,
    mut assets: ResMut<Assets<M2Material>>,
    meshes: Query<(
        &MeshMaterial3d<M2Material>,
        &bevy::camera::visibility::ViewVisibility,
    )>,
) {
    if registry.entries.is_empty() {
        return;
    }
    // Which of the animated materials something visible wears. The scan is
    // over every M2 mesh in the world, but it is two fetches and a probe of a
    // set a couple of dozen long — an order of magnitude under the bind-group
    // rebuilds it prunes.
    let animated: std::collections::HashSet<AssetId<M2Material>> =
        registry.entries.iter().map(|entry| entry.material).collect();
    let mut seen: std::collections::HashSet<AssetId<M2Material>> = std::collections::HashSet::new();
    for (material, visibility) in &meshes {
        if visibility.get() && animated.contains(&material.id()) {
            seen.insert(material.id());
        }
    }
    let now_ms = (time.elapsed_secs_f64() * 1000.0) as u32;
    registry.entries.retain(|entry| {
        if !seen.contains(&entry.material) {
            // Off screen: keep the entry, skip the re-upload. The eviction
            // test still has to run — without it an evicted dressing's entry
            // would outlive it for as long as it stayed unseen.
            return assets.contains(entry.material);
        }
        // The clip clock, the emitters' own rule: wrapped for a looping
        // sequence, held at its end for a one-shot.
        let (start, end) = match &entry.clip {
            Some(c) => (c.start, c.end),
            None => (0, 1),
        };
        let span = end.saturating_sub(start).max(1);
        let t = start + now_ms % span;
        let m = entry.anims.matrix(entry.index, t, start, end, now_ms);
        let row0 = Vec4::new(m[0], m[1], m[2], 1.0);
        let row1 = Vec4::new(m[3], m[4], m[5], 0.0);
        // Read before the write, which is also `water::dress`'s rule, for a
        // second reason as well. A `get_mut` is the re-upload whether or not
        // anything moved, and a re-upload of a material whose textures another
        // material in the same bindless slab also holds permanently costs a
        // slab resource in bevy 0.19. See `water::LiquidFlipbook::anchor`.
        //
        // This skips every track that is not moving: a single-key texture
        // animation, a global sequence standing still, and any batch whose clip
        // has run out. Those are a majority of the registry in an ordinary
        // scene, and without the read each of them is a bind-group rebuild
        // sixty times a second for a matrix that did not change.
        match assets.get(entry.material) {
            Some(current)
                if current.params.uv_row0 == row0 && current.params.uv_row1 == row1 =>
            {
                return true
            }
            // Nothing holds this material any more: the dressing that asked
            // for it has been evicted, so the entry goes with it.
            None => return false,
            Some(_) => {}
        }
        let Some(mut material) = assets.get_mut(entry.material) else {
            return false;
        };
        material.params.uv_row0 = row0;
        material.params.uv_row1 = row1;
        true
    });
}

/// The material store and its pool, as one system parameter.
///
/// Bundled because they are only correct together: adding to the store
/// without recording the handle is the bug this type makes unrepresentable.
/// The renderer had that bug when `materials.add` was reachable from four call
/// sites, none of which could deduplicate.
#[derive(SystemParam)]
pub struct Materials<'w> {
    assets: ResMut<'w, Assets<M2Material>>,
    pool: ResMut<'w, MaterialPool>,
    uv: ResMut<'w, UvAnimations>,
}

impl Materials<'_> {
    /// The handle for this material, building it only if nothing equal exists
    /// and is still alive.
    ///
    /// The second condition is how [`MaterialPool`] evicts: the pool
    /// remembers an id, and an id is only upgradable while something else still
    /// holds the asset. A material every holder has dropped answers `None` here
    /// and is rebuilt — same bits, same key, a new id.
    pub fn intern(&mut self, material: M2Material) -> Handle<M2Material> {
        let key = MaterialKey::of(&material);
        if let Some(&id) = self.pool.by_key.get(&key) {
            if let Some(handle) = self.assets.get_strong_handle(id) {
                return handle;
            }
        }
        let handle = self.assets.add(material);
        self.pool.by_key.insert(key, handle.id());
        handle
    }

    /// The same material with its vertices swaying, interned. Only the ground
    /// foliage asks for it.
    ///
    /// `None` when the handle no longer resolves (a dressing evicted between
    /// the model cache's answer and here) or when it is already a wind
    /// material. It goes through [`Self::intern`] for the reason every other
    /// derived material does: a copy handed back into circulation outside the
    /// pool is a second asset, a second bind group and a second batch set for a
    /// material that is bit-for-bit one that already exists.
    ///
    /// It does not change the texture, the blend, the cutoff or the
    /// two-sidedness, which are the model cache's own, so the swaying grass is
    /// drawn through the material the reference would have drawn it through,
    /// plus a pipeline variant.
    pub fn with_wind(&mut self, handle: &Handle<M2Material>) -> Option<Handle<M2Material>> {
        let material = self.assets.get(handle)?;
        if material.wind {
            return None;
        }
        let mut copy = material.clone();
        copy.wind = true;
        Some(self.intern(copy))
    }

    /// The same material with a different mouseover lift, interned, or
    /// `None` when there is nothing to do or nothing may be done.
    ///
    /// It answers `None` in three cases: the handle no longer resolves (a
    /// dressing rebuilt between the walk and here), the lift is already what was
    /// asked for (the common case by far, once a frame per lit part), or the
    /// material is one of the two dozen rewritten every frame — see
    /// [`UvAnimations::is_moving`], where the reason is.
    ///
    /// It lives here rather than in the caller because the pool's invariant is
    /// this file's: a copy handed back into circulation has to go through
    /// [`Self::intern`] or two batches meaning the same thing stop sharing.
    pub fn with_highlight(
        &mut self,
        handle: &Handle<M2Material>,
        lift: f32,
    ) -> Option<Handle<M2Material>> {
        if self.uv.is_moving(handle.id()) {
            return None;
        }
        let material = self.assets.get(handle)?;
        if material.params.particle.z == lift {
            return None;
        }
        let mut copy = material.clone();
        copy.params.particle.z = lift;
        Some(self.intern(copy))
    }

    /// The same material with a different aura colour painted on it,
    /// interned — Stoneform's stone, a ghost's pallor, Shadowform's purple.
    ///
    /// The same three `None`s as [`Self::with_highlight`], for the same three
    /// reasons, and it composes with it: each copies the material as it stands
    /// and rewrites one field, so a stone-formed dwarf under the pointer is one
    /// material carrying both values, and neither system overwrites the other.
    ///
    /// The colour is packed as `0xRRGGBB + 1` — see the note on `.w` in
    /// `m2.wgsl`, where the offset is explained: `Glowy (Black)` states a
    /// colour of zero and it has to be tellable from no colour at all.
    pub fn with_model_tint(
        &mut self,
        handle: &Handle<M2Material>,
        colour: Option<[u8; 3]>,
    ) -> Option<Handle<M2Material>> {
        if self.uv.is_moving(handle.id()) {
            return None;
        }
        let packed = colour.map_or(0.0, |c| {
            (1 + ((c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32)) as f32
        });
        let material = self.assets.get(handle)?;
        if material.params.particle.w == packed {
            return None;
        }
        let mut copy = material.clone();
        copy.params.particle.w = packed;
        Some(self.intern(copy))
    }

    /// The same material drawn at a different opacity, interned — a
    /// stealthed rogue, a spirit at the graveyard.
    ///
    /// The same three `None`s as [`Self::with_highlight`] for the same three
    /// reasons, and it composes with both of the others: each copies the
    /// material as it stands and rewrites its own field.
    ///
    /// Unlike the other two, this one can change the pipeline. An opaque or
    /// alpha-keyed batch is not blendable at all — [`M2Material::blend`] is in
    /// the pipeline key and decides both the phase and the blend factors — so a
    /// body asked to fade is forced to mode 2 and its real mode is stashed in
    /// `params.body.y` until it is made opaque again. A batch that was already
    /// translucent keeps its own mode: an additive glow scaled by an opacity is
    /// a dimmer glow, and forcing it to ordinary alpha blending would turn a
    /// spell effect into a decal.
    ///
    /// See `crate::world::entities::tint`, the only caller, which says which
    /// half of this is measured and which half is this client's.
    pub fn with_opacity(
        &mut self,
        handle: &Handle<M2Material>,
        opacity: f32,
    ) -> Option<Handle<M2Material>> {
        if self.uv.is_moving(handle.id()) {
            return None;
        }
        let material = self.assets.get(handle)?;
        if material.params.body.x == opacity {
            return None;
        }
        let mut copy = material.clone();
        // Solid again: put back whatever mode this batch really has. A batch
        // that kept its own mode stashed nothing and is restored to itself.
        if opacity >= 1.0 {
            if let Some(blend) = copy.params.stashed_blend() {
                copy.blend = blend;
            }
            copy.params.body.x = 1.0;
            copy.params.body.y = M2Params::NO_STASH;
            return Some(self.intern(copy));
        }
        // Going translucent from solid: modes 0 and 1 are the two that cannot
        // blend, and they are the only two that need stashing.
        if copy.params.body.x >= 1.0 && matches!(copy.blend, 0 | 1) {
            copy.params.stash_blend(copy.blend);
            copy.blend = 2;
        }
        copy.params.body.x = opacity;
        Some(self.intern(copy))
    }

    /// A handle nobody else will ever be handed, registered to be rewritten
    /// every frame from `anims[index]`.
    ///
    /// Not pooled. The pool exists so that two batches meaning the same thing
    /// share a handle and therefore a batch set; a handle whose contents are
    /// rewritten every frame must not be shared, or the other batch's picture
    /// changes with it. The cost is one extra material per batch — two dozen in
    /// the whole game.
    pub fn moving(
        &mut self,
        material: M2Material,
        anims: Arc<vale_assets::world::m2::M2TextureAnims>,
        index: u16,
        clip: Option<crate::render::particles::ParticleClip>,
    ) -> Handle<M2Material> {
        let handle = self.assets.add(material);
        self.uv.entries.push(UvAnimated {
            material: handle.id(),
            anims,
            index,
            clip,
        });
        handle
    }
}

impl Material for M2Material {
    fn fragment_shader() -> ShaderRef {
        crate::render::shader::M2.into()
    }

    /// The vertex stage: Bevy's own, with one block added.
    ///
    /// Overriding it has a cost: every M2 in the world — scenery, buildings'
    /// batches and every skinned character — is drawn through whatever is
    /// named here, so `m2_vertex.wgsl` is upstream's file copied verbatim and
    /// the addition is behind a shader def no other material sets. See that
    /// file's header, which also says what re-syncing it on a Bevy upgrade
    /// involves.
    fn vertex_shader() -> ShaderRef {
        crate::render::shader::M2_VERTEX.into()
    }

    /// The vertex stage for the depth-only passes, with the same addition as
    /// [`Self::vertex_shader`]. The world camera has no depth prepass; the
    /// depth-only pass that draws through this is the sun-shadow pass (`F10`).
    /// A displacement applied in the forward stage and not in this one would
    /// cast the shadow of the undisplaced geometry. See `m2_prepass_vertex.wgsl`.
    fn prepass_vertex_shader() -> ShaderRef {
        crate::render::shader::M2_PREPASS_VERTEX.into()
    }

    /// The alpha test again, for the depth-only passes.
    ///
    /// Declaring this is what makes Bevy run a fragment shader in a depth-only
    /// prepass at all — see the note at the top of `m2_prepass.wgsl`. Without it
    /// every alpha-keyed batch writes depth for its whole quad, so in the
    /// sun-shadow pass (`F10`) a tree casts the shadow of a rectangle rather
    /// than of its leaves.
    fn prepass_fragment_shader() -> ShaderRef {
        crate::render::shader::M2_PREPASS.into()
    }

    /// Which render phase a batch lands in.
    ///
    /// Mode 1 is a `discard`, not a blend — the shader does the test itself, and
    /// saying `Mask` here is what keeps a cut-out leaf in the opaque phase where
    /// it belongs instead of being sorted with the smoke. Everything from 2 up
    /// is genuinely translucent and has to be drawn back to front, whatever
    /// blend factors [`Self::specialize`] then gives it.
    fn alpha_mode(&self) -> AlphaMode {
        match self.blend {
            0 => AlphaMode::Opaque,
            // The cutoff, not a constant: an M2 cuts at 0.5 and a WMO at
            // 224/255, and the two share this material.
            1 => AlphaMode::Mask(self.params.alpha_cutoff),
            _ => AlphaMode::Blend,
        }
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let material = key.bind_group_data;
        // The scene-lit variant, compiled only for models that state their own
        // lights. See [`M2MaterialKey::scene_lit`].
        if material.scene_lit {
            descriptor.fragment.as_mut().map(|f| f.shader_defs.push("SCENE_LIT".into()));
        }

        // On the vertex stage, which covers the prepass's as well. `descriptor`
        // here is whichever pipeline is being specialised — the forward one or
        // the prepass one — and both take their vertex program from this same
        // `vertex` slot, so one push covers both. It is not pushed onto the
        // fragment: nothing in either fragment reads it, and an unread def
        // suggests to a later reader that something does.
        if material.wind {
            descriptor.vertex.shader_defs.push("WIND".into());
        }

        // Transcribed from `Models.applyBlend` in web/models.js, which is the
        // copy that has been checked against the game's own foliage, smoke and
        // water planes. Modes 0 and 1 do not blend at all.
        let factors = match material.blend {
            0 | 1 => None,
            3 => Some((BlendFactor::One, BlendFactor::One)),
            4 => Some((BlendFactor::SrcAlpha, BlendFactor::One)),
            5 => Some((BlendFactor::Dst, BlendFactor::Zero)),
            6 => Some((BlendFactor::Dst, BlendFactor::Src)),
            // 2, and anything the file invents: ordinary alpha blending.
            _ => Some((BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha)),
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = factors.map(|(src, dst)| {
                    let component = BlendComponent {
                        src_factor: src,
                        dst_factor: dst,
                        operation: BlendOperation::Add,
                    };
                    BlendState {
                        color: component,
                        alpha: component,
                    }
                });
            }
        }

        // Material flag 0x04, and it is set on most of the world's greenery: a
        // leaf is one alpha-cut quad and culling half of it leaves the tree
        // half there depending on where you stand.
        if material.two_sided {
            descriptor.primitive.cull_mode = None;
        }

        // Blended geometry is already depth-write-free — Bevy's transparent
        // phase sees to that — but an opaque batch can carry flag 0x10 too,
        // and a billboard that writes depth occludes whatever is behind its own
        // invisible corners.
        if material.no_depth_write {
            if let Some(depth) = descriptor.depth_stencil.as_mut() {
                depth.depth_write_enabled = Some(false);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::m2::M2Light;

    /// A body that is made opaque again goes back to the phase it came from,
    /// including from mode 0, which is the mode the stash previously lost.
    ///
    /// The stash is a float in a uniform and the restore is a `> 0.0` test, so
    /// when the mode itself was stored, "was mode 0" and "was never forced" were
    /// the same value — and mode 0 is `Opaque`, which is nearly every batch of a
    /// character's body. That was the reported bug: a character who had been a
    /// ghost or stealthed came back with every opaque batch still in mode 2,
    /// drawn in the transparent phase, which writes no depth and sorts per
    /// batch, so their limbs drew through and behind one another for the rest
    /// of the session.
    ///
    /// Tested on the params rather than through `Materials`, which needs a
    /// world and an asset server; the pair below is the entire rule.
    #[test]
    fn a_faded_batch_comes_back_to_the_blend_mode_it_started_in() {
        // An untouched material has nothing stashed, which is the state every
        // batch in the world is built in.
        assert_eq!(stashed_blend(M2Params::NO_STASH), None);
        // Only the two modes that cannot blend are ever stashed — see
        // `Materials::with_opacity`, which is where that branch is — and both
        // have to survive the round trip.
        for mode in [0u16, 1] {
            assert_eq!(stashed_blend(stash_blend(mode)), Some(mode), "mode {mode}");
        }
        // The case that was the bug: a stashed `Opaque` must not be the same
        // value as no stash at all. Written as the mode itself it was, the
        // `> 0.0` restore read it as "never forced", and
        // every opaque batch of a body that had been a ghost or stealthed
        // stayed in mode 2 — transparent phase, no depth write, sorted per
        // batch — for the rest of its life.
        assert_ne!(stash_blend(0), M2Params::NO_STASH);
    }

    /// `UI_MainMenu`'s own four lights, verbatim — see `vale glue`, which
    /// prints them. Two point lamps, one directional, and one that is nothing
    /// but a fill.
    fn main_menu() -> Vec<M2Light> {
        let light =
            |kind, diffuse: [f32; 3], k: f32, start, end, ambient: [f32; 3], ak| M2Light {
                kind,
                bone: 0,
                position: [1.0, 2.0, 3.0],
                ambient,
                ambient_intensity: ak,
                diffuse,
                diffuse_intensity: k,
                attenuation_start: start,
                attenuation_end: end,
                direction: [0.0, 0.6, 0.8],
                keys: 1,
            };
        vec![
            light(1, [0.843, 0.498, 0.192], 2.0, 3.0, 6.472, [1.0; 3], 0.0),
            light(1, [0.310, 0.482, 0.329], 0.8, 5.806, 17.472, [1.0; 3], 0.0),
            light(0, [0.224, 0.325, 0.584], 1.35, 6.222, 11.222, [1.0; 3], 0.0),
            // The fill: no diffuse at all, and the only ambient in the scene.
            light(0, [1.0; 3], 0.0, 2.222, 5.556, [0.298, 0.200, 0.200], 0.4),
        ]
    }

    /// The scene's fill is the light with no lamp in it, the lamps are the
    /// three that throw something, and a point lamp is packed by its place
    /// where a directional one is packed by its aim.
    ///
    /// The test checks the numbers, not only the shape. `UI_MainMenu`'s ambient
    /// is `(0.119, 0.080, 0.080)` — a near-black warm fill — and the model is
    /// authored for it; read as `(1, 1, 1)` (the colour without its intensity)
    /// the scene is lit to white. A directional light packed as a position is a
    /// light 92 to 127 yards from what it lights, which under any falloff
    /// contributes nothing; `w` holds the kind rather than a radius to prevent
    /// that.
    #[test]
    fn a_scenes_fill_and_its_lamps_come_apart() {
        let scene = SceneLighting::resolve(&main_menu(), Vec3::from_array);
        assert!(scene.is_lit());
        assert_eq!(scene.ambient.w, 3.0, "three lamps, and the fourth is the fill");
        assert!((scene.ambient.x - 0.298 * 0.4).abs() < 1e-5, "{}", scene.ambient.x);
        assert!((scene.ambient.y - 0.200 * 0.4).abs() < 1e-5);

        // The brazier: a point light, so its place is its position and `w` says
        // so — and its colour is the file's times its intensity.
        assert_eq!(scene.lamps[0].w, 1.0, "a point light");
        assert_eq!(scene.lamps[0].truncate(), Vec3::new(1.0, 2.0, 3.0));
        assert!((scene.lamps[1].x - 0.843 * 2.0).abs() < 1e-4, "{}", scene.lamps[1].x);

        // …and the third is directional: its place is its own unit aim, and
        // nothing about it is a distance.
        assert_eq!(scene.lamps[4].w, 0.0, "a directional light");
        assert!((scene.lamps[4].truncate().length() - 1.0).abs() < 1e-5);
        assert!((scene.lamps[5].z - 0.584 * 1.35).abs() < 1e-4);
        // The fill contributed no lamp, so the fourth slot is untouched.
        assert_eq!(scene.lamps[6], Vec4::ZERO);
    }

    /// A model that states no lights is lit by the world, not by a black
    /// ambient. That is every model in the game outside those two screens, so
    /// getting this wrong would turn the whole world dark.
    #[test]
    fn a_model_with_no_lights_takes_the_suns_branch() {
        let scene = SceneLighting::resolve(&[], Vec3::from_array);
        assert_eq!(scene, SceneLighting::NONE);
        assert!(!scene.is_lit());
        assert_eq!(scene.ambient.w, 0.0, "the shader's own switch is this");
        assert!(scene.key().is_empty(), "and it adds nothing to a dressing key");
    }

    /// A model whose only statement is a fill still takes the scene branch.
    /// The shader picks on the lamp count, so a scene with an ambient and no
    /// lamp has to read as "lit" without claiming a lamp that is not there —
    /// `UI_Human`'s fill is most of what lights it.
    #[test]
    fn a_fill_with_no_lamp_is_still_the_scenes_own_lighting() {
        let fill = vec![M2Light {
            kind: 0,
            bone: 3,
            position: [0.0; 3],
            ambient: [0.3, 0.3, 0.3],
            ambient_intensity: 0.9,
            diffuse: [0.0; 3],
            diffuse_intensity: 0.0,
            attenuation_start: 2.222,
            attenuation_end: 5.556,
            direction: [0.0, 0.0, 1.0],
            keys: 1,
        }];
        let scene = SceneLighting::resolve(&fill, Vec3::from_array);
        assert!(scene.is_lit());
        assert!(scene.ambient.w > 0.5, "the shader's switch is on");
        assert_eq!(scene.ambient.w as u32, 0, "…and it claims no lamp");
        assert!(!scene.key().is_empty(), "so it is part of the dressing");
    }
}
