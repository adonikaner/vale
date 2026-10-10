//! Unit names, drawn as textured quads in the world.
//!
//! ```text
//! look::unitname   which units are named, height in yards <- no window
//! raster           the string -> one RGBA texture, once   <- ab_glyph
//! reconcile        a camera-facing quad per named unit    <- the same M2Material
//!                  as every other draw in the world
//! ```
//!
//! ## Why names are a render pass and not an interface layer
//!
//! A name must be occluded by world geometry the way a spell effect is. An egui
//! layer is painted over the finished frame and has no depth, so as an
//! interface layer a name could only be hidden whole, when a ray from the eye
//! to the unit's head hit something. A name half behind a tree trunk then
//! disappeared entirely, where the 1.12.1 client shows the half that clears the
//! trunk.
//!
//! Each name is a quad in the world with a texture on it, drawn with
//! [`M2Material`], the material every particle, ground decal and selection ring
//! in this client uses. It is therefore in the same sorted transparent phase
//! and takes the same depth test as the trunk. This file contains no occlusion
//! code; the depth buffer does the work.
//!
//! Drawing names as quads has three further effects:
//!
//! * The glyph atlas is not refilled every frame. The egui painter rasterises
//!   per font size, and a size derived from a projection changes every frame
//!   for every unit. That caused an overflow warning and then a panic ("Tried
//!   to allocate a 2720 wide glyph in a 2048 wide texture atlas"). A name is
//!   rasterised once, at [`RASTER_PX`], and distance changes only the quad's
//!   scale.
//! * The size needs no clamp. A unit against the near plane has a large quad.
//! * No projection arithmetic is needed. An interface layer has to convert the
//!   world camera's physical viewport pixels into the interface's logical
//!   space, a conversion this project has made incorrectly before, in the other
//!   direction. A quad is placed in yards and the camera projects it.
//!
//! ## What is in this module and what is in `vale_assets::look::unitname`
//!
//! [`vale_assets::look::unitname`] decides which units get a name, which of the
//! five `UnitName*` CVars applies, and how tall the name is in yards. Those
//! rules need no archive access, and every number in them is the 1.12.1
//! client's. This module holds the part that needs a renderer.
//!
//! ## The billboard is screen-aligned
//!
//! [`face_the_camera`] gives every label the camera's own basis, so the quad is
//! parallel to the near plane and its up is the screen's up.
//!
//! An earlier version turned the quad about the world's up axis only, to keep
//! the text upright in the world. That made a name foreshorten when the camera
//! looked steeply down. The two rules give the same picture at an ordinary
//! camera pitch, so the difference shows only at a steep one.
//!
//! The 1.12.1 client appears not to draw the name in the world: it draws it in
//! screen space, at the projected `PlayerName` point, and a projected 2D string
//! cannot be foreshortened. Three facts about the client support this:
//!
//! * `UNIT_NAME_FONT` is built at 0.99 of the interface's height. That is only
//!   useful for a face that is scaled down by a large and varying amount.
//! * The size given to the draw is not divided by a distance, so the shrink of
//!   a distant name comes from a projection and not from the size.
//! * The string is laid out with 100000.0 as both its maximum width and its
//!   maximum height. That is text layout, not a mesh build, and it uses the
//!   same text subsystem as the damage numbers, whose heights
//!   [`vale_assets::look::worldtext`] records as fractions of the interface's
//!   height.
//!
//! The first two are measurements. "Screen space" is an inference from the font
//! size and the missing distance divide; the client's final draw of the string
//! has not been observed. The foreshortening of the earlier version is certain.
//!
//! A quad is kept because it keeps the depth test. Only the rotation differs
//! from the earlier version. The size is still a world height, so a name still
//! shrinks with distance.
//!
//! ## What this module does not do
//!
//! * A size or a colour per line. The text is up to four lines (the name, a
//!   city title, and a guild or a subname and an owner line; see
//!   [`rules::floating_text`]) and every line has the name's size and colour,
//!   as in the 1.12.1 client, so the text is one raster and one quad.
//! * More than one draw per colour. The names are packed into one atlas and
//!   merged into one mesh per distinct tint (see [`reconcile`]), so the pass is
//!   three or four draws for any population. With one texture, one material and
//!   one draw per name, forty name plates over forty players measured 1.8 ms of
//!   a 12.0 ms frame, a seventh of the frame. The split is by colour because
//!   the colour is carried in the instance's `MeshTag` (see [`params`]) and a
//!   merged mesh has one tag, so one mesh per tint needs no shader change. The
//!   ring table has four tints and a screen rarely shows three.
//! * An atlas that is repacked or shared across sessions. It is packed in
//!   first-sight order and reset when it fills, so a very long session in a
//!   crowded city draws no names for a single frame every few hundred distinct
//!   names. See [`ATLAS_SIDE`].
//! * Fading out at range. The 1.12.1 client keeps a distant name legible; a
//!   quad that shrinks without bound does not. The cutoff distance has not been
//!   measured.

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

