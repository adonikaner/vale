//! **What the client draws to say which unit is which**: the ring on the ground
//! under the target, and the brighten on the model under the pointer.
//!
//! They are one subject because the reference raises them from one place — the
//! selection setter and the mouseover publisher — and two mechanisms, which is
//! why they are two halves of this file rather than two files. The ring is the
//! second of the two things 1.12 uses its ground-decal projector for and the
//! first of them this client draws through [`super::decals`]; the brighten is a
//! material property, and neither is the other's fallback.
//!
//! The circle is a real object in the reference rather than a special case:
//! the client builds a model called **`ObjectSelectionCircle`** and gives it
//! **`Textures\UnitSelectTexture.blp`**, and the projector drapes it over whatever the unit is
//! standing on. That is why a selection circle in the real client follows a
//! hill and pours down a step instead of hovering flat over it, and it is the
//! whole reason this module is thirty lines of policy over
//! [`super::decals::spawn_ground_decal`] rather than a pass of its own.
//!
//! ## The texture states the shape and the client states the colour
//!
//! Decoded: 256x256, palettised with an 8-bit alpha channel, **RGB white in
//! every texel that is not fully transparent**. The alpha is the ring — nothing
//! out to r ≈ 0.28 of the half-width, a soft ~44/255 fill to 0.85, a 136/255 rim
//! at 0.875, gone by 0.97. So the colour is entirely the selector's, which is
//! [`vale_assets::look::selection`], and the blend has to be one that *keeps* the
//! alpha ramp: **mode 4**, `(SrcAlpha, One)` — add-alpha. Mode 3's `(One, One)`
//! would add white across the whole quad and draw a square.
//!
//! ## What is a ring, and when
//!
//! One decal, on the **selection** only. The mouseover gets no ring: the
//! reference's `SetHighlight`/`ClearHighlight` pair is
//! what a hover changes, and the circle object is a single global
//! attached to the *selected* unit — which is also what the two
//! screenshots this round was asked from show, a ring under the target and none
//! under the unit merely being pointed at.
//!
//! ## Two numbers that are this client's and say so
//!
//! * **the radius.** The reference scales the circle object by the unit's own
//!   footprint, and the footprint is `UNIT_FIELD_BOUNDINGRADIUS`
//!   ([`WorldEntity::bounding_radius`]) — divided by [`RIM`], so that the
//!   *bright rim* of the texture lands on the footprint rather than the quad's
//!   corner. Which of the client's several bounding numbers the circle takes is
//!   still not established; what **is** established is that the
//!   number this used to take is the wrong one by a factor of two and a half.
//!
//!   It was [`EntityModel::shadow_radius`] — the M2's own declared
//!   half-extent, which is the same number the blob shadow uses. That box is
//!   authored to cover *every frame of every animation the model has*, so it is
//!   an answer to a different question: `Creature\Boar\Boar.m2` declares 2.5
//!   model yards of horizontal half-extent (`vale model`), which at the
//!   Rockhide Boar's 0.75 scale and over [`RIM`] is a ring **4.3 yards across**,
//!   where the server sends 0.882 for the same creature — a ring of 2.0. That
//!   ratio is the whole of the "the circle is much too large" report.
//!
//!   The wire value is world-space and the quad is posed through the *unit's*
//!   transform, which carries `OBJECT_FIELD_SCALE_X` — so it is divided back out
//!   here, or the scale is applied twice and a tauren gets a ring half again too
//!   big while a gnome gets one too small.
//! * **nothing pulses.** This used to say the reference *does* pulse and that
//!   the pulse belongs to the ground-target reticle. Half of that stands and
//!   half is retracted: the reticle is `Spell-Shadow-Acceptable` and it is real
//!   ([`super::reticle`]), but nothing in its path pulses either — the colour it
//!   hands the projector is a flat `0xffffffff` and the texture
//!   carries the whole shape.
//!
//! ## …and the brighten, which is the other half
//!
//! `SetHighlight` and `ClearHighlight` are a pair
//! taking a **reason** and keeping a bitmask: the mouseover
//! publisher pushes reason 0 and the
//! selection setter reason 1. So the two
//! **stack**, and the glow only drops when the last reason clears — which
//! collapses exactly to set membership: a unit is lit while it is hovered *or*
//! selected. `SetHighlight` then reads three configured bytes and scales each by
//! 1/255; the shipped default is
//! `0xff404040`, so the lift is **0x40/255 per channel**. It reaches the model
//! as `glMaterialfv(GL_EMISSION)` — inside the lighting sum, before the texture
//! modulate — which is where `m2.wgsl` puts it.
//!
//! Three things are this client's and are stated rather than measured: the
//! highlight colour is the shipped default and no CVar can change it here; a
//! batch whose *texture* moves is skipped, because copying its material would
//! stop it scrolling (see [`vale_assets::world::m2::M2TextureAnims`] and
//! `Materials::moving`); and the lift is re-asserted every frame rather than on
//! a change, so that a dressing rebuilt under the pointer comes back lit.

