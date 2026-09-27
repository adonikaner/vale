//! **A unit's name, drawn *in* the world rather than over it.**
//!
//! ```text
//! look::unitname   who gets one, and how tall in yards   <- no window
//! raster           the string -> one RGBA texture, once  <- ab_glyph
//! reconcile        a camera-facing quad per named unit   <- the same M2Material
//!                  everything else in the world takes
//! ```
//!
//! ## Why this is a render pass and not a line of interface
//!
//! It was interface once, and the report that moved it is the whole argument:
//! *"names should exist in the world like a spell effect, so they disappear
//! behind objects."* Painted over the frame — which is what an egui layer is —
//! a name has no depth at all, so the best that could be done was to hide it
//! wholesale when a ray from the eye to the head hit something. A name half
//! behind a tree trunk vanished entirely where the reference shows the half
//! that clears the trunk.
//!
//! Here it is a quad in the world with a texture on it, going through
//! [`M2Material`] — the same material every particle, every ground decal and
//! every selection ring in this client takes — so it lands in the same sorted
//! transparent phase and takes the same depth test as the trunk. Nothing in this
//! file does anything about occlusion, which is the point: the depth buffer
//! already knows.
//!
//! **Three other things fall out of it**, and each was a fault of the old shape:
//!
//! * **the glyph atlas stops thrashing.** The painter rasterises per *font
//!   size*, and a size that comes out of a projection changes every frame for
//!   every unit — an overflow warning and then a panic (*"Tried to allocate a
//!   2720 wide glyph in a 2048 wide texture atlas"*). Here a name is rasterised
//!   **once**, at [`RASTER_PX`], and the distance is the quad's scale.
//! * **the size needs no clamp.** A unit against the near plane makes a big
//!   quad, which is what a big quad is for.
//! * **and no projection arithmetic at all.** The old pass had to convert the
//!   world camera's *physical* viewport pixels into the interface's logical
//!   space, which this repo had already got wrong once in the other direction.
//!   A quad is placed in yards and the camera does the rest.
//!
//! ## What is here and what is one crate over
//!
//! Who gets a name, which of the five `UnitName*` CVars decides, and how tall it
//! is in yards are all [`vale_assets::look::unitname`]'s — decidable with the
//! archives shut, and every number in them the 1.12.1 client's. What is
//! here is the half that needs a renderer.
//!
//! ## The billboard is **screen-aligned**, and that is a correction
//!
//! It turned about up only for several rounds, on the stated grounds that the
//! text should stay upright in the world — and that made a name *foreshorten*
//! when the camera looked steeply down, which is the report that retired it.
//! The two rules are indistinguishable at an ordinary camera pitch, which is
//! how a wrong one survives a long time.
//!
//! The reference does not draw the name in the world at all: it draws it in
//! **screen space**, at the projected `PlayerName` point, and a projected 2D
//! string cannot be foreshortened by anything. Three facts about the client
//! say so, and the third is the load-bearing one —
//!
//! * `UNIT_NAME_FONT` is built at **0.99** of the interface's height, which is
//!   only worth doing for a face that is going to be
//!   scaled *down* by a lot and by a varying amount;
//! * the size handed to the draw **divides by no distance**, so the
//!   shrink a distant name has belongs to a projection rather than to the size;
//! * and the string is laid out with 100000.0 as both its maximum
//!   width and height — a **text layout** call, not a mesh build — and then
//!   registered with the font object, which is the same subsystem
//!   the damage numbers go through and whose heights
//!   [`vale_assets::look::worldtext`] already pins as fractions of the
//!   interface's height.
//!
//! **The first two are measurements and the conclusion is an interpretation**:
//! the call that actually rasterises the laid-out string was not followed to a
//! device call, so "screen space" is read off the shape of the font and the
//! absence of a distance divide rather than seen. What is not in doubt is the
//! symptom, which is what the report was about.
//!
//! A quad is still the right shape — it keeps the depth test, which is the
//! whole reason this stopped being an egui layer — so what changed is only the
//! rotation: [`face_the_camera`] hands every label the camera's **own** basis,
//! so the quad is parallel to the near plane and its up is the screen's up. The
//! size stays a world height, so a name still shrinks with distance exactly as
//! it did.
//!
//! ## What is deliberately not here
//!
//! * **the guild line and the PvP title**, which are two more `UnitName*`
//!   switches and two more lines under the name. The client reads neither field.
//! * **more than one draw per *colour*.** The names are packed into one atlas
//!   and merged into one mesh per distinct tint — see [`reconcile`] — so the
//!   pass is three or four draws whatever the population. It was one texture,
//!   one material and one draw *per name* until the round that measured it:
//!   forty name plates over forty players cost **1.8 ms of a 12.0 ms frame**,
//!   which is a seventh of the frame for a line of text over each head. The
//!   colour is what forces the split: it rides the instance's `MeshTag`
//!   (see [`params`]), and a merged mesh has one tag, so one mesh per tint is
//!   the merge that needs no shader change. There are four tints in the ring
//!   table and a screen rarely holds three.
//! * **a shared atlas across sessions.** It is packed in first-sight order and
//!   **reset** when it fills rather than repacked, so a very long session in a
//!   crowded city loses its names for a single frame every few hundred distinct
//!   ones. See [`ATLAS_SIDE`].
//! * **fading out at range.** The reference keeps a distant name legible; a quad
//!   that shrinks without bound does not. Nothing has measured what the cutoff
//!   is.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use vale_assets::look::unitname as rules;
use crate::render::models::{DrawParams, M2Material, Materials, ModelCache};
use crate::world::session::WorldEntity;

