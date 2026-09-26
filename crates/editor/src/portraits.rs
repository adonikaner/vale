//! A picture of any model the archives name, small enough for a row of a list.
//!
//! ## A name is not a model
//!
//! The placement picker offers 5,816 `.m2` and 816 `.wmo` roots by path and
//! nothing else, and the spell workspace's model browser offers every `.m2`
//! there is the same way. `ElwynnTreeCanopy04` against `ElwynnTreeCanopy05` is
//! not a choice anybody can make by reading. The tileset picker had the same
//! problem one door along and solved it by decoding a BLP; a model has to be
//! rendered, which is `vale portrait`'s question — where to stand to take a
//! model's picture — pointed at an offscreen target and cached the way
//! `ui::thumbnails` caches a texture.
//!
//! ## It is the client's own batches, drawn by a camera of this crate's own
//!
//! `ModelCache::lookup` and `WmoCache::lookup` hand back the same meshes and
//! materials the world is drawn from — the placing tool's ghost is spawned
//! from them — so a picture here is of the file as the client draws it. What
//! this crate adds is the rig: [`SLOTS`] cameras, each alone on a render layer
//! of its own with a lamp of its own, each drawing one model at a time into a
//! [`SIDE`]-square image that egui then samples like any other texture. The
//! client's own portrait pass (`render::portraits`) is the same shape for a
//! unit's face; it keys by unit token and dresses a body, neither of which a
//! path has, so this is its own rig rather than a seam into that one.
//!
//! ## The framing is the model's box, not its camera
//!
//! A creature's portrait camera aims at its head; a tree, a crate and a city
//! have no head and no camera. So the eye stands on a fixed three-quarter
//! line from the model's centre, far enough back that its bounding sphere
//! fills the frame. The box is the union of the batches' own meshes, in the
//! axes they are stored in — the same box the placing tool measures a new
//! building's `MODF` extent from.
//!
//! ## One model per slot per frame, for a settle
//!
//! A picture is a still, and rendering it once would be enough if the render
//! world drew what it was handed on the frame it was handed it. It does not:
//! a pipeline specialised for a new view or a new material compiles on a
//! task and the draw is skipped until it lands, and a mesh or a texture not
//! yet resident uploads over the first frames. So each job renders for
//! [`SETTLE_FRAMES`] and the first of the session for [`FIRST_SETTLE_FRAMES`],
//! which is when every pipeline this rig needs is compiled. The client's
//! portrait pass settles for the same reason and its number is the source of
//! the second constant.
//!
//! **A camera that is not drawing is switched off**, because an active
//! `Camera3d` is a whole `Core3d` graph run per frame whether or not anything
//! is in front of it — measured in the client at ~1.5 ms of CPU encode each.
//! With nothing asked for, this costs a `HashMap` lookup a frame.
//!
//! ## What is asked for is what is on screen
//!
//! A row asks on every frame it is drawn, and the queue keeps the *newest*
//! ask first: a fast scroll through a page leaves behind rows nobody stopped
//! on, and a queue served oldest-first would spend the next second drawing
//! them while the rows under the pointer stayed blank. An ask not repeated
//! for [`STALE_FRAMES`] is dropped unserved.
//!
//! A path the caches will not open is remembered as failed, so a broken row
//! is one lookup and not one a frame — the thumbnail cache's own rule.
//!
//! ## A creature is a model *and its skins*, and a path is not enough
//!
//! A doodad carries its own textures and a creature does not: measured over
//! the whole table, 10,388 of `CreatureDisplayInfo`'s 10,534 rows take their
//! body texture from the client — 3,555 from a texture variation, 6,833 from a
//! baked NPC skin — so a creature drawn by path alone has an empty body slot
//! and comes out magenta. [`Worn`] is the other half: the path, the skins, the
//! hair and the geosets, which together are what one display id looks like.
//!
//! It is the same rig either way. What changes is the call into the cache —
//! `ModelCache::worn_unposed` instead of `lookup` — and the **key**, which has
//! to carry the skins: two display ids of one wolf model are two pictures, and
//! keying by path would give the second one the first one's coat.
//!
//! **In its bind pose**, which is the one thing this is not able to fix. A
//! picture wants no skeleton, and a mesh carrying joints with nothing to bind
//! them to is a mesh the renderer will not draw — so what a row shows is the
//! model as it was authored, which for a character-model NPC is arms out. The
//! posed, animated answer is a `WorldEntity` through the client's own pipeline
//! and it is what the placement ghost is; a picker cannot have fifty of those.
//!
//! ## …and one big picture, under the selector, that can be turned
//!
//! A row's picture is an *icon*: it says which of these is the oak, and it is
//! too small to say whether this oak is the one. So a picker also has **one**
//! larger preview under its list, of whichever model is chosen, and dragging
//! it turns the model round.
//!
//! It is one slot and not a size on the cache, which is the whole of why it
//! is written apart from the rows above. A second size per path would double
//! the images kept for the session — a 256-pixel square is four times a
//! row's — to answer a question about exactly one model at a time. So there
//! is one camera, one image reused for the life of the resource, and the
//! model standing in front of it is swapped when the choice changes. Nothing
//! is cached: coming back to a model draws it again, which costs the settle
//! and no memory.
//!
//! The turn is the *camera's* orbit and not the model's rotation, so the lamp
//! stays where it is and the model is lit the same from every side. At rest
//! it is exactly [`EYE`], which is what the rows use, so the big picture and
//! the row above it are the same view until somebody drags.
//!
//! ## What a picture here is not: the emitters
//!
//! Every picture in this file is the model's **meshes**. A model whose visible
//! substance is particles — which is most spell effects — draws as whatever
//! static geometry it happens to carry, which for `Bloodlust_State_Hand.m2` is
//! a dim smudge where the game shows a ring of fire.
//!
//! That is a limit rather than an omission, and `crate::stage`'s own module
//! comment is where the reason is written down: the client's particle pass
//! billboards every quad against **the** world camera and takes its emission
//! LOD from that camera's distance, so a second view running particles gets
//! them edge-on. The stage solves it by parking the world camera on itself,
//! and two views cannot both do that. So the pickers show geometry, and the
//! model view — `crate::lab`, which is the stage — is where an effect is
//! judged with its particles running. The browser's dialog stands over that
//! stage on purpose, which is what its lighter backdrop is for.
//!
//! **And a still of an effect is its first instant, which is often nothing.**
//! `vale model 'Spells\Firebolt_ImpactDD_Med_Chest.m2'` is the worked
//! example: one batch of two triangles, six emitters, and a transparency
//! track running `0.000..1.000` over 3.3 seconds. Drawn at time zero — which
//! is what every still in this client is, the client's own portraits
//! included — the one quad it has is transparent. An empty pane for a model
//! like that is the right answer rather than a fault, and it was worth two
//! runs to establish that rather than assume it.

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::MeshAabb;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{ClearColorConfig, ImageRenderTarget, RenderTarget};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy_egui::{egui, EguiTextureHandle, EguiUserTextures};

