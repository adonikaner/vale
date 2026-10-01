//! Selection feedback: the ring on the ground under the target, and the
//! brighten on the model under the pointer.
//!
//! The 1.12.1 client changes both when the selection or the mouseover changes,
//! so they share this file. They are two separate mechanisms, in two halves of
//! the file, and neither is a fallback for the other. The ring is a ground
//! decal drawn through [`super::decals`]; 1.12 uses its ground-decal projector
//! for two things, and the ring is the first of them this client draws. The
//! brighten is a material property.
//!
//! The 1.12.1 client draws the selection circle as a model textured with
//! `Textures\UnitSelectTexture.blp` and projects it onto the ground under the
//! unit. The circle therefore follows a hill and bends down a step instead of
//! lying flat. This module does the same through
//! [`super::decals::spawn_ground_decal`], so it holds only the size and colour
//! rules and no render pass of its own.
//!
//! ## The ring texture carries the shape and the selector supplies the colour
//!
//! The decoded texture is 256x256, palettised with an 8-bit alpha channel, and
//! RGB white in every texel that is not fully transparent. The alpha channel
//! holds the ring: zero out to r ≈ 0.28 of the half-width, a soft ~44/255 fill
//! to 0.85, a 136/255 rim at 0.875, and zero again by 0.97. The colour
//! therefore comes entirely from the selector, [`vale_assets::look::selection`].
//! The blend must keep the alpha ramp, so it is mode 4, `(SrcAlpha, One)`
//! (add-alpha). Mode 3, `(One, One)`, would add white across the whole quad and
//! draw a square.
//!
//! ## Which unit gets a ring
//!
//! One decal, under the selected unit only. The mouseover changes the highlight
//! and gets no ring. The 1.12.1 client draws one circle at a time, under the
//! selected unit. Screenshots of the 1.12.1 client show the same: a ring under
//! the target and none under a unit that is only hovered.
//!
//! ## Numbers this client chooses
//!
//! * The radius. The 1.12.1 client sizes the circle by the unit's footprint.
//!   This client takes the footprint from `UNIT_FIELD_BOUNDINGRADIUS`
//!   ([`WorldEntity::bounding_radius`]) and divides it by [`RIM`], so that the
//!   bright rim of the texture lands on the footprint rather than on the quad's
//!   corner. Which of the client's bounding numbers the circle uses is not
//!   established. The number this module used before is wrong by a factor of
//!   two and a half.
//!
//!   That number was [`EntityModel::shadow_radius`], the M2's declared
//!   half-extent, which the blob shadow also uses. The M2 box covers every
//!   frame of every animation of the model, so it is larger than the footprint.
//!   `Creature\Boar\Boar.m2` declares 2.5 model yards of horizontal half-extent
//!   (`vale model`). At the Rockhide Boar's 0.75 scale and divided by [`RIM`],
//!   that is a ring 4.3 yards across. The server sends 0.882 for the same
//!   creature, which is a ring 2.0 yards across. This ratio caused the report
//!   that the circle was much too large.
//!
//!   The wire value is in world space, and the quad is posed through the unit's
//!   transform, which carries `OBJECT_FIELD_SCALE_X`. This module divides the
//!   scale back out. Without that the scale is applied twice: a tauren gets a
//!   ring half again too big and a gnome gets one too small.
//! * No pulse. Neither the selection ring nor the ground-target reticle pulses.
//!   The reticle is `Spell-Shadow-Acceptable` ([`super::reticle`]); the
//!   1.12.1 client draws it in constant white (`0xffffffff`), and its texture
//!   carries the whole shape.
//!
//! ## The highlight on hovered and selected units
//!
//! The 1.12.1 client lights a unit while it is hovered or selected, and the
//! glow stays until both have ended. This module models that as set
//! membership: a unit is lit while it is hovered or selected. The highlight
//! colour is configurable in the client, with a default of `0xff404040`, and
//! each channel is scaled by 1/255, so the lift is 0x40/255 per channel. The
//! lift is an emissive term: it is added inside the lighting sum, before the
//! texture modulate, which is where `m2.wgsl` adds it.
//!
//! Three rules belong to this client and are not measured from the 1.12.1
//! client:
//!
//! * The highlight colour is always the default; no CVar changes it here.
//! * A batch whose texture moves is skipped, because copying its material
//!   would stop it scrolling (see [`vale_assets::world::m2::M2TextureAnims`]
//!   and `Materials::moving`).
//! * The lift is applied every frame rather than on a change, so that a model
//!   whose dressing is rebuilt under the pointer is lit again.

