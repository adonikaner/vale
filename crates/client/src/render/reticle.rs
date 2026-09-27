//! **The circle on the ground while a cast is being aimed at a place** — green
//! where it can go, red where it cannot.
//!
//! The round that made a placed cast work left this out with an honest note
//! saying so: "no circle texture is in the archives under any obvious name —
//! whoever draws it next should be looking for a *model*, not a texture." **The
//! second half of that was wrong, and the first half was searching under the
//! wrong name.** The two files are
//!
//! ```text
//!   Interface\SpellShadow\Spell-Shadow-Acceptable.blp     256x256, DXT3
//!   Interface\SpellShadow\Spell-Shadow-Unacceptable.blp    64x64,  DXT3
//! ```
//!
//! and the client's own word for this is a **shadow**, not a circle.
//!
//! ## It is the third caller of the ground-decal projector
//!
//! The reference builds a box `centre ± radius` in x and y and `± 2.0` in z,
//! binds the texture and hands the whole thing to the ground-decal projector —
//! the same projector the selection ring and the blob shadow go through, which is why
//! this module is policy over [`super::decals::spawn_ground_decal`] rather than
//! a pass of its own, exactly as [`super::selection`] is. It is drawn **twice**
//! per frame in the reference, once in each of two passes; here it is one decal
//! and the sorted transparent phase does the rest.
//!
//! ## Four numbers, all measured, and one of them is a surprise
//!
//! The aiming tick is short enough to quote in full:
//!
//! ```text
//!   if (!SpellIsTargeting() || !(targetMask & 0x60)) -> put it away
//!   circleCentre = the picked world point                  ; written either way
//!   if (legalPlacement(point)) { which = GREEN; radius = min(spellRadius, 20) }
//!   else                       { which = RED;   radius = 0 }
//! ```
//!
//! * **The centre is written before the judgement**, so a red circle is drawn
//!   where a green one would have been rather than not drawn at all.
//! * **The green radius is the spell's own**, off `SpellRadius.dbc` through
//!   `EffectRadiusIndex` — see
//!   [`vale_assets::tables::spellbook::SpellInfo::ground_circle_yards`], which is
//!   where the max-of-two-effects and the 20-yard clamp live.
//! * **The red one is a fixed 1.39 yards** and that is the surprise: the
//!   refusing branch writes the radius global to *zero*, and the draw reads zero
//!   as [`vale_assets::tables::spellbook::GROUND_CIRCLE_MIN_YARDS`]. So the red
//!   circle is not a red version of the green one — it is a small marker, and
//!   the 64x64 texture beside the green one's 256x256 is the file agreeing.
//! * **The ground branch is `targetMask & 0x60`, two bits and not one**:
//!   `TARGET_FLAG_SOURCE_LOCATION` counts as well as
//!   `TARGET_FLAG_DEST_LOCATION`. That is the aiming rule's business rather than
//!   this module's and it is recorded here because this is where it was read.
//!
//! ## …and one thing deliberately not done
//!
//! [`super::selection`]'s module comment says the reference's ground-target
//! reticle **pulses**. Nothing in this path does: the colour handed to the
//! projector is a flat `0xffffffff` and the texture is what carries
//! the shape. That claim is retracted rather than reproduced — if a pulse exists
//! it is somewhere not yet found, and inventing one would be this
//! client adding motion the file does not state.

use bevy::prelude::*;

use crate::interface::action::SpellTargeting;
use crate::render::decals::spawn_ground_decal;
use crate::render::models::{DrawParams, Materials, ModelCache};

/// The green one: this is a legal placement.
const ACCEPTABLE: &str = "Interface\\SpellShadow\\Spell-Shadow-Acceptable.blp";
/// …and the red one, drawn at [`GROUND_CIRCLE_MIN_YARDS`] whatever the spell's
/// own radius is.
const UNACCEPTABLE: &str = "Interface\\SpellShadow\\Spell-Shadow-Unacceptable.blp";

use vale_assets::tables::spellbook::GROUND_CIRCLE_MIN_YARDS;

