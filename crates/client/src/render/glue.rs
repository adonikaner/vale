//! **The 3D scene behind the login and character screens**, drawn through the
//! camera the model itself carries.
//!
//! ```text
//! lua::model::visible   which <Model> frame is up, and what it holds
//!   -> ModelCache        Interface\Glues\Models\UI_MainMenu\UI_MainMenu.m2
//!   -> one root + joints the same shape an attached effect is spawned in
//!   -> M2Camera 0        …and the eye, the aim and the field of view
//! ```
//!
//! ## This is most of what those two screens *are*
//!
//! `<ModelFFX name="AccountLogin" setAllPoints="true" file="…UI_MainMenu.mdx">`
//! fills the window and everything the interface draws is on top of it — the
//! portal, the two cloaked figures, the fire, the burning sky. Character select
//! is the same element with `SetModel` called per race. So a client that loads
//! `Interface\GlueXML\` and does not draw this has a correct login *form* on a
//! black rectangle, which is exactly what the egui stand-in this replaces was.
//!
//! ## The camera is the file's, and that is the whole of the framing
//!
//! `AccountLogin_OnLoad`'s second line is `this:SetCamera(0)` and nothing else
//! in either screen says where to stand. `vale_assets::world::m2::M2Camera` is that
//! camera — an eye, an aim and a **vertical** field of view, all in model space
//! — and `vale glue` prints the seven the glue models carry. Guessing
//! instead would draw the same geometry from the wrong place, which is the
//! failure mode this project's rendering note is about: plausible rather than
//! broken.
//!
//! ## …and the character standing in it
//!
//! Character select is a backdrop **and a character**, and the backdrop says
//! where the character stands: every one of the six `UI_<race>.m2` files carries
//! exactly **two attachment points and no others**, and point 0 is on the
//! camera's own axis, four to five yards in front of it and about a yard below
//! the aim — measured over all six, laterally within 0.15 yards of the axis and
//! vertically 0.75 (dwarf) to 1.24 (night elf) below it, which is the spread of
//! the races' heights. That is a pair of feet. `UI_MainMenu.m2` — the login
//! screen, which has no character — carries none at all, which is the check on
//! the reading. See [`PLINTH_POINT`].
//!
//! The character itself is `vale_assets::look::dress`'d exactly as one in the world
//! is, from `SMSG_CHAR_ENUM`'s own appearance bytes and wardrobe, and the model
//! is `ChrRaces.dbc`'s display id for the race and gender — the one lookup the
//! world never needs, because in the world the server sends a display id.
//!
//! ## Two deviations, all stated — and one that turned out not to be one
//!
//! * **The scene is drawn by the world camera at full screen rather than into
//!   the frame's rectangle.** Both `<Model>` elements that matter here are
//!   `setAllPoints="true"` on `GlueParent`, so the rectangle *is* the screen and
//!   the two agree exactly — but a `<Model>` with a rectangle of its own (the
//!   fourteen portraits in `Interface\FrameXML\`, and character create's own
//!   preview) would need a render target per frame, which this does not do.
//!   That is the shape of the next round on this subject rather than a gap in
//!   this one: nothing in the two screens named here has a smaller rectangle.
//! * **The fog is read and not applied.** `<ModelFFX fogNear="0" fogFar="1200">`
//!   and `CharModelFogInfo`'s per-race colours reach [`crate::lua::widgets::model`] and
//!   stop there; what would use them is the world's own fog uniform, which is
//!   `render::sky`'s and is written from `Light.dbc` for a map nobody is on. The
//!   scenes are authored to look right with it and acceptable without.
//! * ~~**The character on the plinth has empty hands**~~ — **it holds them, and
//!   the deviation this bullet used to state was a wrong inference from a true
//!   measurement.** It said that hanging a weapon needs the item's class,
//!   subclass and sheath type, which `SMSG_CHAR_ENUM` does not carry and no
//!   packet on this screen can fetch. Every word of that is true and the
//!   conclusion does not follow: the reference **never sheathes anything here**.
//!   Its character-select dressing loop calls the weapon attacher with a sheath
//!   type of 0 and a clear "put away" flag, so the point is the hand — and the
//!   one thing it asks about the item is `inventoryType == 14`, which the packet
//!   does carry. The ranged slot is skipped outright. See
//!   [`vale_assets::tables::item::Weapon::from_char_enum`] for the loop.
//!
//! ## The clock is the world's, and `AdvanceTime` is not
//!
//! 1.12 advances a `<Model>` only when its `OnUpdate` calls `AdvanceTime()`, and
//! `CharacterSelect_UpdateModel` does exactly that. This animates on the world's
//! own clock instead — see [`crate::lua::widgets::model::Scene::elapsed`], which records
//! the calls so their *absence* is visible in a test rather than as a frozen
//! login screen.

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

