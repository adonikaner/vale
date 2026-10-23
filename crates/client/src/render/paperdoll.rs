//! The paper dolls: a unit's whole body drawn into a `<PlayerModel>` frame's
//! own rectangle, which is the drawing half of `SetUnit`.
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
//! ## How this differs from the portraits
//!
//! [`super::portraits`] draws a head into a 64-unit square that a `<Texture>`
//! region carries; this draws a body into the 233x224 rectangle a `<Model>`
//! frame is. The render target, the layer per picture, the studio lighting and
//! the dressing rule are shared. Four things differ:
//!
//! * The camera is the model's kind-1 (character-info) camera, not its kind-0
//!   one. See [`vale_assets::look::portrait::BODY_KIND`], which carries the
//!   census and states which half of that is measured and which is read.
//! * The picture is keyed by frame name, not by unit token. `DressUpModel` and
//!   `CharacterModelFrame` are both pointed at `"player"` and are two pictures
//!   at two sizes; the portrait pass keys by token because a token has one face.
//! * The target is the frame's rectangle in physical pixels times
//!   [`SUPERSAMPLE`], so the projection is built at the frame's own aspect and
//!   the painter filters the image down to the frame.
//! * The doll is live. Its camera draws every frame while the frame is shown,
//!   and the body and everything hung on it play their Stand on a looping clock,
//!   as the 1.12.1 character sheet does. That is one `Core3d` graph per open
//!   doll per frame, about 1.5 ms of CPU encode on the machine the rendering
//!   facts were measured on; the portraits are stills because up to nine of
//!   them are on screen at all times.
//!
//! ## What hangs on a doll
//!
//! The worn models: the helm, the pauldrons and the weapons, which hang on the
//! body's points as in the world, and an item visual's models on a held item's
//! own points. The weapons are put away, as on the 1.12.1 character sheet, so
//! each hangs on its sheath point. Particle emitters and ribbons are not drawn
//! here; a glow's mesh is. See [`hang`].

use std::collections::BTreeMap;
use std::sync::Arc;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::render::mesh::skinning::SkinnedMesh;

use vale_assets::tables::item::{AttachedModel, Weapon};
use vale_assets::tables::itemvisual::ItemEffect;
use vale_assets::world::m2::{M2Attachment, M2Skeleton};

use crate::lua::widgets::model::UnitFrame;
use crate::render::axes;
use crate::render::models::material::Materials;
use crate::render::models::{Lookup, ModelAssets, ModelCache};
use crate::render::offscreen::{self, layers, Picture};
use crate::world::entities::{DisplayCache, Joint};

/// [`crate::ui::report::HudReport`] slot 35, under the portraits' 32 and the
/// interface's 31. The interface requests a paper doll the same way it
/// requests a portrait.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(35);

/// The largest and smallest render target a doll may use, per axis.
///
/// The target is a frame's rectangle in physical pixels, clamped. The upper
/// bound guards against a `uiScale`, window size and display density that
/// together ask for a very large texture. The lower bound guards against a
/// frame whose anchors are not yet resolved, where a zero-sized texture is a
/// validation error. 2,048 is [`SUPERSAMPLE`] times about three times the
/// largest frame the game declares per axis (`DressUpModel`'s 316x351).
const MAX_TARGET: u32 = 2048;
const MIN_TARGET: u32 = 32;

/// How many target pixels a doll draws per screen pixel along each axis.
///
/// At one, the doll's quad is sampled once per screen pixel, so its edges and
/// its textures are no finer than one sample each, where the world behind it
/// is drawn with four samples per pixel and full-resolution texture mips. Two
/// gives each screen pixel four samples of the doll, averaged by the painter's
/// filter, on top of the camera's own four.
const SUPERSAMPLE: f32 = 2.0;

/// Marks the root of one doll's model.
///
/// It is a marker on the spawned entity, and is not used as the filter of a
/// `Query<&mut Transform>` in [`follow`]. When it was used that way, the first
/// frame of every real session panicked with `B0001`: [`follow`] already takes
/// [`crate::interface::api::Units`], whose `all` query reads
/// `Option<&Transform>`, and Bevy cannot prove a filtered write disjoint from
/// an unfiltered read. `With<DollRoot>` narrows which entities are yielded; it
/// does not narrow the declared access.
///
/// The turn is written through `Commands` instead, which conflicts with
/// nothing and costs one command per frame while a rotate button is held. None
/// of this repository's checks caught the panic.
#[derive(Component)]
struct DollRoot;

