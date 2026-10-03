//! The 3D scene behind the login and character screens, drawn through the
//! camera the model itself carries.
//!
//! ```text
//! lua::model::visible   which <Model> frame is up, and what it holds
//!   -> ModelCache        Interface\Glues\Models\UI_MainMenu\UI_MainMenu.m2
//!   -> one root + joints the same shape an attached effect is spawned in
//!   -> M2Camera 0        the eye, the aim and the field of view
//! ```
//!
//! ## What the scene is on the two screens
//!
//! `<ModelFFX name="AccountLogin" setAllPoints="true" file="…UI_MainMenu.mdx">`
//! fills the window and everything the interface draws is on top of it: the
//! portal, the two cloaked figures, the fire, the burning sky. Character select
//! is the same element with `SetModel` called per race. Without this scene,
//! `Interface\GlueXML\` draws the login form on a black rectangle.
//!
//! ## The camera comes from the model file
//!
//! `AccountLogin_OnLoad`'s second line is `this:SetCamera(0)` and nothing else
//! in either screen says where to stand. `vale_assets::world::m2::M2Camera` is that
//! camera: an eye, an aim and a vertical field of view, all in model space.
//! `vale glue` prints the seven the glue models carry. A guessed camera would
//! draw the same geometry from the wrong place, and the result would look
//! plausible rather than broken.
//!
//! ## Where the character stands
//!
//! Character select shows a backdrop and a character, and the backdrop says
//! where the character stands. Each of the six `UI_<race>.m2` files carries
//! exactly two attachment points. Point 0 is on the camera's axis, four to five
//! yards in front of it and about a yard below the aim. Measured over all six,
//! it lies laterally within 0.15 yards of the axis and vertically 0.75 (dwarf)
//! to 1.24 (night elf) below it, which is the spread of the races' heights, so
//! point 0 marks the character's feet. `UI_MainMenu.m2`, the login screen, has
//! no character and carries no attachment points. See [`PLINTH_POINT`].
//!
//! The character is `vale_assets::look::dress`'d the same way as one in the
//! world, from `SMSG_CHAR_ENUM`'s appearance bytes and wardrobe. The model is
//! `ChrRaces.dbc`'s display id for the race and gender; in the world the server
//! sends the display id, so this lookup is needed only here.
//!
//! ## Differences from the 1.12.1 client
//!
//! * The scene is drawn by the world camera at full screen rather than into
//!   the frame's rectangle. Both `<Model>` elements used here are
//!   `setAllPoints="true"` on `GlueParent`, so the rectangle is the screen and
//!   the two agree. A `<Model>` with a rectangle of its own (the fourteen
//!   portraits in `Interface\FrameXML\`, and character create's preview) would
//!   need a render target per frame, which this module does not provide.
//!   Nothing in the two screens named here has a smaller rectangle.
//! * The fog is read and not applied. `<ModelFFX fogNear="0" fogFar="1200">`
//!   and `CharModelFogInfo`'s per-race colours reach [`crate::lua::widgets::model`] and
//!   stop there. The world's fog uniform belongs to `render::sky` and is
//!   written from `Light.dbc` for the current map, which does not apply here.
//!   The scenes look right with the fog and acceptable without it.
//!
//! The character on the plinth holds its weapons. `SMSG_CHAR_ENUM` does not
//! carry an item's class, subclass or sheath type, and no packet on this screen
//! can fetch them, but none of them is needed: on character select the client
//! does not sheathe anything. Every weapon goes in the hand (sheath type 0), and
//! the only property of the item that matters is `inventoryType == 14`, which
//! the packet carries. The ranged slot is skipped. See
//! [`vale_assets::tables::item::Weapon::from_char_enum`].
//!
//! ## Animation clock
//!
//! 1.12 advances a `<Model>` only when its `OnUpdate` calls `AdvanceTime()`, and
//! `CharacterSelect_UpdateModel` does that. This module animates on the world's
//! clock instead. [`crate::lua::widgets::model::Scene::elapsed`] records the
//! `AdvanceTime` calls, so a test can detect when they are missing.

use std::sync::Arc;

use bevy::prelude::*;
use bevy::math::Affine3A;
use bevy::render::mesh::skinning::SkinnedMesh;

use vale_assets::tables::item::AttachedModel;
use vale_assets::world::m2::{M2Camera, M2Skeleton};

use crate::render::axes;
use crate::render::models::{Lookup, ModelAssets, ModelCache, SceneLighting};
use crate::render::models::material::Materials;
use crate::world::entities::{AttachedPart, EntityPart, Joint};

/// Which of the backdrop's two attachment points the character stands on.
///
/// Not `m2::attach::SHIELD`, which is what id 0 means on a character model:
/// these six scenes are not characters, and their two points belong to the
/// glue screens that show a body. Point 0 is the one on the camera's axis at
/// standing height; the module comment gives the measurement.
///
/// Character select and character create both stand their character on this
/// point. Both screens place the character at the scene origin, `{0,0,0}`,
/// rotated about +Z by the screen's facing (`SetCharacterSelectFacing`,
/// `SetCharacterCreateFacing`) and then scaled, so there is one standing place
/// per scene. The 1.12.1 client puts the backdrop where its plinth reaches the
/// scene origin; this client puts the backdrop at the origin and the body on
/// its plinth, which gives the same picture.
///
/// Point 1 is not the standing place, and its purpose is unknown. It is 0.77
/// to 1.74 yards off the camera axis and 4.49 to 10.31 along it; standing
/// characters on it put a dwarf twice as far from the eye as a night elf, and
/// neither of them centred.
const PLINTH_POINT: u32 = 0;

/// The name of the character-create screen's `<Model>` frame:
/// `CharacterCreate.xml`'s `<ModelFFX name="CharacterCreate">`, which is what
/// `SetCharCustomizeFrame` names and what [`crate::lua::widgets::model::Scene::frame`]
/// answers while that screen is up.
const CREATE_FRAME: &str = "CharacterCreate";