/// **Which of the backdrop's two attachment points the character stands on.**
///
/// Not `m2::attach::SHIELD`, which is what id 0 means on a *character* model:
/// these six scenes are not characters and their two points are the screen's
/// own, one for each of the two glue screens that show a body. Point 0 is
/// character select's — see the module comment for the measurement that says so,
/// which is that it is the one lying on the camera's own axis at standing
/// height.
///
/// **And character create stands its character on the same one** — an
/// inference that used to sit here said point 1, and it was wrong on the screen.
///
/// Both glue screens place their character with the *same call and the same
/// position*: `SetCharacterCreateFacing` and `SetCharacterSelectFacing` both
/// build the model matrix as a translate, then a rotation about **+Z** by the
/// screen's own facing, then a scale — and the translation both hand it is
/// the origin, `{0,0,0}`. So the reference has **one** place a body
/// stands in one of these scenes, and this client expresses it as this point:
/// the reference puts the *backdrop* where its plinth reaches the scene origin,
/// and this puts the backdrop at the origin and the body on its plinth, which is
/// the same picture by the other route.
///
/// What point 1 is remains unknown and is now known **not** to be this. It is
/// 0.77 to 1.74 yards off the camera axis and 4.49 to 10.31 along it, which is
/// where "the characters are positioned in random incorrect places" came from: a
/// dwarf twice as far from the eye as a night elf, and neither of them centred.
const PLINTH_POINT: u32 = 0;

/// The `<Model>` frame the character-create screen is — `CharacterCreate.xml`'s
/// own `<ModelFFX name="CharacterCreate">`, which is what
/// `SetCharCustomizeFrame` names and what [`crate::lua::widgets::model::Scene::frame`]
/// answers while that screen is up.
const CREATE_FRAME: &str = "CharacterCreate";

/// The scene on screen, and what it took to put it there.
#[derive(Resource, Default)]
pub struct GlueScene {
    /// What the interface last said it was holding — the change detector. `None`
    /// while nothing is showing, which is the whole of a session in the world.
    showing: Option<crate::lua::widgets::model::Scene>,
    /// The root everything spawned hangs off, despawned whole when the scene
    /// changes.
    root: Option<Entity>,
    /// One per joint, plus the identity joint at the end — the same arrangement
    /// `world::entities` uses, and for the same reason: a weightless vertex has
    /// to ride something.
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    /// **Every batch whose colour is animated, and which track animates it.**
    ///
    /// A tinted batch's material reads its `MeshTag` as `0xAARRGGBB` rather than
    /// as a room light, so a batch left at the default zero draws as fully
    /// transparent black — see [`crate::render::models::tint_tag`]. On this
    /// model that is not a subtlety: `UI_MainMenu`'s **sky** is a tinted batch,
    /// and with the tag unwritten the login screen's burning horizon was a flat
    /// grey rectangle the size of the window.
    tinted: Vec<(Entity, vale_assets::world::m2::BatchTint)>,
    /// The colour and transparency tracks those indices point into.
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
    /// Which sequence is playing, resolved once at the spawn.
    sequence: usize,
    /// `Time::elapsed_secs` when it started.
    since: f32,
    /// The camera the file chose, or `None` for a model that carries none —
    /// which leaves the world camera where it was.
    camera: Option<M2Camera>,
    /// **The world camera's own projection, kept while the glue borrows it.**
    ///
    /// [`aim_camera`] writes the *model's* field of view and clip planes onto
    /// the one world camera, and a login screen that never gave them back is a
    /// world played at 42 degrees and clipped at the login scene's own far plane
    /// — visibly a much narrower view than the client had a moment before, with
    /// nothing in any log about it. This is what puts them back, once, on the
    /// frame the scene goes.
    restore: Option<PerspectiveProjection>,
    /// **Where a character would stand in this scene**, as a bone of the
    /// backdrop's own skeleton and a point in that bone's frame — the backdrop's
    /// [`PLINTH_POINT`], resolved once at the spawn. `None` for the login
    /// screen's model, which carries no attachment at all.
    plinth: Option<(usize, [f32; 3])>,
    /// …and who is standing there.
    standing: Standing,
    /// **What the scene states about its own lighting**, resolved once at the
    /// spawn and handed to everything drawn *in* the scene — the backdrop, the
    /// character on the plinth, and whatever that character is wearing. See
    /// [`SceneLighting`], and `render::glue`'s own module note for why the
    /// world's daylight is the wrong answer here.
    lighting: SceneLighting,
}