/// Marks a doll's camera, to tell it apart from the portraits' cameras.
#[derive(Component)]
struct DollCamera;

/// Marks the root of a model hung on a doll: a helm, a pauldron, a weapon, or an
/// item visual on a weapon. [`pose`] writes its `Transform` as a world matrix,
/// so it is a top-level entity rather than a child of [`DollRoot`].
#[derive(Component)]
struct HungRoot;

/// One model hung on a doll, and the models hung on its own points.
///
/// The body's skinned parts are drawn by their joints, which [`pose`] writes
/// as world matrices, so a hung model is placed the same way: its root is
/// given the world matrix of the point it hangs on, and its own joints are
/// that matrix times its own pose. A child of [`DollRoot`] would take the turn
/// a second time.
struct Hung {
    root: Entity,
    joints: Vec<Entity>,
    skeleton: Option<Arc<M2Skeleton>>,
    /// The carrying model's bone, and the point in that bone's frame.
    bone: usize,
    offset: [f32; 3],
    /// This model's own attachment points, which [`Self::nested`] hang on.
    points: Arc<Vec<M2Attachment>>,
    /// An item visual's models on this model's points.
    nested: Vec<Hung>,
    /// Item-visual models asked for on this model's points and still loading.
    pending: Vec<ItemEffect>,
}

impl Hung {
    /// Every root this model and its nested models own, for the teardown.
    fn roots(&self, out: &mut Vec<Entity>) {
        out.push(self.root);
        for nested in &self.nested {
            nested.roots(out);
        }
    }
}

/// One frame's paper doll, and the entities and assets that render it.
struct Doll {
    /// The unit this doll shows. A new unit, a shapeshift or a change of gear
    /// rebuilds the model. The portrait pass makes the same comparison on the
    /// same fields.
    built: Built,
    /// How the doll is shown, which changes the image without changing the
    /// model. It is kept apart from [`Self::built`] because a change to it is
    /// handled differently: a change here re-aims and re-renders, and a change
    /// to `built` removes the model and loads another.
    shown: Shown,
    /// The image, the camera and the body.
    picture: Picture,
    /// When the doll was built, in `Time::elapsed_secs`: the clock its idle
    /// animation runs on.
    since: f32,
    /// The body's attachment points, which the worn models hang on.
    points: Arc<Vec<M2Attachment>>,
    /// The worn models that have loaded and hang on the body.
    hung: Vec<Hung>,
    /// The worn models the dressing asked for that are still loading.
    wanted: Vec<AttachedModel>,
}

/// What a doll shows.
#[derive(Clone, PartialEq)]
struct Built {
    guid: u64,
    display_id: u32,
    appearance: Option<vale_assets::look::character::Appearance>,
    equipment: Vec<(u32, u32)>,
    /// Main hand, off hand, ranged, with their enchantments, which decide the
    /// weapons hung on the doll and their item visuals.
    weapons: [Weapon; 3],
}

/// How a doll is framed and turned, which the interface can change while the
/// unit stays the same.
#[derive(Clone, Copy, PartialEq)]
struct Shown {
    /// The render target's size in pixels: the frame's rectangle at the current
    /// interface scale, clamped.
    size: UVec2,
    /// Radians the subject is turned by, from `SetRotation`.
    rotation: f32,
}

/// Every paper doll on the screen, by the name of the frame it is in.
#[derive(Resource, Default)]
pub struct Dolls {
    taken: BTreeMap<String, Doll>,
}

impl Dolls {
    /// The image for a frame, or `None` while there is none: on the frame a
    /// panel is opened, and on any frame its model is still loading. The
    /// painter then draws nothing rather than a coloured rectangle, the same
    /// rule [`super::portraits`] applies to avoid a white square.
    pub fn image(&self, frame: &str) -> Option<Handle<Image>> {
        Some(self.taken.get(frame)?.picture.image.clone())
    }

    /// How many dolls are kept, for the HUD line.
    pub fn count(&self) -> usize {
        self.taken.len()
    }