/// Marks the merged mesh entity of one tint, so every name of that colour is
/// removed with one despawn. See [`reconcile`].
#[derive(Component)]
struct LabelField;

/// The atlas, the one material over it, the merged fields and the typeface.
#[derive(Resource, Default)]
pub struct Labels {
    /// Name → its place in the atlas. `None` for a string that would not
    /// rasterise; it is stored so the attempt is not repeated every frame.
    art: HashMap<String, Option<Art>>,
    /// The one texture that holds every name. See [`ATLAS_SIDE`], and the
    /// module note for the measured cost of a texture per name.
    atlas: Option<Handle<Image>>,
    /// The atlas's own pixels, kept because the image is rewritten whenever a
    /// name is added to it and a `RENDER_WORLD` asset has no readable copy.
    pixels: Vec<u8>,
    /// The shelf packer's state: where the next name goes, and the height of
    /// the row it goes in. Every name is a wide, short rectangle of the same
    /// height, so shelf packing is sufficient.
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

    /// How many draws the names take: one per distinct tint.
    pub fn draws(&self) -> usize {
        self.fields.len()
    }
}

/// One tint's merged field: every name of that colour, in one mesh.
struct Field {
    entity: Entity,
    mesh: Handle<Mesh>,
    /// Last frame's vertices, so a field in which nothing moved is not uploaded
    /// again: `Assets::get_mut` queues the upload whether or not the value
    /// differs. `render::shadows` uses the same rule for the same reason.
    written: Vec<[f32; 3]>,
}

/// One rasterised name's place in the atlas.
struct Art {
    /// `u0, v0, u1, v1`.
    uv: [f32; 4],
    /// Width over height of the drawn text, which gives the quad its shape at
    /// any size. The height comes from the rules module; only the ratio depends
    /// on the string.
    aspect: f32,
}

/// The side of the square atlas, in texels.
///
/// At [`RASTER_PX`] a name is about 56 texels tall and 100–300 wide, so 1024
/// holds about a hundred. That is more than the default name policy admits,
/// which is the local players and the target. The atlas resets when it is full
/// and does not grow: a session that sees more distinct names than fit draws no
/// names for one frame and starts again. A reset needs much less code than a
/// repack. The atlas is 4 MB and replaces one texture per name.
const ATLAS_SIDE: u32 = 1024;

pub struct LabelPlugin;

impl Plugin for LabelPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report);
        app.init_resource::<Labels>().add_systems(
            Update,
            reconcile
                // After the entity pass, which places the unit the label is
                // above. A label posed from last frame's transform trails its
                // unit across the screen. `questmarks` states the same ordering.
                .after(crate::world::entities::EntitySet)
                // After the camera, because the quad faces the eye and a
                // rotation taken from last frame's rig lags a camera swing.
                .after(crate::world::camera::place),
        );
    }
}