/// Marks one **tint's** merged field of names, so the whole colour goes with
/// one despawn — see [`reconcile`].
#[derive(Component)]
struct LabelField;

/// The atlas, the one material over it, the merged fields and the typeface.
#[derive(Resource, Default)]
pub struct Labels {
    /// The name → where it sits in the atlas. `None` for a string that would
    /// not rasterise, which is remembered so the attempt is not made every
    /// frame.
    art: HashMap<String, Option<Art>>,
    /// **Every name in one texture** — see [`ATLAS_SIDE`], and see the module
    /// note for what a texture per name was costing.
    atlas: Option<Handle<Image>>,
    /// The atlas's own pixels, kept because the image is rewritten whenever a
    /// name is added to it and a `RENDER_WORLD` asset has no readable copy.
    pixels: Vec<u8>,
    /// The shelf packer: where the next name goes, and how tall the row it goes
    /// in is. A name is a wide, short rectangle and they are all the same
    /// height, so a shelf is the whole of the packing this needs.
    pen_x: u32,
    shelf_y: u32,
    shelf_h: u32,
    /// Whether the atlas has been written to since it was last uploaded.
    dirty: bool,
    /// The one material every name shares, over the atlas.
    material: Option<Handle<M2Material>>,
    /// One merged field per distinct tint — see [`reconcile`].
    fields: HashMap<u32, Field>,
    /// How many names were placed this frame, for the HUD line.
    names: usize,
    /// `Fonts\FRIZQT__.TTF`, parsed once. `None` before the archives are open
    /// and after a face that will not parse — see [`face`].
    font: Option<ab_glyph::FontVec>,
    /// Whether [`face`] has already tried and failed, so a broken archive costs
    /// one parse rather than one per frame.
    looked: bool,
}

impl Labels {
    /// How many names are on screen, for the HUD line.
    pub fn count(&self) -> usize {
        self.names
    }

    /// …and how many draws they come to, which is the number the merge is
    /// about.
    pub fn draws(&self) -> usize {
        self.fields.len()
    }
}

/// One tint's merged field: every name of that colour, in one mesh.
struct Field {
    entity: Entity,
    mesh: Handle<Mesh>,
    /// Last frame's vertices, so a field nothing moved in is not re-uploaded —
    /// `render::shadows`' own rule, and for the same reason: `Assets::get_mut`
    /// queues the upload whether or not the value differs.
    written: Vec<[f32; 3]>,
}

/// One rasterised name's place in the atlas.
struct Art {
    /// `u0, v0, u1, v1`.
    uv: [f32; 4],
    /// Width over height of the drawn text, so the quad is the right shape at
    /// any size. The *height* is the rules module's; only the ratio is the
    /// string's own.
    aspect: f32,
}

/// The atlas is this many texels square.
///
/// At [`RASTER_PX`] a name is about 56 texels tall and 100–300 wide, so 1024
/// holds a hundred or so — comfortably more than a name policy admits, which is
/// the local players and your target. **It resets rather than growing**: a
/// session that sees more distinct names than fit loses every name for one
/// frame and starts again, which is a frame nobody will see and is much less
/// machinery than a repack. 4 MB, and it replaces one texture per name.
const ATLAS_SIDE: u32 = 1024;

pub struct LabelPlugin;

impl Plugin for LabelPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report);
        app.init_resource::<Labels>().add_systems(
            Update,
            reconcile
                // **After the entity pass**, which places the unit this hangs
                // over — a label posed off last frame's transform trails its own
                // owner across the screen. The same ordering `questmarks` states.
                .after(crate::world::entities::EntitySet)
                // …and after the camera, because the quad faces the eye and a
                // rotation off last frame's rig lags a camera swing.
                .after(crate::world::camera::place),
        );
    }
}

