//! **The paper dolls** — a unit's whole body drawn into a `<PlayerModel>`
//! frame's own rectangle, which is `SetUnit`'s other half.
//!
//! ```text
//! lua::widgets::model   CharacterModelFrame:SetUnit("player")  a token + a rectangle
//!   -> Units            …which unit that is, this frame
//!   -> assets::dress + ModelCache      dressed exactly as it is in the world
//!   -> look::portrait::body_framing    …and its own character-info camera
//!   -> one camera, one model, one render layer, one image the size of the frame
//!   -> Dolls            …which the painter draws into the rectangle
//! ```
//!
//! ## This is [`super::portraits`] at a different framing, and the difference is
//! the whole file
//!
//! That pass draws a **head** into a 64-unit square a `<Texture>` region carries;
//! this draws a **body** into a 233x224 one that a `<Model>` frame *is*.
//! Everything structural is shared and deliberately so — the render target, the
//! layer per picture, the studio rig, the settle-and-shutter, the dressing door.
//! Four things are not, and each is a decision rather than an omission:
//!
//! * **The camera is the model's kind-1 camera**, not its kind-0 one. See
//!   [`vale_assets::look::portrait::BODY_KIND`], which carries the census and
//!   states which half of that is measured and which is read.
//! * **The picture is keyed by frame name, not by unit token.** `DressUpModel`
//!   and `CharacterModelFrame` are both pointed at `"player"` and are two
//!   different pictures at two different sizes; the portrait pass can key by
//!   token because a token has exactly one face.
//! * **The target is the size of the frame**, so the projection is built at the
//!   frame's own aspect and the image is not resampled. A portrait's 128x96 is a
//!   constant because every portrait hole in the game is the same square.
//! * **The shutter re-opens.** A portrait is final once taken; a paper doll is
//!   turned by the two rotate buttons under it, so the camera comes back on for
//!   a change of angle and settles again. See [`RESETTLE_FRAMES`].
//!
//! ## The pose is a still, and that is a stated deviation
//!
//! The reference's paper doll idles: the character breathes and shifts. This
//! writes `Stand` at time zero and never advances it, exactly as
//! [`super::portraits::pose`] does, and for the measured reason in the rendering
//! facts — an active `Camera3d` is a whole `Core3d` graph run per frame whatever
//! it is looking at, ~1.5 ms of CPU encode on the machine that was measured on.
//! A doll that idles is that cost for as long as a panel is open. `AdvanceTime`
//! is recorded by [`crate::lua::widgets::model`] and not run here, which is what
//! `Scene::elapsed` is for.
//!
//! **What is not drawn, and it is worth knowing before looking at one**: the
//! attachments. A composed character skin carries the armour that is *painted*
//! on — shirt, chest, gloves, boots — and not the pieces that are models hanging
//! off bones: the helm, the shoulders, the cloak and the weapons. So a doll is
//! the character in their armour with a bare head and bare shoulders. That is
//! the same subtraction [`super::portraits::build`] makes and states, and it
//! costs more here, because a helmet is three pixels of a portrait and a third
//! of a character sheet.

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::mesh::skinning::SkinnedMesh;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

use vale_assets::world::m2::M2Skeleton;

use crate::lua::widgets::model::UnitFrame;
use crate::render::axes;
use crate::render::models::material::Materials;
use crate::render::models::{Lookup, ModelAssets, ModelCache};
use crate::world::entities::{DisplayCache, Joint};

/// **How many paper dolls may be up at once**, and it is the population rather
/// than a budget.
///
/// `Interface\FrameXML\` declares five `<PlayerModel>`-family frames and
/// **four** of them are pointed by `SetUnit`: `CharacterModelFrame`,
/// `PetModelFrame`, `DressUpModel` and `TabardModel`. The fifth,
/// `PetStableModel`, is pointed by `SetPetStablePaperdoll(frame)` — which this
/// client does not answer yet — so it is counted here and never asked for. A
/// sixth would be an addon's.
const MAX: usize = 5;

/// The first render layer these use.
///
/// **Above [`super::portraits`]'s nine**, which start at 1 — layer 0 is the
/// world's, 1..=9 are the faces and these are 10..=14. A collision is two
/// subjects in one picture, which is why the two ranges are stated in terms of
/// each other rather than each starting from wherever was free.
const FIRST_LAYER: usize = super::portraits::FIRST_LAYER + super::portraits::MAX;

/// [`crate::ui::report::HudReport`] slot. 35, under the portraits' 32 and the
/// interface's 31: a paper doll is a picture the interface asked for in the
/// same way a face is.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(35);