use vale_assets::look::character::Appearance;
use vale_assets::look::dress::CharacterLook;
use vale_assets::world::m2::Dress;
use vale_client::render::models::{Lookup, Materials, ModelCache};
use vale_client::render::wmos::{self as wmo_render, WmoCache};

/// How many pixels a picture is on a side.
///
/// 128, drawn at 40 to 56 points, so the sampler is always downscaling —
/// the side of 1:1 to be on, as the client's own portrait pass says.
pub const SIDE: u32 = 128;

/// …and how many the one big preview is, which is the size it is drawn at
/// times two, so a panel dragged wider does not show a soft picture.
pub const LARGE_SIDE: u32 = 384;

/// How many models are drawn at once. Each is a camera and a render layer.
const SLOTS: usize = 4;

/// The first render layer the slots use; slot `i` is on `FIRST_LAYER + i`.
///
/// Layer 0 is the world's, 1..=9 the client's portraits, 10..=14 its paper
/// dolls, and 2 is also the stage's. Well above all of them, stated in
/// terms of them, so a collision is two subjects in one picture nowhere.
const FIRST_LAYER: usize = 24;

/// The camera order the slots draw at, before the stage's `-1`, the client's
/// portraits and dolls (`-2..=-15`) and the window's `0`. A target sampled
/// before it was drawn into is a picture one frame stale.
const FIRST_ORDER: isize = -40;

/// How many frames a job renders before its picture is taken as final.
const SETTLE_FRAMES: u8 = 10;

/// …and how many the first job of the session renders — see the module
/// comment. The client's portrait pass settles for 30 and a pipeline for a
/// view no camera has drawn before takes longer than that on this machine.
const FIRST_SETTLE_FRAMES: u8 = 60;

/// How many frames an ask is kept without being repeated.
const STALE_FRAMES: u64 = 30;

/// How many frames the big preview renders when it is shown a new model.
///
/// Longer than a row's, and it is one camera that is only live while a picker
/// is open, so 0.4 s of it costs nothing anybody can see. A row that came out
/// half-drawn is one wrong icon in a list; the pane is the picture somebody is
/// looking at, and a building whose batches had not all uploaded when the
/// shutter closed stays half a building until they choose something else.
const LARGE_SETTLE_FRAMES: u8 = 24;

/// How many frames the big preview renders after it has been turned.
///
/// Two. Nothing is allocated or uploaded for a turn — the model is already
/// resident and only the camera moved — so it needs the frame that draws it
/// and one of margin. The client's paper dolls use the same number for the
/// same gesture, and the argument is theirs: paying a full settle per degree
/// of a held drag would keep the camera live for the whole of it.
const TURN_FRAMES: u8 = 2;

/// Radians the big preview turns per point of drag.
const TURN_RATE: f32 = 0.01;

/// How far above and below the horizon the big preview can be tilted.
const PITCH_LIMIT: f32 = 1.35;

/// The vertical field of view a picture is taken at, in radians. About 40°:
/// narrow enough that a box-fit sphere fills the frame, wide enough that the
/// near corner of a building is not clipped.
const FOV: f32 = 0.70;

/// How far above the world's focus the model stands, in yards.
///
/// The picture is isolated by the *layer* and not by distance — no other
/// camera sees this one's layer whatever is standing in it — so the model
/// could stand anywhere; it stands here so that a directional light's shadow
/// cascade, which the world's casters do reach, finds nothing of the world at
/// this height. See `crate::stage::stage_at`, which is where the stage stands
/// and why.
const ABOVE: f32 = 80.0;

/// The line the eye stands on from the model's centre, in the meshes' own
/// axes (`+Y` up, `-Z` the model's forward, `-X` its left): front, left and
/// above, a three-quarter view.
const EYE: Vec3 = Vec3::new(-0.55, 0.45, -0.70);

/// …and where the lamp shines from, in the same axes: above and a little to
/// the camera's side, so the lit face is the face in the picture.
const LAMP: Vec3 = Vec3::new(-0.40, 0.80, -0.50);

/// Marks a slot's camera.
#[derive(Component)]
pub struct PortraitCamera;

/// What one slot is doing.
struct Slot {
    camera: Entity,
    layer: usize,
    job: Option<Job>,
}

/// One picture being taken.
struct Job {
    path: String,
    /// The model's root once it is standing, and `None` while its batches
    /// are still being asked for.
    root: Option<Entity>,
    image: Option<(Handle<Image>, egui::TextureId)>,
    frames_left: u8,
}

/// The one big preview's camera and the image it always draws into.
///
/// Built once and kept: see the module comment on why the big picture is a
/// slot rather than a second size in the cache.
struct LargeRig {
    camera: Entity,
    layer: usize,
    /// **Held so the image lives**, which is the whole of what this field is
    /// for: egui knows it by an id, and an id naming an asset nobody holds
    /// draws as whatever the atlas has there now.
    #[allow(dead_code)]
    image: Handle<Image>,
    texture: egui::TextureId,
}

/// …and which model is standing in front of it.
struct LargeShown {
    path: String,
    /// The model's root once it is standing, `None` while the caches are
    /// still being asked.
    root: Option<Entity>,
    /// Its box, kept so a turn can re-aim without asking the cache again.
    bounds: Option<(Vec3, Vec3)>,
    /// **Where it was stood**, which is not the same as where a model stood
    /// now would be: the place is the editor camera's focus plus a height,
    /// and that focus eases for seconds after a launch and moves whenever
    /// anybody flies. A re-aim against the *current* place pointed the camera
    /// tens of yards off the model that is actually standing there, which is
    /// a building drawn small and low in the pane while the same building in
    /// the row above it — aimed once, at spawn — filled its square.
    at: Vec3,
    frames_left: u8,
    /// Whether the picture has been drawn at least once. Until it has, the
    /// image still holds the *previous* model, so the panel must be told to
    /// draw nothing rather than the model before this one.
    settled: bool,
}

/// **A model and what it is wearing**, which is what a display id resolves to
/// and what a picture of a creature needs.
///
/// `skins` is `DisplayModel::skins` — slot 0 the body, 1 and 2 the second and
/// third creature variations — and `hair` and `dress` are what
/// `vale_assets::look::dress::dress` answers for the same display row: the
/// one texture no bake holds, and which geosets to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Worn {
    pub path: String,
    pub skins: Vec<String>,
    pub hair: Option<Appearance>,
    pub dress: Dress,
    /// **Set where the body's skin is *composed* rather than read**, which is
    /// the one picture here whose subject is a player: a garment that paints
    /// the wearer and has no geometry of its own, shown on a body.
    ///
    /// `None` for every creature and every doodad, whose skins are files the
    /// tables name. `Some` puts the lookup through
    /// `ModelCache::worn_as_character_unposed`, where [`Self::skins`] is
    /// ignored — the composite's key is built from this instead.
    pub look: Option<CharacterLook>,
    /// …and the texture a cloak lends the wearer's own group-15 geoset, which
    /// is the one piece of equipment that is the body's geometry and the
    /// item's skin. Only ever set beside [`Self::look`].
    pub cloak: Option<String>,
}

