//! The loader thread: archive bytes -> meshes, textures and materials.
//!
//! **This runs off the main thread and shares nothing with it but its two
//! channels.** A [`Request`] goes out, a [`Loaded`] comes back, and
//! [`receive_models`] is the only place the result meets the ECS — which is why
//! this is a module rather than a region of [`super`]: the boundary is a real
//! one, enforced by the channel, and it was previously invisible because it sat
//! in the same file as the cache it feeds.

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
    /// A *player's* body texture, which no file holds: composed from up to nine
    /// `CharSections` layers. Answered as a [`Loaded::Texture`] under `key`, so
    /// it shares the whole skin cache with the ones that are files.
    Character { key: String, look: CharacterLook },
    /// The texture the *hair mesh* wears, which the composite does not hold —
    /// `CharSections` section 3 column 0, where the composite takes the two
    /// scalp textures beside it. Named by an appearance for the same reason the
    /// composite is: two characters with the same hairstyle and colour share it.
    CharacterHair { key: String, look: CharacterLook },
}

/// Something decoded off the main thread. The meshes and images inside are
/// already built — plain data until `Assets::add` moves them to the render
/// world — so the main thread's share of a load is handle bookkeeping.
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
    /// Already an [`Image`], built on the loader thread — see
    /// [`RawTexture::into_image`].
    textures: Vec<Option<Image>>,
    /// The texture *type* of each of those slots, which is what says whether the
    /// client is expected to fill it and with what — see [`skin_slot`].
    kinds: Vec<u32>,
    draws: Vec<PreparedDraw>,
    /// `None` for scenery, and for a skeleton that did not validate: a model in
    /// its bind pose beats one folded inside out.
    skeleton: Option<M2Skeleton>,
    /// The model's own declared bounding box, in the file's axes.
    bounds: [[f32; 3]; 2],
    /// **Where a portrait of this model is taken from**, in the file's axes —
    /// its own kind-0 camera, or a framing derived from its bind pose for the
    /// seven models that carry none. See [`vale_assets::look::portrait`], which is
    /// the rule, and `crate::render::portraits`, which is the only reader.
    portrait: vale_assets::look::portrait::Framing,
    /// …and where a **whole-body** shot of it is taken from — its own kind-1
    /// camera, or a framing derived from the same bind pose. The `<PlayerModel>`
    /// half of the same question; see [`vale_assets::look::portrait::body_framing`]
    /// and `crate::render::paperdoll`, which is its only reader.
    body: vale_assets::look::portrait::Framing,
    /// **Whether this model leans with the ground it stands on** — the header's
    /// `GlobalModelFlags & 3`, and the only thing in the game that says so. See
    /// [`vale_assets::look::conform`].
    conform: vale_assets::look::conform::Conform,
    /// The points other models hang from, in the file's axes.
    attachments: Vec<M2Attachment>,
    /// The `$CSD`/`$SND` events, sorted into their sequences — see
    /// `sound::cues`. Built here, once per file, off `M2::events`.
    cues: vale_assets::world::m2::SoundCues,
    /// **The lights this model carries**, already in Bevy's axes and already
    /// coloured — see [`crate::render::lamps::ModelGlow`]. Empty for almost
    /// everything; a lamppost has one. Built here rather than in the renderer
    /// because the colour is the *texture's* average and the pixels exist only
    /// on this thread, between the decode and the upload.
    glows: Vec<crate::render::lamps::ModelGlow>,
    /// The `BoundingTriangles` hull, in the file's axes — the doodad half of
    /// [`vale_assets::world::collision`]. Empty for most of the world, which is how
    /// the game says *walk through me*.
    collision: vale_assets::world::collision::CollisionMesh,
    /// The **drawn** triangles, in the file's axes, for the mouse pick's narrow
    /// phase — see [`vale_assets::look::pick`]. A different set from the hull above
    /// and present on everything, where the hull is present on almost nothing.
    pick: vale_assets::look::pick::PickMesh,
    /// …and the sphere every branch of the pick's broad phase falls back to —
    /// the header's `boundingBox` centre and its own `boundingRadius`, in the
    /// file's axes. Read rather than derived from [`Self::bounds`]: the file
    /// states the radius and a half-diagonal of the box is a different number.
    model_sphere: vale_assets::look::pick::Sphere,
    /// The particle emitters, in the file's axes, plus the sequence-0 clip the
    /// emitter clock runs on. Empty for most of the world.
    particles: Vec<vale_assets::world::m2::M2Particle>,
    /// The ribbon trails, on the same clip and empty on the same terms.
    ribbons: Vec<vale_assets::world::m2::M2Ribbon>,
    /// **The model's own cameras**, in the file's axes — empty for everything
    /// in the world and the whole framing of the two glue screens. See
    /// `crate::render::glue`.
    cameras: Vec<vale_assets::world::m2::M2Camera>,
    /// …and its lights: the same kind of block, read for the same screens.
    lights: Vec<vale_assets::world::m2::M2Light>,
    clip: Option<crate::render::particles::ParticleClip>,
    /// The colour and transparency tracks, `None` on the unskinned build for
    /// the same reason [`Self::skeleton`] is.
    tints: Option<vale_assets::world::m2::M2Tints>,
    /// The texture matrices, on **both** builds — see `RawDraw::uv`.
    uv_anims: Option<vale_assets::world::m2::M2TextureAnims>,
}