/// How many frames a doll's camera renders after its content is final.
///
/// [`super::portraits::SETTLE_FRAMES`]' argument applies unchanged: the content
/// is final when the joints land, and the render world takes a few frames to
/// allocate the meshes and upload the composed skin.
const SETTLE_FRAMES: u8 = 30;

/// …and how many it renders after a change that is only a change of *angle*.
///
/// Two. Nothing has to be allocated or uploaded for a turn — the model is
/// already resident and only the root's rotation moved — so the camera needs
/// the frame that draws it and one of margin. Paying the full
/// [`SETTLE_FRAMES`] for every degree of a held rotate button would keep the
/// camera live for the whole drag and half a second after it, which is the cost
/// this whole shape exists to avoid.
const RESETTLE_FRAMES: u8 = 2;

/// **The biggest render target a doll may ask for**, per axis, and the smallest.
///
/// A frame's rectangle times the interface scale, clamped — against a `uiScale`
/// and a window that between them could ask for something absurd, and against a
/// frame caught mid-anchor-solve, where a zero-sized texture is a validation
/// error rather than a small picture. 1,024 is four times the largest frame the
/// game declares (`DressUpModel`'s 316x351).
const MAX_TARGET: u32 = 1024;
const MIN_TARGET: u32 = 32;

/// **The root of one doll's model.**
///
/// A marker on the spawn, and deliberately *not* the filter of a
/// `Query<&mut Transform>` in [`follow`]. That is what the first version was,
/// and it panicked on the first frame of every real session with `B0001`:
/// [`follow`] already takes [`crate::interface::api::Units`], whose `all` query reads
/// `Option<&Transform>`, and a filter bevy cannot prove disjoint from an
/// unfiltered read does not make the write disjoint. `With<DollRoot>` narrows
/// *which* entities are yielded; it does not narrow the declared access.
///
/// The turn goes through `Commands` instead, which conflicts with nothing and
/// costs one command for the frames a rotate button is actually held. Nothing
/// in this repo's checks caught it.
#[derive(Component)]
struct DollRoot;

/// …and the same for its camera, which [`shutter`] and `render::portraits`'
/// own shutter would otherwise both take unfiltered.
#[derive(Component)]
struct DollCamera;

/// One frame's picture, and everything it took to take it.
struct Doll {
    /// **Who this is of** — a new unit, a shapeshift, a change of gear: any of
    /// them and the model is rebuilt. The same comparison the portrait pass
    /// makes, against the same fields.
    built: Built,
    /// …and **how it is being shown**, which changes the picture without
    /// changing the model. Kept apart from [`Self::built`] because the two are
    /// answered differently: a change here re-aims and re-renders, a change
    /// there tears the model down and loads another.
    shown: Shown,
    image: Handle<Image>,
    /// …and the id egui knows that image by.
    ///
    /// **Both painters, because both are in service.** The mesh painter binds
    /// the `Handle<Image>` and the egui one — still the default until the
    /// parity soak is done — binds a `TextureId` it has to be handed first. See
    /// [`Dolls::texture`].
    texture: bevy_egui::egui::TextureId,
    camera: Entity,
    root: Entity,
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    layer: usize,
    /// Whether the joints have been written; see [`super::portraits`].
    posed: bool,
    /// How many more frames the camera renders before [`shutter`] switches it
    /// off. Reset to [`RESETTLE_FRAMES`] by a turn.
    settle: u8,
}

/// What a doll is a picture of.
#[derive(Clone, PartialEq)]
struct Built {
    guid: u64,
    display_id: u32,
    appearance: Option<vale_assets::look::character::Appearance>,
    equipment: Vec<(u32, u32)>,
}

/// …and how it is framed and turned, which the interface changes without the
/// unit changing at all.
#[derive(Clone, Copy, PartialEq)]
struct Shown {
    /// The render target's size in pixels — the frame's rectangle at the
    /// interface scale in force, clamped.
    size: UVec2,
    /// Radians the subject is turned by, from `SetRotation`.
    rotation: f32,
}

/// **Every paper doll on the screen**, by the name of the frame it is in.
#[derive(Resource, Default)]
pub struct Dolls {
    taken: BTreeMap<String, Doll>,
}

impl Dolls {
    /// The picture for a frame, or `None` while there is not one — which is the
    /// frame a panel is opened on and any frame its model is still loading. The
    /// painter draws nothing rather than a coloured rectangle, which is
    /// [`super::portraits`]' white-square rule one widget kind over.
    pub fn image(&self, frame: &str) -> Option<Handle<Image>> {
        Some(self.taken.get(frame)?.image.clone())
    }