/// The scene on screen, and what it took to put it there.
#[derive(Resource, Default)]
pub struct GlueScene {
    /// What the interface last said it was holding; a change starts a new
    /// scene. `None` while nothing is showing, which includes all of the time
    /// spent in the world.
    showing: Option<crate::lua::widgets::model::Scene>,
    /// The root everything spawned hangs off, despawned whole when the scene
    /// changes.
    root: Option<Entity>,
    /// One per joint, plus the identity joint at the end: the same arrangement
    /// `world::entities` uses, because a weightless vertex still needs a joint
    /// to follow.
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    /// Every batch whose colour is animated, and which track animates it.
    ///
    /// A tinted batch's material reads its `MeshTag` as `0xAARRGGBB` rather than
    /// as a room light, so a batch left at the default zero draws as fully
    /// transparent black; see [`crate::render::models::tint_tag`].
    /// `UI_MainMenu`'s sky is a tinted batch, and with the tag unwritten the
    /// login screen's horizon draws as a flat grey rectangle the size of the
    /// window.
    tinted: Vec<(Entity, vale_assets::world::m2::BatchTint)>,
    /// The colour and transparency tracks those indices point into.
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
    /// Which sequence is playing, resolved once at the spawn.
    sequence: usize,
    /// `Time::elapsed_secs` when it started.
    since: f32,
    /// The camera the file chose, or `None` for a model that carries none,
    /// which leaves the world camera where it was.
    camera: Option<M2Camera>,
    /// The world camera's own projection, kept while the glue scene uses the
    /// camera.
    ///
    /// [`aim_camera`] writes the model's field of view and clip planes onto the
    /// world camera. If they were not restored, the world would be drawn at 42
    /// degrees and clipped at the login scene's far plane. This value is put
    /// back once, on the frame the scene goes.
    restore: Option<PerspectiveProjection>,
    /// The centred 16:9 viewport [`aim_camera`] has put on the world camera,
    /// as `(position, size)` in physical pixels, or `None` when the window is
    /// no wider than 16:9 and the scene fills it.
    pillarbox: Option<(UVec2, UVec2)>,
    /// The world camera's own output mode, kept while a pillarbox replaces its
    /// clear colour with black, and put back when the pillarbox goes.
    restore_output: Option<bevy::camera::CameraOutputMode>,
    /// Where a character would stand in this scene, as a bone of the
    /// backdrop's skeleton and a point in that bone's frame: the backdrop's
    /// [`PLINTH_POINT`], resolved once at the spawn. `None` for the login
    /// screen's model, which carries no attachment points.
    plinth: Option<(usize, [f32; 3])>,
    /// The character standing on the plinth.
    standing: Standing,
    /// The scene's own lighting, resolved once at the spawn and applied to
    /// everything drawn in the scene: the backdrop, the character on the
    /// plinth, and what that character wears. The world's daylight, computed
    /// for the current map, does not apply here; see [`SceneLighting`] and the
    /// module note on `Light.dbc` above.
    lighting: SceneLighting,
}

/// The character on the plinth: what it was built from, and what that cost.
///
/// A separate struct rather than more fields on [`GlueScene`] because it is torn
/// down and rebuilt as a unit, every time the highlight moves to a row with a
/// different appearance.
///
/// `Default` is hand-written for one field: [`Self::scale`] is a multiplier and
/// its derived default would be zero, which would collapse the character to a
/// point.
struct Standing {
    /// What this was built from, compared each frame to detect a change: the
    /// appearance and wardrobe `glue::glue` derived from the highlighted row.
    /// `None` while nothing is standing there.
    built: Option<crate::glue::glue::Plinth>,
    /// The root the batches and joints hang off. Its `Transform` is written
    /// every frame by [`pose_scene`]. It is a root entity rather than a child
    /// of the scene's root because its skinned parts are posed through
    /// world-space joints, and a second parent transform would place it twice.
    root: Option<Entity>,
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    tinted: Vec<(Entity, vale_assets::world::m2::BatchTint)>,
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
    /// The character's own `Stand`, resolved once at the spawn.
    sequence: usize,
    since: f32,
    /// `CreatureDisplayInfo`'s scale for this body: 1.00 for twelve of the
    /// sixteen player models, and 1.35 and 1.25 for the male and female
    /// tauren. Applied here as in the world; without it a tauren stands a
    /// quarter too short.
    scale: f32,
    /// Attachments whose M2 has not loaded yet, such as a helm or a pair of
    /// pauldrons. Drained by [`dress_the_character`] as each one arrives, the
    /// same way `world::entities::worn` drains a wearer's: the character is
    /// drawn as soon as its body is ready and the gear is added as it loads.
    wanted: Vec<AttachedModel>,
    /// Attachments whose M2 has loaded and been spawned.
    attached: Vec<AttachedPart>,
    /// The character's own attachment points (not the backdrop's), which
    /// place each arriving attachment. Empty while nobody is standing there.
    points: Arc<Vec<vale_assets::world::m2::M2Attachment>>,
}

impl Default for Standing {
    fn default() -> Standing {
        Standing {
            built: None,
            root: None,
            joints: Vec::new(),
            skeleton: None,
            tinted: Vec::new(),
            tints: None,
            sequence: 0,
            since: 0.0,
            scale: 1.0,
            wanted: Vec::new(),
            attached: Vec::new(),
            points: Arc::default(),
        }
    }
}

impl GlueScene {
    /// Whether a glue scene is on screen at all, which is what the camera pass
    /// and the world's own passes need to know.
    pub fn showing(&self) -> bool {
        self.root.is_some()
    }
}

/// Marks the joints this pass spawns.
///
/// A separate marker from `world::entities::Joint`, which the entity pose
/// systems query. Those systems read a `WorldEntity` and a session; a glue
/// scene has neither, so its joints must not match their queries.
#[derive(Component)]
struct GlueJoint;

/// Whether the login and character screens may put a scene on the camera.
///
/// On in the client, where those screens are what is in front of a player until
/// they log in. A host that is showing the world rather than logging in turns it
/// off, because [`aim_camera`] writes the world camera's transform for as long
/// as a scene is up and would otherwise frame everything through
/// `UI_MainMenu.m2`'s own camera.
///
/// It is separate from `WorldTuning::interface`, which switches the widget
/// tree's walk and paint. This switch controls whether the 3D scene behind
/// those widgets is built at all, which has a different cost.
///
/// Turning it off tears down whatever is up, on the next frame, and silences
/// the login theme, which `sound::music` plays only while this is on.
#[derive(Resource, Debug, Clone, Copy)]
pub struct GlueScenes(pub bool);

impl Default for GlueScenes {
    fn default() -> GlueScenes {
        GlueScenes(true)
    }
}

pub struct GluePlugin;

impl Plugin for GluePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GlueScene>()
            .init_resource::<GlueScenes>()
            .add_systems(
            Update,
            (
                follow_scene,
                // After the backdrop, which says where a character stands:
                // `GlueScene::plinth` is resolved from the backdrop's
                // attachment table at its spawn, so a character built before
                // it would have no position.
                follow_character,
                dress_the_character,
                pose_scene,
                // After `camera::place`, which writes the world camera's
                // transform from the rig every frame. This overwrites it;
                // without the ordering the login screen's framing would
                // alternate between the file's camera and the rig's.
                aim_camera.after(crate::world::camera::place),
            )
                .chain(),
        )
        // After the three passes' own visibility switches. `tuning::switch`
        // writes visibility on every frame the settings change and on every
        // `Added`, so this must run after it or be overwritten; that ordering
        // is why this is a separate system.
        .add_systems(Update, stand_the_world_down.after(follow_scene));
    }
}