/// The circle, and what it was last built for.
///
/// Both halves are part of the comparison for [`SelectionRing`]'s own reason: a
/// pointer that crosses out of range changes the texture without moving, and a
/// pointer that moves changes neither. The *position* is not in it, because that
/// is the owner's `Transform` and the projector already re-projects on it.
///
/// [`SelectionRing`]: super::selection::SelectionRing
#[derive(Component)]
struct GroundCircle {
    /// True for the green texture.
    acceptable: bool,
    half: f32,
}

/// The entity the decal hangs off, carrying the picked point.
///
/// A decal's carrier is a frame, and this mode has no entity in the world to be
/// one — the whole point of a placed cast is that it is aimed at a patch of
/// floor rather than at anything. So there is one anchor, spawned with the
/// circle and moved under it.
#[derive(Component)]
struct CircleAnchor;

pub struct ReticlePlugin;

impl Plugin for ReticlePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            follow_pointer
                // **After the pick and the range test**, which are one system in
                // `interface::target` and write both halves this reads. Unordered, the
                // circle is a frame behind the cursor it is under — which on a
                // pointer being swept across a hillside is the difference between
                // the art and the answer.
                .after(crate::interface::target::TargetSet)
                // …and in front of the projection, the rule
                // `render::decals::DecalSet` exists for: a decal spawned after it
                // has run is one frame of an unprojected mesh at the world origin.
                .before(crate::render::decals::DecalSet),
        );
    }
}

/// Keep exactly one circle, under the point a placed cast is being aimed at.
#[allow(clippy::too_many_arguments)]
fn follow_pointer(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    assets: Res<crate::assets::GameAssets>,
    targeting: Res<SpellTargeting>,
    circles: Query<(Entity, &GroundCircle)>,
    anchors: Query<Entity, With<CircleAnchor>>,
    mut placed: Query<(&mut Transform, &mut GlobalTransform), With<CircleAnchor>>,
) {
    // **The mode and the point, both**: the cursor can be up with the pointer on
    // the sky, which draws nothing at all rather than a circle at the origin.
    let want = targeting
        .wants_ground()
        .then(|| targeting.over_ground)
        .flatten()
        .map(|at| (at, targeting.over_valid, half_width(&assets, &targeting)));

    let Some((at, acceptable, half)) = want else {
        for entity in &anchors {
            commands.entity(entity).despawn();
        }
        for (entity, _) in &circles {
            commands.entity(entity).despawn();
        }
        return;
    };

    // The point moves every frame and the *quad* almost never does, so the two
    // are separated: the anchor is written unconditionally and the decal is
    // rebuilt only when the texture or the width changes.
    let at = crate::axes::to_bevy(at);
    match anchors.iter().next() {
        Some(_) => {
            for (mut transform, mut global) in &mut placed {
                // Both, and for the reason `rain_impacts` gives: this is a root
                // with no parent and the projection reads the `GlobalTransform`
                // inside the same schedule, so Bevy's own propagation in
                // `PostUpdate` is a frame too late — which on a pointer sweeping
                // across a hillside is a circle trailing the cursor.
                transform.translation = at;
                *global = GlobalTransform::from_translation(at);
            }
        }
        None => {
            commands.spawn((
                CircleAnchor,
                Transform::from_translation(at),
                GlobalTransform::from_translation(at),
                Visibility::default(),
            ));
        }
    }

    let held = circles.iter().next();
    if held.map(|(_, circle)| (circle.acceptable, circle.half)) == Some((acceptable, half)) {
        return;
    }
    for (entity, _) in &circles {
        commands.entity(entity).despawn();
    }
    let Some(anchor) = anchors.iter().next() else {
        // Spawned this frame by the arm above and not visible to this query
        // until the commands are applied — next frame builds the decal, which is
        // one frame of no circle at the instant the cursor goes up.
        return;
    };
    let texture = if acceptable { ACCEPTABLE } else { UNACCEPTABLE };
    let Some(material) = circle_material(&mut cache, &mut materials, texture) else {
        // The texture has not landed yet — no circle rather than a circle with
        // no picture, which is the selection ring's own answer to the same
        // situation and reached again next frame.
        return;
    };
    let circle = spawn_ground_decal(
        &mut commands,
        &mut meshes,
        anchor,
        // No joint: the anchor has no skeleton and the circle rides its frame.
        None,
        &quad(half),
        material,
    );
    commands
        .entity(circle)
        .insert(GroundCircle { acceptable, half });
}