    /// The same picture as the id egui knows it by — the other painter's door.
    /// `Some` and [`Self::image`]'s `Some` coincide by construction: both
    /// fields are written together when a picture lands.
    pub fn texture(&self, frame: &str) -> Option<bevy_egui::egui::TextureId> {
        Some(self.taken.get(frame)?.texture)
    }

    /// How many are being kept, for the HUD line.
    pub fn count(&self) -> usize {
        self.taken.len()
    }

    /// The lowest render layer nobody is using.
    fn free_layer(&self) -> Option<usize> {
        (FIRST_LAYER..FIRST_LAYER + MAX)
            .find(|layer| self.taken.values().all(|d| d.layer != *layer))
    }
}

pub struct PaperDollPlugin;

impl Plugin for PaperDollPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report);
        app.init_resource::<Dolls>().add_systems(
            Update,
            (follow, pose, shutter)
                .chain()
                // **After the entity pass**, for [`super::portraits`]' reason:
                // it is what puts a `WorldEntity` where the unit lookup can see
                // it, and the dressing rule is shared with it.
                .after(crate::world::entities::EntitySet),
        );
    }
}

/// **What the dolls cost**, on the pass's own HUD line — see
/// [`crate::ui::report`], which is why this is not a line in `hud.rs`.
#[cfg(feature = "diagnostics")]
fn report(dolls: Res<Dolls>, mut hud: ResMut<crate::ui::report::HudReport>) {
    if dolls.taken.is_empty() {
        hud.clear(SLOT, "paperdolls");
        return;
    }
    hud.set(
        crate::ui::report::Section::Interface,
        SLOT,
        "paperdolls",
        format!(
            "paper dolls: {} ({})",
            dolls.taken.len(),
            dolls
                .taken
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
}

/// Build, re-aim and tear down one picture per `<PlayerModel>` frame.
#[allow(clippy::too_many_arguments)]
fn follow(
    mut commands: Commands,
    mut dolls: ResMut<Dolls>,
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    units: crate::interface::api::Units,
    mut cache: ResMut<ModelCache>,
    mut displays: ResMut<DisplayCache>,
    assets: Res<crate::assets::GameAssets>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<&mut Camera, With<DollCamera>>,
    windows: Query<&Window>,
    ui_scale: Res<crate::ui::scale::InterfaceScale>,
    mut egui: ResMut<bevy_egui::EguiUserTextures>,
) {
    let Some(host) = host else { return };
    if host.interface().is_none() || !enabled() {
        return;
    }
    let wanted = host.unit_models();
    // **Anything not asked for this frame comes down**, which is what closing a
    // panel is. A doll costs a model, a camera and a render target, and the
    // interface stops naming it the moment the frame is hidden; keeping the
    // last one up is a picture nobody can see and a target nobody samples.
    let stale: Vec<String> = dolls
        .taken
        .keys()
        .filter(|name| !wanted.iter().any(|w| &&w.frame == name))
        .cloned()
        .collect();
    for name in stale {
        take_down(&mut commands, &mut dolls, &mut egui, &name);
    }
    if wanted.is_empty() {
        return;
    }

    let viewport = viewport(&windows, ui_scale.get());
    for want in wanted {
        let shown = Shown {
            size: target_size(&want, viewport),
            rotation: want.rotation,
        };
        let Some(built) = subject(&units, &want.unit) else {
            // A frame pointed at nobody — an open pet panel with no pet, which
            // is the ordinary state of that panel for every class but two.
            take_down(&mut commands, &mut dolls, &mut egui, &want.frame);
            continue;
        };

        // **The three answers, cheapest first.** Nothing changed; only the
        // angle changed; or the subject changed. The first is every frame a
        // panel is open and must cost one comparison.
        if let Some(doll) = dolls.taken.get(&want.frame) {
            if doll.built == built && doll.shown == shown {
                continue;
            }
            // The same person at a new angle and the same target size: re-aim
            // rather than rebuild. A held rotate button is this arm sixty times
            // a second and it must not load anything.
            if doll.built == built && doll.shown.size == shown.size {
                let (root, camera) = (doll.root, doll.camera);
                // **Through `Commands`, not a `Query<&mut Transform>`** — see
                // [`DollRoot`], which is now a marker for the spawn and not for
                // a query. The root's transform is only ever a pure rotation,
                // so writing the whole component is the same value.
                commands
                    .entity(root)
                    .insert(Transform::from_rotation(turn(shown.rotation)));
                if let Ok(mut cam) = cameras.get_mut(camera) {
                    cam.is_active = true;
                }
                if let Some(doll) = dolls.taken.get_mut(&want.frame) {
                    doll.shown = shown;
                    doll.settle = RESETTLE_FRAMES;
                }
                continue;
            }
        }

        let kind = unit_kind(&units, &want.unit);
        let Some(display) = displays.resolve(&assets, kind, built.display_id) else {
            continue;
        };
        // A transport is a `.wmo` and has no body to draw, exactly as it has no
        // portrait — see `world::entities::spawn_models`, which declines the
        // same paths for the same reason.
        if display.path.to_ascii_lowercase().ends_with(".wmo") {
            continue;
        }
        let Some(tables) = displays.tables() else {
            continue;
        };
        // **The same dressing rule the world uses, through the same door.**
        // This project avoids a second opinion, and one here would put a
        // character sheet in different gear from the body standing in the world
        // behind the panel.
        //
        // **Weapons are not carried**, as in the portrait pass: they are
        // attachments rather than skin, and nothing here spawns an attachment.
        // See the module note, which states what that costs.
        let dressed = vale_assets::look::dress::dress(
            tables,
            &display,
            &vale_assets::look::dress::Wearer {
                appearance: built.appearance,
                equipment: &built.equipment,
                weapons: Default::default(),
                sheath_state: vale_assets::look::sheath::UNARMED,
            },
        );
        let ready = match &dressed.look {
            Some(look) => cache.dressed_as_character(
                &display.path,
                look,
                dressed.dress,
                dressed.cloak.as_deref(),
                None,
                super::portraits::STUDIO,
                &mut meshes,
                &mut materials,
            ),
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
        let Lookup::Ready(model) = ready else {
            // Loading or unreadable; ask again next frame. Whatever is up stays
            // up meanwhile, which for a change of *gear* is the same
            // keep-the-old-picture rule the portrait pass states — a player's
            // equipment arrives piecemeal behind `CMSG_ITEM_QUERY_SINGLE`.
            continue;
        };
        take_down(&mut commands, &mut dolls, &mut egui, &want.frame);
        let Some(layer) = dolls.free_layer() else {
            continue;
        };
        let doll = build(&mut commands, &mut images, &mut egui, &model, built, shown, layer);
        dolls.taken.insert(want.frame.clone(), doll);
    }
}

/// **Whether the paper dolls are drawn at all** — `VALE_NO_PAPERDOLL=1`
/// turns the whole pass off.
///
/// A kill switch for the reason every `VALE_NO_*` switch exists: a
/// cost nobody can subtract is a cost nobody can check. Two runs of one binary
/// differing in this variable price what [`follow`] costs while no panel is
/// open — one `unit_models()` walk a frame, which is the number that has to be
/// nothing — and what it costs while a character sheet is, which is one more
/// camera until it settles.
///
/// Read once. Nothing takes down the dolls that are already up when it flips,
/// because it cannot flip: `OnceLock` is the same shape
/// `render::doodads::scenery_culling` uses.
fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VALE_NO_PAPERDOLL").is_none())
}

/// The interface's own viewport, built exactly as [`crate::ui::framexml`] builds
/// it — one place decides units-to-pixels and this is a reader of it rather
/// than a second opinion.
fn viewport(windows: &Query<&Window>, ui_scale: f64) -> crate::lua::widgets::layout::Viewport {
    match windows.iter().next() {
        Some(window) => crate::lua::widgets::layout::Viewport::of(
            window.width() as f64,
            window.height() as f64,
            ui_scale,
        ),
        // The headless probes have no window at all; the constant space is what
        // `layout` hands a caller that has none.
        None => crate::lua::widgets::layout::Viewport::of(1600.0, 900.0, ui_scale),
    }
}

/// How big a render target the frame wants, in pixels.
fn target_size(want: &UnitFrame, viewport: crate::lua::widgets::layout::Viewport) -> UVec2 {
    let scale = viewport.scale as f32;
    UVec2::new(
        ((want.size[0] * scale).round() as u32).clamp(MIN_TARGET, MAX_TARGET),
        ((want.size[1] * scale).round() as u32).clamp(MIN_TARGET, MAX_TARGET),
    )
}

/// **`SetRotation`'s radians as the model root's turn.**
///
/// About Bevy's `+Y`, which is the model's own up — an `.m2` is `+Z` up and
/// [`axes::to_bevy`] is what swaps them. The model's origin is between its feet
/// on its centreline, so a yaw about it turns the subject in place rather than
/// swinging it out of frame.
///
/// **The sign is a reading**: `Model_RotateLeft` *subtracts* from the stored
/// angle, so a negated yaw is what makes that button turn the character to the
/// viewer's left. Nothing in any file states which way round it goes, and the
/// wrong choice is two working buttons with their labels swapped — which is the
/// cheapest thing on this page to check on screen, and the reason it is stated
/// here rather than buried in the transform.
fn turn(rotation: f32) -> Quat {
    Quat::from_rotation_y(-rotation)
}

/// **The prefix that names a creature by its display id rather than by a unit
/// token** — `SetPetStablePaperdoll`'s way in, and the only one there is for a
/// creature that is not in the world.
///
/// A stabled pet has no guid, no entity and no token: the server states it as a
/// creature *entry*, and the display id behind that entry arrives in
/// `SMSG_CREATURE_QUERY_RESPONSE`. So the frame is pointed at
/// `"displayid:1234"`, which [`subject`] resolves without asking the world
/// about it at all.
///
/// **Not a token the game has**, and deliberately not shaped like one: the
/// colon cannot appear in any of the sixteen real tokens, so a mistyped
/// `"target"` can never be read as one of these and a frame pointed at a real
/// unit can never fall down this path.
pub const DISPLAY_ID_PREFIX: &str = "displayid:";

/// What a `<PlayerModel>`'s subject should be drawn from, or `None` for a name
/// that reaches nobody.
///
/// Two shapes: a **unit token**, which is every caller but one, and a
/// [`DISPLAY_ID_PREFIX`] id, which is the caller that has no unit — see there.
/// A display-id subject carries no guid, no appearance and no equipment,
/// because there is nothing to carry them: it is a creature model and nothing
/// else, which is exactly what a stabled pet is.
fn subject(units: &crate::interface::api::Units, token: &str) -> Option<Built> {
    if let Some(display_id) = token.strip_prefix(DISPLAY_ID_PREFIX) {
        return Some(Built {
            guid: 0,
            display_id: display_id.parse().ok()?,
            appearance: Default::default(),
            equipment: Default::default(),
        });
    }
    let id = crate::interface::api::UnitId::parse(token)?;
    let unit = units.get(id)?;
    Some(Built {
        guid: unit.guid,
        display_id: unit.display_id?,
        appearance: unit.appearance,
        equipment: unit.equipment.clone(),
    })
}

/// …and which display table to resolve it through.
///
/// **A display id is always a creature**, which is the whole reason the two
/// tables are told apart: `CreatureDisplayInfo` is what a stabled pet's id
/// indexes, and reading it in `CharacterDisplayInfo` would resolve a wolf to a
/// human's skin or to nothing.
fn unit_kind(
    units: &crate::interface::api::Units,
    token: &str,
) -> vale_protocol::state::update::ObjectType {
    if token.starts_with(DISPLAY_ID_PREFIX) {
        return vale_protocol::state::update::ObjectType::Unit;
    }
    crate::interface::api::UnitId::parse(token)
        .and_then(|id| units.get(id))
        .map(|unit| unit.kind)
        .unwrap_or(vale_protocol::state::update::ObjectType::Unit)
}

/// Spawn the image, the camera and the model for one doll.
fn build(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    egui: &mut bevy_egui::EguiUserTextures,
    model: &ModelAssets,
    built: Built,
    shown: Shown,
    layer: usize,
) -> Doll {
    let image = images.add(target_image(shown.size));
    let texture = egui.add_image(bevy_egui::EguiTextureHandle::Strong(image.clone()));
    let layers = RenderLayers::layer(layer);

    // --- the model, alone on its layer at the origin, turned by SetRotation ---

    let root = commands
        .spawn((
            Transform::from_rotation(turn(shown.rotation)),
            Visibility::default(),
            DollRoot,
        ))
        .id();
    let mut joints = Vec::new();
    if model.skeleton.is_some() {
        joints.extend((0..model.joint_count).map(|_| {
            commands
                .spawn((GlobalTransform::default(), Joint, ChildOf(root)))
                .id()
        }));
    }
    for draw in &model.draws {
        let mut part = commands.spawn((
            Mesh3d(draw.mesh.clone()),
            MeshMaterial3d(draw.material.clone()),
            Transform::default(),
            ChildOf(root),
            // On every drawn entity: Bevy reads `RenderLayers` per entity and
            // does not propagate it down a hierarchy, and a part left off the
            // layer is a limb the *world* camera draws at the map's origin.
            layers.clone(),
            // The declared box is the widest the model gets over every
            // animation and this draws one still of one pose; a doll culled out
            // of its own panel is an empty rectangle indistinguishable from a
            // model that failed to load. Five entities is not a culling budget
            // worth defending.
            NoFrustumCulling,
        ));
        // **The batch's own bones**, not the model's whole skeleton — see
        // `models::skin_for`. Getting this wrong poses every vertex off the
        // wrong bone, which is a body stretched across the panel.
        if let Some(joints) = crate::render::models::skin_for(draw, &joints) {
            part.insert(SkinnedMesh {
                inverse_bindposes: model.inverse_bindposes.clone(),
                joints,
            });
        }
    }

    // --- and the camera, down the model's character-info axis ---

    let framing = model.body;
    let eye = axes::to_bevy(framing.eye);
    // A degenerate framing is nudged rather than aimed at itself: `looking_at`
    // on a zero-length direction is a NaN basis, and a NaN view matrix takes
    // the whole frame's culling with it. `render::portraits::build` makes the
    // same guard for the same reason.
    let aim = match axes::to_bevy(framing.aim) {
        target if eye.distance_squared(target) < 1e-6 => eye - Vec3::Z,
        target => target,
    };
    let aspect = shown.size.x as f32 / shown.size.y.max(1) as f32;
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                // Before the world's, which is order 0 — a camera drawing into
                // an image that something later in the frame samples has to
                // have finished.
                order: -1 - layer as isize,
                // Transparent: the panel's own art is drawn under this
                // rectangle and shows through wherever the character is not.
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(ImageRenderTarget {
                handle: image.clone(),
                scale_factor: 1.0,
            }),
            // Every model shader in this client ends in `atmosphere::to_frame`
            // and writes the game's own bytes, so a camera left in the default
            // space would encode them a second time and hand the panel a washed
            // out character. See `crate::render::present`.
            bevy::camera::CompositingSpace::Srgb,
            Projection::Perspective(PerspectiveProjection {
                // **`M2Camera::fov` is a diagonal angle** — see
                // `render::lens::vertical_fov` for what settles it
                // — and the aspect it is taken at here is the *frame's*, not
                // the portrait path's fixed 4:3. Taken as vertical instead, a
                // character sheet is framed far too wide and the character's
                // head fills the panel. Unclamped, because this target is not
                // the window: see `render::lens::framed_vertical_fov`.
                fov: crate::render::lens::vertical_fov(framing.fov, aspect),
                near: framing.near.max(0.01),
                far: framing.far.max(framing.near.max(0.01) * 10.0),
                aspect_ratio: aspect,
                ..default()
            }),
            // Nothing here is edge-sampled and nothing is graded, and both are
            // named rather than defaulted because `Camera3d`'s defaults are
            // multisampled and tonemapped.
            Msaa::Off,
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            layers.clone(),
            Transform::from_translation(eye).looking_at(aim, Vec3::Y),
            DollCamera,
            Name::new("paperdoll"),
        ))
        .id();

    Doll {
        built,
        shown,
        image,
        texture,
        camera,
        root,
        joints,
        skeleton: model.skeleton.clone(),
        layer,
        posed: false,
        settle: SETTLE_FRAMES,
    }
}