/// Hides the world's sky, stars, sun and moons while a glue scene is up.
///
/// The scene carries its own sky: `UI_MainMenu.m2`'s batch 6 is
/// `MM_SKY_01.BLP`, and batches 14, 18 and 19 are two layers of drifting cloud
/// on their own texture matrices. The world's dome is drawn behind everything
/// by depth rule rather than by distance: `frag_depth = 0.0` with a
/// `GreaterEqual` test (see [`crate::render::sky`]). Left visible, it replaces
/// the scene's sky, and the horizon draws as a flat blue-grey rectangle. The
/// stars, sun and moons are hidden for the same reason: `Light.dbc`'s values
/// for the current map do not belong in this scene.
///
/// This is a separate system rather than a check in each of the three passes
/// because the condition concerns the glue, not the sky, as `render::decals`
/// owns its own removal rather than the entity pass. Each pass keeps its own
/// switch, and this system overrides them while a scene is up.
fn stand_the_world_down(
    mut commands: Commands,
    scene: Res<GlueScene>,
    camera: Query<Entity, With<crate::world::camera::WorldCamera>>,
    mut world_sky: ParamSet<(
        Query<&mut Visibility, With<crate::render::sky::SkyDome>>,
        Query<&mut Visibility, With<crate::render::stars::StarDome>>,
        Query<&mut Visibility, With<crate::render::celestial::CelestialBody>>,
    )>,
    // The fog's owner, marked changed when the scene goes so it rewrites the
    // camera's fog; see the restore branch. `ResMut` only to reach
    // `set_changed`; nothing here reads or writes a field of it.
    mut sky: ResMut<crate::render::sky::Sky>,
    mut borrowed: Local<bool>,
) {
    // Restored once, on the frame the scene goes. `tuning::switch` writes a
    // pass's visibility only when the settings change or the entity is new, so
    // a sky hidden here and not restored would leave the world with no sky,
    // stars or moons for the rest of the session. [`GlueScene::restore`] does
    // the same for the projection.
    if !scene.showing() {
        if std::mem::take(&mut *borrowed) {
            // The fog is restored on the same edge. While the glue shows, the
            // branch below inserts the scene's fog on the world camera every
            // frame, including the frames after a world entry, where it
            // overwrote the `DistanceFog` `render::sky::apply` had just written
            // (the two systems are not ordered, and this one ran later).
            // Nothing re-applied the world's fog until the next sky
            // re-resolve, at the player's first 4-yard cell crossing. Until
            // then every login showed character select's pink fog, with
            // `NightAir` zeroed and so every lamp pool dark despite 24 lit
            // point lights. Logged at the time: `light: resolved … night 1.00`,
            // `sky: applied … air.x 1.000 to 1 camera(s)`, `lamps: 24 lit`,
            // and a pale screenshot 20 seconds later.
            //
            // The restore is one line because the fog has one owner: marking
            // `Sky` changed makes `apply` re-run on the next frame and rewrite
            // everything the glue may have changed (the fog, the dome stops,
            // the sun, the fill and the clear colour) from the atmosphere it
            // already resolved. Restoring a cached fog here instead would race
            // `apply` on the same frame and be wrong whenever the world
            // re-resolved during the glue (a logout in one map, a login in
            // another).
            sky.set_changed();
            for mut visibility in &mut world_sky.p0() {
                *visibility = Visibility::Inherited;
            }
            for mut visibility in &mut world_sky.p1() {
                *visibility = Visibility::Inherited;
            }
            for mut visibility in &mut world_sky.p2() {
                *visibility = Visibility::Inherited;
            }
        }
        return;
    }
    *borrowed = true;
    // Replace the world's fog with the scene's. `render::sky` puts a
    // `DistanceFog` on the world camera from `Light.dbc`'s `fog_start..fog_end`
    // for the current map (a few hundred yards) and sets the clear colour to
    // the same band. `UI_MainMenu` is 1,200 units deep: its backdrop quad
    // (`MM_SKY_01.BLP`) and its two cloud layers sit at about 1,100, so the
    // world's fog blends them fully to the clear colour, and the sky draws as a
    // flat blue-grey rectangle that looks like a batch that failed to draw.
    //
    // The fog is moved out rather than removed. `m2.wgsl` reads
    // `bevy_pbr::mesh_view_bindings::fog` unconditionally, so a camera with no
    // `DistanceFog` component fails to compile the shader: four
    // `no definition in scope for identifier` errors, and a window painted
    // entirely in the clear colour, interface included.
    //
    // The scene states its own fog (`fogNear="0" fogFar="1200"`, and per-race
    // colours through `SetFogColor`), and where it names a colour that colour
    // is applied; each of character select's six races names one. The login
    // screen names no colour, and its 1,200-unit distance with a black default
    // would fog its sky to black, which the 1.12.1 client does not show. With
    // no colour stated, the fog is moved past the far plane and the scene draws
    // unfogged.
    let (colour, start, end) = match scene.showing.as_ref().and_then(|s| s.fog) {
        Some((rgb, near, far)) => (LinearRgba::rgb(rgb[0], rgb[1], rgb[2]), near, far),
        None => {
            let beyond = scene.camera.map_or(4096.0, |c| c.far_clip);
            (LinearRgba::BLACK, beyond, beyond * 2.0)
        }
    };
    for entity in &camera {
        commands.entity(entity).insert(DistanceFog {
            color: colour.into(),
            falloff: FogFalloff::Linear { start, end },
            ..default()
        });
    }
    for mut visibility in &mut world_sky.p0() {
        *visibility = Visibility::Hidden;
    }
    for mut visibility in &mut world_sky.p1() {
        *visibility = Visibility::Hidden;
    }
    for mut visibility in &mut world_sky.p2() {
        *visibility = Visibility::Hidden;
    }
}