    /// The lowest render layer nobody is using.
    fn free_layer(&self) -> Option<usize> {
        offscreen::free_layer(layers::DOLLS, |layer| {
            self.taken.values().any(|d| d.picture.layer == layer)
        })
    }
}

pub struct PaperDollPlugin;

impl Plugin for PaperDollPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(feature = "diagnostics")]
        app.add_systems(Update, report);
        app.init_resource::<Dolls>().add_systems(
            Update,
            (follow, hang, pose)
                .chain()
                // After the entity pass, for the reason [`super::portraits`]
                // gives: that pass creates the `WorldEntity` the unit lookup
                // reads, and shares the dressing rule.
                .after(crate::world::entities::EntitySet),
        );
    }
}

/// The paper doll count, on this pass's own HUD line. [`crate::ui::report`]
/// explains why this is not a line in `hud.rs`.
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

/// Build, re-aim and remove one doll per `<PlayerModel>` frame.
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
    // Where the interface is drawn, which sizes each render target; see
    // [`crate::ui::scale::InterfaceViewport`].
    viewport: Res<crate::ui::scale::InterfaceViewport>,
    time: Res<Time>,
    tabard: Res<crate::interface::tabard::TabardPreview>,
) {
    let Some(host) = host else { return };
    if host.interface().is_none() || !enabled() {
        return;
    }
    let wanted = host.unit_models();
    // Remove every doll not requested this frame; this is how closing a panel
    // takes effect. A doll costs a model, a camera and a render target, and the
    // interface stops naming it as soon as the frame is hidden. Keeping it
    // would render an image that is never shown into a target that is never
    // sampled.
    let stale: Vec<String> = dolls
        .taken
        .keys()
        .filter(|name| !wanted.iter().any(|w| &&w.frame == name))
        .cloned()
        .collect();
    for name in stale {
        take_down(&mut commands, &mut dolls, &name);
    }
    if wanted.is_empty() {
        return;
    }

    for want in wanted {
        let shown = Shown {
            size: target_size(&want, viewport.view, viewport.dpi),
            rotation: want.rotation,
        };
        let Some(mut built) = subject(&units, &want.unit) else {
            // A frame pointed at no unit, such as an open pet panel with no
            // pet, which is the normal state of that panel for every class but
            // two.
            take_down(&mut commands, &mut dolls, &want.frame);
            continue;
        };
        // The tabard designer's model wears the design in the window and not
        // the guild's saved emblem. The design replaces the emblem pair in
        // the equipment, so the doll is rebuilt when a row is cycled.
        if want.frame == TABARD_MODEL {
            if let Some(design) = tabard.0 {
                built.equipment.retain(|pair| !vale_assets::look::emblem::is_entry(pair));
                built
                    .equipment
                    .extend(vale_assets::look::emblem::entry(design.fields(), true));
            }
        }

        // Three cases, cheapest first: nothing changed, only the angle
        // changed, or the subject changed. The first applies on every frame a
        // panel is open and must cost one comparison.
        if let Some(doll) = dolls.taken.get(&want.frame) {
            if doll.built == built && doll.shown == shown {
                continue;
            }
            // The same unit at a new angle and the same target size: re-aim
            // rather than rebuild. A held rotate button takes this branch sixty
            // times a second, and it must not load anything.
            if doll.built == built && doll.shown.size == shown.size {
                let root = doll.picture.root;
                // Written through `Commands`, not a `Query<&mut Transform>`;
                // see [`DollRoot`], which marks the spawn and is not used as a
                // query filter. The root's transform is always a pure
                // rotation, so writing the whole component gives the same
                // value.
                commands
                    .entity(root)
                    .insert(Transform::from_rotation(turn(shown.rotation)));
                if let Some(doll) = dolls.taken.get_mut(&want.frame) {
                    doll.shown = shown;
                }
                continue;
            }
        }

        let kind = unit_kind(&units, &want.unit);
        let Some(display) = displays.resolve(&assets, kind, built.display_id) else {
            continue;
        };
        // A transport is a `.wmo` and has no body to draw, as it has no
        // portrait. `world::entities::spawn_models` skips the same paths for
        // the same reason.
        if display.path.to_ascii_lowercase().ends_with(".wmo") {
            continue;
        }
        let Some(tables) = displays.tables() else {
            continue;
        };
        // The same dressing rule the world uses, through the same function. A
        // separate rule here could dress the character sheet in different gear
        // from the body standing in the world behind the panel.
        //
        // The weapons are put away, as on the 1.12.1 character sheet, so each
        // hangs on its own sheath point.
        let dressed = vale_assets::look::dress::dress(
            tables,
            &display,
            &vale_assets::look::dress::Wearer {
                appearance: built.appearance,
                equipment: &built.equipment,
                weapons: built.weapons,
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
            // Loading or unreadable; ask again next frame. The current doll
            // stays up meanwhile. For a change of gear this is the rule the
            // portrait pass states: keep the old image, because a player's
            // equipment arrives one item at a time behind
            // `CMSG_ITEM_QUERY_SINGLE`.
            continue;
        };
        take_down(&mut commands, &mut dolls, &want.frame);
        let Some(layer) = dolls.free_layer() else {
            continue;
        };
        let mut doll =
            build(&mut commands, &mut images, &mut materials, &model, built, shown, layer);
        doll.since = time.elapsed_secs();
        // The worn models load behind the body, filtered to the points the
        // body carries, as in the world; see [`hang`].
        doll.wanted = dressed
            .attachments
            .into_iter()
            .filter(|a| model.attachments.iter().any(|p| p.id == a.point))
            .collect();
        dolls.taken.insert(want.frame.clone(), doll);
    }
}

/// Whether the paper dolls are drawn. `VALE_NO_PAPERDOLL=1` turns the whole
/// pass off.
///
/// Like every `VALE_NO_*` switch, it exists so a cost can be measured by
/// removing it. Two runs of one build that differ only in this variable
/// measure what [`follow`] costs while no panel is open (one `unit_models()`
/// walk a frame, which should be close to zero) and while a character sheet
/// is open (one more camera drawing every frame).
///
/// Read once. Nothing removes dolls already shown when the value changes,
/// because it cannot change after the first read: `OnceLock` is the same
/// pattern `render::doodads::scenery_culling` uses.
fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VALE_NO_PAPERDOLL").is_none())
}

