//! The loader thread: archive bytes -> meshes, textures and materials.
//!
//! This code runs off the main thread and shares only its two channels with it.
//! A [`Request`] goes out, a [`Loaded`] comes back, and [`receive_models`] is
//! the only place the result meets the ECS. It is a separate module from
//! [`super`] because the channel is a thread boundary, and keeping it in its
//! own file makes that boundary visible.

use super::*;

// ---------------------------------------------------------------------------
// The loader thread
// ---------------------------------------------------------------------------

/// What the loader thread is being asked for.
pub(super) enum Request {
    Model {
        path: String,
        /// Emit the skinning attributes, if the model has a skeleton at all.
        skinned: bool,
    },
    /// One client-supplied skin, named by a DBC rather than by any model.
    Texture(String),
    /// A player's body texture, which no file holds: composed from up to nine
    /// `CharSections` layers. Answered as a [`Loaded::Texture`] under `key`, so
    /// it shares the skin cache with the textures that are files.
    Character { key: String, look: CharacterLook },
    /// The texture the hair mesh wears, which the composite does not hold:
    /// `CharSections` section 3 column 0. The composite takes the two scalp
    /// textures beside it. Keyed by appearance, as the composite is, so two
    /// characters with the same hairstyle and colour share it.
    CharacterHair { key: String, look: CharacterLook },
}

/// Something decoded off the main thread. The meshes and images inside are
/// already built (plain data until `Assets::add` moves them to the render
/// world), so the main thread's share of a load is handle bookkeeping.
pub(super) enum Loaded {
    Model {
        path: String,
        skinned: bool,
        model: Option<RawModel>,
    },
    Texture {
        path: String,
        texture: Option<Image>,
    },
}

pub(super) struct RawModel {
    /// Parallel to `M2::textures`; `None` for a slot the model does not supply
    /// itself (a creature's skin, which arrives from a DBC in the entity pass).
    /// Already an [`Image`], built on the loader thread; see
    /// [`RawTexture::into_image`].
    textures: Vec<Option<Image>>,
    /// The texture type of each of those slots, which says whether the client
    /// fills the slot and with what; see [`skin_slot`].
    kinds: Vec<u32>,
    draws: Vec<PreparedDraw>,
    /// `None` for scenery, and for a skeleton that did not validate: a model in
    /// its bind pose is drawn instead of one folded inside out.
    skeleton: Option<M2Skeleton>,
    /// The model's own declared bounding box, in the file's axes.
    bounds: [[f32; 3]; 2],
    /// Where a portrait of this model is taken from, in the file's axes: its
    /// own kind-0 camera, or a framing derived from its bind pose for the seven
    /// models that carry none. See [`vale_assets::look::portrait`], which holds
    /// the rule, and `crate::render::portraits`, which is the only reader.
    portrait: vale_assets::look::portrait::Framing,
    /// Where a whole-body shot of this model is taken from: its own kind-1
    /// camera, or a framing derived from the same bind pose. Used by
    /// `<PlayerModel>`; see [`vale_assets::look::portrait::body_framing`] and
    /// `crate::render::paperdoll`, which is its only reader.
    body: vale_assets::look::portrait::Framing,
    /// Whether this model leans with the ground it stands on: the header's
    /// `GlobalModelFlags & 3`, the only data in the game that says so. See
    /// [`vale_assets::look::conform`].
    conform: vale_assets::look::conform::Conform,
    /// The points other models hang from, in the file's axes.
    attachments: Vec<M2Attachment>,
    /// The `$CSD`/`$SND` events, sorted into their sequences; see
    /// `sound::cues`. Built here, once per file, from `M2::events`.
    cues: vale_assets::world::m2::SoundCues,
    /// The `$WTB`/`$WTT` points a weapon's trail runs between, when the model
    /// carries both; see [`vale_assets::look::weapon_trail`].
    trail: Option<vale_assets::look::weapon_trail::TrailPoints>,
    /// The lights this model carries, already in Bevy's axes and already
    /// coloured; see [`crate::render::lamps::ModelGlow`]. Empty for almost
    /// everything; a lamppost has one. Built here rather than in the renderer
    /// because the colour is the texture's average, and the pixels exist only
    /// on this thread, between the decode and the upload.
    glows: Vec<crate::render::lamps::ModelGlow>,
    /// The `BoundingTriangles` hull, in the file's axes: the doodad half of
    /// [`vale_assets::world::collision`]. Empty for most of the world, which
    /// means the player walks through the model.
    collision: vale_assets::world::collision::CollisionMesh,
    /// The drawn triangles, in the file's axes, for the mouse pick's narrow
    /// phase; see [`vale_assets::look::pick`]. A different set from the hull
    /// above, present on every model, where the hull is present on almost none.
    pick: vale_assets::look::pick::PickMesh,
    /// The sphere every branch of the pick's broad phase falls back to: the
    /// header's `boundingBox` centre and its own `boundingRadius`, in the
    /// file's axes. Read rather than derived from [`Self::bounds`]: the file
    /// states the radius, and a half-diagonal of the box is a different number.
    model_sphere: vale_assets::look::pick::Sphere,
    /// The particle emitters, in the file's axes, plus the sequence-0 clip the
    /// emitter clock runs on. Empty for most of the world.
    particles: Vec<vale_assets::world::m2::M2Particle>,
    /// The ribbon trails, on the same clip and empty on the same terms.
    ribbons: Vec<vale_assets::world::m2::M2Ribbon>,
    /// The model's own cameras, in the file's axes. Empty for everything in
    /// the world; the two glue screens take their whole framing from them. See
    /// `crate::render::glue`.
    cameras: Vec<vale_assets::world::m2::M2Camera>,
    /// The model's own lights: the same kind of block as the cameras, read for
    /// the same screens.
    lights: Vec<vale_assets::world::m2::M2Light>,
    clip: Option<crate::render::particles::ParticleClip>,
    /// The colour and transparency tracks, `None` on the unskinned build for
    /// the same reason [`Self::skeleton`] is.
    tints: Option<vale_assets::world::m2::M2Tints>,
    /// The texture matrices, on both the skinned and unskinned builds; see
    /// `RawDraw::uv`.
    uv_anims: Option<vale_assets::world::m2::M2TextureAnims>,
}

/// A decoded BLP and its mip chain, still plain data. Shared with the WMO and
/// terrain passes.
pub(crate) struct RawTexture {
    pub width: u32,
    pub height: u32,
    /// Every level, top first. The chain is the one stored in the file; no
    /// level is generated here.
    pub levels: Vec<Vec<u8>>,
}

impl RawTexture {
    pub fn from_blp(blp: vale_assets::Blp) -> Self {
        let (width, height) = (blp.width, blp.height);
        let mut levels = Vec::with_capacity(1 + blp.mips.len());
        levels.push(blp.rgba);
        levels.extend(blp.mips);
        RawTexture {
            width,
            height,
            levels,
        }
    }

    /// The finished [`Image`], built where the decode happened.
    ///
    /// Called on the loader thread: an `Image` is plain data until
    /// `Assets::add` hands it to the render world. Building it here, including
    /// the mip chain concatenation, keeps that copy off the main thread, where
    /// it was a per-texture cost at every tile crossing.
    pub fn into_image(self) -> Image {
        model_image(self.width, self.height, self.levels)
    }
}

/// Whether an environment-map layer is folded into the batch under it; see
/// [`vale_assets::world::m2::overlay_layers`].
///
/// Read once and cached: this is on the loader thread, once per model.
fn folding() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VALE_NO_FOLD").is_none())
}

