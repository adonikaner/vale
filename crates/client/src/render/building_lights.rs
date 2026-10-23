//! The lights a building states, as the 1.12.1 client applies them to the
//! models standing near it.
//!
//! A WMO lists its lamps in `MOLT` (see [`vale_assets::world::wmo::WmoLight`]).
//! The building's own surfaces do not read them: a room is lit by its baked
//! vertex colours. A unit, a player or a game object inside the room is lit by
//! them. The Goldshire inn's ceiling lamps are `255/255/160`, and that colour
//! is the first point light the reference sets for a character standing in an
//! inn. Without this, a unit indoors had only the sun and the zone's fill, so
//! at dawn every NPC in an inn was the cold blue of the morning outside.
//!
//! The rule, per model:
//!
//! * Every omni light of every building is a candidate. A model takes the
//!   nearest three whose reach ([`WmoLight::model_reach`], the falloff's end)
//!   holds the model's position. Three because the fixed-function budget is
//!   four lights and the sun is the first.
//! * Each adds `colour × intensity × att × (max(N·L, 0) + 1)` to the model's
//!   lighting sum, the first term when the record's diffuse flag is set and
//!   the second when its ambient flag is. `att` is the fixed point-light curve
//!   `1 / (0.7d + 0.03d²)`, per fragment here and per vertex in the client,
//!   with no radius in it.
//!
//! [`WmoLight::model_reach`]: vale_assets::world::wmo::WmoLight::model_reach
//!
//! ## How the lights reach the shader
//!
//! A model's material is shared by every instance of it, so per-unit lights
//! cannot be material parameters (see [`crate::render::models::RoomLight`] for
//! the same constraint on room colours). The lights near the camera go into
//! rows of the shared texture-matrix table
//! ([`crate::render::models::material::LAMP_ROWS`]), at most 255 at once, and
//! a unit's three are three one-byte slot numbers in the low 24 bits of its
//! parts' `MeshTag`. A unit never takes a room colour, so those bits are free
//! on its batches.
//!
//! Slots are stable: a light keeps its slot while it stays near the camera,
//! so a unit standing still keeps its tag and is not retagged every frame.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::render::models::material::UvAnimations;

/// How far from the camera a light is stocked in the table, in yards. A unit
/// further away than this is not lit by lamps; at that distance a lamp's light
/// on it is a few pixels.
const STOCK_RANGE: f32 = 120.0;

/// How far the camera moves before the stock is taken again, in yards.
const RESTOCK_STEP: f32 = 4.0;

/// One building light, placed in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BuildingLight {
    /// Where it hangs, in world space.
    pub at: Vec3,
    /// Its colour times its intensity, in the file's sRGB bytes over 255.
    pub colour: [f32; 3],
    pub(crate) diffuse: bool,
    pub(crate) ambient: bool,
    /// How far from it a model is lit by it, in world yards.
    pub reach: f32,
}

impl BuildingLight {
    /// The table row the shader reads: position, colour, and the two flags.
    fn row(&self) -> [f32; 8] {
        [
            self.at.x,
            self.at.y,
            self.at.z,
            self.colour[0],
            self.colour[1],
            self.colour[2],
            if self.diffuse { 1.0 } else { 0.0 },
            if self.ambient { 1.0 } else { 0.0 },
        ]
    }
}

/// The lights one placed building states. Inserted on the placement by
/// [`crate::render::wmos`], and only when the building has an omni light.
#[derive(Component, Clone, Debug)]
pub(crate) struct BuildingLights(pub(crate) Vec<BuildingLight>);

impl BuildingLights {
    pub(crate) fn of_building(
        lights: &[vale_assets::world::wmo::WmoLight],
        placement: &Transform,
    ) -> Option<BuildingLights> {
        let scale = placement.scale.x.abs();
        let lights: Vec<BuildingLight> = lights
            .iter()
            .filter(|l| l.lights_models())
            .map(|l| BuildingLight {
                at: placement.transform_point(crate::render::axes::to_bevy(l.position)),
                colour: l.colour.map(|c| c * l.intensity),
                diffuse: l.diffuse,
                ambient: l.ambient,
                reach: l.model_reach() * scale,
            })
            .collect();
        (!lights.is_empty()).then_some(BuildingLights(lights))
    }
}