/// Builds a name quad over every unit that should have one and removes the
/// rest.
///
/// The set is recomputed every frame and not updated from events, for the
/// reason `questmarks` gives. A name has to appear when the unit arrives, when
/// its name query is answered, when it is targeted and when a CVar changes.
/// This client raises an event for none of the four, and all four change the
/// same answer.
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
    // The party and the reputation standing are the two inputs to friend-or-foe
    // that are in no file; see [`crate::interface::api::Friendship`]. A name is
    // coloured by the same selector as the selection ring, so it needs the same
    // inputs.
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
    // Gated on `entities`, like the quest marks: a name is part of a unit's
    // appearance, and a run with `--without entities` has no unit to name.
    if !tuning.entities {
        for (_, field) in labels.fields.drain() {
            commands.entity(field.entity).despawn();
        }
        return;
    }
    let me = player.single().ok();
    let friendship = crate::interface::api::Friendship { party: &group, standing: &reputation };
    let tables = assets.display_tables().ok();
    // The policy is read from the five CVars and is not a default. A build
    // placed in a real WoW folder reads that folder's `Config.wtf`.
    let policy = rules::Policy::from_cvars(|name| cvars.flag(name));
    // `GlobalStrings.lua`, for the tags, the rank titles and the owner line.
    let strings = assets.strings();
    let word = |key: &str| strings.get(key).map(str::to_string);
    // The eye the view matrix was built from, in Bevy's axes. It comes from the
    // rig's accessor because the camera's `GlobalTransform` is a frame stale in
    // `Update`: propagation has not run yet.
    let eye = crate::render::axes::to_bevy(rig.eye().to_array());
    // The view direction is the other input to the basis every label takes,
    // taken from the orbit (the eye toward the focus, one yard on) so that it
    // is defined at a zoom of zero. One rotation serves the whole pass; see
    // [`face_the_camera`].
    let toward = crate::render::axes::to_bevy((rig.focus() - rig.at(1.0)).to_array());
    let facing_the_camera = face_the_camera(eye, eye + toward);
    // The quad's axes, taken from that rotation once. Every name is parallel to
    // the near plane, so `right` and `up` are the screen's axes and each corner
    // is the centre plus multiples of them. Building the merged mesh in world
    // space needs nothing else.
    let right = facing_the_camera * Vec3::X;
    let up = facing_the_camera * Vec3::Y;
    let normal = (facing_the_camera * Vec3::Z).to_array();

    // One bucket per tint. The module note explains why the split is by colour.
    let mut wanted: HashMap<u32, Quads> = HashMap::default();
    for (unit, at, model) in &units {
        if unit.name.is_empty() {
            // A unit with an empty name has a name query in flight; it is not a
            // unit without a name. It is skipped, because an empty quad that is
            // re-textured a round trip later flickers, as the loot window's
            // name did.
            continue;
        }
        let candidate = rules::Candidate {
            // A chest is not an NPC. This client keeps game objects in the same
            // table as units, and the 1.12.1 client's naming rule is applied
            // only to units, so the test is in the rules module. See
            // `Candidate::unit`.
            unit: matches!(
                unit.kind,
                vale_protocol::state::update::ObjectType::Unit
                    | vale_protocol::state::update::ObjectType::Player
            ),
            // The 1.12.1 client's test depends on state this client does not
            // read. This client's equivalent is "in the world, alive and named".
            nameable: !unit.dead,
            is_self: unit.is_self,
            player: unit.kind == vale_protocol::state::update::ObjectType::Player,
            targeted: selection.guid == Some(unit.guid),
        };
        if !policy.names(&candidate) {
            continue;
        }
        // The `PlayerName` attachment in the unit's own scale. The 1.12.1
        // client hangs the name from this point and measures the unit's size by
        // the same number, so the ramp is applied to the placed height and not
        // to the model's own.
        let lift = model.map_or(2.0, |model| model.name_anchor) * at.scale.y;
        let over = at.translation + Vec3::Y * lift;
        // The whole text: the name with its tags and rank, then the guild,
        // the subname and the owner line, one under another. See
        // [`rules::floating_text`].
        let text = rules::floating_text(
            &crate::interface::api::name_parts(tables.as_deref(), unit),
            &policy,
            &word,
        );
        let Some((uv, aspect)) =
            art_for(labels, &assets, &mut images, &mut cache, &mut materials, &text)
        else {
            continue;
        };
        // Every line has the name's size, so the quad is one line's height
        // for each line and its bottom edge stays on the anchor.
        let lines = text.split('\n').count() as f32;
        let height = rules::size_for(lift) * lines;
        let half = right * (height * aspect * 0.5);
        let top = up * height;
        let tint = crate::render::models::tint_tag(colour(tables.as_deref(), friendship, me, unit));
        // The quad's bottom edge is on the `PlayerName` point, so the text is
        // above the point and not centred on it.
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

    // The atlas is uploaded before anything is drawn from it, so the first
    // frame a name exists draws that name and not an empty rectangle.
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

    // A tint no unit uses any more has its field despawned.
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
                    // is the identity and not a placement. `render::shadows`
                    // does the same, for the same reason.
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
        // Nothing moved: the corners are the ones already in the mesh, so the
        // asset is left alone. This test is before `meshes.get_mut`, which
        // queues the upload whether or not anything is written.
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

/// A field's mesh before any name is in it. The attributes are declared so the
/// vertex layout is fixed from the first frame, and there are no triangles to
/// rasterise. `render::shadows` has the same `empty_field`, for the same reason.
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

/// The atlas rectangle and aspect ratio for one name, rasterised the first time
/// the name is seen.
///
/// The cache key is the string and not the guid, because a city holds many
/// units named `Stormwind Guard` and they all take the same picture. A `None`
/// is cached too: a string the face has no glyphs for costs one attempt, not
/// one per frame.
///
/// The pixels go into the one atlas ([`ATLAS_SIDE`]) and not into a texture per
/// name. That lets every name on screen share a material and therefore a mesh.
/// The module note gives the measured cost of a texture per name.
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
            // The atlas and its material are built on the first name and not at
            // startup: a session that never shows a name (every `UnitName*`
            // off) pays neither the 4 MB nor the upload.
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
            // The UVs are inset half a texel from each edge. The sampler is
            // linear and the atlas has a neighbour on every side, so UVs on the
            // exact boundary sample the adjacent name: the first letter of
            // another name shows faintly beside the last letter of this one. A
            // texture per name avoids this with `ClampToEdge`; a rectangle
            // inside an atlas has no edge to clamp to.
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
    /// A full atlas is reset and not repacked. Every rectangle is the same
    /// height and they arrive in first-sight order, so a shelf packer wastes
    /// almost nothing and repacking would recover little. A reset costs one
    /// frame with no names while they rasterise again, and it takes a few
    /// hundred distinct names in one session to reach it. Returns `None` only
    /// for a name too large for the atlas; [`MAX_TEXTURE`] bounds the raster
    /// before this is called.
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
/// The address mode is `ClampToEdge`, the one sampler setting that differs from
/// every other texture in this client: `Repeat` bleeds the far edge in under
/// linear filtering. The per-name inset in [`art_for`] prevents the same bleed
/// between two neighbours inside the atlas.
fn atlas_image() -> Image {
    let mut image = Image::new(
        Extent3d { width: ATLAS_SIDE, height: ATLAS_SIDE, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0u8; (ATLAS_SIDE * ATLAS_SIDE * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        // Both worlds, unlike every static texture here: the atlas is rewritten
        // whenever a name is added to it, and a `RENDER_WORLD` image has no
        // main-world copy to write into.
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

/// `Fonts\FRIZQT__.TTF`, read from the archives and parsed once.
///
/// This is the face `UNIT_NAME_FONT` names. See
/// [`vale_assets::look::unitname::FONT`], which documents the Lua global that
/// holds the path.
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

/// Rasterises one name's text as white glyphs over a black shadow, `RGBA8`.
///
/// The text is split at its line breaks and the lines are stacked, each
/// centred on the widest. The bitmap is one line's height for each line, so the caller
/// scales its quad by the line count.
///
/// The glyphs are white and the colour comes from the instance's `MeshTag` (see
/// [`params`]), so one texture serves a name whatever the unit's reaction. The
/// same guard name can be on a hundred units at once, of both factions.
///
/// The shadow is this client's own. The 1.12.1 client gives its head-mounted
/// strings a `0.001` shadow offset, in a space that this client draws in yards,
/// where a millimetre of shadow is not visible. This client offsets the shadow
/// by [`SHADOW_PX`] texels of the raster, which is a constant fraction of the
/// glyph at any distance. Without a shadow a white name over snow cannot be
/// read.
fn raster(font: &ab_glyph::FontVec, text: &str) -> Option<(Vec<u8>, u32, u32)> {
    use ab_glyph::{Font, ScaleFont};
    let scaled = font.as_scaled(ab_glyph::PxScale::from(RASTER_PX));
    // One row of glyphs per line of the text. Each glyph is kept with its
    // pen position in its own line and the line's index, and each line with
    // its width, so that the lines can be centred on the widest.
    let mut glyphs = Vec::new();
    let mut widths = Vec::new();
    for (row, line) in text.split('\n').enumerate() {
        let mut pen = 0.0f32;
        let mut previous = None;
        for c in line.chars() {
            let id = scaled.glyph_id(c);
            if let Some(last) = previous {
                pen += scaled.kern(last, id);
            }
            previous = Some(id);
            glyphs.push((id, pen, row));
            pen += scaled.h_advance(id);
        }
        widths.push(pen);
    }
    let widest = widths.iter().copied().fold(0.0f32, f32::max);
    let ascent = scaled.ascent();
    let line_height = (ascent - scaled.descent()).ceil();
    let pad = SHADOW_PX.ceil() as i32 + 1;
    let width = (widest.ceil() as i32 + pad * 2).max(1) as u32;
    let height = ((line_height * widths.len() as f32) as i32 + pad * 2).max(1) as u32;
    if width > MAX_TEXTURE || height > MAX_TEXTURE {
        return None;
    }

    // The coverage mask is built first and composed below. The shadow is the
    // same mask read at an offset, so the mask must be complete before either
    // is written.
    let mut mask = vec![0f32; (width * height) as usize];
    for (id, x, row) in glyphs {
        let centred = (widest - widths[row]) * 0.5;
        let placed = id.with_scale_and_position(
            RASTER_PX,
            ab_glyph::point(
                x + centred + pad as f32,
                ascent + line_height * row as f32 + pad as f32,
            ),
        );
        // A space, or a codepoint the face has no contours for, has no outline.
        // That is a gap in the word and not a failure.
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
            // The glyph over its own shadow, in one texel. The alpha is the
            // coverage of either; the colour is white only where the glyph is,
            // so a tinted label keeps a black outline.
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

/// The draw parameters every label shares.
///
/// Alpha blend and no depth write put the draw in the sorted transparent phase
/// with the particles. A name behind a tree is occluded by the tree's opaque
/// depth, and two names that overlap blend instead of one cutting a hole in the
/// other.
///
/// `tint` is set so that one material serves every colour. `DrawParams::tint`
/// does not carry a colour; it tells the shader to read the instance's
/// `MeshTag` as `0xAARRGGBB`. `render::selection` uses the same arrangement.
fn params() -> DrawParams {
    DrawParams {
        geoset: 0,
        texture: None,
        // 2 is alpha blend. Not 4 (add-alpha): add-alpha over a bright hillside
        // washes a white name out entirely.
        blend: 2,
        // A name is unlit. The 1.12.1 client draws it in its colour unchanged.
        unlit: true,
        // A screen-aligned quad is never edge-on and never back-facing, so this
        // is a precaution. It matters only on a frame where the basis is
        // degenerate.
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

/// The camera's own rotation. It makes the quad parallel to the near plane:
/// square to the view on every axis, at any pitch, and upright on the screen.
///
/// It is built from the eye and the focus and not read from the camera's
/// `GlobalTransform`, which is a frame stale in `Update` because propagation
/// has not run. The eye and the focus are the two points
/// [`crate::world::camera::place`] built this frame's view matrix from, read
/// through the rig's accessors, so the label's basis and the view's agree.
///
/// The quad's normal is `+Z` and `looking_at` points the camera's `-Z` at its
/// focus, so using the rotation unchanged turns the quad's face towards the
/// eye. The rotation is the same for every label on screen, which is the
/// definition of screen-aligned, so this function does not take the label's
/// position.
///
/// The input is degenerate only if the eye is exactly on the focus, which the
/// rig's minimum reach prevents. `looking_at` does nothing in that case and
/// does not produce a NaN; this function returns the identity.
fn face_the_camera(eye: Vec3, focus: Vec3) -> Quat {
    if (focus - eye).length_squared() < 1e-6 {
        return Quat::IDENTITY;
    }
    Transform::from_translation(eye)
        .looking_at(focus, Vec3::Y)
        .rotation
}

/// The colour of the name.
///
/// This is not measured from the 1.12.1 client: where that client takes a
/// name's colour from has not been established. This client uses the selection
/// ring's selector, `GetSelectionCircleColor`, in its place.
///
/// The shipped `GameTooltip.lua` supports the substitution:
/// `GameTooltip_UnitColor` asks the same three questions in the same order
/// about the same unit, and colours the tooltip's name with the answer. A
/// screenshot shows a hostile creature named red and a neutral one yellow,
/// which matches the selector. The ring table's orange fourth rank is not
/// produced, for the reason `look::selection` gives: `Reaction` has three
/// values here and an unfriendly creature reads as hostile.
fn colour(
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    friendship: crate::interface::api::Friendship<'_>,
    me: Option<&WorldEntity>,
    unit: &WorldEntity,
) -> [f32; 4] {
    use vale_assets::look::selection::{ring_colour, Selected};
    use vale_assets::tables::faction::Reaction;
    // Without the tables or the local player the reaction is Neutral, as in
    // `render::selection`. A yellow name over a wolf in that case is a
    // plausible wrong answer and not a measured one.
    //
    // The reaction is the unit's towards the local player; see
    // [`Selected::reaction`]. This function and the ring once asked it in the
    // other direction. The two directions differ only where the character's own
    // state adds a rule that applies in one direction and not the other.
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

/// [`crate::ui::report::HudReport`] slot 33, directly under the portraits' at
/// 32 and the interface's at 31. A name is one more quad drawn per unit, so its
/// count is listed beside theirs.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(33);

/// Reports how many name quads are in the world, how many draws they take, and
/// how many distinct strings are in the atlas. The last number grows with the
/// variety of names and not with the population: thirty guards with one name
/// are thirty quads and one atlas entry, and thirty differently named creatures
/// are thirty of each.
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

/// The height a name is rasterised at, in texels.
///
/// A name is rasterised once at this height, whatever its distance. It is large
/// enough that a name filling a quarter of a 1080-tall screen is still
/// minified and not magnified, and small enough that a city's distinct names
/// take a few megabytes.
const RASTER_PX: f32 = 48.0;

/// The drop shadow's offset, in texels of the raster.
const SHADOW_PX: f32 = 3.0;

/// A name wider or taller than this many texels is refused and not rasterised.
///
/// Nothing the server sends comes near it: a 1.12 player name is at most twelve
/// characters and a creature's is somewhat longer. The string is server data,
/// so the allocation made from it is bounded.
///
/// It is larger than [`ATLAS_SIDE`] on purpose, and the two tests are
/// different. This one bounds the allocation the raster makes;
/// [`Labels::reserve`] refuses again at the atlas's own width. A check only at
/// the atlas's 1024 would run after the raster had already built its bitmap,
/// for example a 2000-texel one that is then discarded.
const MAX_TEXTURE: u32 = 2048;

#[cfg(test)]
mod tests {
    use super::*;

    /// The quad is square to the view at any pitch. Its normal is its local
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

    /// The quad is upright on the screen and not in the world. With the camera
    /// looking straight down, a quad that is upright in the world is edge-on
    /// and unreadable. The quad's local up must stay perpendicular to the view
    /// and keep its length.
    #[test]
    fn looking_straight_down_does_not_lay_the_name_flat() {
        let focus = Vec3::ZERO;
        let eye = Vec3::new(0.0, 20.0, 0.001);
        let q = face_the_camera(eye, focus);
        let up = q * Vec3::Y;
        let to_eye = (eye - focus).normalize();
        // Perpendicular to the view. A foreshortened quad is one whose up has
        // rotated towards the eye.
        assert!(up.dot(to_eye).abs() < 1e-3, "the name foreshortened: {up:?}");
        assert!((up.length() - 1.0).abs() < 1e-4);
    }

    /// The shelf packer places names left to right and starts a new row when
    /// one does not fit, and every rectangle it returns is inside the atlas.
    ///
    /// A rectangle even one texel outside the atlas would sample another name's
    /// pixels.
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

    /// A full atlas is reset and does not overflow. See [`Labels::reserve`].
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

    /// A degenerate rig, with the eye exactly on the point it is looking at,
    /// gives the identity and not a NaN. A NaN in a `Transform` draws nothing
    /// for the frame.
    #[test]
    fn a_degenerate_camera_is_the_identity_rather_than_a_nan() {
        let q = face_the_camera(Vec3::splat(3.0), Vec3::splat(3.0));
        assert_eq!(q, Quat::IDENTITY);
        assert!(q.to_array().iter().all(|v| v.is_finite()));
    }
}