use bevy::prelude::*;

use crate::interface::target::Selection;
use crate::render::decals::spawn_ground_decal;
use crate::render::models::{DrawParams, ModelCache, Materials};
use crate::world::entities::EntityModel;
use crate::world::session::{LocalPlayer, WorldEntity};
use vale_assets::look::selection::{ring_rgba, Selected};

/// Where the texture's bright rim sits, as a fraction of the image's
/// half-width. Measured off the decoded alpha channel — see the module comment.
const RIM: f32 = 0.875;

/// How much wider than the bare footprint the ring is drawn.
///
/// **This one is chosen by eye and says so.** `UNIT_FIELD_BOUNDINGRADIUS` is the
/// unit's footprint and the reference's circle is visibly wider than that; what
/// the reference actually multiplies it by is not established, so this is a
/// client-side factor tuned against the window rather than a number read out
/// of anything. Anyone who finds the real scale should delete this and say so.
const RING_SCALE: f32 = 1.5;

/// The floor on the quad's half-width, in yards.
///
/// A unit whose field block never stated a bounding radius takes
/// `DEFAULT_BOUNDING_RADIUS` (0.389) and lands under this; the same argument —
/// and the same shape of answer — as the pick box's own minimum. It is a
/// *drawing* floor and deliberately not a correction to the wire value, which
/// nothing else reads through here.
const MIN_RADIUS: f32 = 0.5;

/// The decal under the selected unit. One at a time, by construction.
#[derive(Component)]
pub struct SelectionRing {
    /// Which unit it is under, so a target change is one comparison.
    unit: Entity,
    /// …and how wide it was built, so a unit that *grows* while selected is one
    /// comparison too. A growth aura changes `UNIT_FIELD_BOUNDINGRADIUS` and
    /// `OBJECT_FIELD_SCALE_X` without changing the target, and without this the
    /// ring would keep the size the unit was when it was clicked.
    half: f32,
}

pub struct SelectionPlugin;

impl Plugin for SelectionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                // **After the selection is settled and before the projection
                // reads it.** `interface::target` may retarget in this frame, and a
                // ring spawned after `decals::project_decals` has run is a frame
                // of a ring at the origin — which is under the map.
                follow_selection.after(crate::interface::target::TargetSet),
                // …and the colour every frame, which is cheap and has to be:
                // the ring turns red the instant a flagged player attacks, with
                // nothing to notice but the reaction itself changing.
                tint_ring.after(follow_selection),
            )
                .before(crate::render::decals::DecalSet),
        );
        // **After the dressing, not before it.** A model rebuilt this frame has
        // fresh materials with no lift on them, and a highlight applied before
        // the rebuild is one the rebuild throws away — which is a unit that
        // stops glowing the moment it equips something under the pointer.
        app.add_systems(
            Update,
            apply_highlight.after(crate::world::entities::EntitySet),
        );
    }
}

/// The lift `SetHighlight` writes, per channel — `0x40/255`. See the module
/// comment for where it comes from.
const HIGHLIGHT_LIFT: f32 = 64.0 / 255.0;