/// The three building lights an entity is lit by, as table slots 1..=255,
/// nearest first; 0 is no light. Written by
/// [`crate::world::entities`]'s lighting pass from [`LampSlots::nearest`].
#[derive(Component, Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct UnitLamps(pub(crate) [u8; 3]);

impl UnitLamps {
    /// The low 24 bits of the entity's parts' `MeshTag`.
    pub fn bits(self) -> u32 {
        u32::from(self.0[0]) << 16 | u32::from(self.0[1]) << 8 | u32::from(self.0[2])
    }
}

/// Which light holds which table slot.
#[derive(Resource, Default)]
pub(crate) struct LampSlots {
    /// Slot `n` is `slots[n - 1]`.
    slots: Vec<Option<((Entity, u16), BuildingLight)>>,
    by_light: HashMap<(Entity, u16), u8>,
    /// Where the camera was when the stock was last taken.
    anchor: Option<Vec3>,
}

impl LampSlots {
    /// The nearest three stocked lights whose reach holds `at`, nearest
    /// first.
    pub fn nearest(&self, at: Vec3) -> UnitLamps {
        let mut best: [(f32, u8); 3] = [(f32::INFINITY, 0); 3];
        for (index, slot) in self.slots.iter().enumerate() {
            let Some((_, light)) = slot else {
                continue;
            };
            let distance = light.at.distance(at);
            if distance > light.reach || distance >= best[2].0 {
                continue;
            }
            best[2] = (distance, index as u8 + 1);
            best.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        UnitLamps(best.map(|(_, slot)| slot))
    }

    /// Take the stock: the lights within [`STOCK_RANGE`] of `centre`, nearest
    /// first, at most [`LAMP_ROWS`](crate::render::models::material::LAMP_ROWS).
    /// A light already stocked keeps its slot. Returns the slots whose row
    /// changed.
    fn restock(
        &mut self,
        centre: Vec3,
        buildings: impl Iterator<Item = (Entity, Vec<BuildingLight>)>,
    ) -> Vec<(u8, BuildingLight)> {
        let capacity = crate::render::models::material::LAMP_ROWS as usize;
        if self.slots.len() != capacity {
            self.slots = vec![None; capacity];
        }
        let mut wanted: Vec<(f32, (Entity, u16), BuildingLight)> = buildings
            .flat_map(|(building, lights)| {
                lights.into_iter().enumerate().map(move |(i, light)| (building, i as u16, light))
            })
            .filter_map(|(building, i, light)| {
                let distance = light.at.distance(centre);
                (distance <= STOCK_RANGE).then_some((distance, (building, i), light))
            })
            .collect();
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        wanted.truncate(capacity);
        let keep: HashMap<(Entity, u16), BuildingLight> =
            wanted.iter().map(|(_, key, light)| (*key, *light)).collect();
        // Free what is no longer wanted.
        for slot in self.slots.iter_mut() {
            if slot.as_ref().is_some_and(|(key, _)| !keep.contains_key(key)) {
                *slot = None;
            }
        }
        self.by_light.retain(|key, _| keep.contains_key(key));
        // Place what is new in the free slots.
        let mut written = Vec::new();
        let mut free = (0..capacity).filter(|&i| self.slots[i].is_none()).collect::<Vec<_>>().into_iter();
        for (_, key, light) in wanted {
            if self.by_light.contains_key(&key) {
                continue;
            }
            let Some(index) = free.next() else {
                break;
            };
            let slot = index as u8 + 1;
            self.slots[index] = Some((key, light));
            self.by_light.insert(key, slot);
            written.push((slot, light));
        }
        written
    }
}

/// Stock the table with the lights near the camera, and upload the rows that
/// changed.
fn stock_lamp_table(
    mut slots: ResMut<LampSlots>,
    cameras: Query<&GlobalTransform, With<crate::world::camera::WorldCamera>>,
    buildings: Query<(Entity, &BuildingLights)>,
    added: Query<(), Added<BuildingLights>>,
    mut removed: RemovedComponents<BuildingLights>,
    mut uv: ResMut<UvAnimations>,
    images: Option<ResMut<Assets<Image>>>,
) {
    let Ok(camera) = cameras.single() else {
        return;
    };
    let centre = camera.translation();
    let moved = slots.anchor.is_none_or(|anchor| anchor.distance(centre) >= RESTOCK_STEP);
    let changed = !added.is_empty() || removed.read().count() > 0;
    if !moved && !changed {
        return;
    }
    slots.anchor = Some(centre);
    let written = slots.restock(
        centre,
        buildings.iter().map(|(entity, lights)| (entity, lights.0.clone())),
    );
    let mut dirty = false;
    for (slot, light) in written {
        dirty |= uv.set_lamp_row(slot, light.row());
    }
    if dirty {
        if let Some(mut images) = images {
            uv.upload(&mut images);
        }
    }
}

pub(crate) struct BuildingLightPlugin;

impl Plugin for BuildingLightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LampSlots>().add_systems(Update, stock_lamp_table);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(x: f32, reach: f32) -> BuildingLight {
        BuildingLight {
            at: Vec3::new(x, 0.0, 0.0),
            colour: [1.0, 1.0, 0.627],
            diffuse: true,
            ambient: true,
            reach,
        }
    }

