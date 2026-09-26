//! **The two things that change a unit's own model** rather than hang art on
//! it: what colour it is painted, and how solid it is drawn.
//!
//! Every other visible half of the spell chain hangs art *on* a unit: a glow on
//! its hands, a burst on its chest, a ring on the floor under it. These change
//! the unit itself. The colour is the reason a dwarf using his racial looked
//! exactly like a dwarf not using it — 854 spells state one and none of them had
//! a reader — and the opacity is the reason a rogue in stealth looked exactly
//! like a rogue out of it.
//!
//! They share this file because they share their whole shape: an edge-driven
//! walk over the wearer's parts, a re-interned material, and a cache field on
//! `EntityModel` saying what the materials are currently wearing. What they do
//! *not* share is where the rule comes from, and that difference is written out
//! under [`fade_models`] rather than left to be assumed from the neighbour.
//!
//! ## Where it comes from
//!
//! `SpellVisualKit`'s `charProc` family, value 1 (and 13), with the colour in
//! `charParamZero` as an `f32` whose *value* is a packed `0xRRGGBB`. All of that
//! is `vale_assets::tables::spell::ModelTint`'s, including the eight shipped rows
//! whose names are their own colours and which are what pin the column;
//! `vale spell` prints the census. Nothing here decides what the colour *is*
//! — this module decides only which unit is wearing which one, and how it
//! reaches the batches.
//!
//! ## How it reaches the batches, and why not through the tag
//!
//! Through the **material**, re-interned, exactly as
//! [`crate::render::selection`]'s mouseover lift is — and for the same reason
//! that one does it: the single per-instance word a batch has (`MeshTag`)
//! already carries the room's light for a character indoors, and a stone-formed
//! dwarf standing in a tavern needs both. The colour rides `M2Params`'
//! `particle.w`, a slot the struct already carried and nothing read.
//!
//! **A re-intern is a real cost and it is bounded by being edge-driven.** A
//! painted unit's batches get their own material copies for as long as the aura
//! lasts; that is a handful of materials per painted unit, and the population is
//! small — a stun, a shapeshift, a corpse run. Nothing is written on a frame
//! where nothing changed: [`super::EntityModel::painted`] is what the materials
//! are wearing and the walk only happens when it disagrees with the auras. The
//! reference caches the same comparison in the same place — it tests the
//! packed colour against the last one written and skips the write when it has
//! not moved.
//!
//! ## One policy that is this client's and says so
//!
//! **One aura wins, and this client picks a different one from the reference.**
//! The client keeps a *list* per unit, pushes each new colour on the **front**
//! and reads the head of it — so the most recently applied colour is the one
//! on the model. Nothing this client
//! has says which aura was applied most recently: `UNIT_FIELD_AURA` is a slot
//! array and a new aura takes the lowest *free* slot, which is not the newest
//! slot. So the first in slot order wins here, and the two answers differ only
//! for a unit carrying two colour auras at once — rare, and visibly wrong in
//! neither direction. Combining them was never an option: multiplying a 50%
//! grey by a dark purple gives a colour no row in the table states.
//!
//! **Nothing ramps, and for case 1 that is exact rather than a shortcut** —
//! the client reads `charParamZero` and stops. See `ModelTint`, which says
//! which of the two procedurals does ramp and what this costs there.

use bevy::prelude::*;

use crate::render::models::{M2Material, Materials};
use crate::world::session::WorldEntity;

use super::{DisplayCache, EntityModel};

/// Paint every unit whose auras say so, and unpaint everything that has left
/// the set.
///
/// **After the dressing**, like the highlight and for the same reason: a model
/// rebuilt this frame has fresh materials, and paint applied before the rebuild
/// is paint the rebuild throws away.
pub(super) fn paint_models(
    mut commands: Commands,
    displays: Res<DisplayCache>,
    children: Query<&Children>,
    parts: Query<&MeshMaterial3d<M2Material>>,
    mut materials: Materials,
    mut units: Query<(Entity, &WorldEntity, &mut EntityModel)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Tint);
    let Some(tables) = displays.tables() else {
        return;
    };
    for (entity, world, mut model) in &mut units {
        // One hash lookup per aura slot, over a list that is a handful long and
        // usually empty. Short-circuits on the first, which is the policy above.
        let wanted = world
            .auras
            .iter()
            .find_map(|aura| tables.aura_tint(aura.spell))
            .map(|tint| tint.colour);
        // **The whole world, every frame, for the price of one aura scan.**
        // Nothing is painted almost always, and a unit that is neither painted
        // nor wants to be is done here.
        if wanted.is_none() && model.painted.is_none() {
            continue;
        }
        // **Re-asserted while it is on rather than written on the edge**, which
        // is the highlight's own policy and is here for a reason that bit this
        // module in its first draft: a wearer's parts do not all exist at once.
        // A pauldron is a second load behind its wearer and a spell effect a
        // third, so a part that lands two frames after the aura did would keep
        // the colour it was built with for the life of the buff. The walk below
        // costs nothing on a part that is already right —
        // `Materials::with_model_tint` answers `None` and no command is queued.
        //
        // The write is guarded because it is a `DerefMut` and therefore a change
        // flag on `EntityModel`: nothing filters on that today, and a system
        // that starts to should not find every painted unit dirty every frame.
        if model.painted != wanted {
            model.painted = wanted;
        }
        // The wearer's own batches **and everything hanging off them** — a helm
        // and a pauldron are part of the body being painted. The unlit glows in
        // the same subtree take nothing, which the shader decides rather than
        // this walk: see `m2.wgsl`, where the paint is inside the lit branch.
        for part in std::iter::once(entity).chain(children.iter_descendants(entity)) {
            let Ok(handle) = parts.get(part) else { continue };
            let Some(copy) = materials.with_model_tint(&handle.0, wanted) else {
                continue;
            };
            commands.entity(part).insert(MeshMaterial3d(copy));
        }
    }
}