/// Put a name over every head that should have one, and take down the rest.
///
/// **Reconciled rather than listened to**, on `questmarks`' own argument: a name
/// has to go up when the unit arrives, when its name query comes back, when it
/// is targeted, and when a CVar moves — four statements this client raises none
/// of, and all four are the same fact from a different side.
#[allow(clippy::too_many_arguments)]
fn reconcile(
    mut commands: Commands,
    mut labels: ResMut<Labels>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    tuning: Res<crate::render::tuning::WorldTuning>,
    assets: Res<crate::assets::GameAssets>,
    cvars: Res<crate::settings::cvars::CVars>,
    rig: Res<crate::world::camera::CameraRig>,
    player: Query<&WorldEntity, With<crate::session::LocalPlayer>>,
    selection: Res<crate::interface::target::Selection>,
    // …and the two halves of friend-or-foe that are in no file — see
    // [`crate::interface::api::Friendship`]. A name over a head is coloured by the
    // same selector the ring is, so it needs the same inputs.
    group: Res<crate::interface::party::Party>,
    reputation: Res<crate::interface::reputation::PlayerStanding>,
    units: Query<(
        &WorldEntity,
        &Transform,
        Option<&crate::world::entities::EntityModel>,
    )>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Labels);
    let labels = &mut *labels;
    labels.names = 0;
    // Under `entities`, like the quest marks: a name is part of what a unit
    // looks like, and a run with `--without entities` has no unit to name.
    if !tuning.entities {
        for (_, field) in labels.fields.drain() {
            commands.entity(field.entity).despawn();
        }
        return;
    }
    let me = player.single().ok();
    let friendship = crate::interface::api::Friendship { party: &group, standing: &reputation };
    let tables = assets.display_tables().ok();
    // **The five CVars, not a default.** A build dropped into a real WoW folder
    // reads that folder's `Config.wtf`.
    let policy = rules::Policy::from_cvars(|name| cvars.flag(name));
    // The eye the view matrix was built from, in Bevy's axes — the rig's own
    // accessor rather than the camera's `GlobalTransform`, which is a frame
    // stale in `Update` because propagation has not run yet.
    let eye = crate::render::axes::to_bevy(rig.eye().to_array());
    // …and what it is looking at, which is the other half of the basis every
    // label takes. One rotation for the whole pass — see [`face_the_camera`].
    let facing_the_camera =
        face_the_camera(eye, crate::render::axes::to_bevy(rig.focus().to_array()));
    // **The quad's own axes, taken out of that rotation once.** Every name is
    // parallel to the near plane, so `right` and `up` are the screen's — and
    // the corners are the centre plus multiples of them, which is the whole of
    // what building the merged mesh in world space needs.
    let right = facing_the_camera * Vec3::X;
    let up = facing_the_camera * Vec3::Y;
    let normal = (facing_the_camera * Vec3::Z).to_array();

    // One bucket per tint — see the module note on why the split is by colour.
    let mut wanted: HashMap<u32, Quads> = HashMap::default();
    for (unit, at, model) in &units {
        if unit.name.is_empty() {
            // **A nameless unit is a query in flight, not a unit with no name.**
            // Putting an empty quad up and re-texturing it a round trip later is
            // the same flicker the loot window's name beat was.
            continue;
        }
        let candidate = rules::Candidate {
            // **A chest is not an NPC.** This client keeps game objects in the
            // same table as units and the reference never has to tell them
            // apart, so the test is the rules module's — see `Candidate::unit`.
            unit: matches!(
                unit.kind,
                vale_protocol::state::update::ObjectType::Unit
                    | vale_protocol::state::update::ObjectType::Player
            ),
            // The reference reaches a virtual and two fields this client does
            // not read; what it has is "in the world, alive, and named".
            nameable: !unit.dead,
            is_self: unit.is_self,
            player: unit.kind == vale_protocol::state::update::ObjectType::Player,
            targeted: selection.guid == Some(unit.guid),
        };
        if !policy.names(&candidate) {
            continue;
        }
        // The `PlayerName` attachment in the unit's own scale — the same point
        // the reference hangs a name from, and the same number it measures the
        // unit's size by, which is why the ramp is applied to the *placed*
        // height rather than the model's own.
        let lift = model.map_or(2.0, |model| model.name_anchor) * at.scale.y;
        let over = at.translation + Vec3::Y * lift;
        let Some((uv, aspect)) =
            art_for(labels, &assets, &mut images, &mut cache, &mut materials, &unit.name)
        else {
            continue;
        };
        let height = rules::size_for(lift);
        let half = right * (height * aspect * 0.5);
        let top = up * height;
        let tint = crate::render::models::tint_tag(colour(tables.as_deref(), friendship, me, unit));
        // **Sitting on its own origin**, as the single quad did: the text is
        // above the `PlayerName` point rather than through it.
        let quads = wanted.entry(tint).or_default();
        let base = quads.positions.len() as u32;
        for (corner, texel) in [
            (over - half, [uv[0], uv[3]]),
            (over + half, [uv[2], uv[3]]),
            (over + half + top, [uv[2], uv[1]]),
            (over - half + top, [uv[0], uv[1]]),
        ] {
            quads.positions.push(corner.to_array());
            quads.normals.push(normal);
            quads.uvs.push(texel);
        }
        quads.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        labels.names += 1;
    }

    // **The atlas is uploaded before anything is drawn from it**, so the first
    // frame a name exists draws that name rather than an empty rectangle.
    if labels.dirty {
        labels.dirty = false;
        if let Some(handle) = labels.atlas.clone() {
            if let Some(mut image) = images.get_mut(&handle) {
                image.data = Some(labels.pixels.clone());
            }
        }
    }
    let Some(material) = labels.material.clone() else {
        // No name has rasterised yet, so there is no atlas and nothing to draw.
        for (_, field) in labels.fields.drain() {
            commands.entity(field.entity).despawn();
        }
        return;
    };

    // A tint nothing wears any more takes its field down with it.
    labels.fields.retain(|tint, field| {
        let keep = wanted.contains_key(tint);
        if !keep {
            commands.entity(field.entity).despawn();
        }
        keep
    });
    for (tint, quads) in wanted {
        let field = labels.fields.entry(tint).or_insert_with(|| {
            let mesh = meshes.add(empty_field());
            let entity = commands
                .spawn((
                    LabelField,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    // Every vertex is already in world space, so the transform
                    // is the identity rather than a placement — which is
                    // `render::shadows` arrangement, and for its reason.
                    Transform::default(),
                    bevy::mesh::MeshTag(tint),
                    // One draw whose extent changes every frame: keeping an
                    // `Aabb` current would cost more than the test saves.
                    NoFrustumCulling,
                    bevy::light::NotShadowCaster,
                ))
                .id();
            Field { entity, mesh, written: Vec::new() }
        });
        // Nothing moved: the corners are the ones already in the mesh, so leave
        // the asset alone. **Before** `meshes.get_mut`, which queues the
        // re-upload whether or not anything is written.
        if field.written == quads.positions {
            continue;
        }
        let Some(mut mesh) = meshes.get_mut(&field.mesh) else {
            continue;
        };
        field.written.clone_from(&quads.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, quads.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, quads.normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, quads.uvs);
        mesh.insert_indices(Indices::U32(quads.indices));
    }
}

