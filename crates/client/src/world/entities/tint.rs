//! The two things that change a unit's own model rather than hang art on it:
//! the colour it is drawn through, and how solid it is drawn.
//!
//! Every other part of the spell visual chain hangs a model on a unit. These
//! change the unit's own batches: Stoneform's grey, Shadowform's purple, a
//! ghost's pallor and transparency, a rogue in stealth.
//!
//! Both work the same way: a walk over the wearer's parts, a re-interned
//! material, and a field on `EntityModel` recording what the materials are
//! currently wearing, so nothing is written on a frame where nothing changed.
//! They differ in where the value comes from, which each system's own comment
//! states.
//!
//! ## The colour
//!
//! Two kit procedurals colour a model, both read in
//! `vale_assets::tables::spell`:
//!
//! * `charProc` 1, `ModelTint`: a colour held for as long as an aura is on the
//!   unit. Read from the unit's auras every frame.
//! * `charProc` 13, `ModelFlash`: a colour held for a time after its kit plays
//!   and then eased back to white. Recorded when the kit plays, in
//!   [`super::procedural::Procedurals`]. While it runs it replaces the aura's
//!   colour.
//!
//! The colour reaches the batches through the material, re-interned, as the
//! mouseover highlight in `crate::render::selection` does. The per-instance
//! `MeshTag` already carries the room's light for a unit indoors, and a
//! stone-formed dwarf in a tavern needs both. The colour rides `M2Params`'
//! `particle.w`.
//!
//! A re-intern has a cost, and it is bounded by writing only on a change: a
//! coloured unit's batches get their own material copies while the colour
//! lasts. A flash's fade would write a new copy every frame, so its level is
//! stepped to sixteen values across the fade; see [`FLASH_LEVELS`].
//!
//! ## Which aura's colour wins
//!
//! The 1.12.1 client keeps a list of colours per unit, puts each new one at the
//! front, and draws the front one, so the most recently applied aura wins. This
//! client cannot tell which aura was applied most recently: `UNIT_FIELD_AURA`
//! is a slot array and a new aura takes the lowest free slot. The first aura in
//! slot order wins here. The two rules differ only for a unit carrying two
//! colour auras at once. Combining the colours is not an option: the product of
//! two stated colours is a colour no row states.
//!
//! The same policy picks between two opacity auras.

use bevy::prelude::*;

use crate::render::models::{M2Material, Materials};
use crate::world::session::WorldEntity;

use super::procedural::Procedurals;
use super::{DisplayCache, EntityModel};

/// How many distinct colours a flash's fade is drawn in. The client blends at
/// 256 levels; every level here is a material, so the fade is drawn in steps
/// of 16 levels. A half-second fade at sixty frames a second then writes a new
/// material every other frame rather than every frame.
const FLASH_LEVELS: u8 = 16;

/// The colour a flash is drawn at, or `None` once it has faded to white, which
/// is the identity of the multiply.
fn flash_colour(flash: &vale_assets::tables::spell::ModelFlash, level: u8) -> Option<[u8; 3]> {
    let step = 256 / u16::from(FLASH_LEVELS);
    let level = match level {
        255 => 255,
        l => (u16::from(l) / step * step) as u8,
    };
    let colour = flash.blend(level);
    (colour != [255; 3]).then_some(colour)
}

/// Colours every unit whose flash or auras say so, and clears the colour from
/// every unit that has left the set.
///
/// Runs after the dressing, as the highlight does: a model rebuilt this frame
/// has fresh materials, and a colour applied before the rebuild is lost with
/// them.
pub(super) fn paint_models(
    mut commands: Commands,
    time: Res<Time>,
    displays: Res<DisplayCache>,
    children: Query<&Children>,
    parts: Query<&MeshMaterial3d<M2Material>>,
    mut materials: Materials,
    mut units: Query<(Entity, &WorldEntity, &mut EntityModel, &mut Procedurals)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Tint);
    let Some(tables) = displays.tables() else {
        return;
    };
    let now = time.elapsed_secs_f64();
    for (entity, world, mut model, mut procs) in &mut units {
        // A running flash replaces the aura's colour. `Some(None)` is a flash
        // that has faded to white and still runs.
        let flash = match procs.flash {
            Some((flash, since)) => {
                let elapsed_ms = ((now - since) * 1000.0).max(0.0) as u64;
                match flash.level(elapsed_ms) {
                    Some(level) => Some(flash_colour(&flash, level)),
                    None => {
                        procs.flash = None;
                        None
                    }
                }
            }
            None => None,
        };
        let wanted = match flash {
            Some(colour) => colour,
            // One hash lookup per aura slot, over a list that is usually
            // empty. The first match wins; see the module comment.
            None => world
                .auras
                .iter()
                .find_map(|aura| tables.aura_tint(aura.spell))
                .map(|tint| tint.colour),
        };
        // Almost every unit has no colour and wants none, and is done here.
        if wanted.is_none() && model.painted.is_none() {
            continue;
        }
        // The walk below runs every frame while a colour is on, not only on
        // the change. A wearer's parts load at different times: a pauldron
        // loads after its wearer and a spell effect after that, so a part
        // that lands two frames after the aura would otherwise keep the
        // colour it was built with. The walk costs nothing on a part that is
        // already right: `Materials::with_model_tint` answers `None` and no
        // command is queued.
        //
        // The write is guarded because it marks `EntityModel` changed.
        if model.painted != wanted {
            model.painted = wanted;
        }
        // The wearer's own batches and everything attached to them: a helm
        // and a pauldron are part of the body. The unlit glows in the same
        // subtree are not coloured; `m2.wgsl` applies the colour inside the
        // lit branch only.
        for part in std::iter::once(entity).chain(children.iter_descendants(entity)) {
            let Ok(handle) = parts.get(part) else { continue };
            let Some(copy) = materials.with_model_tint(&handle.0, wanted) else {
                continue;
            };
            commands.entity(part).insert(MeshMaterial3d(copy));
        }
    }
}