/// How big a render target the frame wants, in pixels.
fn target_size(want: &UnitFrame, viewport: crate::lua::widgets::layout::Viewport, dpi: f32) -> UVec2 {
    // The viewport's scale is interface units to logical pixels, and the
    // window's scale factor is logical to physical. A target sized in logical
    // pixels on a high-density display is drawn at a fraction of the
    // display's resolution and magnified. The doll is then drawn at
    // [`SUPERSAMPLE`] times that and filtered down by the painter.
    let scale = viewport.scale as f32 * dpi * SUPERSAMPLE;
    UVec2::new(
        ((want.size[0] * scale).round() as u32).clamp(MIN_TARGET, MAX_TARGET),
        ((want.size[1] * scale).round() as u32).clamp(MIN_TARGET, MAX_TARGET),
    )
}

/// Converts `SetRotation`'s radians into the model root's rotation.
///
/// The rotation is about Bevy's `+Y`, which is the model's up: an `.m2` is
/// `+Z` up, and [`axes::to_bevy`] swaps the axes. The model's origin is between
/// its feet on its centreline, so a yaw about it turns the model in place
/// rather than swinging it out of frame.
///
/// The sign comes from the Lua: `Model_RotateLeft` subtracts from the stored
/// angle, so the yaw is negated to make that button turn the character to the
/// viewer's left. No file states the direction directly. The wrong sign would
/// give two working buttons with their labels swapped, which is easy to check
/// on screen; that is why the sign is documented here.
fn turn(rotation: f32) -> Quat {
    Quat::from_rotation_y(-rotation)
}

/// The prefix that names a creature by its display id rather than by a unit
/// token. `SetPetStablePaperdoll` uses it, and it is the only way to show a
/// creature that is not in the world.
///
/// A stabled pet has no guid, no entity and no token. The server sends it as a
/// creature entry, and the display id for that entry arrives in
/// `SMSG_CREATURE_QUERY_RESPONSE`. So the frame is pointed at
/// `"displayid:1234"`, which [`subject`] resolves without looking up any unit
/// in the world.
///
/// It is not one of the game's tokens and is shaped differently on purpose:
/// none of the sixteen real tokens contains a colon, so a mistyped `"target"`
/// is never read as a display id, and a frame pointed at a real unit never
/// takes this path.
pub const DISPLAY_ID_PREFIX: &str = "displayid:";