impl Worn {
    /// **The key this picture is kept under.**
    ///
    /// The path with the skins after it, because two display ids of one model
    /// are two pictures. A model wearing nothing keys as its path alone, so a
    /// doodad's picture is the same entry it always was.
    pub fn key(&self) -> String {
        // **A composed body keys by the look**, because its skins are empty:
        // two garments on one race are one model and one pair of empty skin
        // slots, so keying by those would give the second one the first one's
        // picture. The equipment is in it for the same reason it is in the
        // client's own composite key.
        if let Some(look) = &self.look {
            let a = &look.appearance;
            let mut key = format!(
                "{}|char:{},{},{},{},{},{},{}",
                key_of(&self.path),
                a.race,
                a.gender,
                a.skin,
                a.face,
                a.hair_style,
                a.hair_colour,
                a.facial_hair
            );
            for (display_id, inventory_type) in &look.equipment {
                key.push_str(&format!(";{display_id}.{inventory_type}"));
            }
            if let Some(cloak) = &self.cloak {
                key.push_str(&format!("|{cloak}"));
            }
            return key;
        }
        match self.skins.iter().all(|skin| skin.is_empty()) {
            true => key_of(&self.path),
            false => format!("{}|{}", key_of(&self.path), self.skins.join("|")),
        }
    }
}

/// **Ask the cache for one [`Worn`]**, whichever of the two kinds it is.
///
/// The branch is the whole of what the two kinds differ by at this level: a
/// creature's body is a file the display tables name and a player's is
/// composed from an appearance and a wardrobe. See `ModelCache`, where both
/// doors are, and [`Worn::look`].
fn dressed(
    worn: &Worn,
    models: &mut ModelCache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Materials,
) -> Lookup {
    match &worn.look {
        Some(look) => models.worn_as_character_unposed(
            &worn.path,
            look,
            worn.dress,
            worn.cloak.as_deref(),
            meshes,
            materials,
        ),
        None => models.worn_unposed(
            &worn.path,
            &worn.skins,
            worn.hair.as_ref(),
            worn.dress,
            meshes,
            materials,
        ),
    }
}

/// The pictures, and what has been asked for.
#[derive(Resource, Default)]
pub struct Portraits {
    ready: HashMap<String, egui::TextureId>,
    /// Asked for and not yet taken, with the frame it was last asked on.
    wanted: HashMap<String, u64>,
    /// What could not be drawn, so it is not asked for again.
    failed: HashSet<String>,
    slots: Vec<Slot>,
    frame: u64,
    /// How many pictures have been started, for the first job's long settle.
    started: usize,
    /// The big preview: its rig, what is standing in it, and what the panel
    /// asked for this frame — see the module comment.
    large: Option<LargeRig>,
    shown: Option<LargeShown>,
    large_wanted: Option<String>,
    /// **Whether this rig has ever drawn anything**, for the long first
    /// settle. Its own flag and deliberately not [`Self::started`], which the
    /// slots bump: the big preview is a second camera at a second size, so a
    /// slot having warmed the pipelines says nothing about this one — and
    /// reading the slots' counter gave the pane ten frames on the frame a
    /// slot took sixty, which photographed a building with half its batches
    /// uploaded.
    large_started: bool,
    /// The orbit the drag has put on it, as offsets from [`EYE`]'s own.
    large_yaw: f32,
    large_pitch: f32,
    /// **What a key that is not a plain path means** — see [`Worn`].
    ///
    /// A side table rather than a field on the job, because every entry point
    /// here takes a key and only the two that build one know about skins.
    /// Never evicted: it is a path and three small values per creature the
    /// picker has shown.
    worn: HashMap<String, Worn>,
}

/// What a picker's row can say about a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready(egui::TextureId),
    /// Asked for, or being drawn.
    Pending,
    /// The caches will not open it.
    Failed,
}

impl Portraits {
    /// Ask for a picture. Cheap and idempotent: a row calls this on every
    /// frame it is drawn, and that is what keeps its ask fresh.
    pub fn want(&mut self, path: &str) {
        let key = key_of(path);
        if self.ready.contains_key(&key) || self.failed.contains(&key) {
            return;
        }
        let frame = self.frame;
        self.wanted.insert(key, frame);
    }

    /// **Ask for a picture of a model wearing supplied skins**, and answer the
    /// key it is kept under — which is what every other call here then takes.
    ///
    /// Cheap and idempotent on [`Self::want`]'s own terms: a row calls it on
    /// every frame it is drawn.
    pub fn want_worn(&mut self, worn: &Worn) -> String {
        let key = worn.key();
        self.worn.entry(key.clone()).or_insert_with(|| worn.clone());
        self.want(&key);
        key
    }

    /// …and the same for the one big preview.
    pub fn want_large_worn(&mut self, worn: &Worn) -> String {
        let key = worn.key();
        self.worn.entry(key.clone()).or_insert_with(|| worn.clone());
        self.want_large(&key);
        key
    }

    /// The picture, if it has been taken.
    pub fn get(&self, path: &str) -> Option<egui::TextureId> {
        self.ready.get(&key_of(path)).copied()
    }

    /// …and what a row should say when it has not.
    pub fn status(&self, path: &str) -> Status {
        let key = key_of(path);
        match self.ready.get(&key) {
            Some(id) => Status::Ready(*id),
            None if self.failed.contains(&key) => Status::Failed,
            None => Status::Pending,
        }
    }

    /// How many pictures are being kept.
    pub fn count(&self) -> usize {
        self.ready.len()
    }

    /// **Ask for the one big preview to be of this model.** Called by the
    /// panel on every frame it draws the pane, exactly as [`Self::want`] is.
    ///
    /// A different model than last frame puts the turn back to where it
    /// started: a drag is about the model it was made on, and carrying it to
    /// the next one shows that one from behind for no reason anybody asked.
    pub fn want_large(&mut self, path: &str) {
        let key = key_of(path);
        let changed = self.shown.as_ref().is_none_or(|shown| shown.path != key)
            && self.large_wanted.as_deref() != Some(key.as_str());
        if changed {
            self.large_yaw = 0.0;
            self.large_pitch = 0.0;
        }
        self.large_wanted = Some(key);
    }

    /// What egui draws the big preview by, once it has been drawn at least
    /// once and is of the model asked for.
    pub fn large_texture(&self, path: &str) -> Option<egui::TextureId> {
        let shown = self.shown.as_ref()?;
        if shown.path != key_of(path) || !shown.settled {
            return None;
        }
        Some(self.large.as_ref()?.texture)
    }

    /// Turn the big preview by a drag, in points.
    pub fn turn_large(&mut self, by: egui::Vec2) {
        if by == egui::Vec2::ZERO {
            return;
        }
        self.large_yaw -= by.x * TURN_RATE;
        self.large_pitch = (self.large_pitch + by.y * TURN_RATE).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        // Re-aimed and re-rendered on the next run of the pass.
        if let Some(shown) = self.shown.as_mut() {
            shown.frames_left = shown.frames_left.max(TURN_FRAMES);
        }
    }