/// Keep the spawned scene in step with what the interface is holding.
fn follow_scene(
    mut commands: Commands,
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut scene: ResMut<GlueScene>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
    enabled: Res<GlueScenes>,
) {
    let Some(host) = host else { return };
    // A host that has switched these off wants whatever is up taken down, which
    // is what asking for `None` does: the comparison below finds it different
    // from what is showing and the teardown runs.
    let wanted = if enabled.0 { host.glue_scene() } else { None };
    // Compared on the file and the sequence, not on the whole record. The
    // facing changes every frame of a drag and the elapsed time changes every
    // frame; rebuilding the scene on either would respawn a 9,892-vertex model
    // sixty times a second.
    let same = match (&scene.showing, &wanted) {
        (Some(a), Some(b)) => a.file == b.file && a.sequence == b.sequence,
        (None, None) => true,
        _ => false,
    };
    if same {
        // The facing still has to be followed, and it is a write to one
        // transform rather than a rebuild.
        scene.showing = wanted;
        return;
    }

    if let Some(root) = scene.root.take() {
        commands.entity(root).despawn();
    }
    scene.joints.clear();
    scene.tinted.clear();
    scene.tints = None;
    scene.skeleton = None;
    scene.camera = None;
    scene.plinth = None;
    // The character is removed with the backdrop, because it stands on the
    // backdrop's attachment point: left in place, it would float where the
    // previous race's plinth was. `follow_character` builds it again on the
    // next frame, from the same record, against the new scene.
    take_down_character(&mut commands, &mut scene.standing);
    scene.showing = wanted.clone();
    let Some(wanted) = wanted else { return };

    // `.mdx` in the interface, `.m2` in the archive: the same rename every
    // model path from a DBC needs; `assets::dress` states it for the wardrobe.
    let path = archive_path(&wanted.file);
    // The lights the file states, read before anything is dressed by them; see
    // [`SceneLighting`] and `vale_assets::world::m2::M2Light`. They are part of
    // the dressing's identity, so they are needed first. `model_lights`
    // answers without building the geometry and requests the file the same
    // way the dressing would.
    //
    // A lamp's place is its position alone. Measured: every light bone in all
    // thirteen glue models is a parentless root whose pivot is that position,
    // with no translation track; `vale glue` checks all 23 and reports any
    // that are not. The scene's root is the identity (neither glue screen
    // calls `SetFacing` or `SetModelScale` on its `<Model>`), so model space is
    // world space and the position needs no composition.
    let Some(lights) = cache.model_lights(&path) else {
        scene.showing = None;
        return;
    };
    let lighting = SceneLighting::resolve(&lights, axes::to_bevy);
    let Lookup::Ready(model) = cache.as_scene(&path, lighting, &mut meshes, &mut materials) else {
        // Loading, or a file the archive does not have. Nothing is shown and
        // `showing` is cleared, so the next frame enters this arm again:
        // `ModelCache::lookup` is idempotent, and re-entering this arm is what
        // polls it.
        scene.showing = None;
        return;
    };

    let mut joints = Vec::new();
    let mut tinted = Vec::new();
    scene.root = Some(spawn(
        &mut commands,
        &mut meshes,
        &model,
        false,
        &mut joints,
        &mut tinted,
    ));
    scene.joints = joints;
    scene.tinted = tinted;
    scene.skeleton = model.skeleton.clone();
    scene.tints = model.tints.clone();
    // Sequence 0, which both screens ask for, clamped to what the file has.
    // `SetSequence(0)` is the first line of both `OnLoad`s and neither calls it
    // again.
    scene.sequence = model
        .skeleton
        .as_ref()
        .map_or(0, |s| (wanted.sequence as usize).min(s.sequences.len().saturating_sub(1)));
    scene.since = time.elapsed_secs();
    scene.camera = model_camera(&model, wanted.camera);
    scene.lighting = lighting;
    scene.plinth = model
        .attachments
        .iter()
        .find(|p| p.id == PLINTH_POINT)
        .map(|p| (p.bone as usize, p.position));
    info!(
        "glue: {path} — {} batches, {} bones, camera {}, plinth {}, {} lamp(s) \
         over ambient ({:.3}, {:.3}, {:.3})",
        model.draws.len(),
        model.joint_count.saturating_sub(1),
        match scene.camera {
            Some(c) => format!("{:.0}deg", c.fov.to_degrees()),
            None => "none (the model carries no camera)".to_string(),
        },
        match scene.plinth {
            Some((bone, at)) => format!("bone {bone} at ({:.2}, {:.2}, {:.2})", at[0], at[1], at[2]),
            None => "none (nobody stands in this scene)".to_string(),
        },
        lighting.ambient.w as u32,
        lighting.ambient.x,
        lighting.ambient.y,
        lighting.ambient.z,
    );
}