/// The name of the tabard designer's `<TabardModel>` frame in
/// `TabardFrame.xml`.
const TABARD_MODEL: &str = "TabardModel";

/// What a `<PlayerModel>` should show, or `None` for a name that resolves to
/// no unit.
///
/// The name is either a unit token, used by every caller but one, or a
/// [`DISPLAY_ID_PREFIX`] id, used by the one caller that has no unit (see that
/// constant). A display-id subject has no guid, no appearance and no
/// equipment: it is a creature model only, which is what a stabled pet is.
fn subject(units: &crate::interface::api::Units, token: &str) -> Option<Built> {
    if let Some(display_id) = token.strip_prefix(DISPLAY_ID_PREFIX) {
        return Some(Built {
            guid: 0,
            display_id: display_id.parse().ok()?,
            appearance: Default::default(),
            equipment: Default::default(),
            weapons: Default::default(),
        });
    }
    let id = crate::interface::api::UnitId::parse(token)?;
    let unit = units.get(id)?;
    Some(Built {
        guid: unit.guid,
        display_id: unit.display_id?,
        appearance: unit.appearance,
        equipment: unit.equipment.clone(),
        weapons: unit.weapons,
    })
}

/// The object type of a name's subject, which selects the display table its
/// display id is resolved through.
///
/// A display-id name is always a creature. A stabled pet's id indexes
/// `CreatureDisplayInfo`; looking it up in `CharacterDisplayInfo` would resolve
/// a wolf to a human's skin, or to nothing.
fn unit_kind(
    units: &crate::interface::api::Units,
    token: &str,
) -> vale_protocol::state::update::ObjectType {
    if token.starts_with(DISPLAY_ID_PREFIX) {
        return vale_protocol::state::update::ObjectType::Unit;
    }
    offscreen::unit_kind(units, token)
}

/// Spawn the image, the camera and the model for one doll.
#[allow(clippy::too_many_arguments)]
fn build(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    materials: &mut Materials,
    model: &ModelAssets,
    built: Built,
    shown: Shown,
    layer: usize,
) -> Doll {
    let picture = offscreen::spawn(
        commands,
        images,
        model,
        // The model's character-info camera.
        model.body,
        shown.size,
        layer,
        // Turned by `SetRotation`.
        (
            Transform::from_rotation(turn(shown.rotation)),
            Visibility::default(),
            DollRoot,
        ),
        offscreen::Lens {
            // The frame's own aspect, not the portraits' fixed 4:3. Read at
            // the wrong aspect, a character sheet is framed far too wide and
            // the character's head fills the panel.
            aspect: shown.size.x as f32 / shown.size.y.max(1) as f32,
            // Transparent: the panel's own art is drawn under this rectangle
            // and shows through wherever the character is not.
            clear: Color::NONE,
            // Four samples, the world camera's default, so the doll's edges
            // match the models behind the panel.
            msaa: Msaa::Sample4,
            name: "paperdoll",
        },
        |material| lifted(materials, material),
    );
    commands.entity(picture.camera).insert(DollCamera);

    Doll {
        built,
        shown,
        picture,
        since: 0.0,
        points: model.attachments.clone(),
        hung: Vec::new(),
        wanted: Vec::new(),
    }
}

