//! The stage's layer and light: every entity the client spawns under an actor
//! put on the stage's layer, and the light the stage is lit by.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use vale_client::render::effect::EffectOf;
use super::{STAGE_LAYER, StageArea, StageUnit};

/// Light the stage with the world's own sun.
///
/// The client's `DirectionalLight`s carry no `RenderLayers`, which means layer
/// 0, which means they do not reach the stage's view. Adding [`STAGE_LAYER`]
/// to them changes nothing about the world — the mask still holds layer 0 —
/// and makes the preview lit by exactly what the game is lit by.
///
/// The alternative, lights of the stage's own, draws the actors as a teal
/// patchwork.
pub(crate) fn light_the_stage(
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
/// every chain bolt at the world's root, so the walk above never reaches them.
/// Each carries the client's [`EffectOf`], naming the entity it belongs to;
/// an effect whose owner is one of the stage's own is stamped with its
/// subtree, and the join is repeated until it finds nothing new, which is what
/// reaches the emitters on a missile's model.
pub(crate) fn stamp_the_layer(
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
    effects: Query<(Entity, &EffectOf)>,
) {
    let layer = RenderLayers::from_layers(&[0, STAGE_LAYER]);
    let world_only = RenderLayers::layer(0);
    // Everything that hangs under a stage root, and every effect owned by
    // something in the set, with its subtree.
    let mut ours: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    // The mannequin's own body, when it is to be kept off the stage: its
    // descendants that are not under one of its hung roots.
    let mut hidden: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    for root in &roots {
        ours.insert(root);
        ours.extend(children.iter_descendants(root));
    }
    loop {
        let before = ours.len();
        for (entity, effect) in &effects {
            if ours.contains(&effect.owner()) && ours.insert(entity) {
                ours.extend(children.iter_descendants(entity));
            }
        }
        if ours.len() == before {
            break;
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
            // touched, and neither is an effect at the world root.
            hidden.insert(root);
            hidden.extend(
                children
                    .iter_descendants(root)
                    .filter(|entity| !shown.contains(entity)),
            );
        }
    }
    for &entity in &ours {
        let wanted = match hidden.contains(&entity) {
            true => &world_only,
            false => &layer,
        };
        if already.get(entity).is_ok_and(|had| had == wanted) {
            continue;
        }
        commands.entity(entity).insert(wanted.clone());
    }
}