/// A decoded BLP and its mip chain, still plain data. Shared with the WMO and
/// terrain passes.
pub(crate) struct RawTexture {
    pub width: u32,
    pub height: u32,
    /// Every level, top first. The chain is the *file's* — Blizzard authored
    /// these — so there is nothing generated here and nothing to get wrong
    /// beyond reading them at the right offsets.
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
    /// On the loader thread deliberately: an `Image` is plain data until
    /// `Assets::add` hands it to the render world, and building it here — the
    /// mip chain concatenation included — is exactly the memcpy that used to be
    /// a per-texture cost on the main thread at every tile crossing.
    pub fn into_image(self) -> Image {
        model_image(self.width, self.height, self.levels)
    }
}

/// Whether an environment-map layer is folded into the batch under it — see
/// [`vale_assets::world::m2::overlay_layers`].
///
/// Read once and cached: this is on the loader thread, once per model.
fn folding() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VALE_NO_FOLD").is_none())
}

/// One batch's geometry, already in Bevy's axes and remapped to its own vertices.
///
/// Shared with the WMO pass, which produces these from a `WmoDraw` — a building
/// is deliberately drawn through this material and this shader, exactly as the
/// WebGL renderer drew one through `models.js`' program. The two differ only in
/// parameters: [`Self::colours`] is empty for an M2, and the alpha-key cutoff is
/// an argument to [`upload_prepared`].
pub(crate) struct RawDraw {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Baked `MOCV` light, one per vertex, or empty — an M2 has no such thing.
    ///
    /// Empty is not the same as black: Bevy sets its `VERTEX_COLORS` shader def
    /// from the mesh's attributes, so a mesh without this attribute compiles a
    /// variant that does not read it at all rather than one that reads zeros.
    pub colours: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    /// Four bones per vertex, indexing this model's joints, or empty on anything
    /// with no skeleton. A vertex with no weights at all carries the *identity*
    /// joint at index `bones`, because `M2::skin_position` deliberately leaves
    /// such a vertex where it is and the skinning shader would otherwise
    /// collapse it to the origin.
    /// **Indices into [`Self::bones`], not into the model's skeleton** — see
    /// there for why the subset exists at all.
    pub joints: Vec<[u16; 4]>,
    /// The matching weights, normalised to sum to 1.
    pub weights: Vec<[f32; 4]>,
    /// **Which of the model's bones this batch actually rides**, sorted, with
    /// the identity joint (index `bones`) among them when any vertex is
    /// weightless. Empty on anything with no skeleton.
    ///
    /// This is the whole of what makes a crowd affordable. Bevy's
    /// `extract_skins` walks *every joint of every visible skinned mesh* every
    /// frame and writes a matrix per joint into the skin buffer — and a batch
    /// carrying the model's whole skeleton pays for all of it whether it rides
    /// two bones or a hundred. A dressed `HumanMale` draws ~18 batches over a
    /// 119-bone skeleton, so the naive list is 2,160 joint reads and 138 KB of
    /// buffer writes **per character per frame**, nearly all of it for bones
    /// the batch's vertices never mention.
    ///
    /// The subset is exact rather than an approximation: the matrices are the
    /// same matrices, indexed compactly. See [`batch_draw`], which builds it in
    /// the same pass that remaps the vertices.
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
    /// **Which of the three lighting laws this draw takes** — see
    /// [`vale_assets::world::wmo::BatchLight`], which is where the argument for
    /// there being three of them is.
    ///
    /// Always [`BatchLight::Sun`] for an M2 as it comes off the file; an M2
    /// standing inside a building is promoted to `Bake` by `lit_by_room` at the
    /// material, because nothing in an M2 knows it is indoors.
    pub light: BatchLight,
    /// Which liquid this draw is, if it is one — see
    /// [`vale_assets::world::wmo::WmoDraw::liquid`]. `None` for masonry, for a
    /// doodad and for an entity.
    ///
    /// **The kind and not the resolved colour**, because this is built on the
    /// loader thread and the colour depends on the map, which only the main
    /// thread knows. `receive_wmos` does the lookup.
    pub liquid: Option<Liquid>,
    /// Which of the model's colour and transparency tracks fade this batch.
    /// `None` for a WMO, for a doodad, and for the great majority of M2
    /// batches. See [`ModelDraw::tint`].
    pub tint: Option<BatchTint>,
    /// **…and the doodad's answer to the same question**: the tint sampled once
    /// at the start of the timeline, as a `MeshTag`, or `None` when the batch
    /// has no tint or this is the skinned build. See the note where it is
    /// built — the light cone that was drawn at 1.0 where its own track says
    /// 0.16 is the reason it exists.
    pub baked_tint: Option<u32>,
    /// Which of the model's **texture matrices** moves this batch's texture,
    /// already through the lookup. `None` for all but two dozen batches in the
    /// game (`vale model`).
    pub uv: Option<u16>,
    /// **The environment-map layers folded onto this batch**: which of the
    /// model's texture slots each reads and the blend mode it reads it with —
    /// see [`vale_assets::world::m2::overlay_layers`], which is the rule, and
    /// `M2Material::overlay_a`, which is what it saves. Both `None` for every
    /// batch in the world that is not a piece of armour or a weapon.
    pub overlays: [Option<(u32, u16)>; vale_assets::world::m2::MAX_OVERLAYS],
    /// This batch is a flat rectangle in the model's own ground plane — see
    /// [`vale_assets::world::m2::M2::ground_quad`]. 120 batches over 34 models in
    /// the whole spell-effect population and `None` for everything else; a
    /// caller that draws on the floor re-renders it through
    /// [`crate::render::decals`] instead of spawning the mesh.
    pub ground: Option<vale_assets::world::m2::GroundQuad>,
}