/// One tint's quads, being built.
#[derive(Default)]
struct Quads {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

/// A field's mesh before any name is in it — the attributes declared so the
/// vertex layout is fixed from the first frame, and no triangles that reach the
/// rasteriser. `render::shadows`' own `empty_field`, for its reason.
fn empty_field() -> Mesh {
    // `default()` rather than `RENDER_WORLD`: this geometry is rewritten every
    // frame, and a mesh whose main-world data was dropped panics on the replace.
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, Vec::<[f32; 2]>::new());
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

/// The atlas rectangle and shape for one name, rasterised on first sight.
///
/// **Cached on the string rather than on the guid**, because a city is full of
/// `Stormwind Guard` and every one of them is the same picture. A `None` is
/// remembered too: a string the face has no glyphs for must cost one attempt,
/// not one per frame.
///
/// The pixels go into the one atlas ([`ATLAS_SIDE`]) rather than into a texture
/// of their own, which is what lets every name on screen share a material and
/// therefore a mesh — see the module note for what the alternative measured.
fn art_for(
    labels: &mut Labels,
    assets: &crate::assets::GameAssets,
    images: &mut Assets<Image>,
    cache: &mut ModelCache,
    materials: &mut Materials,
    name: &str,
) -> Option<([f32; 4], f32)> {
    if !labels.art.contains_key(name) {
        let rastered = face(labels, assets).and_then(|font| raster(font, name));
        let art = rastered.and_then(|(pixels, width, height)| {
            // The atlas and the one material over it, built on the first name
            // rather than at startup: a session that never shows one — every
            // `UnitName*` off — pays neither the 4 MB nor the upload.
            if labels.atlas.is_none() {
                labels.pixels = vec![0u8; (ATLAS_SIDE * ATLAS_SIDE * 4) as usize];
                let handle = images.add(atlas_image());
                labels.material =
                    Some(cache.quad_material(materials, handle.clone(), &params()));
                labels.atlas = Some(handle);
            }
            let (x, y) = labels.reserve(width, height)?;
            for row in 0..height {
                let from = (row * width * 4) as usize;
                let to = (((y + row) * ATLAS_SIDE + x) * 4) as usize;
                labels.pixels[to..to + (width * 4) as usize]
                    .copy_from_slice(&pixels[from..from + (width * 4) as usize]);
            }
            labels.dirty = true;
            // **Half a texel in from each edge.** The sampler is linear and the
            // atlas has a neighbour on every side, so UVs on the exact boundary
            // bleed the next name in — which on a name is the first letter of
            // somebody else appearing faintly beside the last of yours. The old
            // per-name texture used `ClampToEdge` for the same reason; an atlas
            // has no edge to clamp to.
            let side = ATLAS_SIDE as f32;
            let uv = [
                (x as f32 + 0.5) / side,
                (y as f32 + 0.5) / side,
                (x + width) as f32 / side - 0.5 / side,
                (y + height) as f32 / side - 0.5 / side,
            ];
            Some(Art { uv, aspect: width as f32 / height as f32 })
        });
        labels.art.insert(name.to_string(), art);
    }
    labels.art.get(name)?.as_ref().map(|art| (art.uv, art.aspect))
}

impl Labels {
    /// Where the next name goes in the atlas, shelf by shelf.
    ///
    /// **A full atlas resets rather than repacking.** Every rectangle is the
    /// same height and they arrive in first-sight order, so a shelf packer wastes
    /// almost nothing and there is no case where repacking would recover much;
    /// what a reset costs is one frame with no names while they rasterise again,
    /// and it takes a few hundred distinct names in one session to reach it.
    /// `None` only for a name too wide for the atlas at all, which
    /// [`MAX_TEXTURE`] has already refused.
    fn reserve(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width > ATLAS_SIDE || height > ATLAS_SIDE {
            return None;
        }
        if self.pen_x + width > ATLAS_SIDE {
            // Next shelf.
            self.shelf_y += self.shelf_h;
            self.pen_x = 0;
            self.shelf_h = 0;
        }
        if self.shelf_y + height > ATLAS_SIDE {
            warn!(
                "labels: the name atlas filled at {} names; starting it again",
                self.art.len()
            );
            self.art.clear();
            self.pixels.fill(0);
            self.pen_x = 0;
            self.shelf_y = 0;
            self.shelf_h = 0;
        }
        let at = (self.pen_x, self.shelf_y);
        self.pen_x += width;
        self.shelf_h = self.shelf_h.max(height);
        Some(at)
    }
}

/// The atlas as an image the renderer can sample.
///
/// **Clamped rather than repeated**, which is the one sampler setting that
/// differs from every other texture in this client: a `Repeat` bleeds the far
/// edge in under linear filtering. The per-name inset in [`art_for`] is what
/// keeps two neighbours in the atlas from doing the same to each other.
fn atlas_image() -> Image {
    let mut image = Image::new(
        Extent3d { width: ATLAS_SIDE, height: ATLAS_SIDE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0u8; (ATLAS_SIDE * ATLAS_SIDE * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        // **Both worlds**, unlike every static texture here: the atlas is
        // rewritten whenever a name is added to it, and a `RENDER_WORLD` image
        // has no main-world copy to write into.
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// `Fonts\FRIZQT__.TTF`, out of the archives, parsed once.
///
/// The same face `UNIT_NAME_FONT` names — see
/// [`vale_assets::look::unitname::FONT`], which is where the Lua global that
/// holds the path is read.
fn face<'a>(labels: &'a mut Labels, assets: &crate::assets::GameAssets) -> Option<&'a ab_glyph::FontVec> {
    if !labels.looked {
        labels.looked = true;
        labels.font = assets
            .with_archive(|archive| archive.read(rules::FONT).map_err(|e| e.to_string()))
            .ok()
            .and_then(|bytes| ab_glyph::FontVec::try_from_vec(bytes).ok());
        if labels.font.is_none() {
            warn!("labels: {} would not load; no unit names", rules::FONT);
        }
    }
    labels.font.as_ref()
}

/// **One name, rasterised** — white glyphs on a black shadow, `RGBA8`.
///
/// The glyphs are drawn **white** and the colour comes from the instance's
/// `MeshTag` (see [`params`]), so one texture serves a name whatever its
/// reaction — which matters because the commonest name in the game is on a
/// hundred guards and half of them are somebody else's faction.
///
/// The shadow is this client's: the reference gives its head-mounted strings a
/// `0.001` offset in a space whose size this client draws in yards, where a
/// millimetre of shadow is nothing. What is here is [`SHADOW_PX`] of the raster,
/// which is a constant fraction of the glyph however far away it ends up — and a
/// white name over snow is invisible without it, which is the picture this was
/// reported from.
fn raster(font: &ab_glyph::FontVec, text: &str) -> Option<(Vec<u8>, u32, u32)> {
    use ab_glyph::{Font, ScaleFont};
    let scaled = font.as_scaled(ab_glyph::PxScale::from(RASTER_PX));
    let mut pen = 0.0f32;
    let mut glyphs = Vec::new();
    let mut previous = None;
    for c in text.chars() {
        let id = scaled.glyph_id(c);
        if let Some(last) = previous {
            pen += scaled.kern(last, id);
        }
        previous = Some(id);
        glyphs.push((id, pen));
        pen += scaled.h_advance(id);
    }
    let ascent = scaled.ascent();
    let pad = SHADOW_PX.ceil() as i32 + 1;
    let width = (pen.ceil() as i32 + pad * 2).max(1) as u32;
    let height = ((ascent - scaled.descent()).ceil() as i32 + pad * 2).max(1) as u32;
    if width > MAX_TEXTURE || height > MAX_TEXTURE {
        return None;
    }

    // Coverage first, then the compose below: the shadow is the same mask read
    // at an offset, so it has to exist as a mask before either is written.
    let mut mask = vec![0f32; (width * height) as usize];
    for (id, x) in glyphs {
        let placed = id.with_scale_and_position(
            RASTER_PX,
            ab_glyph::point(x + pad as f32, ascent + pad as f32),
        );
        // A space, and any codepoint the face has no contours for, outlines to
        // nothing — which is not a failure, it is a gap in the word.
        let Some(outline) = font.outline_glyph(placed) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, coverage| {
            let px = bounds.min.x as i32 + gx as i32;
            let py = bounds.min.y as i32 + gy as i32;
            if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                return;
            }
            let cell = &mut mask[(py as u32 * width + px as u32) as usize];
            *cell = cell.max(coverage);
        });
    }

    let shift = SHADOW_PX.round() as i32;
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let text_a = mask[(y as u32 * width + x as u32) as usize];
            let (sx, sy) = (x - shift, y - shift);
            let shadow_a = if sx >= 0 && sy >= 0 {
                mask[(sy as u32 * width + sx as u32) as usize]
            } else {
                0.0
            };
            // The glyph over its own shadow, in one texel: the alpha is what
            // either of them covers and the colour is white only where the
            // glyph itself is, so a tinted label keeps a black outline.
            let alpha = text_a + shadow_a * (1.0 - text_a);
            let white = if alpha > 0.0 { text_a / alpha } else { 0.0 };
            let at = ((y as u32 * width + x as u32) * 4) as usize;
            let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            pixels[at] = byte(white);
            pixels[at + 1] = byte(white);
            pixels[at + 2] = byte(white);
            pixels[at + 3] = byte(alpha);
        }
    }
    Some((pixels, width, height))
}