/// Light every part of the hovered and selected units, and unlight everything
/// that has left the set.
///
/// **Set membership, not a reason bitmask.** The reference keeps one per object
/// and drops the glow when the last reason clears; hovered-or-selected is the
/// same answer with nothing to keep, and there are exactly three reasons — the
/// third being a game object under the pointer, which is hovered like a unit
/// and selected like nothing.
///
/// **A lootable body is not a third reason**, and it was briefly written as one
/// here. It has its own effect and the client's own name for it —
/// `SpellVisualEffectName`'s `HARDCODED Loot Art`, hung off the corpse by
/// `crate::world::entities::effects::loot_art`. A lift would have been this
/// repo inventing a mechanism, which is exactly what that module's note is
/// about.
fn apply_highlight(
    hovered: Res<crate::interface::target::Hovered>,
    // …and the game object under the same pointer, which is the same reason
    // reached by the other half of one ray — see
    // [`crate::interface::object::HoveredObject`]. **The template decides**, not
    // usability: a street sign lights up and cannot be clicked at all, which is
    // `GAMEOBJECT_TYPE_GENERIC`'s own `highlight` word — see
    // `vale_assets::look::object::hover_of`.
    object: Res<crate::interface::object::HoveredObject>,
    selection: Res<Selection>,
    children: Query<&Children>,
    // **The blob used to need excluding here and no longer does**: it was a
    // child of the entity, so this walk reached it, and lifting an `unlit`
    // shadow would have taken one unit's blob out of the batch set every shadow
    // in the world shares for no visible change. Every blob is now one root
    // mesh built by `render::shadows`, which `iter_descendants` cannot reach.
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
/// **Re-interned rather than mutated.** A material is shared by every batch that
/// means the same thing, so writing the lift into the asset would light every
/// wolf in the zone; interning the copy is what keeps the pool's invariant, and
/// interning the copy *back* returns the original handle by value — so nothing
/// has to remember what a part was wearing before it was lit.
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
/// **Despawn-and-respawn rather than re-parent**, because a decal's owner is a
/// field it was built with and its projection cache is keyed on the corners it
/// last saw: moving one between units of different sizes would need both
/// rewritten, and a target change is a once-a-fight event.
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
    // The width is part of the comparison, not just the unit — see
    // [`SelectionRing::half`]. It is the same arithmetic on the same two inputs,
    // so `==` on the float is asking "did anything move", which is what is
    // wanted; a tolerance here would be a ring that never followed a slow grow.
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
    // Every ring, not the first: a frame that spawned two (it cannot, but the
    // invariant is cheaper to enforce than to argue) would otherwise leave one.
    for (entity, _) in &rings {
        commands.entity(entity).despawn();
    }
    let Some((unit, half)) = want else { return };
    // **The material is `None` until the texture lands**, which is a frame or
    // two into a session — exactly the blob shadow's own situation, and the
    // answer is the same: no ring yet rather than a ring with no picture. The
    // next frame's comparison above still reads "no ring for this unit" and
    // tries again.
    let Some(material) = ring_material(&mut cache, &mut materials) else {
        return;
    };
    let ring = spawn_ground_decal(
        &mut commands,
        &mut meshes,
        unit,
        // No joint: the ring rides the unit's own frame, which is where the
        // reference's circle object hangs too. A unit's transform carries its
        // facing, and a circle does not mind.
        None,
        &quad(half),
        material,
    );
    commands.entity(ring).insert(SelectionRing { unit, half });
}