/// A [`RawDraw`] whose geometry has already been turned into a [`Mesh`], plus
/// the fields the *material* is built from — which is the split the two threads
/// need: the mesh is pure data and is built where the file was decoded, and the
/// material depends on things only the main thread knows (the map's liquid
/// tint, the texture handles, the material pool).
///
/// This is what stopped a tile crossing being a hitch: `draw_mesh` clones every
/// vertex buffer, and doing that on the main thread for a building's three
/// thousand batches was most of the frame the spike ate. `Assets::add` of a
/// prebuilt mesh is a move.
/// **One batch's geometry kept on the CPU so a dressing can merge it into a
/// neighbour.**
///
/// A dressed character draws about twenty mesh instances and every one of them
/// moves every frame, which is the only population in this renderer that costs
/// anything per instance: a static doodad is extracted once and then free — 615
/// of them subtract to 0.1 ms — while a moving, skinned one is re-propagated,
/// re-extracted and re-skinned every frame. So the number that matters for a
/// crowd is **mesh instances per character**, and most of a character's are the
/// same material over and over: the body's opaque batches all wear the one
/// composed skin, and its alpha-keyed ones all wear the second texture.
///
/// They cannot be merged when the file is read, because which batches a
/// character draws is a *dressing* decision — the geosets its gear selects —
/// and the merge has to happen after that filter. So the vertex arrays are kept
/// beside the uploaded mesh, and [`super::ModelCache::merge_draws`] concatenates
/// whichever ones a given dressing kept.
///
/// **Only for skinned builds, and only for batches that write depth.** The
/// scenery build is drawn in its bind pose and never moves, so it has nothing
/// to gain and would pay the memory; and merging two *translucent* batches
/// would put them in one draw with one depth order, which is exactly the
/// ordering the file's batch order exists to state.
pub(crate) struct MergeSource {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Indices into [`Self::bones`], as [`RawDraw::joints`] is.
    pub joints: Vec<[u16; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    /// See [`RawDraw::bones`] — the merge takes the union of the members'.
    pub bones: Vec<u16>,
}

pub(crate) struct PreparedDraw {
    pub mesh: Mesh,
    /// The same geometry kept on the CPU, when this batch may be merged into a
    /// neighbour — see [`MergeSource`]. `None` for scenery, for a building, and
    /// for every batch whose draw order matters.
    pub merge: Option<std::sync::Arc<MergeSource>>,
    /// See [`RawDraw::bones`]. Shared rather than cloned: every placement of a
    /// model builds a `SkinnedMesh` from this and the list is per *batch*, not
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
    /// See [`RawDraw::liquid`] — still the kind, resolved on the main thread.
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
    /// being cloned — the whole point of preparing off the main thread.
    pub fn from_raw(draw: RawDraw) -> PreparedDraw {
        let bones: std::sync::Arc<[u16]> = draw.bones.as_slice().into();
        // **Which batches may be merged**, and every clause is a way the merge
        // would change the picture rather than only the cost:
        //
        // * no skeleton — scenery, which never moves and has nothing to gain;
        // * `blend` past alpha-key — a translucent batch is ordered against its
        //   neighbours and merging states that order away;
        // * a tint or a texture matrix — both are *per batch* and ride the
        //   instance's own `MeshTag` or its own material, so two merged into
        //   one would have to share a value neither of them holds;
        // * a ground quad — the caller re-renders that batch through the decal
        //   projector instead of spawning it at all;
        // * vertex colours — an M2 has none, and a `WMO` batch that does is not
        //   skinned, so this is a guard rather than a case.
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
    /// resolved — `None` for anything that is not an `MLIQ` surface.
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
            // **Part of the key, and it has to be.** The material is
            // specialised as tinted, so the `MeshTag` beside it is read as a
            // colour — two batches with different constant tints are two
            // different draws and must not share a pooled handle. The count is
            // bounded by the tinted-batch population, which `vale model`
            // puts at a few hundred over the whole game.
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
/// neither a `Sender` nor a `Receiver` is. Neither lock is ever contended —
/// exactly one system touches each.
pub(super) struct Loader {
    requests: Mutex<Sender<Request>>,
    results: Mutex<Receiver<Loaded>>,
}

impl Loader {
    /// `overlay` is the host's source asked before the archives — the cell
    /// `GameAssets::overlay_cell` shares. Read on every request rather than
    /// once, because a host installs its overlay after this thread has
    /// started, and a chain opened without it answers the game's own bytes
    /// for a path the host is overriding. It is an `Arc` clone per request.
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
                // `ItemDisplayInfo.dbc` is 29,604 rows and is only wanted once
                // a *player* is in sight, so it is read on the same terms as
                // `CharSections` — on demand, and kept.
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
                            // `ItemDisplayInfo.dbc` beside it, read the same
                            // way and on the same terms: without it a player
                            // is drawn in their underwear rather than not at
                            // all.
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
                            // Silence here would be the same magenta as before
                            // and no way to tell the two apart, which is the
                            // bug this whole path exists to fix.
                            if texture.is_none() {
                                warn!("no skin composes for {look:?}");
                            }
                            Loaded::Texture {
                                path: key,
                                texture: texture.map(RawTexture::into_image),
                            }
                        }
                        // The hair *mesh's* texture, which is not part of the
                        // composite: it dresses separate geometry, and the M2
                        // asks for it as texture type 6. It comes out of the
                        // same `CharSections` row as the scalp — column 0 where
                        // the composite takes 1 and 2 — so it is resolved here,
                        // where the table already is, rather than by giving the
                        // main thread a second copy of a 4,030-row DBC.
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
                            // dress, so this is only worth a word when the
                            // character *has* hair for it to be missing from.
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

/// **How many joints this build has to pose with** — the model's bone count on
/// the skinned build, and `None` on the other.
///
/// A function rather than two lines inline because it is one half of an
/// asymmetry that has to be read together with the other: the skeleton is kept
/// on **both** builds (it says whether the *model* moves, which is a property of
/// the file) and this is kept on one (it says whether *this build* can be posed).
/// Conflating them dropped the skeleton from the doodad build and cost every
/// piece of scenery in the game its animation. See
/// `joints_are_build_specific_and_the_skeleton_is_not`.
pub(crate) fn joint_count_for(
    skinned: bool,
    skeleton: Option<&vale_assets::world::m2::M2Skeleton>,
) -> Option<usize> {
    skinned.then(|| skeleton.map(|s| s.bones.len())).flatten()
}

/// One M2 and its own textures, decoded.
///
/// A model that will not parse yields `None` and is dropped — a missing tree
/// costs that tree, not the tile.
fn read_model(archive: &mut Archive, path: &str, skinned: bool) -> Option<RawModel> {
    let bytes = archive.read(path).ok()?;
    let model = M2::parse(&bytes).ok()?;

    // `M2::textures` is parallel to the slots the batches index, so a texture
    // used by three batches is decoded once and a slot the model does not fill
    // keeps its position — the *index* is what says which slot it is.
    // **Which batches are lights**, read before the textures so that the decode
    // below knows which of them anybody needs an average of. See
    // `vale_assets::world::glow`, which is the reading, and
    // `crate::render::lamps`, which is what decides a glow is a light.
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
        // **The average, taken here or not at all.** `into_image` moves the
        // pixels into an `Image` bound for the render world, and nothing on the
        // CPU can read them again — so a colour wanted later has to be taken
        // now. Only for the slots a glow names, which for the whole world is a
        // few dozen 32x32 textures.
        if glow_batches.iter().any(|g| g.texture == Some(slot as u32)) {
            if let Some(mean) = crate::render::lamps::glow_colour(&decoded.levels[0]) {
                glow_colours.push((slot as u32, mean));
            }
        }
        textures.push(Some(decoded.into_image()));
    }
    let kinds = model.textures.iter().map(|t| t.kind).collect();

    // …and the lights, joined to their colours. A glow whose texture would not
    // decode is dropped rather than lit white: a lamp with no colour is not a
    // lamp, it is a guess.
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

    // **Every batch, not the visible ones.** Which geosets a model shows is a
    // property of the *entity* wearing it and not of the file — two NPCs
    // sharing `HumanMale.m2` do not share a hairstyle — so the filter cannot
    // happen here, where the answer would be baked once per path. Each batch
    // keeps its geoset id and the dressing selects; the dressing is already
    // per-appearance, so this costs no new cache dimension, only the meshes of
    // the variants nobody is wearing.
    //
    // `None` leaves the skinning attributes off, and with them the skinned
    // pipeline — which a doodad must not have, because it has no joints to bind.
    let joint_count = joint_count_for(skinned, model.skeleton.as_ref());
    // **Which batches are a layer of an earlier one**, and therefore are not
    // drawn at all — their texture and their blend ride the base's material
    // instead. See [`vale_assets::world::m2::overlay_layers`] for the rule
    // and every condition it refuses on, and `M2Material::overlay_a` for what
    // it is worth. `VALE_NO_FOLD=1` turns it off, on the terms every other
    // saving in this file is subtractable on.
    let layers: Vec<Option<usize>> = match folding() {
        true => vale_assets::world::m2::overlay_layers(&model.batches),
        false => vec![None; model.batches.len()],
    };
    // …and, for each base, the layers it absorbed, in file order.
    let mut absorbed: Vec<[Option<(u32, u16)>; vale_assets::world::m2::MAX_OVERLAYS]> =
        vec![Default::default(); model.batches.len()];
    for (i, base) in layers.iter().enumerate() {
        let Some(base) = base else { continue };
        let batch = &model.batches[i];
        // A batch whose texture slot does not resolve cannot be a layer: there
        // would be nothing to sample. `overlay_layers` does not test it, because
        // a texture slot is the renderer's half of the question.
        let Some(texture) = batch.texture else { continue };
        if let Some(slot) = absorbed[*base].iter_mut().find(|s| s.is_none()) {
            *slot = Some((texture, batch.blend));
        }
    }
    let draws = model
        .batches
        .iter()
        .enumerate()
        // A layer is drawn *by* its base and never on its own.
        .filter(|(i, _)| layers[*i].is_none())
        .filter_map(|(i, batch)| batch_draw(&model, batch, joint_count, absorbed[i]))
        // The mesh is built here, on the loader thread, so the main thread's
        // share of this model is `Assets::add` — a move, not a copy.
        .map(PreparedDraw::from_raw)
        .collect();

    // The emitter clock: sequence 0's window, wrapped if that sequence loops.
    // Taken from the model's own sequence table *before* the unskinned build
    // drops the skeleton — a campfire has no joints to bind and still pulses
    // its flame on its own animation. An emitter-only model whose skeleton did
    // not validate has no clip, and the sim falls back to "a gate with any
    // nonzero key is on".
    let clip = model.skeleton.as_ref().and_then(|s| {
        let seq = s.sequences.first()?;
        Some(crate::render::particles::ParticleClip {
            start: seq.start,
            end: seq.end,
            // A read of the 5875 loader: a sequence loops when bit 0 of its
            // flags is clear.
            loops: seq.flags & 1 == 0,
            global_sequences: Arc::from(s.global_sequences.as_slice()),
        })
    });

    // **Where to stand to take this model's picture**, resolved here because
    // the rule wants the *vertices* as well as the camera block — see
    // `vale_assets::look::portrait::bind_bounds` — and this is the last place the
    // whole `M2` is in hand. One `Framing` per file rather than a second parse
    // per unit frame per target change.
    let portrait = vale_assets::look::portrait::framing(&model);
    // …and the full-body one beside it, resolved in the same place and for the
    // same reason. Two `Framing`s per file is 56 bytes; re-parsing an `M2`
    // because the character sheet was opened is a disk hit inside a panel.
    let body = vale_assets::look::portrait::body_framing(&model);
    // **The mouse pick's narrow phase**, taken here for the same reason and in
    // the same place: this is the last point at which the whole `M2` is in hand,
    // and a pick that had to re-read the file would be a disk hit inside a mouse
    // move. The same triangles whichever way the model was asked for — what the
    // pointer can land on is the file's drawn geometry and not the dressing's.
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
        // Read on **both** builds, like the hull: a doodad never asks, but the
        // flag is a property of the file rather than of the dressing and
        // dropping it here would make the two builds disagree.
        conform: vale_assets::look::conform::Conform::of(model.global_flags),
        cues: model.sound_cues(),
        attachments: model.attachments,
        // The same hull whichever way the model was asked for: an entity build
        // and a doodad build of one file are two meshes and one solid shape.
        collision: model.collision,
        pick,
        model_sphere,
        particles: model.particles,
        ribbons: model.ribbons,
        // Carried on **both** builds, like the hull and unlike the skeleton: a
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
        // **Kept on both builds, and that is a correction.** It used to be
        // dropped here alongside the tints, on the reasoning that "a skeleton
        // without the attributes would spawn joints nothing reads" — but the
        // thing that decides whether joints are spawned is
        // [`ModelAssets::joint_count`], which is already `None` unless
        // `skinned`, and every caller iterates `0..joint_count`.
        //
        // The two are different questions. `joint_count` is *can this build be
        // posed*; the skeleton is *does this model move at all*, which is a
        // property of the file and true of both builds. Conflating them made
        // the second unanswerable from the unskinned build — and that is the
        // only build a doodad has, so `render::doodads` could never decide to
        // ask for the other one and **no scenery in the world ever animated**.
        // Every test still passed, because the skeleton it asserted about was
        // the entity build's.
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

/// A *player's* body texture, composed rather than looked up.
///
/// Up to nine layers, each a region of the 256x256 body atlas: the base skin
/// over the whole of it, then underwear, face, scalp and beard painted into
/// their own rectangles. A layer the archive does not have is skipped — 44 of
/// the table's 4,030 textures are named by rows Blizzard shipped without files
/// for — which costs that part and leaves everything else, where holding the
/// whole thing back would put the player back to magenta over a missing beard.
///
/// The mips are **generated** here, unlike everywhere else in this client: this
/// is the one texture with no file to take Blizzard's own chain from.
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
/// **The winding is left alone**, unlike the terrain's. `models.js` drew M2s
/// with `CULL_FACE` on and the default counter-clockwise front face and they
/// looked right, so the file's order is already the one wgpu expects — and the
/// change of basis in [`axes::to_bevy`] is a rotation, which cannot reverse it.
/// (The terrain needed reversing precisely because its pass never enabled
/// culling and so never exercised the question.)
///
/// `joint_count` is the model's bone count when it has a skeleton, and it turns
/// on the two skinning attributes. Bevy compiles its `SKINNED` shader variant
/// from the *presence* of those attributes, so a tree gets a pipeline that does
/// not read them rather than one that reads a bind pose.
pub(crate) fn batch_draw(
    model: &M2,
    batch: &M2Batch,
    joint_count: Option<usize>,
    // See [`RawDraw::overlays`]. `Default::default()` for a caller with no
    // batch list to fold over — the CLI checks and the tests.
    overlays: [Option<(u32, u16)>; vale_assets::world::m2::MAX_OVERLAYS],
) -> Option<RawDraw> {
    let range = batch.index_start as usize..(batch.index_start + batch.index_count) as usize;
    let indices = model.indices.get(range)?;
    if indices.is_empty() {
        return None;
    }
    // **A batch authored invisible for its whole timeline is not drawn.**
    //
    // It is a carrier for emitters rather than geometry anybody was meant to
    // see — see [`vale_assets::world::m2::M2Tints::never_visible`], where
    // Ironforge's lava steam is the worked example. Dropped here rather than
    // shaded away, because it cannot be shaded away: a blend-0 batch is
    // `AlphaMode::Opaque`, `alpha_cut` is 0.0 for that blend so the discard
    // never fires, and the ROP throws the alpha away — so the one thing this
    // renderer could do with the batch is paint it solid.
    if batch.tint.is_some_and(|tint| model.tints.never_visible(tint)) {
        return None;
    }
    // A batch whose indices run past the vertex arrays is dropped rather than
    // panicking on the first one: these are twenty-year-old files.
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
            // Model space becomes Bevy's here, exactly as terrain vertices do.
            // The placement matrix is conjugated by the same basis
            // (`axes::to_bevy_affine`), which is what carries the derived 180°
            // term across intact instead of having it re-eyeballed on a tree.
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

    // **The batch's own bone subset**, built in one pass over the vertices this
    // batch actually kept — see [`RawDraw::bones`] for what it is worth. The
    // map is a flat array over the model's bones rather than a `HashMap`: a
    // skeleton is at most a few hundred entries and this runs per batch per
    // model on the loader thread.
    let bones = match joint_count {
        // The kill switch's other side, and it has to be *here*: the subset and
        // the vertex indices are built together, so turning it off at the spawn
        // instead would leave the indices remapped. See `super::subsetting`.
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
        // An M2 carries no baked light — see the note on the field.
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
        // Liquid is a WMO's `MLIQ`; an M2 has no such thing.
        liquid: None,
        // **Only the skinned build *fades*.** A tint is sampled on an animation
        // clock, and the unskinned build is the one this renderer draws in its
        // bind pose with no clock at all. The two builds are already separate
        // cache entries (`geometry_key`), so this is a property of the entry
        // rather than a special case.
        tint: batch.tint,
        // **…and the unskinned build takes the value instead**, which is the
        // half that was missing and the bug it was.
        //
        // The note this replaces argued that carrying a tint onto a doodad
        // would cost a tag and a material variant "in exchange for a value that
        // never changes". The first half is right; the second is the reason to
        // *bake* it, not to drop it — because the value that never changes is
        // very often not 1.
        //
        // `GnomeStructuralSpotlight02.m2`'s light cone is the worked example:
        // batch 2 is `add-alpha`, `unlit`, `no-depth-write`, and its
        // transparency track is a single key at **0.160**. Drawn at 1.0 it is an
        // opaque white sheet the size of a building, which is what Gadgetzan
        // looked like. The same fault is every authored-faint glow in the game:
        // a brazier's flame halo drawn as a grey box, Orgrimmar's light shafts
        // drawn as grey planes.
        //
        // Sampled at the start of the timeline with no sequence, which for this
        // population is the whole track — see the census in `vale model`,
        // which prints a constant track as `(identity — dropped)` and a real one
        // as `(moves)`. A doodad whose tint genuinely animates gets its first
        // frame held, which is the same bind pose its bones already hold.
        // **Baked on both builds**, and the skinned one is the half that was
        // missing.
        //
        // A batch with a tint track is specialised as *tinted*, so the
        // `MeshTag` beside it is read as `0xAARRGGBB`. On an entity that is
        // fine — `world::entities::effects` writes the sampled tint every
        // frame. On **scenery** nothing writes it: `render::doodads::pose_scenery`
        // writes joint transforms and no tag, so the spawner fell through to
        // its room-light arm and the tinted shader read a room colour as a
        // tint. An animated doodad authored faint was therefore drawn at the
        // room's own brightness, which is the second half of the Ironforge
        // report — the lava steam came out warm orange-brown rather than
        // invisible.
        //
        // A doodad whose tint genuinely animates gets its first frame held,
        // which is the same bind pose its bones already hold; an entity
        // overwrites this on its first tint write.
        baked_tint: batch
            .tint
            .map(|tint| crate::render::models::tint_tag(model.tints.sample_in(tint, None, 0, 0))),
        // **Unlike the tint, this is carried on *both* builds.** A tint is
        // sampled on an entity's own animation clock, which the bind-pose
        // doodad build has none of; a texture matrix is overwhelmingly a
        // *global-sequence* scroll — free-running wall-clock — and a scrolling
        // waterfall placed as scenery is exactly the case that would be lost.
        uv: batch.uv,
        // **Only the skinned build**, and for the tint's reason exactly: the
        // decal rides the quad's own joint, and the bind-pose build has no
        // joints to ride. A doodad is never drawn on the floor anyway — the
        // callers are the spell effects and the persistent areas.
        ground: joint_count.and_then(|_| model.ground_quad(batch)),
    })
}

/// One vertex's four joints and weights, as the skinning shader wants them.
///
/// Two things the file can do that the shader cannot survive unaided:
///
/// * **a vertex with no weights at all.** `M2::skin_position` leaves such a
///   vertex where it is, and a few models have them; the shader has no such
///   branch — it sums `weight * joint`, which for all-zero weights is the
///   origin, so the vertex is dragged to the model's feet. It gets the appended
///   identity joint at full weight instead, which is the same answer.
/// * **a bone index past the end of the skeleton.** The shader indexes the joint
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
        // **Read before the bytes changed** — see `ModelCache::forget`. What
        // arrived is the old file, so it is dropped and asked for again
        // rather than installed. Never taken in a session: the set is empty
        // unless a host has forgotten something.
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
                // Type 0 with a file the archive lacks. A slot the *client* is
                // meant to fill gets its magenta at dressing time instead, so
                // that a dressed entity does not pay for an unused image.
                None => cache.missing.clone(),
            })
            .collect();

        // One identity per joint. M2 vertices are in model space already and
        // `pose` puts the pivot back itself, so there is nothing to undo — see
        // the note in `entities.rs`.
        let joint_count = raw.skeleton.as_ref().map_or(0, |s| s.bones.len() + 1);
        let inverse_bindposes =
            bindposes.add(SkinnedMeshInverseBindposes::from(vec![Mat4::IDENTITY; joint_count]));

        // The meshes were built on the loader thread; adding one is a move.
        let mut mesh_handles = Vec::with_capacity(raw.draws.len());
        // An M2 is never liquid — `MLIQ` is a WMO chunk.
        let mut params = Vec::with_capacity(raw.draws.len());
        // …and each batch's own bone subset, parallel to the two above — see
        // [`RawDraw::bones`].
        let mut bones = Vec::with_capacity(raw.draws.len());
        // …and the CPU copy a dressing merges from — see [`MergeSource`].
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
        // The emitters, materials interned here where the store is to hand.
        // Their textures are the model's own (`own` — the survey counts 0
        // client-supplied slots over every emitter in the game), so the set
        // is a property of the geometry and every dressing shares the `Arc`.
        let particle_set = crate::render::particles::build_set(
            &raw.particles,
            raw.clip.clone(),
            &raw.kinds,
            &own,
            &cache.missing,
            &mut materials,
        );
        // The trails, on the same terms — one material per ribbon, the
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
            // player then does — a model that loads into a tile the player has
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
            glows: Arc::new(raw.glows),
            collision: Arc::new(raw.collision),
            pick: Arc::new(raw.pick),
            model_sphere: raw.model_sphere,
            particles: particle_set,
            ribbons: ribbon_set,
            cameras: Arc::new(raw.cameras),
            lights: Arc::new(raw.lights),
            // Only when a batch actually names one, exactly as the tints are:
            // the block exists on files no batch reads.
            uv_anims: raw
                .uv_anims
                .filter(|a| !a.is_empty() && moved)
                .map(Arc::new),
            clip: raw.clip,
            // Only when a batch actually names one: the tracks exist on plenty
            // of models no batch reads, and carrying them would make
            // `ModelAssets::tints` a promise the draws do not keep.
            tints: raw.tints.filter(|_| tinted).map(Arc::new),
        };
        cache.geometry.insert(key.clone(), geometry);

        // The undressed dressing — every doodad in the world — built here rather
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
/// `None` for a box with no volume — a handful of models declare one, and an
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
/// Both passes go through here, which is what makes "an M2 and a WMO differ in
/// their *parameters*, not in their shader" a structural fact rather than a
/// comment: `alpha_key` and `ambient` are the only two things a caller varies.
///
/// Consumes the prepared draw: its mesh was built on a loader thread and moves
/// into the store, which is what keeps this call cheap enough to run in bursts.
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
        // A building has no skeleton, so no subset — see [`RawDraw::bones`].
        bones: Arc::clone(&draw.bones),
        // …and nothing to merge with: this is the WMO path, whose batches are
        // static geometry drawn in world space. See [`MergeSource`].
        merged: 1,
        // **The batch's own `MLIQ` kind and not the resolved tint**, because a
        // liquid whose light table would not resolve is still liquid: the two
        // differ for lava with no `DisplayTables`, and marking that batch as
        // solid geometry would leave one surface behind every time the water
        // was switched off.
        liquid: draw.liquid,
        mesh: meshes.add(draw.mesh),
        material,
        // A WMO batch has no colour track: `M2Color` is an M2 block.
        tint: None,
        baked_tint: None,
        // …nor a ground quad: that is an M2 authoring pattern.
        ground: None,
    }
}