use bevy::prelude::*;

use crate::interface::target::Selection;
use crate::render::decals::spawn_ground_decal;
use crate::render::models::{DrawParams, ModelCache, Materials};
use crate::world::entities::EntityModel;
use crate::world::session::{LocalPlayer, WorldEntity};
use vale_assets::look::selection::{ring_rgba, Selected};

/// Where the texture's bright rim sits, as a fraction of the image's
/// half-width, measured from the decoded alpha channel. See the module comment.
const RIM: f32 = 0.875;

/// How much wider than the bare footprint the ring is drawn.
///
/// This factor is chosen by eye. `UNIT_FIELD_BOUNDINGRADIUS` is the unit's
/// footprint, and the 1.12.1 client's circle is visibly wider than that. The
/// factor the 1.12.1 client applies is not established, so this value is tuned
/// by eye in this client's window. If the real factor is found, it replaces
/// this one.
const RING_SCALE: f32 = 1.5;

/// The floor on the quad's half-width, in yards.
///
/// A unit whose field block never stated a bounding radius takes
/// `DEFAULT_BOUNDING_RADIUS` (0.389), which is below this floor. The pick box
/// has a minimum for the same reason. This floor applies only to drawing; it
/// does not correct the wire value, and nothing else reads that value through
/// here.
const MIN_RADIUS: f32 = 0.5;

/// The decal under the selected unit. There is at most one.
#[derive(Component)]
pub struct SelectionRing {
    /// The unit the ring is under, so a target change is one comparison.
    unit: Entity,
    /// The half-width the ring was built at, so a unit that grows while
    /// selected is also one comparison. A growth aura changes
    /// `UNIT_FIELD_BOUNDINGRADIUS` and `OBJECT_FIELD_SCALE_X` without changing
    /// the target; without this field the ring would keep the size the unit had
    /// when it was selected.
    half: f32,
}

pub struct SelectionPlugin;

impl Plugin for SelectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                // Runs after the selection is settled and before the projection
                // reads it. `interface::target` may retarget in this frame, and
                // a ring spawned after `decals::project_decals` has run is drawn
                // for one frame at the origin, under the map.
                follow_selection.after(crate::interface::target::TargetSet),
                // The colour is set every frame. It is cheap, and the ring must
                // turn red as soon as a flagged player attacks, when the only
                // input that changes is the reaction.
                tint_ring.after(follow_selection),
            )
                .before(crate::render::decals::DecalSet),
        );
        // Runs after the dressing. A model rebuilt this frame has new
        // materials with no lift on them, and the rebuild discards a highlight
        // applied before it: a unit under the pointer would stop glowing when
        // it equipped an item.
        app.add_systems(
            Update,
            apply_highlight.after(crate::world::entities::EntitySet),
        );
    }
}

/// The highlight lift per channel, `0x40/255`. See the module comment for its
/// source.
pub(crate) const HIGHLIGHT_LIFT: f32 = 64.0 / 255.0;

/// Light every part of the hovered and selected units, and unlight everything
/// that has left the set.
///
/// The lit set is the union of the hovered unit, the hovered game object and
/// the selected unit. The 1.12.1 client keeps the glow until the last of these
/// ends, and the union gives the same result without per-object state. A game
/// object under the pointer is hovered like a unit and is never selected.
///
/// A lootable body does not get the lift. It has its own effect,
/// `SpellVisualEffectName`'s `HARDCODED Loot Art`, attached to the corpse by
/// `crate::world::entities::effects::loot_art`. A lift on a lootable body
/// would be a mechanism the 1.12.1 client does not have; that module's note
/// covers this.
fn apply_highlight(
    hovered: Res<crate::interface::target::Hovered>,
    // The game object under the pointer, found by the same pointer ray as the
    // hovered unit; see [`crate::interface::object::HoveredObject`]. The
    // template decides whether it lights, not whether it is usable: a street
    // sign lights up and cannot be clicked, through
    // `GAMEOBJECT_TYPE_GENERIC`'s `highlight` field. See
    // `vale_assets::look::object::hover_of`.
    object: Res<crate::interface::object::HoveredObject>,
    selection: Res<Selection>,
    children: Query<&Children>,
    // Blob shadows are not excluded here because this walk cannot reach them.
    // Every blob is one root mesh built by `render::shadows`, not a child of
    // the unit, so `iter_descendants` does not visit it. Lifting an `unlit`
    // shadow would take one unit's blob out of the batch that every shadow in
    // the world shares, with no visible change.
    parts: Query<&MeshMaterial3d<crate::render::models::M2Material>>,
    mut commands: Commands,
    mut materials: Materials,
    mut lit: Local<Vec<Entity>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Selection);
    let mut want: Vec<Entity> = Vec::new();
    for root in [
        hovered.entity,
        object.entity.filter(|_| object.hover.highlight),
        selection.entity,
    ]
    .into_iter()
    .flatten()
    {
        if !want.contains(&root) {
            want.push(root);
        }
    }
    for root in lit.iter() {
        if !want.contains(root) {
            set_lift(*root, 0.0, &children, &parts, &mut commands, &mut materials);
        }
    }
    for root in &want {
        set_lift(*root, HIGHLIGHT_LIFT, &children, &parts, &mut commands, &mut materials);
    }
    *lit = want;
}