/// The one draw every label shares.
///
/// **Alpha blend and no depth write**, which is what puts it in the sorted
/// transparent phase beside the particles: a name behind a tree is occluded by
/// the tree's own opaque depth, and two names that overlap each other blend
/// rather than one punching a hole in the other.
///
/// **Tinted**, so one material serves every colour — `DrawParams::tint` does not
/// carry a colour, it says only that the shader should read this instance's
/// `MeshTag` as `0xAARRGGBB`. That is `render::selection`'s arrangement and the
/// reason it is here too.
fn params() -> DrawParams {
    DrawParams {
        geoset: 0,
        texture: None,
        // 2 — alpha blend. Not 4: a name is not a glow, and add-alpha over a
        // bright hillside washes a white name out entirely.
        blend: 2,
        // Nothing lights a name. The reference writes the colour straight in.
        unlit: true,
        // A screen-aligned quad is never edge-on and never back-facing, so this
        // is belt and braces rather than a rule — kept because the one frame it
        // would matter is the one where the basis is degenerate.
        two_sided: true,
        no_depth_write: true,
        light: vale_assets::world::wmo::BatchLight::Sun,
        liquid: None,
        ground: None,
        baked_tint: None,
        tint: Some(vale_assets::world::m2::BatchTint { color: None, transparency: None }),
        uv: None,
        // A quad this client builds has no environment map — see
        // `models::loader::RawDraw::overlays`.
        overlays: Default::default(),
    }
}