/// A batch's material.
///
/// Separate from the mesh because dressing a creature in a DBC skin changes the
/// texture and nothing else: three wolves of different colours are three
/// materials over one set of vertex buffers.
///
/// `lit_by_room` is how a *doodad* takes the interior branch: a `WmoDraw` says
/// so for itself (a wall is vertex-lit by its own `MOCV`), but an M2 batch never
/// does, and an M2 standing inside a building has to. See [`RoomLight`].
pub(super) fn material_for(
    params: &DrawParams,
    texture: Handle<Image>,
    // **The folded environment-map layers' textures**, in `params.overlays`'
    // order — see [`M2Material::overlay_a`]. `None` for a slot with no layer,
    // which then takes the base's own handle: a binding cannot be empty, and
    // bevy ref-counts bindless resources by id so it costs no slab slot.
    overlays: [Option<Handle<Image>>; vale_assets::world::m2::MAX_OVERLAYS],
    alpha_key: f32,
    ambient: Vec3,
    lit_by_room: bool,
    // **What the model states about its own lighting**, which for everything in
    // the world is [`SceneLighting::NONE`] — see that type, where the whole
    // argument for it being a material and not a view binding is.
    scene: SceneLighting,
) -> M2Material {
    let [overlay_a, overlay_b] = overlays;
    M2Material {
        overlay_a: overlay_a.unwrap_or_else(|| texture.clone()),
        overlay_b: overlay_b.unwrap_or_else(|| texture.clone()),
        params: M2Params {
            ambient: ambient.extend(0.0),
            // The cutoff belongs to blend mode 1 and to nothing else: a blended
            // batch that also discarded would punch holes in smoke.
            alpha_cutoff: super::alpha_cut(params.blend, alpha_key),
            unlit: if params.unlit { 1.0 } else { 0.0 },
            // **A tinted batch is not room-lit, because the two want the same
            // 32 bits.** The tag is either a room's colour or the batch's own
            // animated one, and a spell effect's fade is the answer that
            // matters: the effects are additive and unlit, so the room branch
            // would have changed nothing about them anyway. The cost is a
            // *tinted* batch of a model standing in a tavern taking the sun —
            // the same sliver the attachments already take.
            //
            // **The value is a law and not a flag** — 0 the sun, 1 the bake, 2
            // the bake faded toward the sun by the vertex's own alpha. It stays
            // one `f32` rather than becoming a shader def because all three
            // laws are already computed in that fragment (the sun for the
            // lambert the interior lean reads, the bake for everything with a
            // colour attribute), so what a def would buy is a `select` — and it
            // would cost a pipeline variant on a batch population in the
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
            // Still at build time whether or not this batch states a matrix:
            // the value changes every frame, so what fills it is
            // `uv_anim::follow`, which owns the handles of the batches that do.
            uv_row0: UV_STILL.0,
            uv_row1: UV_STILL.1,
            particle: Vec4::new(0.0, if params.tint.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0),
            // Solid: `body.x` is the opacity and 1.0 is "as authored".
            body: Vec4::X,
            // **The layers' blend modes**, zero for a slot with none — see
            // `M2Material::overlay_a`. A batch that folded nothing is every
            // batch in the world but the armour, and takes the `0, 0` the
            // shader steps straight over.
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


/// Consumes the draw so the vertex buffers *move* into the mesh — this runs on
/// the loader threads (via [`PreparedDraw::from_raw`]), where a clone would
/// only cost memory, but keeping it a move is what keeps the temptation to
/// call it from the main thread visible in the signature.
fn draw_mesh(draw: RawDraw) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, draw.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, draw.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, draw.uvs);
    // Only when the model has them: the attribute's *presence* is what sets
    // Bevy's `VERTEX_COLORS` shader def, so adding a black one to every doodad
    // in the world would cost a second pipeline variant and 16 bytes a vertex to
    // say nothing.
    if !draw.colours.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, draw.colours);
    }
    // Likewise the skinning pair: their presence is what compiles Bevy's
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