/// Puts the highlighted character on the plinth, and takes the previous one off.
///
/// Which row, what it looks like and what it wears are decided by
/// `crate::glue::glue::stand_on_plinth` and arrive here as a
/// [`crate::glue::glue::Plinth`]. This function does the rendering part: a
/// display id, a dressing, a mesh per batch and a joint per bone.
fn follow_character(
    mut commands: Commands,
    mut scene: ResMut<GlueScene>,
    state: Res<crate::glue::glue::GlueState>,
    mut cache: ResMut<ModelCache>,
    mut displays: ResMut<crate::world::entities::DisplayCache>,
    assets: Res<crate::assets::GameAssets>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
) {
    // Nobody stands on the login screen: `UI_MainMenu.m2` carries no
    // attachment point, so `plinth` is `None`. The test is on the model, not on
    // which screen is up.
    //
    // The two screens stand their character in the same place (see
    // [`PLINTH_POINT`]), so the screen decides only whose body it is: the
    // highlighted character, or the one being made.
    let who = match on_create_screen(&scene) {
        true => state.create_plinth.clone(),
        false => state.plinth.clone(),
    };
    let wanted = scene.plinth.and(who);
    if scene.standing.built == wanted {
        return;
    }
    take_down_character(&mut commands, &mut scene.standing);
    scene.standing.built = wanted.clone();
    let Some(wanted) = wanted else { return };
    // The backdrop's own lighting rig, which is what lights the character too.
    let lighting = scene.lighting;

    // `ChrRaces.dbc`, which nothing else in this client reads. In the world a
    // unit's model is `UNIT_FIELD_DISPLAYID` from the server; on character
    // select no character is logged in, so the server has sent no display id.
    let display_id = match displays.tables_now(&assets) {
        Some(tables) => tables.race_display(wanted.appearance.race, wanted.appearance.gender),
        // The archives will not open. The record is cleared so the next frame
        // asks again rather than treating the plinth as built, as
        // `follow_scene` does for a model still loading.
        None => {
            scene.standing.built = None;
            return;
        }
    };
    let Some(display_id) = display_id else {
        warn!(
            "no ChrRaces model for race {} gender {}",
            wanted.appearance.race, wanted.appearance.gender
        );
        return;
    };
    let Some(display) = displays.resolve(
        &assets,
        vale_protocol::state::update::ObjectType::Player,
        display_id,
    ) else {
        return;
    };
    let Some(tables) = displays.tables() else {
        return;
    };

    // The same dressing function the world uses: which geosets the head and
    // the gear select, what the helm hides, what hangs off the bones. A second
    // implementation here would diverge, and `vale dress` would check only the
    // world's.
    //
    // The weapons are in hand and the sheath state is melee drawn: on
    // character select the 1.12.1 client puts every weapon in the hand.
    // `glue::glue::Plinth::weapons` turns the packet's two slots into the
    // pair, and `Weapon::from_char_enum` implements the rule.
    let dressed = vale_assets::look::dress::dress(
        tables,
        &display,
        &vale_assets::look::dress::Wearer {
            appearance: Some(wanted.appearance),
            equipment: &wanted.equipment,
            weapons: wanted.weapons,
            sheath_state: vale_assets::look::sheath::MELEE,
        },
    );
    let ready = match &dressed.look {
        Some(look) => cache.dressed_as_character(
            &display.path,
            look,
            dressed.dress,
            dressed.cloak.as_deref(),
            None,
            // Lit by the scene it stands in. `UI_Orc`'s four lamps sit two
            // to five yards from the plinth with ranges to match, so they are
            // the character's key light rather than the scenery's.
            lighting,
            &mut meshes,
            &mut materials,
        ),
        // A race whose display id resolves to a creature-model row rather than
        // a character one. 1.12 has no such race; the model is drawn as the
        // tables name it rather than not at all.
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
    let model = match ready {
        Lookup::Ready(model) => model,
        // Still loading. Clearing the record makes the next frame ask again;
        // `ModelCache` lookups are idempotent. An unreadable model is not
        // retried.
        Lookup::Loading => {
            scene.standing.built = None;
            return;
        }
        Lookup::Failed => return,
    };

    let mut joints = Vec::new();
    let mut tinted = Vec::new();
    scene.standing.root = Some(spawn(
        &mut commands,
        &mut meshes,
        &model,
        true,
        &mut joints,
        &mut tinted,
    ));
    scene.standing.joints = joints;
    scene.standing.tinted = tinted;
    scene.standing.skeleton = model.skeleton.clone();
    scene.standing.tints = model.tints.clone();
    // `Stand`. 1.12 plays no greeting animation on this screen: the row is
    // highlighted and the character idles.
    scene.standing.sequence = model
        .skeleton
        .as_ref()
        .and_then(|s| s.best_sequence(&[vale_assets::world::m2::anim::STAND]))
        .unwrap_or(0);
    scene.standing.since = time.elapsed_secs();
    scene.standing.scale = display.scale;
    // Only attachments this body has a point for, filtered as a wearer in the
    // world is: a model with no shoulder attachment cannot wear pauldrons, and
    // loading the pauldron model anyway would leave it with nowhere to hang.
    scene.standing.wanted = dressed
        .attachments
        .into_iter()
        .filter(|a| model.attachments.iter().any(|p| p.id == a.point))
        .collect();
    scene.standing.points = Arc::clone(&model.attachments);
    info!(
        "glue: character race {} gender {} -> display {display_id}, {} batches, {} attachments",
        wanted.appearance.race,
        wanted.appearance.gender,
        model.draws.len(),
        scene.standing.wanted.len(),
    );
}

/// Hangs the character's outstanding gear on it as each model loads.
///
/// Gear loads after the body, as in `world::entities::worn`: holding the
/// character back until every attachment had loaded would leave the plinth
/// empty for the extra load.
fn dress_the_character(
    mut commands: Commands,
    mut scene: ResMut<GlueScene>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
) {
    let nested_wanted = scene.standing.attached.iter().any(|part| part.wants_nested());
    let Some(root) = scene
        .standing
        .root
        .filter(|_| !scene.standing.wanted.is_empty() || nested_wanted)
    else {
        return;
    };
    let now = time.elapsed_secs();
    let lighting = scene.lighting;
    let points = Arc::clone(&scene.standing.points);
    // Drained rather than iterated: a ready one is hung and removed from the
    // list, and one that will not read is dropped, so a bad model is not
    // retried every frame.
    let mut still_wanted = Vec::new();
    for attachment in std::mem::take(&mut scene.standing.wanted) {
        // No room, because a glue scene has no interior: the character is lit
        // by the scene's lighting.
        match cache.attached(
            &attachment.path,
            attachment.texture.as_deref(),
            None,
            // The wearer's scene lights its gear, as the wearer's room does in
            // the world.
            lighting,
            &mut meshes,
            &mut materials,
        ) {
            Lookup::Loading => still_wanted.push(attachment),
            Lookup::Failed => warn!("glue: attachment {} will not read", attachment.path),
            Lookup::Ready(attached) => {
                let Some((bone, offset)) = points
                    .iter()
                    .find(|p| p.id == attachment.point)
                    .map(|p| (p.bone as usize, p.position))
                else {
                    continue;
                };
                let mut part = crate::world::entities::hang_model(
                    &mut commands,
                    &mut meshes,
                    root,
                    &attached,
                    bone,
                    offset,
                    // Worn, and authored to fit the body it hangs on.
                    1.0,
                    false,
                    None,
                    crate::render::models::sun_scale::NEUTRAL,
                    now,
                );
                // A held item's visual. `SMSG_CHAR_ENUM` carries no
                // enchantment, so the dressing rule chose these from the
                // item's display row alone.
                part.want_nested(attachment.effects);
                scene.standing.attached.push(part);
            }
        }
    }
    scene.standing.wanted = still_wanted;
    // The item-visual models, each a load behind the weapon it hangs on.
    for part in &mut scene.standing.attached {
        crate::world::entities::hang_nested(
            &mut commands,
            &mut cache,
            &mut materials,
            &mut meshes,
            part,
            now,
        );
    }
}

/// Whether the scene on screen is the character-create one.
///
/// Decided by the `<Model>` frame's name rather than by
/// `crate::glue::glue::GlueState::screen`, because the frame is what the
/// renderer already holds and it determines which body stands in the scene.
/// The two normally agree; using the frame means they cannot disagree on the
/// frame a screen changes.
fn on_create_screen(scene: &GlueScene) -> bool {
    scene
        .showing
        .as_ref()
        .is_some_and(|s| s.frame == CREATE_FRAME)
}

/// Takes the character off the plinth, with everything attached to it.
///
/// Despawning the root despawns the batches, the joints and every attachment's
/// root, because all of them are its children; `world::entities` tears an
/// entity down the same way.
fn take_down_character(commands: &mut Commands, standing: &mut Standing) {
    if let Some(root) = standing.root.take() {
        commands.entity(root).despawn();
    }
    *standing = Standing::default();
}

/// Which camera the file offers for the index the interface asked for.
///
/// Falls back to camera 0 rather than to none when `SetCamera(n)` names an
/// `n` the file lacks, because the first camera is the framing the file was
/// authored around.
fn model_camera(model: &ModelAssets, index: u32) -> Option<M2Camera> {
    model
        .cameras
        .get(index as usize)
        .or_else(|| model.cameras.first())
        .copied()
}

/// `Interface\Glues\…\UI_MainMenu.mdx` -> the same path with `.m2`, lower-cased
/// the way the archive chain is keyed.
fn archive_path(file: &str) -> String {
    let file = file.replace('/', "\\");
    match file.to_ascii_lowercase().strip_suffix(".mdx") {
        Some(stem) => format!("{stem}.m2"),
        None => file.to_ascii_lowercase(),
    }
}

/// Spawn the scene's root, its joints and one entity per batch.
///
/// The same arrangement `world::entities::effects::spawn_attached` uses (a root
/// with a `Transform`, N joint entities the skinning reads, and a child per
/// drawn batch) without the parts that concern a wearer: there is no room
/// light, no ground decal and no tint clock, because a glue scene is not an
/// effect on a unit.
///
/// `worn` selects which of the two joint markers to use. The backdrop takes
/// [`GlueJoint`]; the character takes `world::entities::Joint`, because its
/// gear is hung by that module's `hang_model` and posed by its
/// `animate_attachment`, whose queries use that marker. The two must stay
/// distinct: a system holding both mutably needs them provably disjoint, which
/// the `Without` in [`pose_scene`]'s signature provides.
fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    model: &ModelAssets,
    worn: bool,
    joints: &mut Vec<Entity>,
    tinted: &mut Vec<(Entity, vale_assets::world::m2::BatchTint)>,
) -> Entity {
    let root = commands
        .spawn((Transform::default(), Visibility::default()))
        .id();
    // `joint_count`, not `skeleton.is_some()`: the skeleton is present on both
    // builds, and only `joint_count` says whether this build can be posed.
    // See `models::loader`, where testing the skeleton instead left every
    // doodad unanimated.
    if model.joint_count > 0 {
        joints.extend((0..model.joint_count).map(|_| {
            let mut joint = commands.spawn((GlobalTransform::default(), ChildOf(root)));
            if worn {
                joint.insert(Joint);
            } else {
                joint.insert(GlueJoint);
            }
            joint.id()
        }));
    }
    for draw in &model.draws {
        let mut part = commands.spawn((
            Mesh3d(draw.mesh.clone()),
            MeshMaterial3d(draw.material.clone()),
            Transform::default(),
            ChildOf(root),
        ));
        // The declared box, not the bind pose's. The login scene's fire and
        // its cloaked figures move well outside a bind-pose box, and with the
        // camera 5 yards from its middle a box that is too small culls half
        // the picture.
        if let Some(bounds) = model.bounds {
            part.insert(bounds);
        }
        // The batch's own bones, not the model's whole skeleton. `models::skin_for`
        // is the one place that decides the subset, so every spawn site uses
        // the same rule.
        if let Some(joints) = crate::render::models::skin_for(draw, joints) {
            part.insert(SkinnedMesh {
                inverse_bindposes: model.inverse_bindposes.clone(),
                joints,
            });
        }
        // A tinted batch must carry its sampled tag before it is first drawn.
        // The default zero is transparent black, so a batch tagged a frame
        // late is invisible for its first frame.
        if let (Some(tint), Some(tints)) = (draw.tint, &model.tints) {
            let window = model.skeleton.as_ref().and_then(|s| s.sequences.first());
            part.insert(bevy::mesh::MeshTag(crate::render::models::tint_tag(
                tints.sample_in(tint, window, 0, 0),
            )));
            tinted.push((part.id(), tint));
        }
    }
    // The particle emitters. `UI_MainMenu` carries its braziers' fire and the
    // two figures' red glow as particle emitters; without them the scene has
    // no fire and no glow. They follow a joint of the
    // scene's skeleton as a wearer's do, and fall back to the root for a model
    // that has none.
    if let Some(set) = &model.particles {
        let riders = joints.clone();
        crate::render::particles::spawn_emitters(
            commands,
            meshes,
            set,
            root,
            move |_, def| match riders.get(def.bone as usize) {
                Some(joint) => crate::render::particles::Anchor::Joint(*joint),
                None => crate::render::particles::Anchor::Owner,
            },
            1.0,
            None,
        );
    }
    // The ribbon emitters, anchored the same way.
    if let Some(set) = &model.ribbons {
        let riders = joints.clone();
        crate::render::ribbons::spawn_ribbons(
            commands,
            meshes,
            set,
            root,
            move |_, def| match riders.get(def.bone as usize) {
                Some(joint) => crate::render::particles::Anchor::Joint(*joint),
                None => crate::render::particles::Anchor::Owner,
            },
            1.0,
        );
    }
    root
}