/// Write the joints, once — [`super::portraits`]' `pose` over this pass's own
/// table. `Stand` at time zero and never advanced; see the module note.
fn pose(mut dolls: ResMut<Dolls>, mut joints: Query<&mut GlobalTransform, With<Joint>>) {
    for doll in dolls.taken.values_mut() {
        if doll.posed || doll.joints.is_empty() {
            continue;
        }
        let Some(skeleton) = &doll.skeleton else {
            doll.posed = true;
            continue;
        };
        let sequence = skeleton
            .best_sequence(&[vale_assets::world::m2::anim::STAND])
            .unwrap_or(0);
        let pose = skeleton.pose(sequence, 0, 0, None, Default::default());
        for (bone, &joint) in pose.iter().zip(doll.joints.iter()) {
            if let Ok(mut transform) = joints.get_mut(joint) {
                *transform = GlobalTransform::from(Affine3A::from_mat4(axes::pose_to_bevy(bone)));
            }
        }
        // The identity joint on the end, which weightless vertices ride —
        // without it the skinning shader collapses them to the origin.
        if let Some(&last) = doll.joints.last() {
            if let Ok(mut transform) = joints.get_mut(last) {
                *transform = GlobalTransform::default();
            }
        }
        doll.posed = true;
    }
}

/// Switch a settled doll's camera off — [`super::portraits`]' `shutter` over
/// this pass's own table, and the same measured argument. The difference is
/// that [`follow`] turns one back on for a change of angle.
fn shutter(mut dolls: ResMut<Dolls>, mut cameras: Query<&mut Camera, With<DollCamera>>) {
    for doll in dolls.taken.values_mut() {
        // Content not final yet: the pose system is still waiting for the joint
        // entities to exist. Keep rendering.
        if !(doll.posed || doll.joints.is_empty()) {
            continue;
        }
        if doll.settle > 0 {
            doll.settle -= 1;
            continue;
        }
        if let Ok(mut camera) = cameras.get_mut(doll.camera) {
            if camera.is_active {
                camera.is_active = false;
            }
        }
    }
}