/// **The camera's own rotation**, so the quad is parallel to the near plane —
/// square to the view on every axis, from any pitch, and upright on the screen.
///
/// Built from the eye and the focus rather than read off the camera's
/// `GlobalTransform`, for the reason the eye itself is: propagation has not run
/// in `Update`, so the component is a frame stale. These are the same two
/// points [`crate::world::camera::place`] built this frame's view matrix from —
/// the rig's own accessors — so the label's basis and the view's cannot
/// disagree.
///
/// The quad's normal is `+Z` and `looking_at` points the camera's `-Z` at its
/// focus, so adopting the rotation whole turns the quad's face back towards the
/// eye. It is the *same* rotation for every label on screen, which is what
/// "screen-aligned" means and is why this does not take the label's position.
///
/// Degenerate only if the eye sits exactly on the focus, which the rig's own
/// minimum reach forbids; `looking_at` is a no-op in that case rather than a
/// NaN, and identity is the honest answer.
fn face_the_camera(eye: Vec3, focus: Vec3) -> Quat {
    if (focus - eye).length_squared() < 1e-6 {
        return Quat::IDENTITY;
    }
    Transform::from_translation(eye)
        .looking_at(focus, Vec3::Y)
        .rotation
}

/// What colour the name is.
///
/// **A reuse rather than a measurement, and it is the one thing here to
/// distrust.** The reference's name-holder has no colour field at all, so
/// where its colour comes from was never found. What is drawn instead is the
/// selection ring's own selector, `GetSelectionCircleColor`.
///
/// The cross-check that makes it a stand-in rather than a guess is in the
/// archives: `GameTooltip_UnitColor` in the shipped `GameTooltip.lua` asks the
/// *same three questions in the same order* about the same unit, one layer up,
/// and colours the tooltip's name with the answer. A screenshot since has shown
/// a hostile creature named red and a neutral one yellow, which is that selector
/// answering correctly — what is still missing is the ring table's **orange**
/// fourth rank, for the reason `look::selection` already gives: `Reaction` is
/// three-valued here and an unfriendly creature reads hostile.
fn colour(
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    friendship: crate::interface::api::Friendship<'_>,
    me: Option<&WorldEntity>,
    unit: &WorldEntity,
) -> [f32; 4] {
    use vale_assets::look::selection::{ring_colour, Selected};
    use vale_assets::tables::faction::Reaction;
    // **No table, no opinion**, which is `render::selection`'s own rule: the
    // rule answers Neutral for a table it does not have, and a yellow name over
    // a wolf is exactly the kind of plausible wrong answer this project counts
    // separately.
    //
    // **The reaction is the unit's towards *us*** — see [`Selected::reaction`],
    // whose own comment said so while both this and the ring asked it the other
    // way round for eight rounds. It only shows where the two directions
    // differ, and the whole of what the character's own state adds to the rule
    // is legs that fire in one direction and not the other.
    let (reaction, i_attack_it, attacks_me) = match (tables, me) {
        (Some(tables), Some(me)) => (
            friendship.reaction(tables, unit, me),
            crate::interface::api::can_attack_between(
                tables, friendship.party, friendship.standing, me, unit,
            ),
            crate::interface::api::can_attack_between(
                tables, friendship.party, friendship.standing, unit, me,
            ),
        ),
        _ => (Reaction::Neutral, false, false),
    };
    let argb = ring_colour(&Selected {
        player_controlled: unit.kind == vale_protocol::state::update::ObjectType::Player,
        attacks_me,
        i_attack_it,
        pvp: unit.unit_flags & crate::interface::api::UNIT_FLAG_PVP != 0,
        dead: unit.dead,
        reaction,
    });
    let byte = |shift: u32| ((argb >> shift) & 0xff) as f32 / 255.0;
    [byte(16), byte(8), byte(0), byte(24)]
}