/// Pose the scene, and the character standing in it.
fn pose_scene(
    scene: Res<GlueScene>,
    time: Res<Time>,
    mut roots: Query<&mut Transform>,
    // Two of these queries write `GlobalTransform` and two write `MeshTag`, so
    // Bevy requires each pair to be provably disjoint or it rejects the
    // system. The `Without` filters provide that: the backdrop's joints are
    // `GlueJoint` and never `Joint`, and the scene's drawn parts carry no
    // `EntityPart`, which a piece of the character's gear does.
    mut joints: Query<&mut GlobalTransform, (With<GlueJoint>, Without<Joint>)>,
    mut tags: Query<&mut bevy::mesh::MeshTag, Without<EntityPart>>,
    // The character's joints and tags, which use `world::entities`' markers
    // because its gear is hung and animated by that module's functions.
    mut worn_joints: Query<&mut GlobalTransform, With<Joint>>,
    mut worn_tags: Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
) {
    let Some(root) = scene.root else { return };
    // The scene's own facing, which the drag does not change. `SetFacing` on
    // this frame turns the whole backdrop (the portal, the ground, the sky),
    // and neither glue screen calls it. The drag calls
    // `SetCharacterSelectFacing`, which turns the character below and leaves
    // this at zero; see `crate::lua::widgets::model::Scene::character_facing`.
    let facing = scene.showing.as_ref().map_or(0.0, |s| s.facing);
    let scale = scene.showing.as_ref().map_or(1.0, |s| s.scale);
    let placement = Transform {
        rotation: axes::facing(facing),
        scale: Vec3::splat(scale),
        ..default()
    };
    if let Ok(mut transform) = roots.get_mut(root) {
        *transform = placement;
    }
    let raw = ((time.elapsed_secs() - scene.since) * 1000.0) as u32;
    // A sequence loops when bit 0 of its flags is clear. The glue scenes'
    // sequences loop, which keeps the fire burning and the sky drifting.
    let elapsed = match &scene.skeleton {
        Some(skeleton)
            if skeleton
                .sequences
                .get(scene.sequence)
                .is_none_or(|s| s.flags & 1 == 0) =>
        {
            skeleton.phase(scene.sequence, raw)
        }
        _ => raw,
    };
    // Colour animation runs before the skeleton test, because a tinted batch
    // need not be skinned: the sky plane is a rigid quad whose only animation
    // is its colour.
    if let Some(tints) = &scene.tints {
        let window = scene
            .skeleton
            .as_ref()
            .and_then(|s| s.sequences.get(scene.sequence));
        for (part, tint) in &scene.tinted {
            if let Ok(mut tag) = tags.get_mut(*part) {
                tag.0 = crate::render::models::tint_tag(
                    tints.sample_in(*tint, window, elapsed, raw),
                );
            }
        }
    }
    let world_from_model = placement.compute_affine();
    let (Some(skeleton), false) = (&scene.skeleton, scene.joints.is_empty()) else {
        return;
    };
    let pose = skeleton.pose(scene.sequence, elapsed, raw, None, Default::default());
    for (bone, &joint) in pose.iter().zip(scene.joints.iter()) {
        if let Ok(mut transform) = joints.get_mut(joint) {
            *transform = GlobalTransform::from(
                world_from_model * Affine3A::from_mat4(axes::pose_to_bevy(bone)),
            );
        }
    }
    // The identity joint at the end, which weightless vertices follow.
    if let Some(&last) = scene.joints.last() {
        if let Ok(mut transform) = joints.get_mut(last) {
            *transform = GlobalTransform::from(world_from_model);
        }
    }

    // The character, whose frame is the scene's plinth bone. Composed from the
    // pose above rather than from the raw attachment position, because that
    // position is in its bone's frame. The two agree for the six backdrops
    // (their plinth bones are unanimated roots), but the composition keeps the
    // code correct for a backdrop whose plinth bone moves.
    let Some((bone, offset)) = scene.plinth else { return };
    let Some(bone) = pose.get(bone) else { return };
    let world_from_character = world_from_model
        * Affine3A::from_mat4(
            axes::pose_to_bevy(bone)
                * Mat4::from_translation(axes::to_bevy(offset))
                // The drag, applied here and nowhere else. It rotates about
                // the character's up, which after the axis change is Bevy's
                // +Y, so the feet stay on the plinth.
                * Mat4::from_quat(axes::facing(
                    scene.showing.as_ref().map_or(0.0, |s| s.character_facing),
                ))
                // The body's DBC scale, innermost so it does not move the
                // feet off the point. See [`Standing::scale`].
                * Mat4::from_scale(Vec3::splat(scene.standing.scale)),
        );
    pose_character(
        &scene.standing,
        world_from_character,
        time.elapsed_secs(),
        raw,
        &mut roots,
        &mut worn_joints,
        &mut worn_tags,
        &mut tags,
    );
}