/// One batch's geometry, already in Bevy's axes and remapped to its own vertices.
///
/// Shared with the WMO pass, which produces these from a `WmoDraw`. A building
/// is drawn through this material and this shader, as the WebGL renderer drew
/// one through `models.js`' program. The two differ only in parameters:
/// [`Self::colours`] is empty for an M2, and the alpha-key cutoff is an
/// argument to [`upload_prepared`].
pub(crate) struct RawDraw {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Baked `MOCV` light, one per vertex, or empty; an M2 has none.
    ///
    /// Empty is not the same as black: Bevy sets its `VERTEX_COLORS` shader def
    /// from the mesh's attributes, so a mesh without this attribute compiles a
    /// variant that does not read it at all rather than one that reads zeros.
    pub colours: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    /// Four bones per vertex, indexing this model's joints, or empty on anything
    /// with no skeleton. A vertex with no weights at all carries the identity
    /// joint at index `bones`, because `M2::skin_position` leaves such a vertex
    /// where it is and the skinning shader would otherwise collapse it to the
    /// origin.
    /// These are indices into [`Self::bones`], not into the model's skeleton;
    /// see that field for why the subset exists.
    pub joints: Vec<[u16; 4]>,
    /// The matching weights, normalised to sum to 1.
    pub weights: Vec<[f32; 4]>,
    /// The model bones this batch's vertices use, sorted, with the identity
    /// joint (index `bones`) among them when any vertex is weightless. Empty on
    /// anything with no skeleton.
    ///
    /// This subset keeps the per-frame cost of a crowd down. Bevy's
    /// `extract_skins` walks every joint of every visible skinned mesh every
    /// frame and writes a matrix per joint into the skin buffer, so a batch
    /// carrying the model's whole skeleton pays for all of it whether it uses
    /// two bones or a hundred. A dressed `HumanMale` draws ~18 batches over a
    /// 119-bone skeleton, so the full list is 2,160 joint reads and 138 KB of
    /// buffer writes per character per frame, nearly all of it for bones the
    /// batch's vertices never reference.
    ///
    /// The subset is exact: the matrices are the same matrices, indexed
    /// compactly. See [`batch_draw`], which builds it in the same pass that
    /// remaps the vertices.
    pub bones: Vec<u16>,
    /// The M2 geoset this batch belongs to; 0 for a WMO, which has no such
    /// concept and is always drawn whole.
    pub geoset: u16,
    /// Index into the model's texture list.
    pub texture: Option<usize>,
    pub blend: u16,
    pub unlit: bool,
    pub two_sided: bool,
    pub no_depth_write: bool,
    /// Which of the three lighting models this draw uses; see
    /// [`vale_assets::world::wmo::BatchLight`], which explains why there are
    /// three.
    ///
    /// Always [`BatchLight::Sun`] for an M2 as it comes off the file; an M2
    /// standing inside a building is changed to `Bake` by `lit_by_room` at the
    /// material, because nothing in an M2 file says it is indoors.
    pub light: BatchLight,
    /// Which liquid this draw is, if it is one; see
    /// [`vale_assets::world::wmo::WmoDraw::liquid`]. `None` for masonry, for a
    /// doodad and for an entity.
    ///
    /// This holds the liquid kind, not the resolved colour, because it is built
    /// on the loader thread and the colour depends on the map, which only the
    /// main thread knows. `receive_wmos` does the lookup.
    pub liquid: Option<Liquid>,
    /// Which of the model's colour and transparency tracks fade this batch.
    /// `None` for a WMO, for a doodad, and for the great majority of M2
    /// batches. See [`ModelDraw::tint`].
    pub tint: Option<BatchTint>,
    /// The tint for a doodad batch: sampled once at the start of the timeline,
    /// as a `MeshTag`, or `None` when the batch has no tint or this is the
    /// skinned build. See the note where it is built: without it, a light cone
    /// whose own track says 0.16 is drawn at 1.0.
    pub baked_tint: Option<u32>,
    /// Which of the model's texture matrices moves this batch's texture,
    /// already resolved through the lookup. `None` for all but two dozen
    /// batches in the game (`vale model`).
    pub uv: Option<u16>,
    /// The environment-map layers folded onto this batch: which of the model's
    /// texture slots each reads and the blend mode it uses. See
    /// [`vale_assets::world::m2::overlay_layers`], which holds the rule, and
    /// `M2Material::overlay_a`, which stores the result. Both `None` for every
    /// batch in the world that is not a piece of armour or a weapon.
    pub overlays: [Option<(u32, u16)>; vale_assets::world::m2::MAX_OVERLAYS],
    /// This batch is a flat rectangle in the model's own ground plane; see
    /// [`vale_assets::world::m2::M2::ground_quad`]. 120 batches over 34 models in
    /// the whole spell-effect population and `None` for everything else; a
    /// caller that draws on the floor re-renders it through
    /// [`crate::render::decals`] instead of spawning the mesh.
    pub ground: Option<vale_assets::world::m2::GroundQuad>,
}

/// One batch's geometry kept on the CPU so a dressing can merge it into a
/// neighbour.
///
/// A dressed character draws about twenty mesh instances and every one of them
/// moves every frame. Characters are the only population in this renderer that
/// costs anything per instance: a static doodad is extracted once and then
/// costs nothing (615 of them account for 0.1 ms), while a moving, skinned one
/// is re-propagated, re-extracted and re-skinned every frame. The cost of a
/// crowd therefore scales with mesh instances per character, and most of a
/// character's instances share a material: the body's opaque batches all wear
/// the one composed skin, and its alpha-keyed ones all wear the second texture.
///
/// They cannot be merged when the file is read, because which batches a
/// character draws depends on its dressing (the geosets its gear selects), and
/// the merge has to happen after that filter. So the vertex arrays are kept
/// beside the uploaded mesh, and [`super::ModelCache::merge_draws`] concatenates
/// whichever ones a given dressing kept.
///
/// Kept only for skinned builds, and only for batches that write depth. The
/// scenery build is drawn in its bind pose and never moves, so it has nothing
/// to gain and would pay the memory. Merging two translucent batches would put
/// them in one draw with one depth order, which discards the ordering that the
/// file's batch order specifies.
pub(crate) struct MergeSource {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Indices into [`Self::bones`], as [`RawDraw::joints`] is.
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    /// See [`RawDraw::bones`]; the merge takes the union of the members' lists.
    pub bones: Vec<u16>,
}

/// A [`RawDraw`] whose geometry has already been turned into a [`Mesh`], plus
/// the fields the material is built from. The split follows the two threads:
/// the mesh is pure data and is built where the file was decoded, and the
/// material depends on things only the main thread knows (the map's liquid
/// tint, the texture handles, the material pool).
///
/// Building the mesh here keeps tile crossings from stalling the frame:
/// `draw_mesh` clones every vertex buffer, and doing that on the main thread
/// for a building's three thousand batches took most of the stalled frame.
/// `Assets::add` of a prebuilt mesh is a move.
pub(crate) struct PreparedDraw {
    pub mesh: Mesh,
    /// The same geometry kept on the CPU, when this batch may be merged into a
    /// neighbour; see [`MergeSource`]. `None` for scenery, for a building, and
    /// for every batch whose draw order matters.
    pub merge: Option<std::sync::Arc<MergeSource>>,
    /// See [`RawDraw::bones`]. Shared rather than cloned: every placement of a
    /// model builds a `SkinnedMesh` from this and the list is per batch, not
    /// per instance.
    pub bones: std::sync::Arc<[u16]>,
    pub geoset: u16,
    pub texture: Option<usize>,
    pub blend: u16,
    pub unlit: bool,
    pub two_sided: bool,
    pub no_depth_write: bool,
    /// See [`RawDraw::light`].
    pub light: BatchLight,
    /// See [`RawDraw::liquid`]; still the kind, resolved on the main thread.
    pub liquid: Option<Liquid>,
    /// See [`RawDraw::tint`].
    pub tint: Option<BatchTint>,
    /// See [`RawDraw::baked_tint`].
    pub baked_tint: Option<u32>,
    /// See [`RawDraw::uv`].
    pub uv: Option<u16>,
    /// See [`RawDraw::ground`].
    pub ground: Option<vale_assets::world::m2::GroundQuad>,
    /// See [`RawDraw::overlays`].
    pub overlays: [Option<(u32, u16)>; vale_assets::world::m2::MAX_OVERLAYS],
}