/// Rewrite one root's whole subtree to a material with this lift.
///
/// The material is re-interned, not mutated. A material is shared by every
/// batch with the same parameters, so writing the lift into the asset would
/// light every wolf in the zone. Interning the lifted copy keeps the pool's
/// invariant, and interning it back at lift 0 returns the original handle, so
/// nothing has to record which material a part had before it was lit.
fn set_lift(
    root: Entity,
    lift: f32,
    children: &Query<&Children>,
    parts: &Query<&MeshMaterial3d<crate::render::models::M2Material>>,
    commands: &mut Commands,
    materials: &mut Materials,
) {
    for part in std::iter::once(root).chain(children.iter_descendants(root)) {
        let Ok(handle) = parts.get(part) else { continue };
        let Some(swapped) = materials.with_highlight(&handle.0, lift) else {
            continue;
        };
        commands.entity(part).insert(MeshMaterial3d(swapped));
    }
}

/// Keep exactly one ring, under whatever is selected.
///
/// The ring is despawned and respawned, not re-parented. A decal's owner is a
/// field set when it is built, and its projection cache is keyed on the
/// corners it last saw. Moving a ring between units of different sizes would
/// require rewriting both, and a target change happens about once a fight.
fn follow_selection(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    selection: Res<Selection>,
    units: Query<(&WorldEntity, &Transform), With<EntityModel>>,
    rings: Query<(Entity, &SelectionRing)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Selection);
    // The comparison includes the width as well as the unit; see
    // [`SelectionRing::half`]. The width comes from the same arithmetic on the
    // same two inputs, so `==` on the float detects any change in those inputs.
    // A tolerance would stop the ring following a slow growth.
    let wanted = selection
        .entity
        .and_then(|unit| units.get(unit).ok().map(|(w, t)| (unit, w, t)));
    let want = wanted.map(|(unit, world, transform)| {
        (unit, quad_half(world.bounding_radius, transform.scale.max_element()))
    });
    let held = rings.iter().next();
    if held.map(|(_, ring)| (ring.unit, ring.half)) == want {
        return;
    }
    // Despawns every ring, not only the first, so that the at-most-one
    // invariant holds even if a frame ever spawned two.
    for (entity, _) in &rings {
        commands.entity(entity).despawn();
    }
    let Some((unit, half)) = want else { return };
    // The material is `None` until the texture has loaded, a frame or two
    // into a session. As with the blob shadow, no ring is drawn until then,
    // rather than a ring with no texture. The next frame's comparison above
    // still finds no ring for this unit and tries again.
    let Some(material) = ring_material(&mut cache, &mut materials) else {
        return;
    };
    let ring = spawn_ground_decal(
        &mut commands,
        &mut meshes,
        unit,
        // No joint: the ring follows the unit's own transform, as the 1.12.1
        // client's circle follows the unit. The transform also carries the
        // unit's facing, which does not change a circle.
        None,
        &quad(half),
        material,
    );
    commands.entity(ring).insert(SelectionRing { unit, half });
}