/// Poses the character on the plinth: its joints, its colour animation and its
/// gear.
///
/// The same three steps `world::entities::pose` takes for a unit in the world,
/// on a scene that has no unit: the joints, the tinted batches, and each
/// attachment's frame and clock. Separate from [`pose_scene`], which poses the
/// backdrop.
#[allow(clippy::too_many_arguments)]
fn pose_character(
    standing: &Standing,
    world_from_character: Affine3A,
    now: f32,
    now_ms: u32,
    roots: &mut Query<&mut Transform>,
    worn_joints: &mut Query<&mut GlobalTransform, With<Joint>>,
    worn_tags: &mut Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
    tags: &mut Query<&mut bevy::mesh::MeshTag, Without<EntityPart>>,
) {
    let Some(root) = standing.root else { return };
    if let Ok(mut transform) = roots.get_mut(root) {
        *transform = Transform::from_matrix(world_from_character.into());
    }
    let raw = ((now - standing.since) * 1000.0) as u32;
    let Some(skeleton) = &standing.skeleton else {
        return;
    };
    // `Stand` loops, so the character's idle animation repeats.
    let elapsed = skeleton.phase(standing.sequence, raw);
    if let Some(tints) = &standing.tints {
        let window = skeleton.sequences.get(standing.sequence);
        for (part, tint) in &standing.tinted {
            if let Ok(mut tag) = tags.get_mut(*part) {
                tag.0 =
                    crate::render::models::tint_tag(tints.sample_in(*tint, window, elapsed, raw));
            }
        }
    }
    if standing.joints.is_empty() {
        return;
    }
    let pose = skeleton.pose(standing.sequence, elapsed, raw, None, Default::default());
    for (bone, &joint) in pose.iter().zip(standing.joints.iter()) {
        // The character's joints carry `Joint`, as its gear's do, because the
        // gear is hung by `world::entities::hang_model` and posed by that
        // module's functions. The backdrop's joints carry `GlueJoint`, which
        // is why [`pose_scene`] has two joint queries.
        if let Ok(mut transform) = worn_joints.get_mut(joint) {
            *transform = GlobalTransform::from(
                world_from_character * Affine3A::from_mat4(axes::pose_to_bevy(bone)),
            );
        }
    }
    if let Some(&last) = standing.joints.last() {
        if let Ok(mut transform) = worn_joints.get_mut(last) {
            *transform = GlobalTransform::from(world_from_character);
        }
    }
    // The gear's writes, collected as the world's pass collects them and
    // applied here, where the queries are available.
    let mut writes = crate::world::entities::RigWrites::default();
    for one in &standing.attached {
        let Some(local) = one.local(&pose) else { continue };
        if let Ok(mut transform) = roots.get_mut(one.root()) {
            *transform = Transform::from_matrix(local);
        }
        crate::world::entities::animate_attachment(
            one,
            world_from_character * Affine3A::from_mat4(local),
            // No camera basis: nothing a character wears on this screen is
            // billboarded, and the world's camera is the scene's here anyway.
            None,
            now,
            now_ms,
            &mut writes,
        );
    }
    // The roots of the models nested on the gear: an item visual's models on
    // a held weapon. The gear's own roots were written in the loop above.
    writes.apply_roots(roots);
    writes.apply(worn_joints, worn_tags);
}

/// Puts the world camera where the model file's camera is, for as long as a
/// glue scene is up.
///
/// Overwrites what `world::camera::place` wrote, which is why it is ordered
/// after it. The rig is left untouched: it is the world's camera state, and a
/// login screen must not change where the last session's camera was pointing.
fn aim_camera(
    mut scene: ResMut<GlueScene>,
    windows: Query<&Window>,
    mut camera: Query<
        (&mut Transform, &mut Projection, &mut Camera),
        With<crate::world::camera::WorldCamera>,
    >,
) {
    let Ok((mut transform, mut projection, mut lens)) = camera.single_mut() else {
        return;
    };
    let Some(shot) = scene.camera.filter(|_| scene.showing()) else {
        // Restore the world camera's projection, once; see
        // [`GlueScene::restore`]. Nothing is written when there is nothing to
        // restore: a `Mut` dereferenced mutably every frame marks `Projection`
        // changed every frame for the whole session in the world, which costs
        // frame time.
        if let Some(saved) = scene.restore.take() {
            if let Projection::Perspective(perspective) = &mut *projection {
                *perspective = saved;
            }
        }
        set_pillarbox(&mut scene, &mut lens, None);
        return;
    };
    // The file's field of view is converted by `render::lens`, with the
    // vertical angle held at its 16:9 value for wider windows; `render::lens`
    // documents that clamp as a deviation. The world camera uses the same two
    // rules, and past 16:9 its view widens; the glue scene is pillarboxed
    // instead (below).
    let aspect = windows
        .iter()
        .next()
        .map(|w| crate::render::lens::window_aspect(w.width(), w.height()))
        .unwrap_or(crate::render::lens::WIDEST_FRAMED_ASPECT);
    // A window wider than 16:9 shows the scene in a centred 16:9 viewport
    // with black bars at the sides. The glue scenes are authored for at most
    // that shape: past it, the 16:9 vertical angle `framed_vertical_fov` holds
    // lets the camera see beyond the backdrop's edges (the main menu's sky
    // card ends inside a 21:9 view), and the alternative, a narrower vertical
    // angle, crops the character on the plinth.
    let wanted = windows
        .iter()
        .next()
        .and_then(|w| pillarbox(w.physical_size()));
    set_pillarbox(&mut scene, &mut lens, wanted);
    let eye = axes::to_bevy(shot.position);
    let target = axes::to_bevy(shot.target);
    // A degenerate camera, with eye and target in the same place, would make
    // `looking_at` produce a NaN basis and break the whole frame's culling.
    // The parser's guards make it unreachable; the check is cheap.
    if eye.distance_squared(target) < 1e-6 {
        return;
    }
    *transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
    if let Projection::Perspective(perspective) = &mut *projection {
        // Saved before the first write and not after any of them, so that a
        // second scene does not "restore" the first scene's framing.
        if scene.restore.is_none() {
            scene.restore = Some(perspective.clone());
        }
        perspective.fov = crate::render::lens::framed_vertical_fov(shot.fov, aspect);
        // The near and far planes are the file's, not the world's streaming
        // radius. The glue scenes state their own (28 yards for the orc
        // backdrop, 1,778 for the main menu), and a far plane an order of
        // magnitude further out causes z-fighting across the whole picture.
        perspective.near = shot.near_clip.max(0.01);
        perspective.far = shot.far_clip.max(perspective.near * 10.0);
    }
}