impl PreparedDraw {
    /// Consumes the draw so the vertex buffers move into the mesh instead of
    /// being cloned, which is the purpose of preparing off the main thread.
    pub fn from_raw(draw: RawDraw) -> PreparedDraw {
        let bones: std::sync::Arc<[u16]> = draw.bones.as_slice().into();
        // Which batches may be merged. Each excluded case is one where merging
        // would change the rendered result, not only the cost:
        //
        // * no skeleton: scenery, which never moves and has nothing to gain;
        // * `blend` past alpha-key: a translucent batch is ordered against its
        //   neighbours, and merging discards that order;
        // * a tint or a texture matrix: both are per batch and are carried by
        //   the instance's own `MeshTag` or its own material, so two batches
        //   merged into one would have to share a value neither of them holds;
        // * a ground quad: the caller re-renders that batch through the decal
        //   projector instead of spawning it;
        // * vertex colours: an M2 has none, and a `WMO` batch that has them is
        //   not skinned, so this check is a guard and never excludes anything.
        let mergeable = !draw.bones.is_empty()
            && draw.blend <= 1
            && draw.tint.is_none()
            && draw.uv.is_none()
            && draw.ground.is_none()
            && draw.colours.is_empty();
        let merge = mergeable.then(|| {
            std::sync::Arc::new(MergeSource {
                positions: draw.positions.clone(),
                normals: draw.normals.clone(),
                uvs: draw.uvs.clone(),
                joints: draw.joints.clone(),
                weights: draw.weights.clone(),
                indices: draw.indices.clone(),
                bones: draw.bones.clone(),
            })
        });
        let geoset = draw.geoset;
        let texture = draw.texture;
        let blend = draw.blend;
        let unlit = draw.unlit;
        let two_sided = draw.two_sided;
        let no_depth_write = draw.no_depth_write;
        let light = draw.light;
        let liquid = draw.liquid;
        let tint = draw.tint;
        let baked_tint = draw.baked_tint;
        let uv = draw.uv;
        let ground = draw.ground;
        let overlays = draw.overlays;
        PreparedDraw {
            mesh: draw_mesh(draw),
            merge,
            bones,
            geoset,
            texture,
            blend,
            unlit,
            two_sided,
            no_depth_write,
            light,
            liquid,
            tint,
            baked_tint,
            uv,
            ground,
            overlays,
        }
    }

    /// The material parameters of this draw, with the liquid light the caller
    /// resolved: `None` for anything that is not an `MLIQ` surface.
    pub fn params(&self, liquid: Option<LiquidLight>) -> DrawParams {
        DrawParams {
            geoset: self.geoset,
            texture: self.texture,
            blend: self.blend,
            unlit: self.unlit,
            two_sided: self.two_sided,
            no_depth_write: self.no_depth_write,
            light: self.light,
            liquid,
            tint: self.tint,
            // Part of the material key. The material is specialised as tinted,
            // so the `MeshTag` beside it is read as a colour: two batches with
            // different constant tints are different draws and must not share
            // a pooled handle. The count is bounded by the tinted-batch
            // population, which `vale model` puts at a few hundred over the
            // whole game.
            baked_tint: self.baked_tint,
            uv: self.uv,
            ground: self.ground,
            overlays: self.overlays,
        }
    }
}

/// The thread that reads M2s, and the two channels to it.
///
/// Both ends are behind a `Mutex` because a Bevy resource has to be `Sync` and
/// neither a `Sender` nor a `Receiver` is. Neither lock is ever contended:
/// exactly one system touches each.
pub(super) struct Loader {
    requests: Mutex<Sender<Request>>,
    results: Mutex<Receiver<Loaded>>,
}

impl Loader {
    /// `overlay` is the host's source, consulted before the archives: the cell
    /// `GameAssets::overlay_cell` shares. It is read on every request rather
    /// than once, because a host installs its overlay after this thread has
    /// started, and a chain opened without it returns the game's own bytes for
    /// a path the host is overriding. It costs an `Arc` clone per request.
    pub(super) fn start(
        gamedata_dir: String,
        overlay: Arc<Mutex<Option<vale_assets::archive::Overlay>>>,
    ) -> Loader {
        let (request_tx, request_rx) = mpsc::channel::<Request>();
        let (result_tx, result_rx) = mpsc::channel::<Loaded>();
        std::thread::Builder::new()
            .name("m2-loader".into())
            .spawn(move || {
                // Opened once, here, rather than per model: a tile is ~130
                // distinct models and the chain is ~19 archives.
                let mut archive = match Archive::open(&gamedata_dir) {
                    Ok(archive) => Some(archive),
                    Err(e) => {
                        error!("model loader: {e}");
                        None
                    }
                };
                // `CharSections.dbc`, read on the first player and then kept:
                // it is 3,671 rows and every player in the world composes from
                // it. `None` until asked for, so a session that never sees a
                // player never pays for it.
                let mut sections: Option<Option<CharSections>> = None;
                // `ItemDisplayInfo.dbc` is 29,604 rows and is only needed once
                // a player is in sight, so it is read the same way as
                // `CharSections`: on demand, and kept.
                let mut items: Option<Option<ItemDisplays>> = None;
                // Every request still gets an answer when there is no archive,
                // so the cache marks those models failed rather than pending.
                while let Ok(request) = request_rx.recv() {
                    if let Some(chain) = archive.as_mut() {
                        let current = overlay.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        chain.set_overlay(current);
                    }
                    let answer = match request {
                        Request::Model { path, skinned } => {
                            let model =
                                archive.as_mut().and_then(|a| read_model(a, &path, skinned));
                            if model.is_none() {
                                warn!("model {path} will not read");
                            }
                            Loaded::Model {
                                path,
                                skinned,
                                model,
                            }
                        }
                        Request::Texture(path) => {
                            let texture = archive.as_mut().and_then(|a| read_texture(a, &path));
                            if texture.is_none() {
                                warn!("texture {path} will not read");
                            }
                            Loaded::Texture {
                                path,
                                texture: texture.map(RawTexture::into_image),
                            }
                        }
                        Request::Character { key, look } => {
                            let table = sections.get_or_insert_with(|| {
                                let table = archive.as_mut().and_then(read_char_sections);
                                if table.is_none() {
                                    warn!("CharSections.dbc will not read; players stay magenta");
                                }
                                table
                            });
                            // `ItemDisplayInfo.dbc`, read the same way. If it
                            // fails, a player is drawn without equipment
                            // rather than not at all.
                            let wardrobe = items.get_or_insert_with(|| {
                                let table = archive.as_mut().and_then(read_item_displays);
                                if table.is_none() {
                                    warn!("ItemDisplayInfo.dbc will not read; players stay undressed");
                                }
                                table
                            });
                            let texture = match (archive.as_mut(), table.as_ref()) {
                                (Some(a), Some(t)) => {
                                    compose_character(a, t, wardrobe.as_ref(), &look)
                                }
                                _ => None,
                            };
                            // Logged because a failed composition draws the
                            // same magenta as a missing texture, and the log
                            // line is the only way to tell the two apart.
                            if texture.is_none() {
                                warn!("no skin composes for {look:?}");
                            }
                            Loaded::Texture {
                                path: key,
                                texture: texture.map(RawTexture::into_image),
                            }
                        }
                        // The hair mesh's texture, which is not part of the
                        // composite: it dresses separate geometry, and the M2
                        // requests it as texture type 6. It comes from the
                        // same `CharSections` row as the scalp (column 0, where
                        // the composite takes 1 and 2), so it is resolved here,
                        // where the table is already loaded, rather than by
                        // giving the main thread a second copy of a 4,030-row
                        // DBC.
                        Request::CharacterHair { key, look } => {
                            let table = sections.get_or_insert_with(|| {
                                let table = archive.as_mut().and_then(read_char_sections);
                                if table.is_none() {
                                    warn!("CharSections.dbc will not read; players stay magenta");
                                }
                                table
                            });
                            let texture = match (archive.as_mut(), table.as_ref()) {
                                (Some(a), Some(t)) => t
                                    .skin(&look.appearance)
                                    .hair
                                    .and_then(|path| read_texture(a, &path)),
                                _ => None,
                            };
                            // A bald style has no hair row and no geometry to
                            // dress, so the warning is logged only when the
                            // character has a hair style.
                            if texture.is_none() && look.appearance.hair_style != 0 {
                                warn!("no hair texture for {look:?}");
                            }
                            Loaded::Texture {
                                path: key,
                                texture: texture.map(RawTexture::into_image),
                            }
                        }
                    };
                    if result_tx.send(answer).is_err() {
                        return;
                    }
                }
            })
            .expect("spawn the model loader thread");
        Loader {
            requests: Mutex::new(request_tx),
            results: Mutex::new(result_rx),
        }
    }