    #[test]
    fn a_unit_takes_the_nearest_three_whose_reach_holds_it() {
        let mut slots = LampSlots::default();
        let building = World::new().spawn_empty().id();
        let lights = vec![light(1.0, 7.0), light(-2.0, 7.0), light(4.0, 7.0), light(0.5, 7.0), light(3.0, 2.0)];
        let written = slots.restock(Vec3::ZERO, std::iter::once((building, lights)));
        assert_eq!(written.len(), 5);
        let slot_of = |x: f32| written.iter().find(|(_, l)| l.at.x == x).unwrap().0;
        // The light at 3 is nearer than the one at 4, but its reach of 2 does
        // not hold the origin.
        assert_eq!(slots.nearest(Vec3::ZERO), UnitLamps([slot_of(0.5), slot_of(1.0), slot_of(-2.0)]));
        // Out of every reach: no lights.
        assert_eq!(slots.nearest(Vec3::new(50.0, 0.0, 0.0)), UnitLamps::default());
    }

    #[test]
    fn a_light_keeps_its_slot_and_a_light_out_of_range_frees_one() {
        let mut slots = LampSlots::default();
        let mut world = World::new();
        let near = world.spawn_empty().id();
        let far = world.spawn_empty().id();
        let first = slots.restock(
            Vec3::ZERO,
            [(near, vec![light(1.0, 7.0)]), (far, vec![light(100.0, 7.0)])].into_iter(),
        );
        assert_eq!(first.len(), 2);
        let near_slot = slots.by_light[&(near, 0)];
        // The camera walks away from the far building: the near light keeps its
        // slot and nothing is rewritten for it.
        let second = slots.restock(
            Vec3::new(-50.0, 0.0, 0.0),
            [(near, vec![light(1.0, 7.0)]), (far, vec![light(100.0, 7.0)])].into_iter(),
        );
        assert!(second.is_empty());
        assert_eq!(slots.by_light[&(near, 0)], near_slot);
        assert!(!slots.by_light.contains_key(&(far, 0)));
    }

    #[test]
    fn the_tag_bits_put_the_nearest_light_highest() {
        assert_eq!(UnitLamps([1, 2, 3]).bits(), 0x0001_0203);
        assert_eq!(UnitLamps::default().bits(), 0);
    }
}