/// How many samples the anisotropic filter may take on a world texture —
/// **the `anisotropic` CVar's, which is 5875's own setting for it.**
///
/// The Direct3D trace of the reference (rendering facts, *The reference client
/// traced*) sets every ground layer and every model texture to
/// `D3DTEXF_ANISOTROPIC` with `MAXANISOTROPY` at the CVar's value — 16 on the
/// install the reference pictures come from — and the alpha map beside them
/// to 1. "Our ground is blurrier than the reference's" was the report, and at
/// a pitched-down camera an isotropic mip is chosen for the *shortest* axis of
/// the footprint, which is exactly a blur along the long one.
///
/// **This was 1 for several rounds on a measurement**: at 16 a regular lattice
/// stood over the ground at one Tanaris framing, changing phase at chunk
/// boundaries. The alpha atlas was sampled anisotropically then too, and it
/// is not now — the reference keeps its alpha map isotropic — which is the
/// likely source of a lattice aligned to chunks. If it comes back, that is
/// the thing to subtract first.
///
/// 5875 registers `anisotropic` at `"1"`, so a folder that has
/// never set it gets the stock picture; the value is read once at start-up by
/// [`read_anisotropy`], because the sampler is built on the loader thread with
/// no world to ask. wgpu has no mip bias, so the reference's `+0.25`
/// `MIPMAPLODBIAS` on the same samplers is not reproduced.
static ANISOTROPY: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(1);