    /// Ask for something. False if the thread is gone.
    pub(super) fn request(&self, request: Request) -> bool {
        let tx = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        tx.send(request).is_ok()
    }

    fn drain(&self) -> Vec<Loaded> {
        let rx = self.results.lock().unwrap_or_else(|e| e.into_inner());
        rx.try_iter().collect()
    }
}

/// How many joints this build poses with: the model's bone count on the
/// skinned build, and `None` on the unskinned build.
///
/// A named function so it is read together with the skeleton rule: the
/// skeleton is kept on both builds (it says whether the model moves, which is a
/// property of the file), and the joint count is kept on the skinned build only
/// (it says whether this build can be posed). Treating the two as one value
/// dropped the skeleton from the doodad build, and no scenery in the game
/// animated. See `joints_are_build_specific_and_the_skeleton_is_not`.
pub(crate) fn joint_count_for(
    skinned: bool,
    skeleton: Option<&vale_assets::world::m2::M2Skeleton>,
) -> Option<usize> {
    skinned.then(|| skeleton.map(|s| s.bones.len())).flatten()
}

/// One M2 and its own textures, decoded.
///
/// A model that will not parse yields `None` and is dropped; the rest of the
/// tile still loads.
fn read_model(archive: &mut Archive, path: &str, skinned: bool) -> Option<RawModel> {
    let bytes = archive.read(path).ok()?;
    let model = M2::parse(&bytes).ok()?;

    // `M2::textures` is parallel to the slots the batches index, so a texture
    // used by three batches is decoded once and a slot the model does not fill
    // keeps its position; the index identifies the slot.
    // Which batches are lights, read before the textures so that the decode
    // below knows which textures need an average colour. See
    // `vale_assets::world::glow`, which reads them, and `crate::render::lamps`,
    // which decides that a glow is a light.
    let glow_batches = vale_assets::world::glow::glows(&model);
    let mut glow_colours: Vec<(u32, Vec3)> = Vec::new();
    let mut textures = Vec::with_capacity(model.textures.len());
    for (slot, texture) in model.textures.iter().enumerate() {
        if texture.kind != 0 || texture.file_name.is_empty() {
            textures.push(None);
            continue;
        }
        let Some(decoded) = archive
            .read(&texture.file_name)
            .ok()
            .and_then(|raw| blp::decode_mipped(&raw).ok())
            .map(RawTexture::from_blp)
        else {
            textures.push(None);
            continue;
        };
        // The average colour must be taken here. `into_image` moves the pixels
        // into an `Image` bound for the render world, and nothing on the CPU
        // can read them afterwards. Only done for the slots a glow names, which
        // for the whole world is a few dozen 32x32 textures.
        if glow_batches.iter().any(|g| g.texture == Some(slot as u32)) {
            if let Some(mean) = crate::render::lamps::glow_colour(&decoded.levels[0]) {
                glow_colours.push((slot as u32, mean));
            }
        }
        textures.push(Some(decoded.into_image()));
    }
    let kinds = model.textures.iter().map(|t| t.kind).collect();

    // The lights, joined to their colours. A glow whose texture would not
    // decode is dropped rather than lit white, because without the texture
    // there is no colour to give it.
    let glows: Vec<crate::render::lamps::ModelGlow> = glow_batches
        .iter()
        .filter_map(|g| {
            let colour = glow_colours
                .iter()
                .find(|(slot, _)| Some(*slot) == g.texture)
                .map(|(_, c)| *c)?;
            Some(crate::render::lamps::ModelGlow::of(g, colour))
        })
        .collect();

    // Every batch is kept, not only the visible ones. Which geosets a model
    // shows is a property of the entity wearing it, not of the file (two NPCs
    // sharing `HumanMale.m2` do not share a hairstyle), so the filter cannot
    // happen here, where the result would be cached once per path. Each batch
    // keeps its geoset id and the dressing selects; the dressing is already
    // per-appearance, so this adds no cache dimension, only the meshes of the
    // variants nobody is wearing.
    //
    // `None` leaves the skinning attributes off, and with them the skinned
    // pipeline, which a doodad must not use because it has no joints to bind.
    let joint_count = joint_count_for(skinned, model.skeleton.as_ref());
    // Which batches are a layer of an earlier one. These are not drawn; their
    // texture and blend are carried by the base batch's material instead. See
    // [`vale_assets::world::m2::overlay_layers`] for the rule and every
    // condition that excludes a batch, and `M2Material::overlay_a` for what it
    // saves. `VALE_NO_FOLD=1` turns it off, as every other saving in this file
    // can be turned off for measurement.
    let layers: Vec<Option<usize>> = match folding() {
        true => vale_assets::world::m2::overlay_layers(&model.batches),
        false => vec![None; model.batches.len()],
    };
    // For each base, the layers it absorbed, in file order.
    let mut absorbed: Vec<[Option<(u32, u16)>; vale_assets::world::m2::MAX_OVERLAYS]> =
        vec![Default::default(); model.batches.len()];
    for (i, base) in layers.iter().enumerate() {
        let Some(base) = base else { continue };
        let batch = &model.batches[i];
        // A batch whose texture slot does not resolve cannot be a layer: there
        // would be nothing to sample. `overlay_layers` does not test this,
        // because resolving a texture slot is the renderer's job.
        let Some(texture) = batch.texture else { continue };
        if let Some(slot) = absorbed[*base].iter_mut().find(|s| s.is_none()) {
            *slot = Some((texture, batch.blend));
        }
    }
    let draws = model
        .batches
        .iter()
        .enumerate()
        // A layer is drawn by its base and never on its own.
        .filter(|(i, _)| layers[*i].is_none())
        .filter_map(|(i, batch)| batch_draw(&model, batch, joint_count, absorbed[i]))
        // The mesh is built here, on the loader thread, so the main thread's
        // share of this model is `Assets::add`, a move, not a copy.
        .map(PreparedDraw::from_raw)
        .collect();

    // The emitter clock: sequence 0's window, wrapped if that sequence loops.
    // Taken from the model's own sequence table before the unskinned build
    // drops the skeleton: a campfire has no joints to bind and still pulses
    // its flame on its own animation. An emitter-only model whose skeleton did
    // not validate has no clip, and the sim falls back to "a gate with any
    // nonzero key is on".
    let clip = model.skeleton.as_ref().and_then(|s| {
        let seq = s.sequences.first()?;
        Some(crate::render::particles::ParticleClip {
            start: seq.start,
            end: seq.end,
            // A sequence loops when bit 0 of its flags is clear.
            loops: seq.flags & 1 == 0,
            global_sequences: Arc::from(s.global_sequences.as_slice()),
        })
    });

    // The camera position for this model's portrait, resolved here because
    // the rule needs the vertices as well as the camera block (see
    // `vale_assets::look::portrait::bind_bounds`), and this is the last place
    // the whole `M2` is available. One `Framing` per file avoids a second parse
    // per unit frame per target change.
    let portrait = vale_assets::look::portrait::framing(&model);
    // The full-body framing, resolved in the same place for the same reason.
    // Two `Framing`s per file is 56 bytes; re-parsing an `M2` when the
    // character sheet is opened would be a disk read inside a panel.
    let body = vale_assets::look::portrait::body_framing(&model);
    // The mouse pick's narrow phase, built here for the same reason: this is
    // the last point at which the whole `M2` is available, and a pick that had
    // to re-read the file would be a disk read inside a mouse move. The
    // triangles are the same whichever build was requested: the pointer hits
    // the file's drawn geometry, not the dressing's.
    let pick = vale_assets::look::pick::PickMesh::of(&model);
    let model_sphere = vale_assets::look::pick::model_sphere(&model);

    Some(RawModel {
        textures,
        kinds,
        draws,
        glows,
        bounds: model.bounds,
        portrait,
        body,
        // Read on both builds, like the hull: a doodad never uses it, but the
        // flag is a property of the file rather than of the dressing, and
        // dropping it here would make the two builds disagree.
        conform: vale_assets::look::conform::Conform::of(model.global_flags),
        cues: model.sound_cues(),
        trail: vale_assets::look::weapon_trail::TrailPoints::of(&model.events),
        attachments: model.attachments,
        // The same hull whichever build was requested: an entity build and a
        // doodad build of one file are two meshes and one solid shape.
        collision: model.collision,
        pick,
        model_sphere,
        particles: model.particles,
        ribbons: model.ribbons,
        // Carried on both builds, like the hull and unlike the tints: a
        // camera is a property of the file rather than of the dressing, and the
        // login scene is loaded through the undressed path.
        cameras: model.cameras,
        lights: model.lights,
        clip,
        // Dropped on the unskinned build so that the two cannot disagree: a
        // batch only carries a tint index on the skinned build (see
        // `batch_draw`).
        tints: if skinned {
            Some(model.tints)
        } else {
            None
        },
        uv_anims: Some(model.uv_anims),
        // Kept on both builds. Whether joints are spawned is decided by
        // [`ModelAssets::joint_count`], which is already `None` unless
        // `skinned`, and every caller iterates `0..joint_count`, so a skeleton
        // on the unskinned build spawns no joints.
        //
        // `joint_count` says whether this build can be posed; the skeleton
        // says whether the model moves at all, which is a property of the file
        // and the same for both builds. Dropping the skeleton here with the
        // tints leaves the unskinned build (the only build a doodad has) unable
        // to say whether the model moves, so `render::doodads` never requests
        // the skinned build and no scenery animates. Tests that assert on the
        // entity build's skeleton do not detect that.
        skeleton: model.skeleton,
    })
}