/// The quad's half-width, in the entity's own scaled frame.
///
/// [`RING_SCALE`] is chosen by eye; the other two factors are measured.
/// [`RIM`] puts the texture's bright ring on the footprint rather than 14%
/// inside the quad's corner. The division by `scale` removes
/// `OBJECT_FIELD_SCALE_X`, which `place_entities` puts on the transform that
/// poses the decal's corners, and which the server has already applied to the
/// wire value (`Unit::UpdateModelData`). Applying it twice has no effect at
/// scale 1.0 and is wrong by 44% on a tauren.
fn quad_half(bounding_radius: f32, scale: f32) -> f32 {
    // The server does not send a scale of zero, but dividing by it would give
    // a ring of infinite radius, and one NaN in a mesh breaks the whole frame.
    let scale = if scale > 1e-3 { scale } else { 1.0 };
    (bounding_radius * RING_SCALE / scale / RIM).max(MIN_RADIUS)
}

/// The ring's quad, in the model space [`vale_assets::world::m2::GroundQuad`] is
/// stated in — WoW axes, `z = 0`, bilinear rectangle order — with the whole
/// image on it.
fn quad(half: f32) -> vale_assets::world::m2::GroundQuad {
    vale_assets::world::m2::GroundQuad {
        // Unused: the decal takes `None` for its joint, so nothing indexes this.
        bone: 0,
        corners: [
            [-half, -half, 0.0],
            [half, -half, 0.0],
            [-half, half, 0.0],
            [half, half, 0.0],
        ],
        uvs: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
    }
}

/// The one material every ring in the world shares.
///
/// The material is tinted, so the colour is set per instance.
/// `DrawParams::tint` does not carry a colour; it tells the shader to read the
/// instance's `MeshTag` as `0xAARRGGBB`. One material therefore serves every
/// reaction, and [`tint_ring`] writes the selector's dword into the tag.
fn ring_material(
    cache: &mut ModelCache,
    materials: &mut Materials,
) -> Option<Handle<crate::render::models::M2Material>> {
    cache.simple_material(
        materials,
        vale_assets::look::selection::TEXTURE,
        &DrawParams {
            geoset: 0,
            texture: None,
            // Add-alpha. See the module comment for why not 3.
            blend: 4,
            // The selection ring is unlit: the 1.12.1 client draws it in the
            // selector's colour regardless of the scene lighting.
            unlit: true,
            // Seen from above only, like the blob.
            two_sided: false,
            no_depth_write: false,
            light: vale_assets::world::wmo::BatchLight::Sun,
            liquid: None,
            ground: None,
            // Not an `M2Color` track; see this function's doc comment.
            baked_tint: None,
            tint: Some(vale_assets::world::m2::BatchTint {
                color: None,
                transparency: None,
            }),
            uv: None,
            // A quad this client builds has no environment map — see
            // `models::loader::RawDraw::overlays`.
            overlays: Default::default(),
        },
    )
}

/// Re-resolve the ring's colour into its `MeshTag`, every frame.
///
/// Written unconditionally rather than on a change: the inputs are the two
/// attackability questions and the unit's health, all of which move without
/// anything in this module being told, and the write is four bytes.
fn tint_ring(
    assets: Res<crate::assets::GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    units: Query<&WorldEntity>,
    // Party membership and reputation standing: the two inputs to
    // friend-or-foe that are not in a DBC file. See
    // [`crate::interface::api::Friendship`]. The renderer draws the ring and
    // `render::labels` draws the name plate. Both used to consult only
    // `FactionTemplate.dbc`, after the interface had stopped doing so, and drew
    // a green ring and a green name under a unit the player could attack.
    group: Res<crate::interface::party::Party>,
    reputation: Res<crate::interface::reputation::PlayerStanding>,
    mut rings: Query<(&SelectionRing, &mut bevy::mesh::MeshTag)>,
) {
    let friendship = crate::interface::api::Friendship { party: &group, standing: &reputation };
    for (ring, mut tag) in &mut rings {
        let Ok(unit) = units.get(ring.unit) else { continue };
        let colour = ring_rgba(&selected(&assets, friendship, player.single().ok(), unit));
        tag.0 = crate::render::models::tint_tag(colour);
    }
}