/// The quad's half-width, in the **entity's own scaled frame**.
///
/// [`RING_SCALE`] is the one factor here that is taste rather than measurement;
/// the other two are not cosmetic. [`RIM`] puts the texture's bright ring
/// on the footprint rather than 14% inside the quad's corner; the scale divides
/// out `OBJECT_FIELD_SCALE_X`, which `place_entities` has put on the transform
/// the decal's corners are posed through and which the server has *already*
/// folded into the wire value (`Unit::UpdateModelData`). Applying it twice is
/// invisible on everything at scale 1.0 and wrong by 44% on a tauren, which is
/// exactly the shape of error that gets found by eye a milestone later.
fn quad_half(bounding_radius: f32, scale: f32) -> f32 {
    // A scale of zero is not a thing the server sends, but a divide by it is a
    // ring of infinite radius and one NaN in a mesh is a whole frame.
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
/// **Tinted, so the colour is per instance.** `DrawParams::tint` does not carry
/// a colour — it says only that the shader should read this instance's
/// `MeshTag` as `0xAARRGGBB` — which is exactly what is wanted here: one
/// material for the pool whatever the reaction, and the selector's dword
/// written into the tag by [`tint_ring`].
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
            // Nothing lights a selection ring: the reference writes the
            // selector's dword straight into the decal's vertex colours.
            unlit: true,
            // Seen from above only, like the blob.
            two_sided: false,
            no_depth_write: false,
            light: vale_assets::world::wmo::BatchLight::Sun,
            liquid: None,
            ground: None,
            // Not an `M2Color` track — see this function's own note.
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
    // **The two halves of friend-or-foe that are not in a file** — see
    // [`crate::interface::api::Friendship`]. The ring is drawn by the renderer and
    // the name plate by `render::labels`, and both were asking
    // `FactionTemplate.dbc` on its own long after the interface had stopped:
    // that is a green ring and a green name under a unit the same client will
    // let you attack, which is exactly how it was reported.
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
    // **No table, no opinion**, which is `lua::api::unit_rank`'s own rule: the
    // rule answers Neutral for a table it does not have, and a yellow ring
    // under a wolf is exactly the kind of plausible wrong answer this project
    // counts separately.
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
        // The two directions, each through the same `can_attack` the picker and
        // the interface use — which is the whole point of asking it here rather
        // than folding a reaction and a flags word by hand.
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

    /// **The quad is the footprint divided by the rim**, so that the bright ring
    /// lands where [`RING_SCALE`] puts it rather than a texel short of the
    /// corner. A ring built at the raw radius draws 14% small, which is the kind
    /// of error that looks like a slightly wrong art asset for ever.
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

    /// **The entity's scale is divided out, because the server already applied
    /// it** — the trap this whole function exists for.
    ///
    /// The corners go through the unit's `Transform`, which carries
    /// `OBJECT_FIELD_SCALE_X`; `UNIT_FIELD_BOUNDINGRADIUS` is world-space and
    /// carries the same scale (`Unit::UpdateModelData`). So the property to pin
    /// is not "the number is divided" but **what the ring measures on the
    /// ground**, which must be the wire value — times [`RING_SCALE`] and nothing
    /// else — whatever the entity's scale is.
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

    /// A unit whose field block never stated a radius still gets a ring — the
    /// same one-sided answer the pick box gives, and for the same reason.
    #[test]
    fn a_unit_with_no_stated_radius_still_gets_a_ring() {
        assert_eq!(quad_half(0.0, 1.0), MIN_RADIUS);
        // …and a scale the server cannot really send does not produce a NaN or
        // a ring the size of the map.
        assert_eq!(quad_half(0.9, 0.0), quad_half(0.9, 1.0));
    }

    /// **The measurement the round turned on**, kept as a number rather than as
    /// a sentence: the M2 box and the wire value are answers to different
    /// questions and differ by a factor of two and a half on the creature the
    /// report arrived about.
    ///
    /// `vale model 'Creature\Boar\Boar.m2'` declares a box of
    /// `[-2.6, -1.3, -0.4]..[2.4, 2.2, 2.5]` — 2.5 model yards of horizontal
    /// half-extent — and the Rockhide Boar (display 389) wears it at scale 0.75.
    /// `creature_display_info_addon.bounding_radius` for that display is 0.882.
    ///
    /// Stated against the **wire value** rather than against what this module
    /// draws, so that tuning [`RING_SCALE`] cannot quietly move the number the
    /// round turned on: the two data sources disagree by 2.126 whatever the ring
    /// is finally drawn at.
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