/// One client-supplied skin: a creature's texture variation or a baked NPC face.
fn read_texture(archive: &mut Archive, path: &str) -> Option<RawTexture> {
    let raw = archive.read(path).ok()?;
    Some(RawTexture::from_blp(blp::decode_mipped(&raw).ok()?))
}

fn read_item_displays(archive: &mut Archive) -> Option<ItemDisplays> {
    let raw = archive
        .read(&vale_assets::tables::dbc::dbc_path("ItemDisplayInfo"))
        .ok()?;
    ItemDisplays::parse(&raw).ok()
}

fn read_char_sections(archive: &mut Archive) -> Option<CharSections> {
    let raw = archive
        .read(&vale_assets::tables::dbc::dbc_path("CharSections"))
        .ok()?;
    CharSections::parse(&raw).ok()
}

/// A player's body texture, composed rather than looked up.
///
/// Up to nine layers, each a region of the 256x256 body atlas: the base skin
/// over the whole of it, then underwear, face, scalp and beard painted into
/// their own rectangles. A layer the archive does not have is skipped (44 of
/// the table's 4,030 textures are named by rows that shipped without files).
/// Skipping loses only that layer; failing the whole composite would draw the
/// player magenta because of, for example, a missing beard.
///
/// The mips are generated here, unlike everywhere else in this client, because
/// this is the one texture with no file to take a stored mip chain from.
fn compose_character(
    archive: &mut Archive,
    sections: &CharSections,
    items: Option<&ItemDisplays>,
    look: &CharacterLook,
) -> Option<RawTexture> {
    // Which layers, in which order, over which regions: the asset crate's rule,
    // and the same call `vale dress` makes to print them. `exists` walks the
    // archive's listing, which is built once and cached.
    let recipe = vale_assets::look::dress::skin_recipe(sections, items, look, |path| {
        archive.exists(path)
    });
    if recipe.layers.is_empty() {
        return None;
    }

    let mut composite = Composite::new();
    let mut painted = 0;
    for layer in &recipe.layers {
        let Some(image) = archive
            .read(&layer.path)
            .ok()
            .and_then(|raw| blp::decode(&raw).ok())
        else {
            continue;
        };
        composite.paint(layer.region, image.width, image.height, &image.rgba);
        painted += 1;
    }
    if painted == 0 {
        return None;
    }

    let side = vale_assets::look::character::COMPOSITE_SIDE;
    let top = composite.into_rgba();
    let mut levels = vec![top.clone()];
    levels.extend(vale_assets::look::character::generate_mips(side, side, &top));
    Some(RawTexture {
        width: side,
        height: side,
        levels,
    })
}