/// Draws a unit at the opacity its auras state, easing between values.
///
/// Two sources, in this order:
///
/// * a state kit's `charProc` 14 opacity, which is the 1.12.1 client's rule:
///   Ghost at 0.5, Vanish at 0.3, Shadowform at 0.65. See
///   `vale_assets::tables::spell::KitProcedurals::opacity`.
/// * `UNIT_FIELD_BYTES_1`'s creep or ghost flag with no such aura, drawn at
///   [`STEALTH_OPACITY`]. This one is this client's addition. Stealth (1784)
///   has no `SpellVisual` row, so nothing in the spell tables states an
///   opacity for it, and the 1.12.1 client does not fade a stealthed unit at
///   all. The flag is vmangos' `UnitVisFlags`, set by
///   `Aura::HandleModStealth` and by `SPELL_AURA_GHOST`.
///
/// A change of value eases over one second along `t³`, as the client eases
/// the kit's opacity; see [`Procedurals::opacity_at`]. While easing, the value
/// is stepped to multiples of 8/255, because each value is a material.
///
/// This is a separate system from [`paint_models`] because an opacity changes
/// the batch's pipeline: an opaque body cannot blend until its blend mode is
/// changed. See [`crate::render::models::Materials::with_opacity`], which
/// makes that change and reverses it.
pub(super) fn fade_models(
    mut commands: Commands,
    time: Res<Time>,
    displays: Res<DisplayCache>,
    children: Query<&Children>,
    parts: Query<&MeshMaterial3d<M2Material>>,
    mut materials: Materials,
    mut units: Query<(Entity, &WorldEntity, &mut EntityModel, &mut Procedurals)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Tint);
    let now = time.elapsed_secs_f64();
    let spells = displays.tables().and_then(|tables| tables.spells());
    for (entity, world, mut model, mut procs) in &mut units {
        let target = spells
            .and_then(|spells| world.auras.iter().find_map(|aura| spells.aura_opacity(aura.spell)))
            .or_else(|| world.not_solid().then_some(f32::from(STEALTH_OPACITY) / 255.0))
            .unwrap_or(1.0);
        // Almost every unit is solid and wants to be, and is done here
        // without touching its change flag.
        if target == 1.0 && procs.opacity.is_none() && model.faded.is_none() {
            continue;
        }
        let value = procs.opacity_at(target, now);
        let wanted = (value < 1.0).then(|| {
            let bytes = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            if value == target {
                bytes
            } else {
                bytes & !7
            }
        });
        if model.faded != wanted {
            model.faded = wanted;
        }
        let opacity = wanted.map_or(1.0, |o| f32::from(o) / 255.0);
        // The wearer's own batches and everything attached to them, re-asserted
        // every frame for the reason `paint_models` gives.
        for part in std::iter::once(entity).chain(children.iter_descendants(entity)) {
            let Ok(handle) = parts.get(part) else { continue };
            let Some(copy) = materials.with_opacity(&handle.0, opacity) else {
                continue;
            };
            commands.entity(part).insert(MeshMaterial3d(copy));
        }
    }
}

/// How solid a creeping or ghostly body with no opacity aura is drawn, out of
/// 255.
///
/// A chosen number, not a measured one; see [`fade_models`]. Half is far
/// enough from solid to read as hidden at a glance, and far enough from
/// invisible that a player can still see their own character.
const STEALTH_OPACITY: u8 = 128;

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::spell::ModelFlash;

    #[test]
    fn a_flash_steps_its_fade_and_ends_on_white() {
        let flash = ModelFlash { colour: [0, 0, 0], hold_ms: 0, fade_ms: 1000 };
        assert_eq!(flash_colour(&flash, 255), Some([0, 0, 0]));
        // Levels 128..143 are drawn as 128.
        assert_eq!(flash_colour(&flash, 140), flash_colour(&flash, 128));
        assert_ne!(flash_colour(&flash, 144), flash_colour(&flash, 128));
        assert_eq!(flash_colour(&flash, 15), None, "the last step is white");
    }
}