/// [`crate::ui::report::HudReport`] slot. 33, immediately under the portraits'
/// at 32 and the interface's at 31: a name is a picture over a head, and the
/// number worth reading beside "N quads" is how many *more* the world is paying
/// for.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(33);

/// **Up** is how many quads are in the world; **cut** is how many distinct
/// strings have a texture, which is the number that matters if the population
/// ever grows — a field of thirty guards is one cut and thirty quads, and a
/// field of thirty differently-named creatures is thirty of each.
#[cfg(feature = "diagnostics")]
fn report(labels: Res<Labels>, mut hud: ResMut<crate::ui::report::HudReport>) {
    if labels.names == 0 && labels.art.is_empty() {
        hud.clear(SLOT, "names");
        return;
    }
    hud.set(
            crate::ui::report::Section::Scene,
        SLOT,
        "names",
        format!(
            "names: {} up in {} draw(s), {} in the atlas",
            labels.names,
            labels.draws(),
            labels.art.len()
        ),
    );
}

/// How tall a name is rasterised, in texels.
///
/// **Once per name, whatever the distance** — which is the whole reason this is
/// a render pass. Big enough that a name filling a quarter of a 1080-tall screen
/// is still sampling down rather than up, small enough that a city's worth of
/// distinct names is a few megabytes.
const RASTER_PX: f32 = 48.0;

/// The drop shadow's offset, in texels of the raster.
const SHADOW_PX: f32 = 3.0;

/// A name this wide in texels is refused rather than rasterised.
///
/// Nothing the server sends comes near it — a 1.12 name is at most twelve
/// characters and a creature's rather more — but a string is server data and an
/// unbounded allocation off one is the shape this project treats as a fault
/// rather than an unlikely case.
///
/// **It is deliberately larger than [`ATLAS_SIDE`]**, and the two are not the
/// same test: this one bounds the *allocation* the raster makes, and
/// [`Labels::reserve`] refuses again at the atlas's own width. A single check at
/// 1024 would leave the raster free to build a 2000-texel bitmap on the way to
/// being thrown away.
const MAX_TEXTURE: u32 = 2048;

#[cfg(test)]
mod tests {
    use super::*;