/// One batch as its own vertex buffer.
///
/// The winding is left as the file stores it, unlike the terrain's. `models.js`
/// drew M2s with `CULL_FACE` on and the default counter-clockwise front face
/// and they rendered correctly, so the file's order is already the one wgpu
/// expects, and the change of basis in [`axes::to_bevy`] is a rotation, which
/// cannot reverse it. (The terrain needed reversing because its pass never
/// enabled culling, so its winding was never tested.)
///
/// `joint_count` is the model's bone count when it has a skeleton, and it turns
/// on the two skinning attributes. Bevy compiles its `SKINNED` shader variant
/// from the presence of those attributes, so a tree gets a pipeline that does
/// not read them rather than one that reads a bind pose.
pub(crate) fn batch_draw(
    model: &M2,
    batch: &M2Batch,
    joint_count: Option<usize>,
    // See [`RawDraw::overlays`]. `Default::default()` for a caller with no
    // batch list to fold over: the CLI checks and the tests.
    overlays: [Option<(u32, u16)>; vale_assets::world::m2::MAX_OVERLAYS],
) -> Option<RawDraw> {
    let range = batch.index_start as usize..(batch.index_start + batch.index_count) as usize;
    let indices = model.indices.get(range)?;
    if indices.is_empty() {
        return None;
    }
    // A batch authored invisible for its whole timeline is not drawn.
    //
    // Such a batch carries emitters and is not meant to be seen; see
    // [`vale_assets::world::m2::M2Tints::never_visible`], where Ironforge's
    // lava steam is the worked example. It is dropped here because the shader
    // cannot hide it: a blend-0 batch is `AlphaMode::Opaque`, `alpha_cut` is
    // 0.0 for that blend so the discard never fires, and the ROP discards the
    // alpha, so the batch would be drawn solid.
    if batch.tint.is_some_and(|tint| model.tints.never_visible(tint)) {
        return None;
    }
    // A batch whose indices run past the vertex arrays is dropped rather than
    // panicking on the first one, so a malformed file cannot crash the loader.
    let vertices = model
        .positions
        .len()
        .min(model.normals.len())
        .min(model.uvs.len());
    if indices.iter().any(|&i| i as usize >= vertices) {
        return None;
    }

    let mut remap: HashMap<u16, u32> = HashMap::default();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut joints: Vec<[u16; 4]> = Vec::new();
    let mut weights: Vec<[f32; 4]> = Vec::new();
    let mut out: Vec<u32> = Vec::with_capacity(indices.len());

    for &index in indices {
        let next = remap.len() as u32;
        let mapped = *remap.entry(index).or_insert_with(|| {
            let i = index as usize;
            // Model space is converted to Bevy's here, as terrain vertices
            // are. The placement matrix is conjugated by the same basis
            // (`axes::to_bevy_affine`), which carries the derived 180° term
            // across unchanged instead of it being adjusted by eye per model.
            positions.push(axes::to_bevy(model.positions[i]).to_array());
            normals.push(axes::to_bevy(model.normals[i]).to_array());
            uvs.push(model.uvs[i]);
            if let Some(bones) = joint_count {
                let (j, w) = vertex_skin(model, i, bones);
                joints.push(j);
                weights.push(w);
            }
            next
        });
        out.push(mapped);
    }

    // The batch's own bone subset, built in one pass over the vertices this
    // batch kept; see [`RawDraw::bones`] for what it saves. The map is a flat
    // array over the model's bones rather than a `HashMap`: a skeleton is at
    // most a few hundred entries and this runs per batch per model on the
    // loader thread.
    let bones = match joint_count {
        // The kill switch is checked here: the subset and the vertex indices
        // are built together, so turning it off at the spawn instead would
        // leave the indices remapped. See `super::subsetting`.
        Some(count) if !super::subsetting() => (0..=count as u16).collect(),
        Some(count) => {
            let mut seen = vec![u16::MAX; count + 1];
            let mut used: Vec<u16> = Vec::new();
            for vertex in &joints {
                for &bone in vertex {
                    let slot = usize::from(bone);
                    // A bone index is already clamped into range by
                    // `vertex_skin`, so anything past the end is a file this
                    // client has not seen; leave it alone rather than widening
                    // the array under it.
                    if slot < seen.len() && seen[slot] == u16::MAX {
                        seen[slot] = 0;
                        used.push(bone);
                    }
                }
            }
            used.sort_unstable();
            for (compact, &bone) in used.iter().enumerate() {
                seen[usize::from(bone)] = compact as u16;
            }
            for vertex in &mut joints {
                for bone in vertex {
                    if let Some(&compact) = seen.get(usize::from(*bone)) {
                        *bone = compact;
                    }
                }
            }
            used
        }
        None => Vec::new(),
    };

    Some(RawDraw {
        positions,
        normals,
        uvs,
        // An M2 carries no baked light; see the note on the field.
        colours: Vec::new(),
        indices: out,
        joints,
        weights,
        bones,
        overlays,
        geoset: batch.geoset,
        texture: batch.texture.map(|t| t as usize),
        blend: batch.blend,
        unlit: batch.unlit,
        two_sided: batch.two_sided,
        no_depth_write: batch.no_depth_write,
        light: BatchLight::Sun,
        // Liquid is a WMO's `MLIQ`; an M2 has none.
        liquid: None,
        // Only the skinned build fades. A tint is sampled on an animation
        // clock, and the unskinned build is drawn in its bind pose with no
        // clock. The two builds are already separate cache entries
        // (`geometry_key`), so this is a property of the entry rather than a
        // special case.
        tint: batch.tint,
        // The tint value sampled once, for builds that do not animate it.
        //
        // Dropping the tint on a doodad would save a tag and a material
        // variant, but the constant value is often not 1, so it is baked
        // instead. `GnomeStructuralSpotlight02.m2`'s light cone is the worked
        // example: batch 2 is `add-alpha`, `unlit`, `no-depth-write`, and its
        // transparency track is a single key at 0.160. Drawn at 1.0 it is an
        // opaque white sheet the size of a building (seen in Gadgetzan). Every
        // faint glow in the game has the same problem: a brazier's flame halo
        // drawn as a grey box, Orgrimmar's light shafts drawn as grey planes.
        //
        // Sampled at the start of the timeline with no sequence, which for this
        // population is the whole track; see the census in `vale model`,
        // which prints a constant track as `(identity — dropped)` and a real one
        // as `(moves)`.
        //
        // Baked on both builds. A batch with a tint track is specialised as
        // tinted, so the `MeshTag` beside it is read as `0xAARRGGBB`. On an
        // entity, `world::entities::effects` writes the sampled tint every
        // frame. On scenery nothing writes it: `render::doodads::pose_scenery`
        // writes joint transforms and no tag. Without a baked value the spawner
        // falls through to its room-light arm and the tinted shader reads a
        // room colour as a tint, so an animated doodad authored faint is drawn
        // at the room's brightness (Ironforge's lava steam draws warm
        // orange-brown instead of invisible).
        //
        // A doodad whose tint animates gets its first frame held, matching the
        // bind pose its bones hold; an entity overwrites this on its first
        // tint write.
        baked_tint: batch
            .tint
            .map(|tint| crate::render::models::tint_tag(model.tints.sample_in(tint, None, 0, 0))),
        // Unlike the tint, this is carried on both builds. A tint is sampled
        // on an entity's own animation clock, which the bind-pose doodad build
        // does not have; a texture matrix is almost always a global-sequence
        // scroll, which runs on wall-clock time, and dropping it would stop a
        // scrolling waterfall placed as scenery.
        uv: batch.uv,
        // Only the skinned build, for the same reason as the tint: the decal
        // follows the quad's own joint, and the bind-pose build has no joints.
        // A doodad is never drawn on the floor in any case; the callers are the
        // spell effects and the persistent areas.
        ground: joint_count.and_then(|_| model.ground_quad(batch)),
    })
}

/// One vertex's four joints and weights, as the skinning shader wants them.
///
/// Two cases in the file that the shader does not handle by itself:
///
/// * A vertex with no weights at all. `M2::skin_position` leaves such a
///   vertex where it is, and a few models have them. The shader has no such
///   branch: it sums `weight * joint`, which for all-zero weights is the
///   origin, so the vertex would move to the model's feet. It gets the appended
///   identity joint at full weight instead, which gives the same result.
/// * A bone index past the end of the skeleton. The shader indexes the joint
///   array whatever the weight is, so an out-of-range index reads some other
///   model's matrix rather than being ignored. Clamped onto the identity joint.
fn vertex_skin(model: &M2, vertex: usize, bones: usize) -> ([u16; 4], [f32; 4]) {
    let identity = bones as u16;
    let (Some(weights), Some(indices)) = (
        model.bone_weights.get(vertex),
        model.bone_indices.get(vertex),
    ) else {
        return ([identity; 4], [1.0, 0.0, 0.0, 0.0]);
    };
    // Normalised by the total rather than by 255: the format says they sum to
    // 255, and a file that disagrees would otherwise scale the vertex.
    let total: u32 = weights.iter().map(|&w| w as u32).sum();
    if total == 0 {
        return ([identity; 4], [1.0, 0.0, 0.0, 0.0]);
    }
    let mut joints = [identity; 4];
    let mut out = [0.0f32; 4];
    for k in 0..4 {
        if (indices[k] as usize) < bones {
            joints[k] = indices[k] as u16;
        }
        out[k] = weights[k] as f32 / total as f32;
    }
    (joints, out)
}

// ---------------------------------------------------------------------------
// Decoded model -> Bevy assets
// ---------------------------------------------------------------------------