/// Hang the worn models on each doll as their files load, and the item visuals
/// on the held items.
///
/// Each load starts after the body's and the body does not wait for it, as in
/// the world. The doll's camera draws every frame, so a model appears in the
/// image from the frame its file finishes loading.
#[allow(clippy::too_many_arguments)]
fn hang(
    mut commands: Commands,
    mut dolls: ResMut<Dolls>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for doll in dolls.taken.values_mut() {
        let nested_wanted = doll.hung.iter().any(|h| !h.pending.is_empty());
        if doll.wanted.is_empty() && !nested_wanted {
            continue;
        }
        let layers = RenderLayers::layer(doll.picture.layer);
        let mut still_wanted = Vec::new();
        for worn in std::mem::take(&mut doll.wanted) {
            match cache.attached(
                &worn.path,
                worn.texture.as_deref(),
                None,
                super::portraits::STUDIO,
                &mut meshes,
                &mut materials,
            ) {
                Lookup::Loading => still_wanted.push(worn),
                Lookup::Failed => warn!("paper doll: {} will not read", worn.path),
                Lookup::Ready(model) => {
                    let Some(point) = doll.points.iter().find(|p| p.id == worn.point) else {
                        continue;
                    };
                    let mut hung = spawn_hung(&mut commands, &mut materials, &model, &layers, point, true);
                    let points = Arc::clone(&hung.points);
                    hung.pending = worn
                        .effects
                        .into_iter()
                        .filter(|e| points.iter().any(|p| p.id == e.point))
                        .collect();
                    doll.hung.push(hung);
                }
            }
        }
        doll.wanted = still_wanted;
        for hung in &mut doll.hung {
            let mut still_pending = Vec::new();
            for effect in std::mem::take(&mut hung.pending) {
                // No lighting: an item visual is an unlit glow, as in the world.
                match cache.attached(
                    &effect.path,
                    None,
                    None,
                    crate::render::models::SceneLighting::NONE,
                    &mut meshes,
                    &mut materials,
                ) {
                    Lookup::Loading => still_pending.push(effect),
                    Lookup::Failed => warn!("paper doll: item visual {} will not read", effect.path),
                    Lookup::Ready(model) => {
                        let Some(point) = hung.points.iter().find(|p| p.id == effect.point) else {
                            continue;
                        };
                        let nested =
                            spawn_hung(&mut commands, &mut materials, &model, &layers, point, false);
                        hung.nested.push(nested);
                    }
                }
            }
            hung.pending = still_pending;
        }
    }
}

/// A part's material with the mouseover highlight's lift on it.
///
/// The doll is lit as a hovered or selected unit is in the world: the same
/// `0x40/255` added per channel ([`crate::render::selection::HIGHLIGHT_LIFT`]),
/// through the same interned copy. Without it the studio rig alone leaves the
/// character sheet noticeably darker than a highlighted unit in the world. A
/// material with a moving texture matrix keeps its own, as the selection pass
/// leaves it.
fn lifted(
    materials: &mut Materials,
    material: &Handle<crate::render::models::material::M2Material>,
) -> Handle<crate::render::models::material::M2Material> {
    materials
        .with_highlight(material, crate::render::selection::HIGHLIGHT_LIFT)
        .unwrap_or_else(|| material.clone())
}

/// Spawn one hung model's root, joints and parts on a doll's layer, hanging
/// on `point` of the model that carries it. [`pose`] places it.
fn spawn_hung(
    commands: &mut Commands,
    materials: &mut Materials,
    model: &ModelAssets,
    layers: &RenderLayers,
    point: &M2Attachment,
    // Lit as a highlighted unit, which a worn model is and an item visual's
    // glow is not: the selection pass leaves an unlit glow alone too.
    lift: bool,
) -> Hung {
    let root = commands
        .spawn((Transform::default(), Visibility::default(), HungRoot))
        .id();
    let joints: Vec<Entity> = if model.skeleton.is_some() {
        (0..model.joint_count)
            .map(|_| {
                commands
                    .spawn((GlobalTransform::default(), Joint, ChildOf(root)))
                    .id()
            })
            .collect()
    } else {
        Vec::new()
    };
    // The sequence the model plays at time zero, as for the body.
    let sequence = model
        .skeleton
        .as_ref()
        .and_then(|s| s.best_sequence(&[vale_assets::world::m2::anim::STAND]))
        .unwrap_or(0);
    let window = model
        .skeleton
        .as_ref()
        .and_then(|s| s.sequences.get(sequence));
    for draw in &model.draws {
        let mut part = commands.spawn((
            Mesh3d(draw.mesh.clone()),
            MeshMaterial3d(if lift {
                lifted(materials, &draw.material)
            } else {
                draw.material.clone()
            }),
            Transform::default(),
            ChildOf(root),
            layers.clone(),
            NoFrustumCulling,
        ));
        // A batch whose colour is animated reads its tag as the colour, and
        // the default zero is transparent, so an item visual's glow would be
        // invisible. The tag is its colour at time zero, as in the world.
        if let (Some(tint), Some(tints)) = (draw.tint, &model.tints) {
            part.insert(bevy::mesh::MeshTag(crate::render::models::tint_tag(
                tints.sample_in(tint, window, 0, 0),
            )));
        }
        if let Some(joints) = crate::render::models::skin_for(draw, &joints) {
            part.insert(SkinnedMesh {
                inverse_bindposes: model.inverse_bindposes.clone(),
                joints,
            });
        }
    }
    Hung {
        root,
        joints,
        skeleton: model.skeleton.clone(),
        bone: point.bone as usize,
        offset: point.position,
        points: model.attachments.clone(),
        nested: Vec::new(),
        pending: Vec::new(),
    }
}