    /// Whether the big preview has been turned off its resting view.
    pub fn large_turned(&self) -> bool {
        self.large_yaw != 0.0 || self.large_pitch != 0.0
    }

    /// Put it back to the view the rows are drawn from.
    pub fn reset_large_turn(&mut self) {
        self.large_yaw = 0.0;
        self.large_pitch = 0.0;
        if let Some(shown) = self.shown.as_mut() {
            shown.frames_left = shown.frames_left.max(TURN_FRAMES);
        }
    }

    /// The next path to draw: the newest ask not already in a slot.
    fn next_wanted(&self) -> Option<String> {
        let busy = |path: &str| {
            self.slots
                .iter()
                .any(|slot| slot.job.as_ref().is_some_and(|job| job.path == path))
        };
        self.wanted
            .iter()
            .filter(|(path, _)| !busy(path))
            .max_by_key(|(path, asked)| (**asked, std::cmp::Reverse(path.as_str())))
            .map(|(path, _)| path.clone())
    }
}

/// The key a path is kept under: lower case, and the `.mdx` the tiles spell
/// swapped for the `.m2` the archives hold, which is the placing tool's own
/// rule at its own door.
fn key_of(path: &str) -> String {
    // **A dressed key is already a key.** It is a path with its skins after it
    // — see [`Worn::key`] — and every entry point here takes either, so
    // re-keying one would put `.m2` on the end of a `.blp`. The separator
    // cannot occur in an archive path.
    if path.contains('|') {
        return path.to_string();
    }
    let lower = path.to_ascii_lowercase();
    match lower.ends_with(".wmo") {
        true => lower,
        false => vale_assets::world::m2::model_path(&lower),
    }
}

pub struct PortraitPlugin;

impl Plugin for PortraitPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Portraits>()
            .add_systems(Update, take_pictures);
    }
}

/// Build the rig on first use, then keep every slot busy while anything is
/// wanted.
#[allow(clippy::too_many_arguments)]
fn take_pictures(
    mut commands: Commands,
    mut portraits: ResMut<Portraits>,
    mut models: ResMut<ModelCache>,
    mut buildings: ResMut<WmoCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: Materials,
    mut images: ResMut<Assets<Image>>,
    mut egui: ResMut<EguiUserTextures>,
    camera: Res<crate::camera::EditorCamera>,
    mut cameras: Query<(&mut Camera, &mut Transform, &mut Projection), With<PortraitCamera>>,
) {
    portraits.frame += 1;
    let frame = portraits.frame;
    portraits
        .wanted
        .retain(|_, asked| frame.saturating_sub(*asked) <= STALE_FRAMES);
    let idle = portraits.wanted.is_empty()
        && portraits.slots.iter().all(|slot| slot.job.is_none())
        && portraits.large_wanted.is_none()
        && portraits.shown.is_none();
    if idle {
        return;
    }
    if portraits.slots.is_empty() {
        for i in 0..SLOTS {
            let layer = FIRST_LAYER + i;
            let camera = spawn_rig(
                &mut commands,
                layer,
                FIRST_ORDER - i as isize,
                ClearColorConfig::Custom(Color::NONE),
            );
            portraits.slots.push(Slot {
                camera,
                layer,
                job: None,
            });
        }
        // …and the big preview's, on the layer after the slots'. Its image is
        // made once and reused for every model it shows — see the module
        // comment.
        let layer = FIRST_LAYER + SLOTS;
        let camera = spawn_rig(
            &mut commands,
            layer,
            FIRST_ORDER - SLOTS as isize,
            ClearColorConfig::Custom(LARGE_CLEAR),
        );
        let image = images.add(target_image(LARGE_SIDE));
        let texture = egui.add_image(EguiTextureHandle::Strong(image.clone()));
        commands
            .entity(camera)
            .insert(RenderTarget::Image(ImageRenderTarget {
                handle: image.clone(),
                scale_factor: 1.0,
            }));
        portraits.large = Some(LargeRig {
            camera,
            layer,
            image,
            texture,
        });
        // The cameras exist from the next frame; nothing to point them at yet.
        return;
    }

    let stand = crate::stage::stage_at(&camera) + Vec3::Y * ABOVE;
    for at in 0..portraits.slots.len() {
        // --- a finished picture comes down, and a free slot takes the next ---
        let finished = portraits.slots[at]
            .job
            .as_ref()
            .is_some_and(|job| job.root.is_some() && job.frames_left == 0);
        if finished {
            let job = portraits.slots[at].job.take().expect("checked above");
            if let Some(root) = job.root {
                commands.entity(root).despawn();
            }
            if let Some((_, id)) = job.image {
                portraits.ready.insert(job.path.clone(), id);
            }
            portraits.wanted.remove(&job.path);
            if let Ok((mut cam, _, _)) = cameras.get_mut(portraits.slots[at].camera) {
                cam.is_active = false;
            }
        }
        if portraits.slots[at].job.is_none() {
            let Some(path) = portraits.next_wanted() else {
                continue;
            };
            portraits.slots[at].job = Some(Job {
                path,
                root: None,
                image: None,
                frames_left: 0,
            });
        }

        // --- a job that is standing renders down its settle ---
        let layer = portraits.slots[at].layer;
        let camera_entity = portraits.slots[at].camera;
        let settle = match portraits.started {
            0 => FIRST_SETTLE_FRAMES,
            _ => SETTLE_FRAMES,
        };
        {
            let job = portraits.slots[at].job.as_mut().expect("filled above");
            if job.root.is_some() {
                job.frames_left = job.frames_left.saturating_sub(1);
                continue;
            }
        }
        // **The key, out of the borrow.** What the cache is asked depends on
        // whether this key names a dressing, which is another field of the
        // same resource the job is borrowed from.
        let key = portraits.slots[at]
            .job
            .as_ref()
            .expect("filled above")
            .path
            .clone();

        // --- …and one that is not asks the cache, and stands when it answers ---
        let draws = match key.ends_with(".wmo") {
            true => match buildings.lookup(&key) {
                wmo_render::Lookup::Ready(ready) => Some(ready.draws().to_vec()),
                wmo_render::Lookup::Failed => None,
                _ => continue,
            },
            // **A creature is looked up wearing its skins**, which is the
            // whole of what [`Worn`] is for — see the module comment, and
            // `ModelCache::worn_unposed`, which is the corner of that cache
            // this needs.
            false => match portraits.worn.get(&key).cloned() {
                Some(worn) => match dressed(&worn, &mut models, &mut meshes, &mut materials) {
                    Lookup::Ready(assets) => Some(assets.draws.clone()),
                    Lookup::Failed => None,
                    Lookup::Loading => continue,
                },
                None => match models.lookup(&key) {
                    Lookup::Ready(assets) => Some(assets.draws.clone()),
                    Lookup::Failed => None,
                    Lookup::Loading => continue,
                },
            },
        };
        let bounds = draws.as_deref().and_then(|draws| bounds_of(draws, &meshes));
        let (Some(draws), Some((lo, hi))) = (draws, bounds) else {
            portraits.failed.insert(key.clone());
            portraits.wanted.remove(&key);
            portraits.slots[at].job = None;
            continue;
        };

        let root = stand_model(&mut commands, &draws, layer, stand);
        let image = images.add(target_image(SIDE));
        let texture = egui.add_image(EguiTextureHandle::Strong(image.clone()));
        aim_at(&mut cameras, camera_entity, stand, lo, hi, 0.0, 0.0);
        // **`RenderTarget` is a component and not a field of `Camera`** in this
        // Bevy — the stage's `follow_the_pane` makes the same note.
        commands
            .entity(camera_entity)
            .insert(RenderTarget::Image(ImageRenderTarget {
                handle: image.clone(),
                scale_factor: 1.0,
            }));
        let job = portraits.slots[at].job.as_mut().expect("filled above");
        job.root = Some(root);
        job.image = Some((image, texture));
        job.frames_left = settle;
        portraits.started += 1;
    }

    keep_the_large(
        &mut commands,
        &mut portraits,
        &mut models,
        &mut buildings,
        &mut meshes,
        &mut materials,
        &mut cameras,
        stand,
    );
}

