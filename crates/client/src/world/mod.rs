//! The world the player is in: the session that describes it, the object
//! state the server sends, the entities in it, and the camera looking at it.
//!
//! ```text
//! session.rs      LiveSession -> ECS components keyed by GUID
//! incoming.rs     the one place a packet's event becomes client state: the
//!                 fan-out from the session's queue to each subject
//! entities/       a model for every entity: skins, joints, animation, fades
//! motion.rs       the 20 -> 60 Hz interpolator, for every unit the server
//!                 describes
//! predict.rs      the one entity drawn ahead of the server instead: the local
//!                 player
//! facing.rs       the heading a unit is drawn at, which a strafe turns away
//!                 from the direction it moves
//! camera.rs       the game's camera controls, and the camera's collision
//! crowd.rs        a population the server never sent, for measuring the
//!                 passes above; `diagnostics` only
//!
//! object state that is not an entity:
//! proficiency.rs  which item classes the character may use
//! spellmods.rs    what the character's talents change in a spell's numbers
//! templates.rs    an item template arriving, and who was waiting for it
//! areatrigger.rs  the client's side of an instance portal: the refusal
//! desync.rs       what the client measured when the server refused an action
//!                 for distance
//! ```
//!
//! `motion` and `predict` answer the same question in opposite ways, depending
//! on which side of the network the answer comes from. A creature's position
//! is the server's, and it is drawn a step and a half behind so the sample is
//! always between two known positions. The local player's position is this
//! client's own simulation, drawn from the keys in the frame they were pressed.
//! See `predict` for the ~50 ms that saves.
//!
//! The boundary with [`crate::render`] is not simulation against drawing:
//! `entities` spawns meshes and `camera` writes a `Transform`. Everything here
//! is keyed by a GUID or by where the player is, and means something only with
//! a server; everything in `render` is keyed by a file path or a placement. A
//! pass that would still make sense with the network unplugged belongs in
//! `render`.

pub mod areatrigger;
pub mod camera;
#[cfg(feature = "diagnostics")]
pub mod crowd;
pub mod desync;
pub mod entities;
pub mod facing;
pub mod incoming;
pub mod motion;
pub mod predict;
pub mod proficiency;
pub mod session;
pub mod spellmods;
pub mod templates;

/// The camera, the session and the entity passes, as one group. See
/// [`crate::render::RenderPlugins`] for why each directory registers its own.
pub struct WorldPlugins;

impl bevy::app::Plugin for WorldPlugins {
    fn build(&self, app: &mut bevy::app::App) {
        app.add_plugins((
            camera::CameraPlugin,
            session::SessionPlugin,
            // Entities use the render pass's models and materials: a creature
            // and a tree are both M2s, and an entity adds a skeleton, not a
            // different way of reading the file.
            entities::EntityPlugin,
        ));
        // The crowd writes a member's placement in the same part of the frame
        // `place_entities` writes a real entity's, which the measurement
        // depends on; see [`crowd::walk`].
        #[cfg(feature = "diagnostics")]
        {
        use bevy::ecs::schedule::IntoScheduleConfigs;
        app.add_systems(
            bevy::app::Update,
            (crowd::populate, crowd::walk)
                .chain()
                .run_if(crowd::is_wanted)
                .after(session::place_entities)
                .before(entities::EntitySet),
        );
        }
    }
}

/// The object state that is not an entity, as one group. Separate from
/// [`WorldPlugins`] because these need no GPU, so the schedule test in
/// `crate::interface` can add them.
///
/// `incoming` has no plugin: `crate::interface::action`'s plugin registers
/// its system.
pub struct StatePlugins;

impl bevy::app::Plugin for StatePlugins {
    fn build(&self, app: &mut bevy::app::App) {
        app.add_plugins((
            proficiency::ProficiencyPlugin,
            spellmods::SpellModPlugin,
            templates::TemplatesPlugin,
            areatrigger::AreaTriggerPlugin,
            desync::DesyncPlugin,
        ));
    }
}