/// Pose every doll: the body and the hung models play their idle on the
/// doll's own clock, and every matrix carries the doll's turn.
///
/// The body's skinned parts are drawn by their joints, which replace the
/// mesh's own transform, so the turn on [`DollRoot`] reaches only its
/// unskinned parts; the joints take it here. A hung model is placed on the
/// carrying bone's pose of this frame, so a pauldron moves with the shoulder
/// it rides.
fn pose(
    time: Res<Time>,
    dolls: Res<Dolls>,
    mut joints: Query<&mut GlobalTransform, With<Joint>>,
    mut roots: Query<&mut Transform, With<HungRoot>>,
) {
    let now = time.elapsed_secs();
    let now_ms = (now * 1000.0) as u32;
    for doll in dolls.taken.values() {
        let raw = ((now - doll.since).max(0.0) * 1000.0) as u32;
        let world = Affine3A::from_quat(turn(doll.shown.rotation));
        let body_pose = doll
            .picture
            .skeleton
            .as_ref()
            .map(|skeleton| idle_pose(skeleton, raw, now_ms));
        if let Some(pose) = &body_pose {
            offscreen::write_joints(&mut joints, &doll.picture.joints, pose, world);
        }
        for hung in &doll.hung {
            pose_hung(&mut joints, &mut roots, hung, world, body_pose.as_deref(), raw, now_ms);
        }
    }
}

/// A skeleton's pose `raw` milliseconds into its Stand, looped. A model with
/// no Stand plays its first sequence, as an attached model does in the world.
fn idle_pose(skeleton: &M2Skeleton, raw: u32, now_ms: u32) -> Vec<[f32; 12]> {
    let sequence = skeleton
        .best_sequence(&[vale_assets::world::m2::anim::STAND])
        .unwrap_or(0);
    let elapsed = skeleton.phase(sequence, raw);
    skeleton.pose(sequence, elapsed, now_ms, None, Default::default())
}

/// Place one hung model on the carrying model's pose, then its own nested
/// models on its pose.
///
/// A carrier with no skeleton is in its bind pose, where a point is already in
/// model space. A bone the carrier does not have hangs the model nowhere.
fn pose_hung(
    joints: &mut Query<&mut GlobalTransform, With<Joint>>,
    roots: &mut Query<&mut Transform, With<HungRoot>>,
    hung: &Hung,
    carrier: Affine3A,
    carrier_pose: Option<&[[f32; 12]]>,
    raw: u32,
    now_ms: u32,
) {
    let bone = match carrier_pose {
        Some(pose) => match pose.get(hung.bone) {
            Some(bone) => axes::pose_to_bevy_affine(bone),
            None => return,
        },
        None => Affine3A::IDENTITY,
    };
    let world = carrier * bone * Affine3A::from_translation(axes::to_bevy(hung.offset));
    if let Ok(mut transform) = roots.get_mut(hung.root) {
        *transform = Transform::from_matrix(Mat4::from(world));
    }
    let own_pose = hung
        .skeleton
        .as_ref()
        .map(|skeleton| idle_pose(skeleton, raw, now_ms));
    if let Some(pose) = &own_pose {
        offscreen::write_joints(joints, &hung.joints, pose, world);
    }
    for nested in &hung.nested {
        pose_hung(joints, roots, nested, world, own_pose.as_deref(), raw, now_ms);
    }
}