/// The character on the plinth: what it was built from, and what that cost.
///
/// A struct rather than nine more fields on [`GlueScene`] because it is torn
/// down and rebuilt as one — every time the highlight moves to a row with a
/// different face, which on this screen is often.
///
/// `Default` is hand-written for one field: [`Self::scale`] is a multiplier and
/// its derived default would be **zero**, which is a character collapsed to a
/// point rather than an empty plinth.
struct Standing {
    /// **What this was built from**, and the whole change detector: the
    /// appearance and wardrobe `game::glue` derived from the highlighted row.
    /// `None` while nothing is standing there.
    built: Option<crate::game::session::glue::Plinth>,
    /// The root the batches and joints hang off. Its `Transform` is written
    /// every frame by [`pose_scene`] — a **root** entity rather than a child of
    /// the scene's own root, because its skinned parts are posed through
    /// world-space joints and composing two parents would be a second place for
    /// the placement to be decided.
    root: Option<Entity>,
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    tinted: Vec<(Entity, vale_assets::world::m2::BatchTint)>,
    tints: Option<Arc<vale_assets::world::m2::M2Tints>>,
    /// The character's own `Stand`, resolved once at the spawn.
    sequence: usize,
    since: f32,
    /// **`CreatureDisplayInfo`'s own scale for this body**, which is 1.00 for
    /// twelve of the sixteen player models and **1.35 and 1.25** for the male
    /// and female tauren. Applied here for the same reason the world applies it:
    /// without it a tauren stands a quarter too short, which reads as the plinth
    /// being wrong rather than as a missing multiply.
    scale: f32,
    /// Attachments whose own M2 has not landed yet — a helm, a pair of
    /// pauldrons. Drained by [`dress_the_character`] as each one arrives, on the
    /// same terms `world::entities::worn` drains a wearer's: the character is
    /// drawn as soon as its own body is ready and the gear catches up.
    wanted: Vec<AttachedModel>,
    /// …and the ones that have.
    attached: Vec<AttachedPart>,
    /// The **character's** own attachment points, which is what an arriving
    /// pauldron is placed by. Empty while nobody is standing there.
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
/// **Its own marker rather than `world::entities::Joint`**, deliberately: that
/// one is walked by the entity pose systems, which read a `WorldEntity` and a
/// session. A glue scene has neither and must not be visited by them.
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
/// It is not `WorldTuning::interface`. That switch is the widget tree's walk and
/// paint and states so; this is whether the 3D scene behind those widgets is
/// built at all, which is a different subtraction and a different cost.
///
/// Turning it off tears down whatever is up, on the next frame.
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
                // **After the backdrop**, which is what says where a character
                // stands: `GlueScene::plinth` is resolved from the backdrop's
                // own attachment table at its spawn, so a character built before
                // it would be built with nowhere to be.
                follow_character,
                dress_the_character,
                pose_scene,
                // **After `camera::place`**, which writes the world camera's
                // transform from the rig every frame. This overwrites it, and
                // an ordering left to Bevy would give a login screen whose
                // framing flickers between the file's camera and the rig's.
                aim_camera.after(crate::world::camera::place),
            )
                .chain(),
        )
        // **After the three passes' own visibility switches**, which is the
        // whole of why this is a system and not a branch: `tuning::switch`
        // writes every frame the settings change and on every `Added`, so a
        // decision made before it is a decision overwritten.
        .add_systems(Update, stand_the_world_down.after(follow_scene));
    }
}