/// The quad's half-width, in yards — which is the radius itself.
///
/// **No rim correction, deliberately, and this differs from the selection
/// ring.** That one divides by `RIM` so the texture's bright circle lands on the
/// unit's footprint; the reference builds *this* box as `centre ± radius`
/// outright, so its drawn ring sits inside the area it describes —
/// measured at **0.82 of the half-width** for the green texture and 0.71 for the
/// red, alpha-weighted over the decoded image. Correcting for that would draw a
/// truer circle and a less faithful one; the number is recorded here so whoever
/// wants the other trade knows what it costs.
fn half_width(assets: &crate::assets::GameAssets, targeting: &SpellTargeting) -> f32 {
    let stated = || {
        let info = assets
            .display_tables()
            .ok()?
            .spellbook()?
            .info(targeting.spell()?)?;
        Some(info.ground_circle_yards())
    };
    circle_half(targeting.over_valid, stated())
}

/// The two branches, without a world to ask.
///
/// **A refused placement takes the small marker whatever the spell says**, which
/// is the one thing here that reads like a bug and is not: the reference writes
/// the radius to *zero* on the red branch and the draw reads zero as 1.39
/// yards. A spell whose own radius rows are both zero takes the same fallback
/// through the same line, which is why the two cases are one expression rather
/// than two.
fn circle_half(acceptable: bool, stated: Option<f32>) -> f32 {
    match stated.filter(|r| acceptable && *r > 0.0) {
        Some(radius) => radius,
        None => GROUND_CIRCLE_MIN_YARDS,
    }
}

/// The circle's quad, in the model space [`vale_assets::world::m2::GroundQuad`] is
/// stated in — WoW axes, `z = 0`, bilinear rectangle order.
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

/// One material per texture, out of the same pool every other decal uses.
///
/// **Alpha-blended, not add-alpha.** The selection ring is white with the shape
/// in its alpha, so adding it is right; these two carry their *colour* — a flat
/// `(24, 250, 0)` green and a `(199, 10, 0)` red — with the alpha as a mask, and
/// adding a saturated green over grass draws a green square wherever the mask is
/// weak rather than a circle.
fn circle_material(
    cache: &mut ModelCache,
    materials: &mut Materials,
    texture: &str,
) -> Option<Handle<crate::render::models::M2Material>> {
    cache.simple_material(
        materials,
        texture,
        &DrawParams {
            geoset: 0,
            texture: None,
            blend: 2,
            // Nothing lights a targeting circle; it is interface art that
            // happens to be drawn on the floor.
            unlit: true,
            two_sided: false,
            no_depth_write: false,
            light: vale_assets::world::wmo::BatchLight::Sun,
            liquid: None,
            ground: None,
            tint: None,
            baked_tint: None,
            uv: None,
            // A quad this client builds has no environment map — see
            // `models::loader::RawDraw::overlays`.
            overlays: Default::default(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The green circle is the spell's own radius and the red one is never
    /// anything but the marker — including for a spell that states a radius.
    ///
    /// Getting this backwards is not a crash and not a wrong colour: it is a
    /// red circle eight yards across telling the player an area they cannot
    /// reach is the area they would cover, which is exactly the plausible-wrong
    /// shape this repo keeps paying for.
    #[test]
    fn the_refused_circle_is_the_marker_whatever_the_spell_says() {
        assert_eq!(circle_half(true, Some(8.0)), 8.0);
        assert_eq!(circle_half(false, Some(8.0)), GROUND_CIRCLE_MIN_YARDS);
        // …and a spell whose own radius rows are both zero takes the same
        // fallback on the *green* branch, through the same line of the client.
        assert_eq!(circle_half(true, Some(0.0)), GROUND_CIRCLE_MIN_YARDS);
        // …as does one this client has no catalog row for at all.
        assert_eq!(circle_half(true, None), GROUND_CIRCLE_MIN_YARDS);
    }
}