/// Remove one doll: the model, the camera and the image. The image asset is
/// freed when the doll's handle, the last strong one, is dropped here.
fn take_down(
    commands: &mut Commands,
    dolls: &mut Dolls,
    frame: &str,
) {
    let Some(doll) = dolls.taken.remove(frame) else {
        return;
    };
    doll.picture.take_down(commands);
    let mut roots = Vec::new();
    for hung in &doll.hung {
        hung.roots(&mut roots);
    }
    for root in roots {
        commands.entity(root).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doll(camera: Entity, joints: Vec<Entity>, layer: usize) -> Doll {
        Doll {
            built: Built {
                guid: 0,
                display_id: 0,
                appearance: None,
                equipment: Vec::new(),
                weapons: Default::default(),
            },
            shown: Shown {
                size: UVec2::new(233, 224),
                rotation: 0.61,
            },
            picture: Picture {
                image: Handle::default(),
                camera,
                root: Entity::from_raw_u32(9).expect("an entity id"),
                joints,
                skeleton: None,
                layer,
            },
            since: 0.0,
            points: Arc::new(Vec::new()),
            hung: Vec::new(),
            wanted: Vec::new(),
        }
    }

    /// Doll layers are handed out one per doll and run out rather than
    /// repeating. A sixth `<PlayerModel>` from an addon gets no layer.
    #[test]
    fn layers_are_handed_out_one_apiece_and_run_out() {
        let mut dolls = Dolls::default();
        let mut used = Vec::new();
        for i in 0..layers::DOLLS.len() {
            let layer = dolls.free_layer().expect("a free layer");
            assert!(!used.contains(&layer), "layer {layer} handed out twice");
            used.push(layer);
            dolls.taken.insert(
                format!("frame{i}"),
                doll(Entity::from_raw_u32(1).expect("an entity id"), Vec::new(), layer),
            );
        }
        assert_eq!(dolls.free_layer(), None, "and they run out");
    }

    /// Every doll camera draws before the world camera, which is order 0, and
    /// no two share an order. If two cameras shared an order, a target could be
    /// sampled before it was drawn into: a panel one frame old, or empty on the
    /// first frame.
    #[test]
    fn every_doll_camera_draws_before_the_world_and_none_share_an_order() {
        let orders: Vec<isize> = layers::DOLLS.map(offscreen::order).collect();
        assert!(orders.iter().all(|o| *o < 0));
        let mut sorted = orders.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), orders.len());
    }

    /// A target is the frame's rectangle in pixels, clamped at both ends. The
    /// lower bound matters most: a frame that is shown but whose anchors are not
    /// yet resolved has an empty rectangle, and a zero-sized texture is a wgpu
    /// validation error.
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
        let sheet = target_size(&want(233.0, 224.0), viewport, 1.0);
        let scale = viewport.scale as f32;
        assert_eq!(sheet.x, (233.0 * scale * SUPERSAMPLE).round() as u32);
        assert!(sheet.x >= MIN_TARGET && sheet.y >= MIN_TARGET);
        // A display at twice the density asks for twice the pixels.
        let dense = target_size(&want(233.0, 224.0), viewport, 2.0);
        assert_eq!(dense.x, (233.0 * scale * 2.0 * SUPERSAMPLE).round() as u32);
        // The lower and upper bounds.
        assert_eq!(target_size(&want(0.0, 0.0), viewport, 1.0), UVec2::splat(MIN_TARGET));
        assert_eq!(
            target_size(&want(100_000.0, 100_000.0), viewport, 1.0),
            UVec2::splat(MAX_TARGET)
        );
    }

    /// `Model_OnLoad`'s default of 0.61 is a rotation, and zero is the
    /// identity. The test checks that the two are different rotations and that
    /// the rotation is about the vertical axis: a yaw, not a roll. An earlier
    /// stub's note described `SetRotation` as a roll.
    #[test]
    fn the_turn_is_a_yaw_and_the_files_own_default_is_not_the_identity() {
        assert_eq!(turn(0.0), Quat::IDENTITY);
        let default = turn(0.61);
        assert!(default.angle_between(Quat::IDENTITY) > 0.5);
        // About Bevy's up axis, so a point in front of the model moves
        // sideways and stays at its height.
        let front = default * Vec3::new(0.0, 0.0, 1.0);
        assert!((front.y).abs() < 1e-6, "a yaw does not lift anything: {front}");
        assert!(front.x.abs() > 0.1, "and it does move it sideways: {front}");
    }
}