/// **Take the *world's* sky down while a glue scene is up.**
///
/// The scene carries its own — `UI_MainMenu.m2`'s batch 6 is
/// `MM_SKY_01.BLP` and batches 14, 18 and 19 are two layers of drifting cloud
/// on their own texture matrices — and the world's dome is drawn **behind
/// everything by rule rather than by distance**: `frag_depth = 0.0` with a
/// `GreaterEqual` test (see [`crate::render::sky`]). So it does not merely sit
/// behind the login scene, it *replaces* its sky, and the failure is the flat
/// blue-grey rectangle the burning horizon should be. The stars and the sun and
/// moons go with it for the same reason: `Light.dbc`'s answer for a map nobody
/// is on has nothing to do with this picture.
///
/// Written here rather than folded into each of the three passes because the
/// condition is about the *glue* rather than about the sky — the same reason
/// `render::decals` owns its own retirement rather than the entity pass owning
/// it. Each pass keeps its own switch and this one wins while a scene is up.
fn stand_the_world_down(
    mut commands: Commands,
    scene: Res<GlueScene>,
    camera: Query<Entity, With<crate::world::camera::WorldCamera>>,
    mut world_sky: ParamSet<(
        Query<&mut Visibility, With<crate::render::sky::SkyDome>>,
        Query<&mut Visibility, With<crate::render::stars::StarDome>>,
        Query<&mut Visibility, With<crate::render::celestial::CelestialBody>>,
    )>,
    // **The fog's owner, told to take the camera back** — see the give-back
    // branch. `ResMut` only to reach `set_changed`; nothing here reads or
    // writes a field of it.
    mut sky: ResMut<crate::render::sky::Sky>,
    mut borrowed: Local<bool>,
) {
    // **Given back on the edge, once**, which is the half a first draft of this
    // leaves out: `tuning::switch` writes a pass's visibility only when the
    // settings change or the entity is new, so a sky hidden here and never
    // un-hidden is a *world* with no sky, no stars and no moons for the rest of
    // the session, with nothing in any log about it. The same shape as
    // [`GlueScene::restore`] one field over, and the same cost when missed.
    if !scene.showing() {
        if std::mem::take(&mut *borrowed) {
            // **…and the fog is given back the same way, which was the
            // login-fog bug.** While the glue shows, the branch below inserts
            // the *scene's* fog on the world camera every frame — including
            // the frames after a world entry, where it silently overwrote the
            // `DistanceFog` `render::sky::apply` had just written (there is no
            // ordering between the two, and this one ran later). Nothing then
            // re-applied until the next sky re-resolve, which is the player's
            // first 4-yard cell crossing — so every login stood in the char
            // screen's pink fog, with `NightAir` zeroed and therefore **every
            // lamp pool dark** despite 24 burning point lights, until the
            // first step. Measured end to end: `light: resolved … night 1.00`,
            // `sky: applied … air.x 1.000 to 1 camera(s)`, `lamps: 24 lit` —
            // and a pale screenshot 20 seconds later.
            //
            // The give-back is one line because the fog has one owner:
            // marking `Sky` changed makes `apply` re-run on the next frame and
            // rewrite everything the glue may have disturbed — the fog, the
            // dome stops, the sun, the fill and the clear colour — from the
            // atmosphere it already resolved. Restoring a *cached* fog here
            // instead would race `apply` on the shared frame and lose whenever
            // the world re-resolved during the glue (a logout in one map, a
            // login in another).
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
    // **And the world's fog with it, which is the half that was invisible.**
    // `render::sky` puts a `DistanceFog` on the world camera from `Light.dbc`'s
    // `fog_start..fog_end` for whichever map is current — a few hundred yards —
    // and sets the clear colour to the same band. `UI_MainMenu` is **1,200
    // units deep**: its backdrop quad (`MM_SKY_01.BLP`) and its two drifting
    // cloud layers sit at about 1,100, so every one of them was blended
    // *exactly* to the clear colour. On screen that is not "a fogged sky", it is
    // no sky at all — a flat blue-grey rectangle where the burning horizon
    // should be, indistinguishable from a batch that failed to draw.
    //
    // **Pushed out rather than removed, and that is not a nicety.** `m2.wgsl`
    // reads `bevy_pbr::mesh_view_bindings::fog` unconditionally, so a camera
    // with no `DistanceFog` component fails to *compile the shader* — four
    // `no definition in scope for identifier` lines and a window painted
    // entirely in the clear colour, interface included. Measured, once, by
    // trying it.
    //
    // The scene states its own fog (`fogNear="0" fogFar="1200"`, and per-race
    // colours through `SetFogColor`), and where it names a colour that is what
    // is applied — character select's six races each do. The **login screen
    // names no colour at all**, and its 1,200-unit distance with a black default
    // would fog its own sky to black, which is not what the reference client
    // shows; so with no colour stated the fog is moved past the far plane, which
    // is a scene drawn as authored.
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
    // **Compared on the file and the sequence, not on the whole record.** The
    // facing changes every frame of a drag and the elapsed time changes every
    // frame full stop; rebuilding the scene on either would respawn a 9,892
    // vertex model sixty times a second.
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
    // **The character goes with the backdrop**, because where it stands is the
    // backdrop's own attachment point: a character left standing while the scene
    // it was placed in is replaced is a character floating wherever the last
    // race's plinth was. `follow_character` builds it again on the next frame,
    // from the same record, against the new scene.
    take_down_character(&mut commands, &mut scene.standing);
    scene.showing = wanted.clone();
    let Some(wanted) = wanted else { return };

    // `.mdx` in the interface, `.m2` in the archive — the same rename every DBC
    // in this game needs; see `assets::dress`, which states it for the wardrobe.
    let path = archive_path(&wanted.file);
    // **The lights the file states, before anything is dressed by them** — see
    // [`SceneLighting`] and `vale_assets::world::m2::M2Light`. They are part of the
    // dressing's identity, so they have to be in hand first; `model_lights`
    // answers off the geometry build and requests the file exactly as the
    // dressing would.
    //
    // **A lamp's place is its position and nothing else**, which is a
    // measurement rather than a shortcut: every light bone in all thirteen glue
    // models is a parentless root whose pivot *is* that position, carrying no
    // translation track — `vale glue` checks all 23 and reports how many are
    // not. And the scene's own root is the identity (neither glue screen ever
    // calls `SetFacing` or `SetModelScale` on its `<Model>`), so model space is
    // world space and the position needs no composition at all.
    let Some(lights) = cache.model_lights(&path) else {
        scene.showing = None;
        return;
    };
    let lighting = SceneLighting::resolve(&lights, axes::to_bevy);
    let Lookup::Ready(model) = cache.as_scene(&path, lighting, &mut meshes, &mut materials) else {
        // Loading, or a file the archive does not have. Nothing is shown and the
        // *record* is kept as `showing`, so the next frame does not re-request:
        // `ModelCache::lookup` is already idempotent, and re-entering this arm
        // is what polls it.
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
    // **Sequence 0, because that is what both screens ask for**, clamped to what
    // the file has. `SetSequence(0)` is the first line of both `OnLoad`s and
    // neither ever calls it again.
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

/// **Put the highlighted character on the plinth**, and take the last one off.
///
/// The whole of the decision — which row, what it looks like, what it is wearing
/// — is `crate::game::session::glue::stand_on_plinth`'s and arrives here as a
/// [`crate::game::session::glue::Plinth`]. What is left is the half that needs a
/// renderer: a display id, a dressing, a mesh per batch and a joint per bone.
fn follow_character(
    mut commands: Commands,
    mut scene: ResMut<GlueScene>,
    state: Res<crate::game::session::glue::GlueState>,
    mut cache: ResMut<ModelCache>,
    mut displays: ResMut<crate::world::entities::DisplayCache>,
    assets: Res<crate::assets::GameAssets>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
) {
    // **Nobody stands on the login screen**, and `plinth` being `None` is how
    // the scene itself says so — `UI_MainMenu.m2` carries no attachment point.
    // So this is not a test of which screen is up: it is the model's own answer.
    //
    // **The two screens stand their character in the same place** — see
    // [`PLINTH_POINT`], where the two call sites that prove it are — so the
    // screen decides only *whose* body it is: the highlighted character, or the
    // one being made.
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

    // **`ChrRaces.dbc`, which nothing else in this client reads.** In the world
    // a unit's model is `UNIT_FIELD_DISPLAYID` off the wire; character select is
    // the one screen where the server has said nothing at all, because nothing
    // has been logged into.
    let display_id = match displays.tables_now(&assets) {
        Some(tables) => tables.race_display(wanted.appearance.race, wanted.appearance.gender),
        // The archives will not open. The record is cleared so the next frame
        // asks again rather than deciding the plinth is settled — the same shape
        // `follow_scene` uses for a model still loading.
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

    // **The same rule the world dresses by**, called through the same door: what
    // geosets the head and the gear select, what the helm hides, what hangs off
    // the bones. A second opinion here is exactly what this project avoids, and
    // what `vale dress` would then be checking less than half of.
    //
    // **The hands are full and the state is melee-drawn**, which is the whole of
    // the character-select rule: the reference hands its weapon attacher a
    // sheath type of 0 and a clear "put away" flag, so the point is the hand.
    // `game::glue::Plinth::weapons` is where the packet's two slots become the
    // pair, and `Weapon::from_char_enum` carries the rule.
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
            // **Lit by the scene it is standing in**, which for character
            // select is most of the point: `UI_Orc`'s four lamps sit two to
            // five yards from the plinth with ranges to match, so they are the
            // character's key light rather than the scenery's.
            lighting,
            &mut meshes,
            &mut materials,
        ),
        // A race whose display id resolves to a creature-model row rather than a
        // character one — not a shape 1.12 has, and drawn as the tables name it
        // rather than not at all.
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
        // Still loading, or unreadable. Clearing the record is what makes the
        // next frame ask again; `ModelCache` is idempotent about it.
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
    // **`Stand`, which is what a character on this screen does.** 1.12 plays no
    // greeting animation here: the row is highlighted and the character idles.
    scene.standing.sequence = model
        .skeleton
        .as_ref()
        .and_then(|s| s.best_sequence(&[vale_assets::world::m2::anim::STAND]))
        .unwrap_or(0);
    scene.standing.since = time.elapsed_secs();
    scene.standing.scale = display.scale;
    // Only what this body has a point for, exactly as a wearer in the world is
    // filtered — a model with no shoulder attachment cannot wear pauldrons, and
    // asking the archive for the model anyway loads it to hang it nowhere.
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

/// Hang the character's outstanding gear on it as each model lands.
///
/// The same second-load-behind-the-body shape `world::entities::worn` has, and
/// for the same reason: holding the character back until every pauldron had
/// loaded would leave the plinth empty for the extra round trip.
fn dress_the_character(
    mut commands: Commands,
    mut scene: ResMut<GlueScene>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    time: Res<Time>,
) {
    let Some(root) = scene.standing.root.filter(|_| !scene.standing.wanted.is_empty()) else {
        return;
    };
    let now = time.elapsed_secs();
    let lighting = scene.lighting;
    let points = Arc::clone(&scene.standing.points);
    // Drained rather than iterated: one that is ready is hung and forgotten, one
    // that will not read is dropped, so a bad model is not retried every frame.
    let mut still_wanted = Vec::new();
    for attachment in std::mem::take(&mut scene.standing.wanted) {
        // **No room**, because a glue scene has none: the character is lit by
        // whatever the scene is lit by, not by an interior this screen has no
        // notion of.
        match cache.attached(
            &attachment.path,
            attachment.texture.as_deref(),
            None,
            // The wearer's scene lights its gear, exactly as the wearer's room
            // does in the world.
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
                scene.standing.attached.push(crate::world::entities::hang_model(
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
                ));
            }
        }
    }
    scene.standing.wanted = still_wanted;
}

/// Whether the scene on screen is the **character-create** one.
///
/// By the `<Model>` frame's own name rather than by
/// `crate::game::session::glue::GlueState::screen`, because it is the frame that decides
/// both halves of this question — which attachment point a body stands on, and
/// which body — and the frame is what the renderer already holds. The two agree,
/// and taking the one already in hand means they cannot come apart on the frame
/// a screen changes.
fn on_create_screen(scene: &GlueScene) -> bool {
    scene
        .showing
        .as_ref()
        .is_some_and(|s| s.frame == CREATE_FRAME)
}

/// Take the character off the plinth, whole.
///
/// Despawning the root takes the batches, the joints and every attachment's own
/// root with it, because all of them are its children — the same arrangement
/// `world::entities` tears an entity down by.
fn take_down_character(commands: &mut Commands, standing: &mut Standing) {
    if let Some(root) = standing.root.take() {
        commands.entity(root).despawn();
    }
    *standing = Standing::default();
}

/// Which camera the file offers for the index the interface asked for.
///
/// Falls back to camera 0 rather than to nothing, because `SetCamera(n)` for an
/// `n` the file lacks is the interface asking for a framing that does not exist
/// — and the first one is the framing the file was authored around.
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
/// The same arrangement `world::entities::effects::spawn_attached` uses — a root
/// with a `Transform`, N joint entities the skinning reads, and a child per
/// drawn batch — minus everything that is about a *wearer*: there is no room
/// light, no ground decal and no tint clock, because a glue scene is a scene
/// rather than a spell on somebody.
///
/// `worn` says which of the two joint markers to use, and it is not cosmetic.
/// The **backdrop** is a scene and takes [`GlueJoint`]; the **character** takes
/// `world::entities::Joint`, because its gear is hung by that module's own
/// `hang_model` and posed by its own `animate_attachment`, whose queries are
/// written against that marker. The two must stay distinguishable: a system
/// holding both mutably needs them provably disjoint, which is what the
/// `Without` in [`pose_scene`]'s signature is.
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
    // **`joint_count`, not `skeleton.is_some()`** — the two answer different
    // questions and only this one says whether *this build* can be posed. It
    // was already iterating `0..joint_count`, so the guard was decoration; it
    // is stated correctly now because the skeleton is on both builds. See
    // `models::loader`, where dropping it cost every doodad its animation.
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
        // **The declared box, not the bind pose's.** The login scene's fire and
        // its cloaked figures move well outside a bind-pose box, and a camera
        // sitting 5 yards from the middle of it is exactly the framing where a
        // too-small box culls half the picture.
        if let Some(bounds) = model.bounds {
            part.insert(bounds);
        }
        // **The batch's own bones, not the model's whole skeleton** — see
        // `models::skin_for`, which is the one place that decides it and is a
        // function precisely because this call site was the one the round that
        // introduced the subset forgot.
        if let Some(joints) = crate::render::models::skin_for(draw, joints) {
            part.insert(SkinnedMesh {
                inverse_bindposes: model.inverse_bindposes.clone(),
                joints,
            });
        }
        // **A tinted batch must carry a tag before it is ever drawn**, and the
        // sampled value rather than a placeholder: the default zero is
        // transparent black, so a batch tagged a frame late is a batch that
        // spends the frame the eye arrives on invisible.
        if let (Some(tint), Some(tints)) = (draw.tint, &model.tints) {
            let window = model.skeleton.as_ref().and_then(|s| s.sequences.first());
            part.insert(bevy::mesh::MeshTag(crate::render::models::tint_tag(
                tints.sample_in(tint, window, 0, 0),
            )));
            tinted.push((part.id(), tint));
        }
    }
    // **The emitters, which are the fire and the eyes.** `UI_MainMenu` carries
    // its braziers and the two figures' red glow as particle emitters, so a
    // scene spawned without them is the same geometry unlit and unmoving. They
    // ride a joint of the scene's own skeleton exactly as a wearer's do, and
    // fall back to the root for a model that has none.
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
    // …and the trails, on the same terms.
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
    // **Five queries, and the `Without`s are what make them legal.** Two of them
    // write `GlobalTransform` and two write `MeshTag`, so Bevy needs each pair
    // provably disjoint or it refuses the system outright — the backdrop's
    // joints are `GlueJoint` and never `Joint`, and the scene's drawn parts
    // carry no `EntityPart` where a piece of the character's gear does.
    mut joints: Query<&mut GlobalTransform, (With<GlueJoint>, Without<Joint>)>,
    mut tags: Query<&mut bevy::mesh::MeshTag, Without<EntityPart>>,
    // …and the character's own, which take `world::entities`' markers because
    // its gear is hung and animated by that module's functions.
    mut worn_joints: Query<&mut GlobalTransform, With<Joint>>,
    mut worn_tags: Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
) {
    let Some(root) = scene.root else { return };
    // **The scene's own facing, which is not the drag.** `SetFacing` on this
    // frame turns the whole backdrop — the portal, the ground, the sky — and
    // neither glue screen ever calls it. What the drag writes is
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
    // Looping is the file's own answer — bit 0 clear — and the glue scenes are
    // authored to loop, which is what makes the fire burn and the sky drift.
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
    // **The fade, before the skeleton test**, because a tinted batch need not be
    // skinned — the sky plane is a rigid quad whose whole animation is its
    // colour.
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
    // The identity joint at the end, which weightless vertices ride.
    if let Some(&last) = scene.joints.last() {
        if let Ok(mut transform) = joints.get_mut(last) {
            *transform = GlobalTransform::from(world_from_model);
        }
    }

    // **And the character, whose frame is the scene's plinth bone.** Composed
    // out of the pose above rather than out of the raw attachment position,
    // because that position is written in its bone's frame — the two agree for
    // the six backdrops (their plinth bones are unanimated roots) and the
    // composition is what makes that a *fact about these files* rather than
    // something this code depends on.
    let Some((bone, offset)) = scene.plinth else { return };
    let Some(bone) = pose.get(bone) else { return };
    let world_from_character = world_from_model
        * Affine3A::from_mat4(
            axes::pose_to_bevy(bone)
                * Mat4::from_translation(axes::to_bevy(offset))
                // **The drag, applied here and nowhere else.** About the
                // character's own up, which after the axis change is Bevy's +Y —
                // and a rotation about up leaves the feet on the plinth.
                * Mat4::from_quat(axes::facing(
                    scene.showing.as_ref().map_or(0.0, |s| s.character_facing),
                ))
                // …and the body's own DBC scale, innermost so it does not move
                // the feet off the point. See [`Standing::scale`].
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

/// Pose the character on the plinth, its fade and its gear.
///
/// The same three steps `world::entities::pose` takes for a unit in the world,
/// on a scene that has no unit: the joints, the tinted batches, and each
/// attachment's frame and own clock. Split out only because [`pose_scene`] is
/// about the backdrop and these are two subjects.
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
    // `Stand` loops, which is what makes a character on this screen breathe.
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
        // **The character's joints are `GlueJoint`s and its gear's are
        // `Joint`s**, which is why there are two queries: the body is spawned by
        // this file and the gear by `world::entities::hang_model`, whose markers
        // are that module's.
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
    // applied here, where the queries are to hand.
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
    writes.apply(worn_joints, worn_tags);
}

/// **Stand where the file says**, for as long as a glue scene is up.
///
/// Overwrites what `world::camera::place` wrote, which is why it is ordered
/// after it. The rig is left untouched: it is the *world's* camera state and a
/// login screen must not disturb where the last session's camera was pointing.
fn aim_camera(
    mut scene: ResMut<GlueScene>,
    windows: Query<&Window>,
    mut camera: Query<
        (&mut Transform, &mut Projection),
        With<crate::world::camera::WorldCamera>,
    >,
) {
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };
    let Some(shot) = scene.camera.filter(|_| scene.showing()) else {
        // **Give the world its own camera back**, once. See
        // [`GlueScene::restore`], which is what this takes — and note that
        // nothing is *touched* when there is nothing to give back: a `Mut`
        // dereferenced every frame marks `Projection` changed every frame, for
        // the whole of a session in the world, which is a cost with no symptom
        // but the frame time.
        if let Some(saved) = scene.restore.take() {
            if let Projection::Perspective(perspective) = &mut *projection {
                *perspective = saved;
            }
        }
        return;
    };
    // **The reference's own conversion, clamped at 16:9** — both halves are
    // `render::lens`, and the clamp is the stated deviation there. The world
    // camera takes the same two rules from the same place: it used to be a
    // fixed vertical angle that never read the aspect, which is why this note
    // once said it needed no counterpart.
    let aspect = windows
        .iter()
        .next()
        .map(|w| crate::render::lens::window_aspect(w.width(), w.height()))
        .unwrap_or(crate::render::lens::WIDEST_FRAMED_ASPECT);
    let eye = axes::to_bevy(shot.position);
    let target = axes::to_bevy(shot.target);
    // A degenerate camera — eye and target in the same place — would make
    // `looking_at` produce a NaN basis and take the whole frame's culling with
    // it. The parser's guards make it unreachable; the check costs nothing.
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
        // The near plane is the file's; the far plane is deliberately the
        // file's too rather than the world's streaming radius, because the glue
        // scenes state their own (28 yards for the orc backdrop, 1,778 for the
        // main menu) and a far plane an order of magnitude out is z-fighting
        // across the whole picture.
        perspective.near = shot.near_clip.max(0.01);
        perspective.far = shot.far_clip.max(perspective.near * 10.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Which screen a scene is decides *whose* body is on the plinth**, and
    /// nothing else — the place is the same for both, which is
    /// [`PLINTH_POINT`]'s own note.
    ///
    /// The question has to be asked of the **frame** rather than of the glue
    /// state, because the frame is what the renderer holds and the two must not
    /// come apart on the frame a screen changes. A login screen answers `false`
    /// and has no plinth at all, which is the third case.
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
        // …and a client showing nothing at all is not showing that.
        assert!(!on_create_screen(&GlueScene::default()));
    }

    /// **The five queries [`pose_scene`] holds are legal**, which is not a thing
    /// the type system says.
    ///
    /// Two of them write `GlobalTransform` and two write `MeshTag`, and Bevy
    /// refuses a system whose mutable accesses are not *provably* disjoint — at
    /// run time, on the first update, with a panic. The proof is the `Without`
    /// in each pair, and it is exactly the kind of thing that survives review and
    /// takes the client down on the frame the character screen opens. Nothing
    /// here needs an archive or a window: the conflict is decided when the system
    /// is initialised, before it has anything to pose.
    #[test]
    fn the_pose_systems_queries_do_not_conflict() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<GlueScene>()
            .add_systems(Update, pose_scene);
        app.update();
    }

    /// **`.mdx` in the interface, `.m2` in the archive**, and the case is the
    /// archive's. Getting either wrong is a login screen with no background and
    /// nothing in the log but a cache miss.
    #[test]
    fn the_interfaces_path_becomes_the_archives() {
        assert_eq!(
            archive_path(r"Interface\Glues\Models\UI_MainMenu\UI_MainMenu.mdx"),
            r"interface\glues\models\ui_mainmenu\ui_mainmenu.m2"
        );
        // `SetBackgroundModel` builds its path with `..` concatenation and the
        // race's own capitalisation, so this is the shape that really arrives.
        assert_eq!(
            archive_path(r"Interface\Glues\Models\UI_Scourge\UI_Scourge.mdx"),
            r"interface\glues\models\ui_scourge\ui_scourge.m2"
        );
        // A file already named `.m2` is left alone but still normalised.
        assert_eq!(archive_path("Foo/Bar.m2"), r"foo\bar.m2");
    }
}
