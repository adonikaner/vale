//! The stage's layer and light: every entity the client spawns under an actor
//! put on the stage's layer, and the light the stage is lit by.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use super::{CASTER_GUID, STAGE_LAYER, StageArea, StageUnit, TARGET_GUID};

/// Light the stage with the world's own sun.
///
/// The client's `DirectionalLight`s carry no `RenderLayers`, which means layer
/// 0, which means they do not reach the stage's view. Adding layer 2 to them
/// changes nothing about the world — the mask still holds layer 0 — and makes
/// the preview lit by exactly what the game is lit by.
///
/// The alternative, lights of the stage's own, draws the actors as a teal
/// patchwork.
pub fn light_the_stage(
    mut commands: Commands,
    lights: Query<(Entity, Option<&RenderLayers>), With<DirectionalLight>>,
) {
    let both = RenderLayers::from_layers(&[0, STAGE_LAYER]);
    for (light, layers) in &lights {
        if layers.is_some_and(|had| *had == both) {
            continue;
        }
        commands.entity(light).insert(both.clone());
    }
}

/// Put everything the client hangs under an actor on the stage's layer.
///
/// `RenderLayers` is not inherited: each drawn entity carries its own and
/// defaults to layer 0. The client resolves a unit into a subtree over the
/// frames after it is spawned — its batches, its attachments, the effect models
/// a kit hangs, their emitters — so this walks the actors' descendants and
/// stamps whatever has arrived since.
///
/// Every frame, because the subtree changes every time the spell does. It is a
/// walk of a few dozen entities; the world's is thirty thousand and is not
/// touched.
///
/// Most of a spell is not under the actor at all. The client spawns a
/// missile, every particle emitter, every ribbon trail, every ground quad and
/// every chain bolt at the world's root, tied to what owns them by an
/// `owner` entity or a guid rather than by the hierarchy, so the walk above
/// never reaches them. A stage that stamps only the descendants draws the
/// bodies, the poses and the hand models and none of those root-level
/// effects, so most spell effects do not render at all. Each of those kinds
/// is found by its component and stamped when its owner is one of the stage's
/// own; the owner join is the client's read-only accessor on each component.
#[allow(clippy::too_many_arguments)]
pub fn stamp_the_layer(
    mut commands: Commands,
    roots: Query<
        Entity,
        Or<(
            With<StageUnit>,
            With<StageArea>,
            With<crate::preview::lab::Mannequin>,
        )>,
    >,
    // The model view hides the body and not what hangs on it. With the
    // lab's `alone` set, the mannequin's own parts are stamped with the
    // world's layer only, so the stage camera does not see them, and only
    // what hangs under one of its hung roots is stamped onto the stage. The
    // body is still there — it is what the effect is posed against.
    lab: Res<crate::preview::lab::Lab>,
    mannequins: Query<
        (Entity, &vale_client::world::entities::EntityModel),
        With<crate::preview::lab::Mannequin>,
    >,
    children: Query<&Children>,
    already: Query<&RenderLayers>,
    // A missile is not a child of its caster. `render::missiles` spawns it
    // at the root of the world with a transform of its own. There are none in
    // an editor that is not previewing, so this needs no proximity test: the
    // only thing throwing one is the stage.
    missiles: Query<Entity, With<vale_client::render::missiles::Missile>>,
    emitters: Query<(Entity, &vale_client::render::particles::Emitter)>,
    ribbons: Query<(Entity, &vale_client::render::ribbons::Ribbon)>,
    decals: Query<(Entity, &vale_client::render::decals::GroundDecal)>,
    bolts: Query<(Entity, &vale_client::render::lightning::Bolt)>,
) {
    let layer = RenderLayers::from_layers(&[0, STAGE_LAYER]);
    let world_only = RenderLayers::layer(0);
    // Everything that hangs under a stage root or a missile, as a set — the
    // owners the root-level effects are matched against.
    let mut ours: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    // The mannequin's own body, when it is to be kept off the stage: its
    // descendants that are not under one of its hung roots.
    let mut hidden: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    for root in roots.iter().chain(missiles.iter()) {
        ours.insert(root);
        for entity in children.iter_descendants(root) {
            ours.insert(entity);
        }
    }
    if lab.alone {
        for (root, model) in &mannequins {
            let mut shown: std::collections::HashSet<Entity> = std::collections::HashSet::new();
            for hung in model.hung_roots() {
                shown.insert(hung);
                shown.extend(children.iter_descendants(hung));
            }
            // The body minus the effect. The stage's other actors are not
            // touched, and neither is a missile.
            hidden.insert(root);
            hidden.extend(
                children
                    .iter_descendants(root)
                    .filter(|entity| !shown.contains(entity)),
            );
        }
    }
    let stamp = |entity: Entity, layer: &RenderLayers, commands: &mut Commands| {
        if already.get(entity).is_ok_and(|had| had == layer) {
            return;
        }
        commands.entity(entity).insert(layer.clone());
    };
    for &entity in &ours {
        let wanted = match hidden.contains(&entity) {
            true => &world_only,
            false => &layer,
        };
        stamp(entity, wanted, &mut commands);
    }
    // The root-level effects, by owner. A geometry-model emitter's instances
    // are its children, which the descendant walk covers.
    let mut owned: Vec<Entity> = Vec::new();
    owned.extend(
        emitters
            .iter()
            .filter(|(_, e)| ours.contains(&e.owner()))
            .map(|(entity, _)| entity),
    );
    owned.extend(
        ribbons
            .iter()
            .filter(|(_, r)| ours.contains(&r.owner()))
            .map(|(entity, _)| entity),
    );
    owned.extend(
        decals
            .iter()
            .filter(|(_, d)| ours.contains(&d.owner()))
            .map(|(entity, _)| entity),
    );
    owned.extend(
        bolts
            .iter()
            .filter(|(_, bolt)| {
                bolt.points().first().is_some_and(|guid| {
                    *guid == CASTER_GUID
                        || *guid == TARGET_GUID
                        || *guid == crate::preview::lab::MANNEQUIN_GUID
                })
            })
            .map(|(entity, _)| entity),
    );
    for entity in owned {
        stamp(entity, &layer, &mut commands);
        for child in children.iter_descendants(entity) {
            stamp(child, &layer, &mut commands);
        }
    }
}