/// Keep the one big preview showing whatever the panel last asked for.
///
/// Its shape is a slot's, with two differences the module comment argues:
/// nothing is cached, so a model that goes off screen is taken down and drawn
/// again if it comes back; and the camera is re-aimed while it is being
/// turned, which is what the [`TURN_FRAMES`] budget is for.
#[allow(clippy::too_many_arguments)]
fn keep_the_large(
    commands: &mut Commands,
    portraits: &mut Portraits,
    models: &mut ModelCache,
    buildings: &mut WmoCache,
    meshes: &mut Assets<Mesh>,
    materials: &mut Materials,
    cameras: &mut Query<(&mut Camera, &mut Transform, &mut Projection), With<PortraitCamera>>,
    stand: Vec3,
) {
    let Some(rig) = portraits.large.as_ref() else {
        return;
    };
    let (camera_entity, layer) = (rig.camera, rig.layer);
    // **Taken, not read**: the panel asks again on every frame it draws the
    // pane, so a frame with no ask is a pane that is not on screen.
    let wanted = portraits.large_wanted.take();

    // --- nothing wants it, or it wants something else: take down what stands ---
    let keep = portraits
        .shown
        .as_ref()
        .zip(wanted.as_ref())
        .is_some_and(|(shown, wanted)| shown.path == *wanted);
    if !keep {
        if let Some(shown) = portraits.shown.take() {
            if let Some(root) = shown.root {
                commands.entity(root).despawn();
            }
        }
        if let Ok((mut cam, _, _)) = cameras.get_mut(camera_entity) {
            cam.is_active = false;
        }
        let Some(path) = wanted else { return };
        portraits.shown = Some(LargeShown {
            path,
            root: None,
            bounds: None,
            at: stand,
            frames_left: 0,
            settled: false,
        });
    }
    let (yaw, pitch) = (portraits.large_yaw, portraits.large_pitch);
    // **The first picture through this rig renders for a long settle**, and
    // that is not tidiness: the budget is a guess at how long the render world
    // takes to compile a pipeline for a view nothing has drawn before, and a
    // guess that is short leaves the image wrong *for ever*, because the
    // camera goes off at zero and never comes back. It was short twice — an
    // empty pane in the browser's dialog, and half a building in the placement
    // picker — and both times because it was keyed on a counter belonging to
    // the rows. See [`Portraits::large_started`].
    let settle = match portraits.large_started {
        false => FIRST_SETTLE_FRAMES,
        true => LARGE_SETTLE_FRAMES,
    };
    let Some(shown) = portraits.shown.as_mut() else {
        return;
    };

    // --- standing already: render down the budget, re-aiming as it turns ---
    if shown.root.is_some() {
        if shown.frames_left == 0 {
            if let Ok((mut cam, _, _)) = cameras.get_mut(camera_entity) {
                if cam.is_active {
                    cam.is_active = false;
                }
            }
            return;
        }
        shown.frames_left -= 1;
        if shown.frames_left == 0 {
            shown.settled = true;
        }
        if let Some((lo, hi)) = shown.bounds {
            // **Where it was stood, not where one would stand now** — see
            // [`LargeShown::at`].
            aim_at(cameras, camera_entity, shown.at, lo, hi, yaw, pitch);
        }
        return;
    }

    // --- …and one that is not asks the caches, exactly as a slot does ---
    let draws = match shown.path.ends_with(".wmo") {
        true => match buildings.lookup(&shown.path) {
            wmo_render::Lookup::Ready(ready) => Some(ready.draws().to_vec()),
            wmo_render::Lookup::Failed => None,
            _ => return,
        },
        // …and a creature wearing its skins, exactly as a row's is — see
        // [`Worn`].
        false => match portraits.worn.get(&shown.path).cloned() {
            Some(worn) => match dressed(&worn, models, meshes, materials) {
                Lookup::Ready(assets) => Some(assets.draws.clone()),
                Lookup::Failed => None,
                Lookup::Loading => return,
            },
            None => match models.lookup(&shown.path) {
                Lookup::Ready(assets) => Some(assets.draws.clone()),
                Lookup::Failed => None,
                Lookup::Loading => return,
            },
        },
    };
    let bounds = draws.as_deref().and_then(|draws| bounds_of(draws, meshes));
    let (Some(draws), Some((lo, hi))) = (draws, bounds) else {
        // The rows' own failure set, so the cross the row draws and the one
        // the pane draws are the same answer.
        let path = shown.path.clone();
        portraits.shown = None;
        portraits.failed.insert(path);
        return;
    };
    // Stood where the pane was opened, and aimed there for as long as it is
    // up — see [`LargeShown::at`].
    let at = shown.at;
    let root = stand_model(commands, &draws, layer, at);
    aim_at(cameras, camera_entity, at, lo, hi, yaw, pitch);
    shown.root = Some(root);
    shown.bounds = Some((lo, hi));
    shown.frames_left = settle;
    portraits.large_started = true;
}

/// Point one of the rig's cameras at a model's box, from an orbit off
/// [`EYE`], and switch it on.
fn aim_at(
    cameras: &mut Query<(&mut Camera, &mut Transform, &mut Projection), With<PortraitCamera>>,
    camera: Entity,
    stand: Vec3,
    lo: Vec3,
    hi: Vec3,
    yaw: f32,
    pitch: f32,
) {
    let Framing {
        eye,
        aim,
        near,
        far,
    } = frame_box(lo, hi, eye_direction(yaw, pitch));
    let Ok((mut cam, mut placement, mut projection)) = cameras.get_mut(camera) else {
        return;
    };
    cam.is_active = true;
    *placement = Transform::from_translation(stand + eye).looking_at(stand + aim, Vec3::Y);
    if let Projection::Perspective(own) = &mut *projection {
        own.near = near;
        own.far = far;
    }
}

