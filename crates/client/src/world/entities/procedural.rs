//! What a kit's client-side procedurals leave on the unit it plays on.
//!
//! [`vale_assets::tables::spell::KitProcedurals`] is read per kit. When a kit
//! plays on a unit ([`super::spell_effects`] decides when), [`Procedurals::play`]
//! records its timed colour and arms its weapon trail here. The colour is read
//! back by [`super::tint::paint_models`]; the trail waits for the unit's next
//! animation and is then drawn by [`crate::render::weapon_trail`].
//!
//! The kit's opacity is not recorded here when the kit plays. It holds for as
//! long as the aura that played it, so it is read from the unit's auras every
//! frame, as the aura colour is; [`Procedurals::opacity`] only holds the ease
//! between two values.

use super::*;
use crate::render::weapon_trail::{spawn_strip, TrailAnchors, TrailMaterial, WeaponTrailStrip};
use vale_assets::look::weapon_trail::TrailPoints;
use vale_assets::tables::spell::{KitProcedurals, ModelFlash, WeaponTrail};

/// How long an opacity change takes, in seconds. The 1.12.1 client eases a
/// kit's opacity in and back out over one second, along `t³`.
pub(super) const OPACITY_EASE_SECS: f64 = 1.0;

/// The kit procedurals' state on one unit.
#[derive(Component, Default)]
pub struct Procedurals {
    /// The timed colour running on the unit, and when its kit played, in
    /// seconds. A second flash replaces the first.
    pub(super) flash: Option<(ModelFlash, f64)>,
    /// The weapon trail armed on the unit. The next animation the unit starts
    /// draws it. [`super::spell_effects`] runs before the pose in the frame,
    /// so a kit whose own animation starts in the same frame is drawn by it.
    pub(super) trail: Option<WeaponTrail>,
    /// The aura spells whose state kit has been played on this unit, in slot
    /// order. A spell entering this list plays its state kit's procedurals.
    pub(super) state_auras: Vec<u32>,
    /// The opacity ease: from, to, and when it began, in seconds.
    pub(super) opacity: Option<(f32, f32, f64)>,
}

impl Procedurals {
    /// Records what `kit` does when it plays at `now`.
    pub(super) fn play(&mut self, kit: &KitProcedurals, now: f64) {
        if let Some(flash) = kit.flash {
            self.flash = Some((flash, now));
        }
        if let Some(trail) = kit.trail {
            self.trail = Some(trail);
        }
    }

    /// The opacity the unit is drawn at `now`, easing towards `target`. A new
    /// target starts a new ease from wherever the last one had reached.
    pub(super) fn opacity_at(&mut self, target: f32, now: f64) -> f32 {
        let current = |ease: Option<(f32, f32, f64)>| match ease {
            None => 1.0,
            Some((from, to, since)) => {
                let t = ((now - since) / OPACITY_EASE_SECS).clamp(0.0, 1.0) as f32;
                from + (to - from) * t * t * t
            }
        };
        match self.opacity {
            Some((_, to, _)) if to == target => {}
            None if target == 1.0 => return 1.0,
            ease => self.opacity = Some((current(ease), target, now)),
        }
        let value = current(self.opacity);
        // Settled back at solid: nothing left to ease.
        if target == 1.0 && value >= 1.0 {
            self.opacity = None;
        }
        value
    }
}

/// The two points on a weapon model its trail runs between, on the weapon's
/// attachment root. Inserted by [`super::hang_model`] on every attached model
/// that carries both.
#[derive(Component, Clone, Copy)]
pub struct Blade(pub TrailPoints);

/// Starts an armed trail on every weapon the unit carries, on the first
/// animation the unit starts after the kit that armed it.
///
/// Runs between the pose, which is what starts an animation, and the sheath
/// reconcile, which takes the record of it.
pub(super) fn start_trails(
    mut commands: Commands,
    time: Res<Time>,
    material: Option<Res<TrailMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    blades: Query<&Blade>,
    strips: Query<(Entity, &WeaponTrailStrip)>,
    mut units: Query<(&EntityModel, &Playback, &mut Procedurals)>,
) {
    let Some(material) = material else {
        return;
    };
    let now = time.elapsed_secs_f64();
    for (model, playback, mut procs) in &mut units {
        let Some(trail) = procs.trail else {
            continue;
        };
        if playback.played.is_none() {
            continue;
        }
        procs.trail = None;
        let now_ms = (now * 1000.0) as u64;
        for part in &model.attached {
            let Ok(Blade(points)) = blades.get(part.root) else {
                continue;
            };
            // A trail started on a weapon that already has one restarts it.
            for (strip, existing) in &strips {
                if existing.owner() == part.root {
                    commands.entity(strip).despawn();
                }
            }
            let joint = |bone: u16| part.joints.get(usize::from(bone)).copied().unwrap_or(part.root);
            let anchors = TrailAnchors {
                bottom: joint(points.bottom.bone),
                top: joint(points.top.bone),
            };
            spawn_strip(
                &mut commands,
                &mut meshes,
                &material,
                part.root,
                anchors,
                *points,
                &trail,
                now_ms,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opacity_eases_along_the_cube_and_back() {
        let mut procs = Procedurals::default();
        assert_eq!(procs.opacity_at(1.0, 0.0), 1.0);
        assert_eq!(procs.opacity_at(0.5, 10.0), 1.0, "the ease starts where the unit is");
        let half = procs.opacity_at(0.5, 10.5);
        assert!((half - (1.0 - 0.5 * 0.125)).abs() < 1e-6, "t³ at t = 0.5: {half}");
        assert_eq!(procs.opacity_at(0.5, 11.0), 0.5);
        assert_eq!(procs.opacity_at(1.0, 20.0), 0.5, "the way back starts at 0.5");
        assert_eq!(procs.opacity_at(1.0, 21.0), 1.0);
        assert!(procs.opacity.is_none(), "settled");
    }

    #[test]
    fn a_later_kit_replaces_an_earlier_flash_and_trail() {
        let mut procs = Procedurals::default();
        let first = KitProcedurals {
            flash: Some(ModelFlash { colour: [1, 2, 3], hold_ms: 10, fade_ms: 10 }),
            trail: Some(WeaponTrail { colour: [1, 1, 1], alpha: 1, duration_ms: 1 }),
            opacity: None,
        };
        procs.play(&first, 1.0);
        let second = KitProcedurals {
            flash: Some(ModelFlash { colour: [9, 9, 9], hold_ms: 10, fade_ms: 10 }),
            ..Default::default()
        };
        procs.play(&second, 2.0);
        assert_eq!(procs.flash.map(|(f, at)| (f.colour, at)), Some(([9, 9, 9], 2.0)));
        assert_eq!(procs.trail.map(|t| t.duration_ms), Some(1), "a kit with no trail keeps the armed one");
    }
}