/// Turn finished loads into meshes, images and materials.
pub(super) fn receive_models(
    mut cache: ResMut<ModelCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: Materials,
    mut bindposes: ResMut<Assets<SkinnedMeshInverseBindposes>>,
) {
    if let Some(loader) = &cache.loader {
        let arrived = loader.drain();
        cache.arrived.extend(arrived);
    }
    if cache.arrived.is_empty() {
        return;
    }

    let take = cache.arrived.len().min(UPLOAD_BUDGET);
    for loaded in cache.arrived.drain(..take).collect::<Vec<_>>() {
        let (path, skinned, raw) = match loaded {
            Loaded::Model {
                path,
                skinned,
                model,
            } => (path, skinned, model),
            // A skin, which costs one image and belongs to no model in
            // particular: the same bake dresses dozens of NPCs.
            Loaded::Texture { path, texture } => {
                let state = match texture {
                    Some(image) => TextureState::Ready(images.add(image)),
                    None => TextureState::Failed,
                };
                cache.set_texture(path, state);
                continue;
            }
        };

        let key = geometry_key(&path, skinned);
        cache.pending.remove(&key);
        // Read before the bytes changed; see `ModelCache::forget`. What
        // arrived is the old file, so it is dropped and requested again
        // rather than installed. Not reached in a normal session: the set is
        // empty unless a host has called `forget`.
        if cache.refetch.remove(&key) {
            cache.request_model(&path, skinned);
            continue;
        }
        let Some(raw) = raw else {
            cache.failed.insert(path);
            continue;
        };
        cache.loaded.insert(path.clone());

        let own: Vec<Handle<Image>> = raw
            .textures
            .into_iter()
            .map(|texture| match texture {
                Some(image) => images.add(image),
                // Type 0 with a file the archive lacks. A slot the client is
                // meant to fill gets its magenta at dressing time instead, so
                // that a dressed entity does not pay for an unused image.
                None => cache.missing.clone(),
            })
            .collect();

        // One identity per joint. M2 vertices are in model space already and
        // `pose` puts the pivot back itself, so there is nothing to undo; see
        // the note in `entities.rs`.
        let joint_count = raw.skeleton.as_ref().map_or(0, |s| s.bones.len() + 1);
        let inverse_bindposes =
            bindposes.add(SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY; joint_count]));

        // The meshes were built on the loader thread; adding one is a move.
        let mut mesh_handles = Vec::with_capacity(raw.draws.len());
        // An M2 is never liquid; `MLIQ` is a WMO chunk.
        let mut params = Vec::with_capacity(raw.draws.len());
        // Each batch's own bone subset, parallel to the two above; see
        // [`RawDraw::bones`].
        let mut bones = Vec::with_capacity(raw.draws.len());
        // The CPU copy a dressing merges from; see [`MergeSource`].
        let mut merge = Vec::with_capacity(raw.draws.len());
        let mut tinted = false;
        let mut moved = false;
        for prepared in raw.draws {
            tinted |= prepared.tint.is_some();
            moved |= prepared.uv.is_some();
            params.push(prepared.params(None));
            bones.push(Arc::clone(&prepared.bones));
            merge.push(prepared.merge.clone());
            mesh_handles.push(meshes.add(prepared.mesh));
        }
        // The emitters, with materials interned here where the store is
        // available. Their textures are the model's own (`own`; the survey
        // counts 0 client-supplied slots over every emitter in the game), so
        // the set is a property of the geometry and every dressing shares the
        // `Arc`.
        let particle_set = crate::render::particles::build_set(
            &raw.particles,
            raw.clip.clone(),
            &raw.kinds,
            &own,
            &cache.missing,
            &mut materials,
        );
        // The trails, built the same way: one material per ribbon, the
        // model's own textures, one set shared by every dressing. No magenta
        // fallback here: see `ribbons::build_set`.
        let ribbon_set = crate::render::ribbons::build_set(
            &raw.ribbons,
            raw.clip.clone(),
            &raw.kinds,
            &own,
            &mut materials,
        );

        let geometry = Geometry {
            // Freshly arrived, so it survives the next sweep whatever the
            // player then does; a model that loads into a tile the player has
            // already walked past is dropped on the sweep after that.
            used: cache.age(),
            meshes: mesh_handles,
            draws: params,
            bones,
            merge,
            kinds: raw.kinds,
            own,
            skeleton: raw.skeleton.map(Arc::new),
            inverse_bindposes,
            joint_count,
            bounds: model_bounds(raw.bounds),
            portrait: raw.portrait,
            body: raw.body,
            conform: raw.conform,
            attachments: Arc::new(raw.attachments),
            cues: Arc::new(raw.cues),
            trail: raw.trail,
            glows: Arc::new(raw.glows),
            collision: Arc::new(raw.collision),
            pick: Arc::new(raw.pick),
            model_sphere: raw.model_sphere,
            particles: particle_set,
            ribbons: ribbon_set,
            cameras: Arc::new(raw.cameras),
            lights: Arc::new(raw.lights),
            // Only when a batch names one, as with the tints: the block exists
            // on files no batch reads.
            uv_anims: raw
                .uv_anims
                .filter(|a| !a.is_empty() && moved)
                .map(Arc::new),
            clip: raw.clip,
            // Only when a batch names one: the tracks exist on many models no
            // batch reads, and carrying them would make `ModelAssets::tints`
            // non-empty for models whose draws are never tinted.
            tints: raw.tints.filter(|_| tinted).map(Arc::new),
        };
        cache.geometry.insert(key.clone(), geometry);

        // The undressed dressing (every doodad in the world), built here rather
        // than on demand because this system already holds the materials. The
        // skinned build has no undressed use, so it waits for a display id to
        // say what to dress it in.
        if !skinned {
            cache.dress(
                &path,
                false,
                &[],
                Dress::Creature,
                None,
                false,
                SceneLighting::NONE,
                &mut meshes,
                &mut materials,
            );
        }
    }
}

/// The model's declared bounding box, in Bevy's axes.
///
/// `None` for a box with no volume: a handful of models declare one, and an
/// empty `Aabb` would cull them at every angle. They fall back to Bevy's own,
/// computed from the bind pose.
pub(super) fn model_bounds(bounds: [[f32; 3]; 2]) -> Option<Aabb> {
    let (a, b) = (axes::to_bevy(bounds[0]), axes::to_bevy(bounds[1]));
    // The change of basis flips signs, so min and max swap on two of the three
    // axes: take them component-wise rather than assuming which is which.
    let (min, max) = (a.min(b), a.max(b));
    if (max - min).min_element() <= 0.0 {
        return None;
    }
    Some(Aabb::from_min_max(min, max))
}

/// One prepared batch as a Bevy mesh and material.
///
/// Both passes go through here, so an M2 and a WMO differ only in their
/// parameters, never in their shader: `alpha_key` and `ambient` are the only
/// two things a caller varies.
///
/// Consumes the prepared draw: its mesh was built on a loader thread and moves
/// into the store, which keeps this call cheap enough to run in bursts.
pub(crate) fn upload_prepared(
    draw: PreparedDraw,
    texture: Handle<Image>,
    alpha_key: f32,
    ambient: Vec3,
    liquid: Option<LiquidLight>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Materials,
) -> ModelDraw {
    let material = materials.intern(material_for(
        &draw.params(liquid),
        texture,
        // A building's batches are never layered: `overlay_layers` is an M2
        // rule and a `WMO` batch carries no `no_depth_write` overlay of the
        // same triangles.
        Default::default(),
        alpha_key,
        ambient,
        // A WMO batch says for itself whether it is vertex-lit; only an M2
        // standing in a room needs telling.
        false,
        // A building is in the world and the world's lighting is the view's.
        SceneLighting::NONE,
    ));
    ModelDraw {
        // A building has no skeleton, so no subset; see [`RawDraw::bones`].
        bones: Arc::clone(&draw.bones),
        // Nothing to merge with: this is the WMO path, whose batches are
        // static geometry drawn in world space. See [`MergeSource`].
        merged: 1,
        // The batch's own `MLIQ` kind, not the resolved tint, because a liquid
        // whose light table does not resolve is still liquid: the two differ
        // for lava with no `DisplayTables`, and marking that batch as solid
        // geometry would leave one surface visible when the water is switched
        // off.
        liquid: draw.liquid,
        mesh: meshes.add(draw.mesh),
        material,
        // A WMO batch has no colour track: `M2Color` is an M2 block.
        tint: None,
        baked_tint: None,
        // No ground quad either: that is an M2 authoring pattern.
        ground: None,
    }
}