    /// **A name is square to the view from any pitch**, which is the whole of
    /// the report this replaced a rule for. The quad's normal is its local
    /// `+Z`; turned by the camera's basis it must point back along the view,
    /// whether the camera is level with the unit or straight above it.
    #[test]
    fn the_quad_faces_the_camera_on_every_axis() {
        let focus = Vec3::new(10.0, 5.0, -3.0);
        for eye in [
            focus + Vec3::new(0.0, 0.0, 12.0),        // level, behind
            focus + Vec3::new(9.0, 2.0, 9.0),         // the ordinary camera
            focus + Vec3::new(0.0, 20.0, 0.001),      // straight down
            focus + Vec3::new(0.0, -20.0, 0.001),     // and straight up
        ] {
            let q = face_the_camera(eye, focus);
            let normal = q * Vec3::Z;
            let to_eye = (eye - focus).normalize();
            assert!(
                normal.dot(to_eye) > 0.999,
                "eye {eye:?}: the quad's face is {:.3} off the view",
                normal.dot(to_eye)
            );
        }
    }

    /// **…and it is upright on the *screen*, not in the world**, which is the
    /// half the old rule had right for the wrong reason: with the camera
    /// looking straight down, "upright in the world" is edge-on and unreadable.
    /// The quad's local up must stay perpendicular to the view and never
    /// collapse.
    #[test]
    fn looking_straight_down_does_not_lay_the_name_flat() {
        let focus = Vec3::ZERO;
        let eye = Vec3::new(0.0, 20.0, 0.001);
        let q = face_the_camera(eye, focus);
        let up = q * Vec3::Y;
        let to_eye = (eye - focus).normalize();
        // Perpendicular to the view — a foreshortened quad is one whose up has
        // rotated towards the eye.
        assert!(up.dot(to_eye).abs() < 1e-3, "the name foreshortened: {up:?}");
        assert!((up.length() - 1.0).abs() < 1e-4);
    }

    /// **The shelf packer lays names left to right and drops to a new row when
    /// one will not fit**, and every rectangle it hands back is inside the
    /// atlas.
    ///
    /// The rectangles matter more than the packing does: a name placed even one
    /// texel outside would sample some other name's pixels, which is text that
    /// reads as somebody else's.
    #[test]
    fn the_packer_stays_inside_the_atlas_and_moves_down_a_shelf() {
        let mut labels = Labels::default();
        let (w, h) = (300u32, 56u32);
        let mut rows = std::collections::BTreeSet::new();
        for _ in 0..12 {
            let (x, y) = labels.reserve(w, h).expect("a name that fits");
            assert!(x + w <= ATLAS_SIDE, "ran off the right edge");
            assert!(y + h <= ATLAS_SIDE, "ran off the bottom");
            rows.insert(y);
        }
        assert!(rows.len() > 1, "twelve 300-wide names do not fit on one 1024 shelf");
        // The shelves are the name's own height apart, so nothing overlaps.
        let mut previous = None;
        for y in rows {
            if let Some(last) = previous {
                assert!(y - last >= h, "two shelves overlap");
            }
            previous = Some(y);
        }
    }

    /// **A full atlas starts again rather than overflowing**, which is the one
    /// case where a wrong rectangle would be a wrong picture — see
    /// [`Labels::reserve`].
    #[test]
    fn a_full_atlas_resets_rather_than_running_off_the_end() {
        let mut labels = Labels::default();
        labels.pixels = vec![0u8; (ATLAS_SIDE * ATLAS_SIDE * 4) as usize];
        // Fill it: full-width shelves, so the atlas is exhausted in as many
        // reservations as it has rows.
        let (w, h) = (ATLAS_SIDE, 64u32);
        for _ in 0..(ATLAS_SIDE / h) {
            labels.art.insert(format!("{}", labels.art.len()), None);
            let (x, y) = labels.reserve(w, h).expect("a name that fits");
            assert!(y + h <= ATLAS_SIDE);
            assert_eq!(x, 0);
        }
        // The next one does not fit anywhere, so the atlas is emptied.
        let (x, y) = labels.reserve(w, h).expect("the reset makes room");
        assert_eq!((x, y), (0, 0));
        assert!(labels.art.is_empty(), "the cache went with it, or the UVs would lie");
    }

    /// A name wider than the whole atlas is refused rather than clamped.
    #[test]
    fn a_name_too_wide_for_the_atlas_is_refused() {
        let mut labels = Labels::default();
        assert!(labels.reserve(ATLAS_SIDE + 1, 32).is_none());
        assert!(labels.reserve(32, ATLAS_SIDE + 1).is_none());
    }

    /// A degenerate rig — the eye exactly on the point it is looking at — is
    /// the identity rather than a NaN, and a NaN in a `Transform` is a whole
    /// frame of nothing.
    #[test]
    fn a_degenerate_camera_is_the_identity_rather_than_a_nan() {
        let q = face_the_camera(Vec3::splat(3.0), Vec3::splat(3.0));
        assert_eq!(q, Quat::IDENTITY);
        assert!(q.to_array().iter().all(|v| v.is_finite()));
    }
}