/// Spawn one model's batches under a root of their own, on a layer.
fn stand_model(
    commands: &mut Commands,
    draws: &[vale_client::render::models::ModelDraw],
    layer: usize,
    stand: Vec3,
) -> Entity {
    let layers = RenderLayers::layer(layer);
    let root = commands
        .spawn((Transform::from_translation(stand), Visibility::default()))
        .id();
    for draw in draws {
        let mut batch = commands.spawn((
            Mesh3d(draw.mesh.clone()),
            MeshMaterial3d(draw.material.clone()),
            Transform::default(),
            ChildOf(root),
            // On every drawn entity: `RenderLayers` is per entity and is not
            // inherited, and a part left off the layer is a part the world
            // camera draws, eighty yards up.
            layers.clone(),
            // The declared box is computed a frame after the spawn and the
            // whole model is in frame by construction.
            NoFrustumCulling,
        ));
        // **A tinted batch carries its colour in its mesh tag**, and a
        // spawner that does not write one hands the shader a zero — which is
        // read as `0x00000000`, alpha and all, so the batch draws as nothing.
        // `render::doodads` is the one other caller and makes the same note.
        //
        // It is most of what a spell effect is made of: `Firebolt_ImpactDD_
        // Med_Chest.m2` previewed as an empty box for two runs before this
        // line, while a tree — whose batches carry no tint at all — was fine
        // from the first.
        if let Some(tint) = draw.baked_tint {
            batch.insert(bevy::mesh::MeshTag(tint));
        }
    }
    root
}

/// **What the big preview clears to**, where a row's picture clears to
/// nothing.
///
/// A row is drawn over its own fill and over its hover highlight, so a
/// transparent background is what makes it sit in the row rather than on it.
/// The pane is a box of its own — and most of the spell effects it is asked
/// to show are *additive* art, which adds to what is behind it: added to
/// transparent black it comes out with almost no alpha and composites to
/// nearly nothing, which is a black pane with a faint smudge in it. Added to
/// an opaque dark ground it is the colour it is in the game. The value is
/// `theme::SUNK`, so the picture's ground and the box's are the same.
const LARGE_CLEAR: Color = Color::srgb(0.078, 0.090, 0.110);

/// One slot's camera and lamp, alone on their layer.
fn spawn_rig(
    commands: &mut Commands,
    layer: usize,
    order: isize,
    clear: ClearColorConfig,
) -> Entity {
    let layers = RenderLayers::layer(layer);
    commands.spawn((
        DirectionalLight {
            // A fixed, neutral key. The world's sun is the light table's
            // answer for the hour, which at midnight is a black square, and
            // a picker is not a place to judge a model at dusk. The world's
            // ambient still reaches this view — it is a global resource —
            // so a night thumbnail is a little dimmer, never black.
            color: Color::srgb(0.92, 0.90, 0.85),
            // The whole value is in the colour, as the world's own sun is.
            illuminance: 1.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::default().looking_to(-LAMP.normalize(), Vec3::Y),
        layers.clone(),
        Name::new("model portrait lamp"),
    ));
    commands
        .spawn((
            PortraitCamera,
            Camera3d::default(),
            Camera {
                order,
                is_active: false,
                // Transparent for a row, so its own fill and its hover show
                // through wherever the model is not; opaque for the pane —
                // see [`LARGE_CLEAR`].
                clear_color: clear,
                ..default()
            },
            // The same three declarations the client's portrait cameras make,
            // and for the same reasons: every model shader ends in
            // `atmosphere::to_frame` and writes the game's own bytes, so a
            // camera left in the default space encodes them twice; the target
            // is an image and a multisampled one needs a resolve of its own;
            // nothing here is graded.
            bevy::camera::CompositingSpace::Srgb,
            Msaa::Off,
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            Projection::Perspective(PerspectiveProjection {
                fov: FOV,
                near: 0.1,
                far: 1000.0,
                aspect_ratio: 1.0,
                ..default()
            }),
            layers,
            Transform::default(),
            Name::new("model portrait"),
        ))
        .id()
}

/// The union of the batches' own boxes, in the axes the meshes are stored in.
///
/// `None` for a model with no geometry at all, which is an effect that is all
/// emitters — there is nothing to take a picture of.
fn bounds_of(
    draws: &[vale_client::render::models::ModelDraw],
    meshes: &Assets<Mesh>,
) -> Option<(Vec3, Vec3)> {
    let mut lo = Vec3::INFINITY;
    let mut hi = Vec3::NEG_INFINITY;
    for draw in draws {
        let Some(aabb) = meshes.get(&draw.mesh).and_then(|mesh| mesh.compute_aabb()) else {
            continue;
        };
        lo = lo.min(Vec3::from(aabb.min()));
        hi = hi.max(Vec3::from(aabb.max()));
    }
    let finite = lo.is_finite() && hi.is_finite() && (hi - lo).max_element() > 1e-4;
    finite.then_some((lo, hi))
}

/// Where the eye stands and what it looks at, relative to the model's origin.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Framing {
    eye: Vec3,
    aim: Vec3,
    near: f32,
    far: f32,
}

/// Stand on the [`EYE`] line, far enough back that every corner of the box
/// is inside the square frame, with a little margin.
///
/// **The corners and not the bounding sphere.** A tree is four times as tall
/// as it is wide, and a sphere round it is mostly air: fitted that way the
/// first pictures were a twig in the middle of the square. Each corner needs
/// the eye at least as far as its own depth plus its lateral offset over the
/// tangent of the half-angle; the frame fits when the eye is at the largest
/// of those.
fn frame_box(lo: Vec3, hi: Vec3, toward_eye: Vec3) -> Framing {
    let centre = (lo + hi) * 0.5;
    let radius = ((hi - lo).length() * 0.5).max(0.05);
    let toward_eye = toward_eye.normalize_or(EYE.normalize());
    let forward = -toward_eye;
    let right = Vec3::Y.cross(forward).normalize_or(Vec3::X);
    let up = forward.cross(right).normalize_or(Vec3::Y);
    let half_tan = (FOV * 0.5).tan().max(1e-3);
    let mut distance = 0.05f32;
    for corner in 0..8 {
        let point = Vec3::new(
            if corner & 1 == 0 { lo.x } else { hi.x },
            if corner & 2 == 0 { lo.y } else { hi.y },
            if corner & 4 == 0 { lo.z } else { hi.z },
        ) - centre;
        let depth_toward_eye = point.dot(toward_eye);
        let lateral = point.dot(right).abs().max(point.dot(up).abs());
        distance = distance.max(depth_toward_eye + lateral / half_tan);
    }
    let distance = distance * 1.06;
    Framing {
        eye: centre + toward_eye * distance,
        aim: centre,
        near: (distance - radius * 1.2).max(0.02),
        far: distance + radius * 1.2 + 1.0,
    }
}

