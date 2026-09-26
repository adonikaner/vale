//! The shape an entity gets when its model will not resolve.
//!
//! Its own module, small as it is, because it is a named system in
//! [`super::EntityPlugin`]'s chain and because the alternative — appending it to
//! whichever neighbour has room — is exactly how this file reached three and a
//! half thousand lines.

use super::*;

/// **Why this entity has no model**, in words, for the one instrument that can
/// answer the question a box raises — see [`crate::ui::boxes`], which writes it
/// over the box on `F4` → render → *error boxes*.
///
/// A `String` per entity with no model, which is a few dozen bytes on a
/// population that is mostly the invisible half of the world. It is carried
/// whether or not the entity draws a box, because "why is there nothing here"
/// is the same question as "why is there a box here" and the kinds that draw
/// nothing are the harder half of it.
#[derive(Component)]
pub struct FallbackReason {
    /// The sentence, for [`crate::ui::boxes`] and the inspector.
    pub why: String,
    /// **Whether this is a failure or the specification**, and the whole reason
    /// the two are not one field.
    ///
    /// Most things in the world with no model are *meant* to have none — see
    /// [`deserves_a_box`] — and an instrument that shouted about those would be
    /// shouting on every campfire in the game. Only a `true` here draws a box
    /// or raises a warning; a `false` is a read-out for somebody who clicked
    /// the thing and asked.
    pub fault: bool,
}

/// Does an entity deserve a box when its model will not resolve?
///
/// **Only where the absence is a bug**, and that is the whole rule. A box is a
/// diagnostic: it says *the server described something here and this client
/// could not draw it*. For a unit or a player that is true and worth shouting
/// about — all 10,534 creature display ids resolve (`vale npc`), so a box on
/// one of those is a real failure.
///
/// ## A game object with no display id is invisible **by design**
///
/// This used to say a game object was in the same position as a unit, on the
/// grounds that "every one of them is something the 1.12 client puts on
/// screen". That is false, and it was the whole of a bug reported as *grey
/// boxes inside the fires all over the world* — the Goldshire inn hearths, the
/// Ironforge braziers, every campfire.
///
/// The thing in the fire is **entry 2061, `Campfire Damage`: type 6, a trap,
/// with `displayId` 0** (measured in `gameobject_template`, the world database
/// this repo names as an authority in its own right). vmangos does not send a
/// field whose value is zero, so a template display id of 0 arrives as no
/// display id at all — and it means *draw nothing*, which is exactly what the
/// 1.12 client does with it. The trap sits inside the fire that is drawn by
/// something else, which is why the boxes landed so precisely on them.
///
/// It is not one object. The same table holds spell circles, spawners,
/// waterfalls, moonwells, cave mouths and the rest of the invisible scenery —
/// **75 templates with `displayId` 0**, most of them type 6 (trap) or type 8
/// (spell focus), and the world is spawned with many instances of each. So the
/// old rule did not have one exception, it had a population.
///
/// The absence is still *recorded* — [`FallbackReason`] is attached either way
/// and the inspector reads it — because "this is invisible on purpose" is a
/// useful answer to somebody who clicked it. What changes is that it no longer
/// draws a shape or raises a warning, neither of which is true of it.
///
/// For the other four kinds it is **not** a failure, and drawing one was the
/// bug. An `Item` object is a thing in a bag, a `Container` is the bag, a
/// `Corpse` object with no display id is bones the client has nothing to draw
/// for, and a `DynamicObject` is a *spell* — the persistent area of a Blizzard
/// or a Flamestrike — whose art is its `SpellVisual` and never a shape of its
/// own. The real client draws no geometry for any of them.
///
/// What that cost while it was wrong: a corpse and an item lying in the road at
/// Goldshire each drew a flat `#66FF99` slab, and a dynamic object drew one
/// **scaled by the spell's own radius** — so casting Blizzard put a grey box
/// several yards across on the ground, which is exactly what a frost spell
/// rendering "totally wrong" looked like. None of it was a spell-effect bug;
/// the effect was never the thing on screen.
pub(super) fn deserves_a_box(kind: ObjectType, display_id: Option<u32>) -> bool {
    match kind {
        // Whether it is a failure depends on what is missing, which is why this
        // takes the display id and not only the kind: a game object that named
        // a model and could not be given one is still a bug worth a box.
        ObjectType::GameObject => display_id.is_some(),
        ObjectType::Unit | ObjectType::Player => true,
        _ => false,
    }
}

/// A coloured box for anything with no model **that should have had one**.
///
/// See [`deserves_a_box`] for which kinds those are and why the other four draw
/// nothing at all. The HUD's `with no model` count is unchanged and counts
/// every kind, so what this stops is the *drawing*, not the reporting — an
/// entity that resolves to nothing still shows up in the number.
pub(super) fn fallback_shapes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    // **A transport is excluded rather than filtered.** A boat is a `.wmo`, so
    // the M2 spawner declines it and marks it `NoModel` — and it is drawn all
    // the same, by `render::ships`, out of the WMO cache. Without this every
    // ship in the game would carry a grey box inside it. The marker rather than
    // the type byte, because the type byte is zero until the template's own
    // query round trip lands and the box would be spawned in the meantime.
    added: Query<
        (Entity, &WorldEntity),
        (
            With<NoModel>,
            Without<Mesh3d>,
            Without<crate::render::ships::ShipBody>,
        ),
    >,
) {
    if added.is_empty() {
        return;
    }
    let cube = meshes.add(Cuboid::new(1.0, 2.0, 1.0));
    for (entity, world) in &added {
        if !world.is_self && !deserves_a_box(world.kind, world.display_id) {
            continue;
        }
        let colour = match world.kind {
            _ if world.is_self => Color::srgb(1.0, 0.9, 0.2),
            ObjectType::Player => Color::srgb(0.3, 0.7, 1.0),
            ObjectType::Unit => Color::srgb(1.0, 0.4, 0.4),
            _ => Color::srgb(0.6, 0.6, 0.6),
        };
        commands.entity(entity).insert((
            Mesh3d(cube.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: colour,
                unlit: true,
                ..default()
            })),
        ));
    }
}