/// **Draw a body the server has hidden as a body you can see through** — a
/// rogue in stealth, a druid prowling, a spirit walking back from the
/// graveyard.
///
/// ## Which half of this is measured and which is not
///
/// **The flag is measured and the fade is not, and the difference matters.**
/// `UNIT_FIELD_BYTES_1`'s fourth byte is the game's own statement — vmangos'
/// `UnitVisFlags`, set from `Aura::HandleModStealth` and from `SPELL_AURA_GHOST`
/// — and it is the *only* statement there is: Stealth (1784) has no
/// `SpellVisual` row at all, so there is no kit, no `charProc` colour and no
/// pose column anywhere in `Spell.dbc` to read instead. Reaching for the flag
/// is therefore not a choice.
///
/// **What the 5875 client does with it is another matter, and the honest answer
/// is that it does not appear to fade anything.** The creep bit drives the
/// two gait arms (see `crate::world::entities::pose`), a few spell-targeting
/// and audibility conditions, and one test that combines it with the ghost bit
/// into a *boolean*. None of them touches an alpha, a material or a colour, and
/// `PLAYER_FIELD_BYTES2`'s own stealth bit — which vmangos also sets — is not
/// read by the client at all. So **this fade is this client's own addition**, on the same terms as
/// the grass sway: written down here as an addition rather than reported as a
/// reconstruction, because the two are different claims.
///
/// [`STEALTH_OPACITY`] is therefore a chosen number and says so.
///
/// ## Why it is not folded into the paint above
///
/// A colour and an opacity look like one job and are not: the colour rides a
/// float slot the material already had, where the opacity has to change the
/// batch's **pipeline** — an opaque body cannot blend at all until its mode is
/// forced. See [`crate::render::models::Materials::with_opacity`], which is
/// where that and the way back out of it are.
pub(super) fn fade_models(
    mut commands: Commands,
    children: Query<&Children>,
    parts: Query<&MeshMaterial3d<M2Material>>,
    mut materials: Materials,
    mut units: Query<(Entity, &WorldEntity, &mut EntityModel)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Tint);
    for (entity, world, mut model) in &mut units {
        let wanted = world.not_solid().then_some(STEALTH_OPACITY);
        // **The whole world, every frame, for the price of one byte test.**
        // Nothing is faded almost always, and a unit that is neither faded nor
        // wants to be is done here.
        if wanted.is_none() && model.faded.is_none() {
            continue;
        }
        if model.faded != wanted {
            model.faded = wanted;
        }
        let opacity = wanted.map_or(1.0, |o| o as f32 / 255.0);
        // The wearer's own batches **and everything hanging off them** — a helm,
        // a pauldron and a drawn weapon are all part of the body that has gone
        // into the shadows. Re-asserted while it is on rather than written on
        // the edge, which is [`paint_models`]' policy for [`paint_models`]'
        // reason: a pauldron is a load behind its wearer, and a part that lands
        // two frames after the aura did would otherwise stay solid for the life
        // of the buff.
        for part in std::iter::once(entity).chain(children.iter_descendants(entity)) {
            let Ok(handle) = parts.get(part) else { continue };
            let Some(copy) = materials.with_opacity(&handle.0, opacity) else {
                continue;
            };
            commands.entity(part).insert(MeshMaterial3d(copy));
        }
    }
}

/// How solid a creeping or ghostly body is drawn, 0..255.
///
/// **A chosen number, not a measured one** — see [`fade_models`], which has the
/// seven addresses that say the reference does not fade at all. Half is the
/// value every later build of this game settled on for the same state, it is far
/// enough from solid to read as "this one is hidden" at a glance, and it is far
/// enough from invisible that a player can still see their own character's feet
/// on the ground they are walking over.
const STEALTH_OPACITY: u8 = 128;