/// **Where the eye stands, as an orbit off [`EYE`].**
///
/// `(0, 0)` is exactly [`EYE`] normalized, which is what keeps the big
/// preview's resting view identical to the rows'; a drag moves the eye round
/// the model and up and down from there. Spherical about the model's own up,
/// so a turned preview cannot roll.
fn eye_direction(yaw: f32, pitch: f32) -> Vec3 {
    let base = EYE.normalize();
    let resting_pitch = base.y.clamp(-1.0, 1.0).asin();
    let resting_yaw = base.x.atan2(base.z);
    let (yaw, pitch) = (
        resting_yaw + yaw,
        (resting_pitch + pitch).clamp(-PITCH_LIMIT, PITCH_LIMIT),
    );
    Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    )
}

/// The image a picture is drawn into: `Rgba8UnormSrgb`, transparent until the
/// camera has run, render-world only — the client's own portrait target, for
/// the reasons written on it.
fn target_image(side: u32) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    image
}

/// Draw a path's picture into `rect`, or what stands for one: a sunk square
/// while it is on its way, and a faint cross for a model that will not open.
pub fn paint(ui: &egui::Ui, portraits: &mut Portraits, path: &str, rect: egui::Rect) {
    portraits.want(path);
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(3), crate::ui::theme::SUNK);
    match portraits.status(path) {
        Status::Ready(id) => {
            painter.image(
                id,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        Status::Failed => {
            let stroke = egui::Stroke::new(1.0, crate::ui::theme::INK_FAINT);
            let inset = rect.shrink(rect.width() * 0.3);
            painter.line_segment([inset.left_top(), inset.right_bottom()], stroke);
            painter.line_segment([inset.right_top(), inset.left_bottom()], stroke);
        }
        Status::Pending => {}
    }
}

/// **The one big preview, under a picker's list**: the chosen model, drawn
/// large enough to judge and turned by dragging it.
///
/// `side` is how tall and wide the box is, in points. Answers the pane's own
/// response, so a caller can put something over it.
pub fn paint_large(
    ui: &mut egui::Ui,
    portraits: &mut Portraits,
    path: &str,
    side: f32,
) -> egui::Response {
    let side = side.max(48.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::drag());
    portraits.want_large(path);
    if response.dragged() {
        portraits.turn_large(response.drag_delta());
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(4), crate::ui::theme::SUNK);
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(4),
        egui::Stroke::new(1.0, crate::ui::theme::LINE),
        egui::StrokeKind::Inside,
    );
    match (portraits.large_texture(path), portraits.status(path)) {
        (Some(id), _) => {
            painter.image(
                id,
                rect.shrink(1.0),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        // The rows' own failure set answers here too, so a model that will
        // not open says so in the pane rather than sitting empty.
        (None, Status::Failed) => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "does not open",
                egui::FontId::proportional(11.0),
                crate::ui::theme::BAD,
            );
        }
        (None, _) => {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "drawing…",
                egui::FontId::proportional(11.0),
                crate::ui::theme::INK_FAINT,
            );
        }
    }
    response.on_hover_text("Drag to turn the model.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every corner of a box lands inside the frame, the tallest one near
    /// its edge, and the clip planes bracket the box.
    #[test]
    fn a_box_is_framed_whole_and_fills_the_frame() {
        let (lo, hi) = (Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 4.0, 1.0));
        let framing = frame_box(lo, hi, eye_direction(0.0, 0.0));
        let centre = (lo + hi) * 0.5;
        assert_eq!(framing.aim, centre);
        let view = Transform::from_translation(framing.eye)
            .looking_at(framing.aim, Vec3::Y)
            .to_matrix()
            .inverse();
        let half_tan = (FOV * 0.5).tan();
        let mut widest = 0.0f32;
        for corner in 0..8 {
            let point = Vec3::new(
                if corner & 1 == 0 { lo.x } else { hi.x },
                if corner & 2 == 0 { lo.y } else { hi.y },
                if corner & 4 == 0 { lo.z } else { hi.z },
            );
            let in_view = view.transform_point3(point);
            let depth = -in_view.z;
            assert!(depth > framing.near && depth < framing.far, "{depth}");
            let extent = in_view.x.abs().max(in_view.y.abs()) / (depth * half_tan);
            assert!(extent <= 1.0, "corner {corner} is out of frame: {extent}");
            widest = widest.max(extent);
        }
        assert!(widest > 0.9, "the box fills the frame: {widest}");
        // Above and in front rather than below or behind.
        assert!(framing.eye.y > centre.y && framing.eye.z < centre.z);
    }

    /// **…and from every angle the drag can reach, on a building's shape.**
    ///
    /// The fit is a max over the corners in the *camera's* axes, so it has to
    /// be redone for the direction actually used — a pane turned to look down
    /// on a wide, flat building measures its footprint where the resting view
    /// measured its height. A building drawn small in the middle of a turned
    /// pane is what a wrong answer here looks like.
    #[test]
    fn a_wide_building_fills_the_frame_from_every_angle_a_drag_can_reach() {
        let (lo, hi) = (Vec3::new(-20.0, 0.0, -15.0), Vec3::new(20.0, 12.0, 15.0));
        let half_tan = (FOV * 0.5).tan();
        for step in 0..12 {
            let yaw = step as f32 * std::f32::consts::TAU / 12.0;
            for pitch in [-PITCH_LIMIT, -0.5, 0.0, 0.5, PITCH_LIMIT] {
                let framing = frame_box(lo, hi, eye_direction(yaw, pitch));
                let view = Transform::from_translation(framing.eye)
                    .looking_at(framing.aim, Vec3::Y)
                    .to_matrix()
                    .inverse();
                let mut widest = 0.0f32;
                for corner in 0..8 {
                    let point = Vec3::new(
                        if corner & 1 == 0 { lo.x } else { hi.x },
                        if corner & 2 == 0 { lo.y } else { hi.y },
                        if corner & 4 == 0 { lo.z } else { hi.z },
                    );
                    let in_view = view.transform_point3(point);
                    let depth = -in_view.z;
                    assert!(
                        depth > framing.near,
                        "yaw {yaw} pitch {pitch}: {depth} in front of {}",
                        framing.near
                    );
                    assert!(
                        depth < framing.far,
                        "yaw {yaw} pitch {pitch}: {depth} past {}",
                        framing.far
                    );
                    let extent = in_view.x.abs().max(in_view.y.abs()) / (depth * half_tan);
                    assert!(
                        extent <= 1.0,
                        "yaw {yaw} pitch {pitch}: corner out of frame at {extent}"
                    );
                    widest = widest.max(extent);
                }
                assert!(
                    widest > 0.85,
                    "yaw {yaw} pitch {pitch}: fills only {widest}"
                );
            }
        }
    }

    /// A degenerate box is still a framing rather than a division by zero.
    #[test]
    fn a_point_is_framed_from_a_hands_breadth() {
        let framing = frame_box(Vec3::ZERO, Vec3::ZERO, eye_direction(0.0, 0.0));
        assert!(framing.eye.is_finite() && framing.near > 0.0 && framing.far > framing.near);
    }

    /// **The big preview at rest is the rows' own view**, which is what keeps
    /// the picture under the list and the icon in it the same model seen the
    /// same way. A turn moves the eye round and up, and the pitch is capped
    /// short of the pole so the up vector never degenerates.
    #[test]
    fn the_resting_orbit_is_the_rows_own_and_a_turn_moves_off_it() {
        let resting = eye_direction(0.0, 0.0);
        assert!((resting - EYE.normalize()).length() < 1e-5, "{resting}");
        let turned = eye_direction(1.0, 0.0);
        assert!((turned.length() - 1.0).abs() < 1e-5);
        assert!((turned - resting).length() > 0.5, "a turn moves the eye");
        assert!(
            (turned.y - resting.y).abs() < 1e-5,
            "yaw alone keeps the height"
        );
        let raised = eye_direction(0.0, 0.4);
        assert!(
            raised.y > resting.y,
            "a positive pitch looks down from higher"
        );
        // …and past the cap it stops rather than going over the top.
        let over = eye_direction(0.0, 9.0);
        assert!(over.y < 1.0 && over.y > 0.9, "{over}");
        assert!(eye_direction(0.0, -9.0).y < -0.9);
        // A box framed from a turned eye is still framed whole.
        let framing = frame_box(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 4.0, 1.0), turned);
        assert!(framing.eye.is_finite() && framing.far > framing.near);
    }

    /// A different model puts the turn back, and the same one does not.
    #[test]
    fn asking_for_another_model_puts_the_turn_back() {
        let mut portraits = Portraits::default();
        portraits.want_large("World\\A.m2");
        portraits.large_yaw = 1.0;
        portraits.large_pitch = 0.2;
        portraits.want_large("world\\a.m2");
        assert_eq!(
            (portraits.large_yaw, portraits.large_pitch),
            (1.0, 0.2),
            "the same model keeps it"
        );
        assert!(portraits.large_turned());
        portraits.want_large("World\\B.mdx");
        assert_eq!((portraits.large_yaw, portraits.large_pitch), (0.0, 0.0));
        assert!(!portraits.large_turned());
        assert_eq!(portraits.large_wanted.as_deref(), Some("world\\b.m2"));
        // …and nothing is shown until the pass has drawn one.
        assert_eq!(portraits.large_texture("World\\B.mdx"), None);
    }

    /// A drag turns it, and the pitch is clamped at both ends.
    #[test]
    fn a_drag_turns_it_within_the_pitch_limit() {
        let mut portraits = Portraits::default();
        portraits.turn_large(egui::vec2(0.0, 0.0));
        assert!(!portraits.large_turned(), "a still pointer is not a turn");
        portraits.turn_large(egui::vec2(50.0, 0.0));
        assert!((portraits.large_yaw + 50.0 * TURN_RATE).abs() < 1e-6);
        for _ in 0..100 {
            portraits.turn_large(egui::vec2(0.0, 40.0));
        }
        assert_eq!(portraits.large_pitch, PITCH_LIMIT);
        for _ in 0..200 {
            portraits.turn_large(egui::vec2(0.0, -40.0));
        }
        assert_eq!(portraits.large_pitch, -PITCH_LIMIT);
        portraits.reset_large_turn();
        assert!(!portraits.large_turned());
    }

    /// The newest ask is served first, an ask is a set, and a failure is
    /// remembered.
    #[test]
    fn the_newest_ask_is_served_first_and_a_failure_is_not_asked_again() {
        let mut portraits = Portraits::default();
        portraits.frame = 1;
        portraits.want("World\\A.m2");
        portraits.frame = 2;
        portraits.want("World\\B.mdx");
        portraits.want("world\\a.m2");
        assert_eq!(portraits.wanted.len(), 2, "an ask is a set");
        assert_eq!(
            portraits.next_wanted().as_deref(),
            Some("world\\a.m2"),
            "re-asked, so newest"
        );
        portraits.frame = 3;
        portraits.want("World\\B.mdx");
        assert_eq!(
            portraits.next_wanted().as_deref(),
            Some("world\\b.m2"),
            "and .mdx is .m2"
        );

        portraits.failed.insert("world\\b.m2".into());
        portraits.wanted.clear();
        portraits.want("World\\B.m2");
        assert!(portraits.wanted.is_empty());
        assert_eq!(portraits.status("World\\B.m2"), Status::Failed);
        assert_eq!(portraits.status("World\\A.m2"), Status::Pending);
        assert_eq!(portraits.get("World\\A.m2"), None);
    }

    /// A `.wmo` keeps its extension; the model swap is for models.
    #[test]
    fn a_building_is_keyed_as_itself() {
        assert_eq!(key_of("World\\wmo\\X\\Y.WMO"), "world\\wmo\\x\\y.wmo");
        assert_eq!(key_of("World\\Z.MDX"), "world\\z.m2");
    }

    fn wolf(skins: &[&str]) -> Worn {
        Worn {
            path: "Creature\\Wolf\\Wolf.m2".into(),
            skins: skins.iter().map(|skin| skin.to_string()).collect(),
            hair: None,
            dress: Dress::Creature,
            look: None,
            cloak: None,
        }
    }

    /// **Two display ids of one model are two pictures**, so the key carries
    /// the skins. Keying by path alone would give the second wolf the first
    /// one's coat, which is a picture that is wrong rather than missing.
    #[test]
    fn two_coats_of_one_model_are_two_keys() {
        let grey = wolf(&["Creature\\Wolf\\WolfSkinGrey.blp"]);
        let black = wolf(&["Creature\\Wolf\\WolfSkinBlack.blp"]);
        assert_ne!(grey.key(), black.key());
        assert!(
            grey.key().starts_with("creature\\wolf\\wolf.m2|"),
            "{}",
            grey.key()
        );
    }

    /// …and a model wearing nothing keys as its own path, so a doodad's
    /// picture is the entry it always was.
    #[test]
    fn a_model_wearing_nothing_keys_as_its_path() {
        assert_eq!(wolf(&[]).key(), "creature\\wolf\\wolf.m2");
        // An empty slot is a slot, and it is still nothing worn.
        assert_eq!(wolf(&["", ""]).key(), "creature\\wolf\\wolf.m2");
    }

    /// **Keying a key is the identity.** Every entry point here takes either a
    /// path or a dressed key, and re-keying one would put `.m2` on the end of
    /// a `.blp` — a lookup that matches nothing and a row that stays blank.
    #[test]
    fn a_dressed_key_is_already_a_key() {
        let key = wolf(&["Creature\\Wolf\\WolfSkinGrey.blp"]).key();
        assert_eq!(key_of(&key), key);
    }

    /// The slot layers and orders are above and before everything else in
    /// the app, and distinct from each other.
    #[test]
    fn the_slots_are_alone_on_their_layers_and_draw_before_the_world() {
        let layers: Vec<usize> = (0..SLOTS).map(|i| FIRST_LAYER + i).collect();
        assert!(layers
            .iter()
            .all(|layer| *layer > 14 && *layer != crate::stage::STAGE_LAYER));
        let orders: Vec<isize> = (0..SLOTS).map(|i| FIRST_ORDER - i as isize).collect();
        assert!(orders.iter().all(|order| *order < -15));
        let mut sorted = orders.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), orders.len());
    }
}