/// Take one picture down: the model, the camera and the image.
///
/// **Egui's handle goes with it**, for [`super::portraits`]' reason: leaving it
/// registered against an image asset nobody holds is a texture leak of one
/// target per panel open, which for a character sheet opened and closed all
/// session is a memory report nobody can explain.
fn take_down(
    commands: &mut Commands,
    dolls: &mut Dolls,
    egui: &mut bevy_egui::EguiUserTextures,
    frame: &str,
) {
    let Some(doll) = dolls.taken.remove(frame) else {
        return;
    };
    commands.entity(doll.root).despawn();
    commands.entity(doll.camera).despawn();
    egui.remove_image(&doll.image);
}

/// The image a doll is drawn into.
///
/// `Rgba8UnormSrgb` for `render::portraits::target_image`'s reason: the
/// camera's main texture is raw bytes (`CompositingSpace::Srgb`), Bevy's
/// upscaling blit applies `SRGB_TO_LINEAR` on the way out because it assumes
/// the destination re-encodes, and this is the destination.
fn target_image(size: UVec2) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: size.x.max(1),
            height: size.y.max(1),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        // Transparent black, so a frame whose camera has not run yet shows the
        // panel's own art rather than a coloured rectangle.
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.usage =
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doll(camera: Entity, posed: bool, joints: Vec<Entity>, settle: u8, layer: usize) -> Doll {
        Doll {
            built: Built {
                guid: 0,
                display_id: 0,
                appearance: None,
                equipment: Vec::new(),
            },
            shown: Shown {
                size: UVec2::new(233, 224),
                rotation: 0.61,
            },
            image: Handle::default(),
            texture: bevy_egui::egui::TextureId::Managed(0),
            camera,
            root: Entity::from_raw_u32(9).expect("an entity id"),
            joints,
            skeleton: None,
            layer,
            posed,
            settle,
        }
    }

    /// **The dolls' layers never touch the portraits'**, which is the one thing
    /// the two passes share a resource for and the one they cannot get wrong
    /// quietly: a collision is a face and a body drawn into each other's
    /// picture, and both would still be well-formed images.
    #[test]
    fn the_layers_start_where_the_portraits_end_and_never_overlap() {
        let faces: Vec<usize> =
            (super::super::portraits::FIRST_LAYER..super::super::portraits::FIRST_LAYER + super::super::portraits::MAX)
                .collect();
        let bodies: Vec<usize> = (FIRST_LAYER..FIRST_LAYER + MAX).collect();
        assert!(!faces.iter().any(|f| bodies.contains(f)), "{faces:?} {bodies:?}");
        assert!(!bodies.contains(&0), "layer 0 is the world's");
        assert_eq!(bodies.first(), Some(&10));
    }

    /// …and they are handed out one apiece and **run out** rather than
    /// repeating, which is what a sixth `<PlayerModel>` from an addon meets.
    #[test]
    fn layers_are_handed_out_one_apiece_and_run_out() {
        let mut dolls = Dolls::default();
        let mut used = Vec::new();
        for i in 0..MAX {
            let layer = dolls.free_layer().expect("a free layer");
            assert!(!used.contains(&layer), "layer {layer} handed out twice");
            used.push(layer);
            dolls.taken.insert(
                format!("frame{i}"),
                doll(Entity::from_raw_u32(1).expect("an entity id"), true, Vec::new(), 0, layer),
            );
        }
        assert_eq!(dolls.free_layer(), None, "and they run out");
    }

    /// Every doll camera draws **before the world**, which is order 0, and no
    /// two share an order. Two cameras on one order is a target sampled before
    /// it has been drawn into: a panel one frame stale, or empty on the first.
    #[test]
    fn every_doll_camera_draws_before_the_world_and_none_share_an_order() {
        let orders: Vec<isize> = (FIRST_LAYER..FIRST_LAYER + MAX)
            .map(|layer| -1 - layer as isize)
            .collect();
        assert!(orders.iter().all(|o| *o < 0));
        let mut sorted = orders.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), orders.len());
    }

    /// **A target is the frame's rectangle in pixels, and it is clamped at both
    /// ends.** The low end is the one that matters: a frame caught between
    /// being shown and its anchors solving has a rectangle of nothing, and a
    /// zero-sized texture is a wgpu validation error rather than a small
    /// picture.
    #[test]
    fn a_target_is_the_frames_rectangle_in_pixels_clamped_at_both_ends() {
        let viewport = crate::lua::widgets::layout::Viewport::of(1600.0, 900.0, 1.0);
        let want = |w, h| UnitFrame {
            frame: "CharacterModelFrame".into(),
            unit: "player".into(),
            size: [w, h],
            rotation: 0.0,
        };
        // The character sheet's own 233x224, at whatever this window's scale is.
        let sheet = target_size(&want(233.0, 224.0), viewport);
        let scale = viewport.scale as f32;
        assert_eq!(sheet.x, (233.0 * scale).round() as u32);
        assert!(sheet.x >= MIN_TARGET && sheet.y >= MIN_TARGET);
        // …and the two ends.
        assert_eq!(target_size(&want(0.0, 0.0), viewport), UVec2::splat(MIN_TARGET));
        assert_eq!(
            target_size(&want(100_000.0, 100_000.0), viewport),
            UVec2::splat(MAX_TARGET)
        );
    }

    /// **`Model_OnLoad`'s 0.61 is a turn, and zero is not.** The one thing this
    /// pins is that the two are different rotations and that the mapping is
    /// about the vertical axis — a yaw, not a roll, which is what the stub note
    /// this replaced claimed `SetRotation` was.
    #[test]
    fn the_turn_is_a_yaw_and_the_files_own_default_is_not_the_identity() {
        assert_eq!(turn(0.0), Quat::IDENTITY);
        let default = turn(0.61);
        assert!(default.angle_between(Quat::IDENTITY) > 0.5);
        // About Bevy's up, so the axis it moves is the horizontal one: a point
        // in front of the model swings sideways and stays at its height.
        let front = default * Vec3::new(0.0, 0.0, 1.0);
        assert!((front.y).abs() < 1e-6, "a yaw does not lift anything: {front}");
        assert!(front.x.abs() > 0.1, "and it does move it sideways: {front}");
    }

    /// **A settled doll's camera goes off and an unposed one's stays on** —
    /// [`super::super::portraits`]' own measured optimization, over this pass's
    /// table. Deactivating before the pose lands bakes a bind-pose character,
    /// or an empty rectangle, into the panel.
    #[test]
    fn the_shutter_closes_after_settling_and_never_early() {
        let mut app = App::new();
        app.init_resource::<Dolls>().add_systems(Update, shutter);
        let settled = app.world_mut().spawn((Camera::default(), DollCamera)).id();
        let waiting = app.world_mut().spawn((Camera::default(), DollCamera)).id();
        {
            let mut dolls = app.world_mut().resource_mut::<Dolls>();
            dolls
                .taken
                .insert("posed".into(), doll(settled, true, Vec::new(), 2, FIRST_LAYER));
            // Skinned and not yet posed: the joints exist and the pose has not
            // landed, so the countdown must not even start.
            dolls.taken.insert(
                "waiting".into(),
                doll(waiting, false, vec![settled], 2, FIRST_LAYER + 1),
            );
        }
        for _ in 0..3 {
            app.update();
        }
        assert!(!app.world().entity(settled).get::<Camera>().unwrap().is_active);
        assert!(app.world().entity(waiting).get::<Camera>().unwrap().is_active);
        let dolls = app.world().resource::<Dolls>();
        assert_eq!(dolls.taken.get("waiting").unwrap().settle, 2);
    }

    /// **A turn re-opens the shutter**, which is the one behaviour that is this
    /// pass's and not the portrait pass's: a portrait is final once taken, and
    /// a paper doll has two buttons under it whose whole job is to change it.
    /// The budget it re-opens for is [`RESETTLE_FRAMES`] rather than the full
    /// [`SETTLE_FRAMES`], because nothing is loaded for a turn.
    #[test]
    fn a_re_settle_is_shorter_than_a_first_settle() {
        assert!(RESETTLE_FRAMES < SETTLE_FRAMES);
        assert!(RESETTLE_FRAMES >= 1, "a turn still needs the frame that draws it");
    }
}