/// The selector's five questions, asked of the live world.
fn selected(
    assets: &crate::assets::GameAssets,
    friendship: crate::interface::api::Friendship<'_>,
    me: Option<&WorldEntity>,
    unit: &WorldEntity,
) -> Selected {
    use vale_assets::tables::faction::Reaction;
    let tables = assets.display_tables().ok();
    // Without the tables, every question answers false and the reaction is
    // Neutral, the same rule as `lua::api::unit_rank`. A yellow ring under a
    // wolf is a plausible wrong answer, and this project tracks that kind of
    // error separately.
    let (reaction, i_attack_it, attacks_me) = match (tables.as_deref(), me) {
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
    Selected {
        player_controlled: unit.kind == vale_protocol::state::update::ObjectType::Player,
        // Both directions go through the same `can_attack` the picker and the
        // interface use, so the ring cannot disagree with them, as it could if
        // this combined a reaction and a flags word by hand.
        attacks_me,
        i_attack_it,
        pvp: unit.unit_flags & crate::interface::api::UNIT_FLAG_PVP != 0,
        dead: unit.dead,
        reaction,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The quad is the footprint divided by the rim, so that the bright ring
    /// lands where [`RING_SCALE`] puts it rather than short of the corner. A
    /// ring built at the raw radius draws 14% small, which looks like a slightly
    /// wrong texture rather than a code error.
    #[test]
    fn the_quad_puts_the_rim_where_the_scale_asks() {
        let footprint = 1.4_f32;
        let half = quad_half(footprint, 1.0);
        let quad = quad(half);
        // The rim of the texture, in yards from the centre.
        let rim = half * RIM;
        let wanted = footprint * RING_SCALE;
        assert!((rim - wanted).abs() < 1e-4, "rim at {rim} for a {footprint} footprint");
        // Bilinear rectangle order, and the whole image on it.
        assert_eq!(quad.corners[0], [-half, -half, 0.0]);
        assert_eq!(quad.corners[3], [half, half, 0.0]);
        assert_eq!(quad.uvs[3], [1.0, 1.0]);
    }

    /// The entity's scale is divided out, because the server has already
    /// applied it to the wire value.
    ///
    /// The corners go through the unit's `Transform`, which carries
    /// `OBJECT_FIELD_SCALE_X`; `UNIT_FIELD_BOUNDINGRADIUS` is in world space and
    /// already includes the same scale (`Unit::UpdateModelData`). The test
    /// therefore checks the ring's size on the ground, which must be the wire
    /// value times [`RING_SCALE`] at every entity scale.
    #[test]
    fn the_ring_is_the_wire_radius_on_the_ground_at_any_scale() {
        let footprint = 1.4_f32;
        for scale in [0.5_f32, 1.0, 1.5, 3.0] {
            // What the rim lands on in the world: the quad's half-width, through
            // the rim fraction, through the transform that poses it.
            let drawn = quad_half(footprint, scale) * RIM * scale;
            assert!(
                (drawn - footprint * RING_SCALE).abs() < 1e-4,
                "at scale {scale} the rim lands at {drawn} for a {footprint} footprint",
            );
        }
    }

    /// A unit whose field block never stated a radius still gets a ring of
    /// [`MIN_RADIUS`], as the pick box has a minimum for the same reason.
    #[test]
    fn a_unit_with_no_stated_radius_still_gets_a_ring() {
        assert_eq!(quad_half(0.0, 1.0), MIN_RADIUS);
        // A scale of zero, which the server does not send, gives neither a NaN
        // nor a ring the size of the map.
        assert_eq!(quad_half(0.9, 0.0), quad_half(0.9, 1.0));
    }

    /// The M2 box and the wire value measure different things and differ by a
    /// factor of two and a half on the creature in the "circle much too large"
    /// report. This test records that measurement.
    ///
    /// `vale model 'Creature\Boar\Boar.m2'` declares a box of
    /// `[-2.6, -1.3, -0.4]..[2.4, 2.2, 2.5]`, which is 2.5 model yards of
    /// horizontal half-extent, and the Rockhide Boar (display 389) uses it at
    /// scale 0.75. `creature_display_info_addon.bounding_radius` for that
    /// display is 0.882.
    ///
    /// The test compares the box with the wire value, not with what this module
    /// draws, so that changing [`RING_SCALE`] does not change it: the two data
    /// sources differ by 2.126 at any ring size.
    #[test]
    fn the_m2_box_is_the_wrong_footprint_by_two_and_a_half() {
        const BOX_HALF_EXTENT: f32 = 2.5;
        const SCALE: f32 = 0.75;
        const WIRE: f32 = 0.882;
        // The model box as a world footprint: posed through the display's scale.
        let box_footprint = BOX_HALF_EXTENT * SCALE;
        assert!(
            (box_footprint / WIRE - 2.126).abs() < 0.01,
            "{box_footprint} against {WIRE}",
        );
    }
}