/// The anisotropy a world texture's sampler is built with — see [`ANISOTROPY`].
pub(crate) fn anisotropy() -> u16 {
    ANISOTROPY.load(std::sync::atomic::Ordering::Relaxed)
}

/// Read the `anisotropic` CVar into [`ANISOTROPY`], once, before any texture
/// is built. wgpu takes 1..=16, and `1` is "off".
pub(crate) fn read_anisotropy(cvars: Option<Res<crate::game::cvars::CVars>>) {
    let samples = cvars.map_or(1.0, |c| c.number("anisotropic")).round();
    ANISOTROPY.store(
        (samples as i64).clamp(1, 16) as u16,
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// A world texture and its mip chain. These are authored to tile, and none of
/// them is the alpha atlas, so unlike the terrain's blend maps they repeat.
///
/// **The whole chain goes up, not just the top.** A ground texture repeats eight
/// times across a 33-yard chunk and a tree's leaves are a few hundred texels
/// across at fifty yards, so a screen pixel routinely covers dozens of texels;
/// sampling only the top mip aliases them into a moire that crawls as the camera
/// moves, which is what the terrain and the distant foliage looked like. The
/// levels come out of the BLP — Blizzard authored them — so this generates
/// nothing.
///
/// wgpu wants the levels concatenated in order, which is what
/// `TextureDataOrder::LayerMajor` means for a single-layer 2D texture, and
/// `Image::new` would `debug_assert` on the extra bytes — hence `new_uninit`
/// and the descriptor set by hand.
pub(crate) fn model_image(width: u32, height: u32, levels: Vec<Vec<u8>>) -> Image {
    // Trust nothing about the chain's shape: a level that is not exactly its own
    // size would make the sampler read the next level's bytes, so the chain is
    // cut at the first one that does not measure up rather than uploaded and
    // hoped for.
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
        // them at the transition — a visible band sliding over the ground.
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: anisotropy(),
        ..default()
    });
    image
}

/// Magenta, for a texture the archive did not have — the same loud convention
/// the ground uses. A model that quietly fell back to grey would look like a
/// lighting bug, which is exactly how the NPC skin work found its two gaps.
pub(crate) fn missing_image() -> Image {
    model_image(1, 1, vec![vec![255, 0, 255, 255]])
}