/// A batch's material.
///
/// Separate from the mesh because dressing a creature in a DBC skin changes the
/// texture and nothing else: three wolves of different colours are three
/// materials over one set of vertex buffers.
///
/// `lit_by_room` makes a doodad take the interior branch: a `WmoDraw` states
/// its own lighting (a wall is vertex-lit by its own `MOCV`), but an M2 batch
/// never does, and an M2 standing inside a building needs the interior branch.
/// See [`RoomLight`].
pub(super) fn material_for(
    params: &DrawParams,
    texture: Handle<Image>,
    // The folded environment-map layers' textures, in `params.overlays`'
    // order; see [`M2Material::overlay_a`]. `None` for a slot with no layer,
    // which then takes the base's own handle: a binding cannot be empty, and
    // bevy ref-counts bindless resources by id so it costs no slab slot.
    overlays: [Option<Handle<Image>>; vale_assets::world::m2::MAX_OVERLAYS],
    alpha_key: f32,
    ambient: Vec3,
    lit_by_room: bool,
    // What the model states about its own lighting, which for everything in
    // the world is [`SceneLighting::NONE`]; see that type for why it is part
    // of the material and not a view binding.
    scene: SceneLighting,
) -> M2Material {
    let [overlay_a, overlay_b] = overlays;
    M2Material {
        overlay_a: overlay_a.unwrap_or_else(|| texture.clone()),
        overlay_b: overlay_b.unwrap_or_else(|| texture.clone()),
        uv_table: UV_TABLE,
        params: M2Params {
            ambient: ambient.extend(0.0),
            // The cutoff belongs to blend mode 1 and to nothing else: a blended
            // batch that also discarded would punch holes in smoke.
            alpha_cutoff: super::alpha_cut(params.blend, alpha_key),
            unlit: if params.unlit { 1.0 } else { 0.0 },
            // A tinted batch is not room-lit, because both use the same 32-bit
            // `MeshTag`. The tag holds either a room's colour or the batch's
            // own animated tint, and the tint takes priority for spell effect
            // fades: the effects are additive and unlit, so the room branch
            // would not change them. As a result, a tinted batch of a model
            // standing in a tavern is lit by the sun, as attachments already
            // are.
            //
            // The value selects a lighting model, not a flag: 0 the sun, 1 the
            // bake, 2 the bake faded toward the sun by the vertex's own alpha.
            // It stays one `f32` rather than becoming a shader def because all
            // three are already computed in that fragment (the sun for the
            // lambert the interior lean reads, the bake for everything with a
            // colour attribute), so a def would only replace a `select`, and
            // it would add a pipeline variant across a batch population in the
            // thousands. See `m2.wgsl`.
            vertex_lit: match params.light {
                _ if params.tint.is_some() => 0.0,
                BatchLight::Blend => 2.0,
                BatchLight::Bake => 1.0,
                BatchLight::Sun if lit_by_room => 1.0,
                BatchLight::Sun => 0.0,
            },
            liquid: if params.liquid.is_some() { 1.0 } else { 0.0 },
            liquid_close: liquid_vec(params.liquid.map(|l| (l.close, l.shallow_alpha))),
            liquid_far: liquid_vec(params.liquid.map(|l| (l.far, l.deep_alpha))),
            // Still at build time whether or not this batch has a texture
            // matrix. For a batch that has one, `Materials::moving` gives the
            // material its own row of `UV_TABLE` (recorded in `uv_row0.w`), and
            // `material::follow_uv_animations` writes the matrix into that row
            // every frame.
            uv_row0: UV_STILL.0,
            uv_row1: UV_STILL.1,
            particle: Vec4::new(0.0, if params.tint.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0),
            // Solid: `body.x` is the opacity and 1.0 is "as authored".
            body: Vec4::X,
            // The layers' blend modes, zero for a slot with none; see
            // `M2Material::overlay_a`. Every batch in the world except armour
            // folds nothing and takes `0, 0`, which the shader skips.
            overlay: Vec4::new(
                f32::from(params.overlays[0].map_or(0, |(_, blend)| blend)),
                f32::from(params.overlays[1].map_or(0, |(_, blend)| blend)),
                0.0,
                0.0,
            ),
            scene_ambient: scene.ambient,
            scene_lamps: scene.lamps,
        },
        texture,
        blend: params.blend,
        two_sided: params.two_sided,
        no_depth_write: params.no_depth_write,
        // Only the ground foliage sways; see `M2Material::wind`.
        wind: false,
    }
}

/// One half of a [`LiquidLight`] as the shader's `vec4`: rgb plus the opacity
/// that end of the depth ramp carries. Zero when the draw is not liquid, which
/// is every draw but the `MLIQ` surfaces.
pub(crate) fn liquid_vec(half: Option<([f32; 3], f32)>) -> Vec4 {
    match half {
        Some((rgb, alpha)) => Vec4::new(rgb[0], rgb[1], rgb[2], alpha),
        None => Vec4::ZERO,
    }
}


/// Consumes the draw so the vertex buffers move into the mesh. This runs on
/// the loader threads (via [`PreparedDraw::from_raw`]), where a clone would
/// only cost memory, but taking the draw by value makes the cost of calling it
/// from the main thread visible in the signature.
fn draw_mesh(draw: RawDraw) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, draw.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, draw.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, draw.uvs);
    // Only when the model has them: the attribute's presence sets Bevy's
    // `VERTEX_COLORS` shader def, so adding a black one to every doodad in the
    // world would cost a second pipeline variant and 16 bytes a vertex for no
    // effect.
    if !draw.colours.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, draw.colours);
    }
    // Likewise the skinning pair: their presence compiles Bevy's
    // `SKINNED` variant, so a doodad gets a pipeline that never reads them.
    if !draw.joints.is_empty() {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_JOINT_INDEX,
            VertexAttributeValues::Uint16x4(draw.joints),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, draw.weights);
    }
    mesh.insert_indices(Indices::U32(draw.indices));
    mesh
}

/// How many samples the anisotropic filter may take on a world texture: the
/// value of the `anisotropic` CVar, which is the 1.12.1 client's own setting
/// for it.
///
/// The 1.12.1 client filters every ground layer and every model texture
/// anisotropically, with the maximum anisotropy at the CVar's value (16 on the
/// install the reference pictures come from), and filters the alpha map beside
/// them isotropically. Without it the ground is blurrier than the reference:
/// at a pitched-down camera an isotropic filter chooses the mip for the
/// shortest axis of the footprint, which blurs along the long axis.
///
/// At 16, with the alpha atlas also sampled anisotropically, a regular lattice
/// appeared over the ground at one Tanaris framing and changed phase at chunk
/// boundaries; the value was held at 1 until the cause was found. The alpha
/// atlas is now sampled isotropically, as the 1.12.1 client samples its alpha
/// map, and an anisotropic alpha atlas is the likely source of a lattice
/// aligned to chunks. If the lattice reappears, check the alpha atlas sampler
/// first.
///
/// The 1.12.1 client's default for `anisotropic` is `"1"`, so a folder that
/// has never set it gets the stock result; the value is read once at start-up
/// by [`read_anisotropy`], because the sampler is built on the loader thread
/// with no world to query. wgpu has no mip bias, so the 1.12.1 client's
/// `+0.25` mip LOD bias on the same samplers is not reproduced.
static ANISOTROPY: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(1);

/// The anisotropy a world texture's sampler is built with; see [`ANISOTROPY`].
pub(crate) fn anisotropy() -> u16 {
    ANISOTROPY.load(std::sync::atomic::Ordering::Relaxed)
}

/// Read the `anisotropic` CVar into [`ANISOTROPY`], once, before any texture
/// is built. wgpu takes 1..=16, and `1` is "off".
pub(crate) fn read_anisotropy(cvars: Option<Res<crate::settings::cvars::CVars>>) {
    let samples = cvars.map_or(1.0, |c| c.number("anisotropic")).round();
    ANISOTROPY.store(
        (samples as i64).clamp(1, 16) as u16,
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// A world texture and its mip chain. These are authored to tile, and none of
/// them is the alpha atlas, so unlike the terrain's blend maps they repeat.
///
/// The whole mip chain is uploaded, not only the top level. A ground texture
/// repeats eight times across a 33-yard chunk and a tree's leaves are a few
/// hundred texels across at fifty yards, so a screen pixel often covers dozens
/// of texels; sampling only the top mip aliases them into a moire that moves
/// with the camera, as seen on the terrain and the distant foliage. The levels
/// come from the BLP file, so this function generates none.
///
/// wgpu wants the levels concatenated in order, which is what
/// `TextureDataOrder::LayerMajor` means for a single-layer 2D texture, and
/// `Image::new` would `debug_assert` on the extra bytes, so this uses
/// `new_uninit` and sets the descriptor by hand.
pub(crate) fn model_image(width: u32, height: u32, levels: Vec<Vec<u8>>) -> Image {
    // The chain's shape is checked: a level that is not exactly its expected
    // size would make the sampler read the next level's bytes, so the chain is
    // cut at the first level with the wrong size.
    let usable = levels
        .iter()
        .enumerate()
        .take_while(|(level, data)| {
            let (w, h) = ((width >> level).max(1), (height >> level).max(1));
            data.len() == (w as usize) * (h as usize) * 4
        })
        .count()
        .max(1);

    let mut data = Vec::with_capacity(levels.iter().take(usable).map(Vec::len).sum());
    for level in levels.into_iter().take(usable) {
        data.extend_from_slice(&level);
    }

    let mut image = Image::new_uninit(
        bevy::render::render_resource::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = usable as u32;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        // Without this the chain is uploaded and then never used: wgpu's default
        // mipmap filter is nearest, which picks one level and snaps between
        // them at the transition, a visible band moving over the ground.
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: anisotropy(),
        ..default()
    });
    image
}

/// Magenta, for a texture the archive did not have, as the ground uses. A
/// grey fallback would look like a lighting bug; magenta marks the texture as
/// missing. It exposed two gaps in NPC skins.
pub(crate) fn missing_image() -> Image {
    model_image(1, 1, vec![vec![255, 0, 255, 255]])
}