/// The centred 16:9 viewport for a window of `size` physical pixels, as
/// `(position, size)`, or `None` when the window is 16:9 or narrower.
fn pillarbox(size: UVec2) -> Option<(UVec2, UVec2)> {
    let (width, height) = (size.x, size.y);
    if height == 0 {
        return None;
    }
    let framed = (height as f32 * crate::render::lens::WIDEST_FRAMED_ASPECT).round() as u32;
    (framed < width).then(|| (UVec2::new((width - framed) / 2, 0), UVec2::new(framed, height)))
}

/// Put `wanted` on the world camera as its viewport, or take the viewport
/// away, and make the area outside it black. Writes nothing when `wanted` is
/// what is already applied.
///
/// The camera is taken as `Mut` so that the early return leaves it untouched:
/// a `&mut Camera` taken from it every frame marks the camera changed every
/// frame, and a changed camera is re-extracted with all of its view state,
/// which measured at 10 ms of extraction a frame.
///
/// Bevy clears the whole output image with the camera's output clear colour
/// before it copies the viewport into it, and that colour is otherwise the
/// global `ClearColor`, which the sky sets to the fog band. So the bars take a
/// black output clear, and the camera's own output mode is restored when the
/// pillarbox goes.
fn set_pillarbox(scene: &mut GlueScene, lens: &mut Mut<Camera>, wanted: Option<(UVec2, UVec2)>) {
    if scene.pillarbox == wanted {
        return;
    }
    scene.pillarbox = wanted;
    match wanted {
        Some((position, size)) => {
            lens.viewport = Some(bevy::camera::Viewport {
                physical_position: position,
                physical_size: size,
                ..default()
            });
            let current = lens.output_mode;
            let own = *scene.restore_output.get_or_insert(current);
            if let bevy::camera::CameraOutputMode::Write { blend_state, .. } = own {
                lens.output_mode = bevy::camera::CameraOutputMode::Write {
                    blend_state,
                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                };
            }
        }
        None => {
            lens.viewport = None;
            if let Some(own) = scene.restore_output.take() {
                lens.output_mode = own;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window wider than 16:9 gets a centred 16:9 viewport; 16:9 and
    /// narrower get none.
    #[test]
    fn only_a_window_wider_than_sixteen_by_nine_is_pillarboxed() {
        assert_eq!(
            pillarbox(UVec2::new(3440, 1387)),
            Some((UVec2::new(487, 0), UVec2::new(2466, 1387)))
        );
        assert_eq!(pillarbox(UVec2::new(1920, 1080)), None);
        assert_eq!(pillarbox(UVec2::new(1024, 768)), None);
        assert_eq!(pillarbox(UVec2::new(0, 0)), None);
    }

    /// The screen decides only whose body is on the plinth; the place is the
    /// same for both (see [`PLINTH_POINT`]).
    ///
    /// The screen is read from the `<Model>` frame rather than from the glue
    /// state, because the frame is what the renderer holds and the two must not
    /// disagree on the frame a screen changes. With no scene showing, the
    /// answer is `false`.
    #[test]
    fn which_screen_is_up_is_read_off_the_model_frames_own_name() {
        let scene = |frame: &str| GlueScene {
            showing: Some(crate::lua::widgets::model::Scene {
                frame: frame.to_string(),
                file: r"Interface\Glues\Models\UI_Human\UI_Human.mdx".to_string(),
                sequence: 0,
                camera: 0,
                fog: None,
                facing: 0.0,
                character_facing: 0.0,
                scale: 1.0,
                unit: None,
                rotation: 0.0,
                elapsed: 0.0,
            }),
            ..GlueScene::default()
        };
        assert!(on_create_screen(&scene(CREATE_FRAME)));
        assert!(!on_create_screen(&scene("CharacterSelect")));
        // With no scene showing, it is not the create screen.
        assert!(!on_create_screen(&GlueScene::default()));
    }

    /// The five queries [`pose_scene`] holds do not conflict. The type system
    /// does not check this.
    ///
    /// Two of them write `GlobalTransform` and two write `MeshTag`, and Bevy
    /// panics on the first update of a system whose mutable accesses are not
    /// provably disjoint. The `Without` in each pair makes them disjoint.
    /// The test needs no archive or window: the conflict is detected when the
    /// system is initialised, before it has anything to pose.
    #[test]
    fn the_pose_systems_queries_do_not_conflict() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<GlueScene>()
            .add_systems(Update, pose_scene);
        app.update();
    }

    /// `.mdx` in the interface becomes `.m2` in the archive, lower-cased as the
    /// archive is keyed. Getting either wrong leaves the login screen with no
    /// background and only a cache miss in the log.
    #[test]
    fn the_interfaces_path_becomes_the_archives() {
        assert_eq!(
            archive_path(r"Interface\Glues\Models\UI_MainMenu\UI_MainMenu.mdx"),
            r"interface\glues\models\ui_mainmenu\ui_mainmenu.m2"
        );
        // `SetBackgroundModel` builds its path with `..` concatenation and the
        // race's own capitalisation, so this is the form that arrives.
        assert_eq!(
            archive_path(r"Interface\Glues\Models\UI_Scourge\UI_Scourge.mdx"),
            r"interface\glues\models\ui_scourge\ui_scourge.m2"
        );
        // A file already named `.m2` is left alone but still normalised.
        assert_eq!(archive_path("Foo/Bar.m2"), r"foo\bar.m2");
    }
}
