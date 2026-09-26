//! The world the player is standing in: the session that describes it, the
//! entities in it, and the eye looking at it.
//!
//! ```text
//! session.rs   LiveSession -> ECS components keyed by GUID
//! entities.rs  a model for every entity: skins, joints, animation, fades
//! motion.rs    the 20 -> 60 Hz interpolator, for everyone the server describes
//! predict.rs   …and the one entity that is drawn *ahead* of it instead
//! facing.rs    the heading a unit is *drawn* at, which a strafe turns off its aim
//! camera.rs    the game's own controls, and the eye that collides
//! crowd.rs     …and a population the server never sent, for pricing the above
//! ```
//!
//! `motion` and `predict` are the same question answered opposite ways round,
//! and the split is which side of the network the answer comes from: a creature
//! is the server's opinion and is drawn a step and a half behind so the sample
//! is always bracketed, where the local player is *this client's own*
//! simulation and is drawn from the keys in the frame they were pressed. See
//! `predict`'s own comment for the ~50 ms that buys.
//!
//! The split against [`crate::render`] is not "simulation versus drawing" —
//! `entities` spawns meshes and `camera` writes a `Transform`. It is **what the
//! server's answers mean** against **how a file becomes a picture**: everything
//! here is keyed by a GUID or by where the player is, and everything there is
//! keyed by a path or a placement. A pass that would still make sense with the
//! network unplugged belongs in `render`.

pub mod camera;
#[cfg(feature = "diagnostics")]
pub mod crowd;
pub mod entities;
pub mod facing;
pub mod motion;
pub mod predict;
pub mod session;

/// Every pass in this directory, as one plugin — see
/// [`crate::render::RenderPlugins`] for why the registration lives here rather
/// than in `lib.rs`.
pub struct WorldPlugins;

impl bevy::app::Plugin for WorldPlugins {
    fn build(&self, app: &mut bevy::app::App) {
        app.add_plugins((
            camera::CameraPlugin,
            session::SessionPlugin,
            // Entities go through the render pass's models and materials: a
            // creature and a tree are both M2s, and what an entity adds is a
            // skeleton, not a different way of reading the file.
            entities::EntityPlugin,
        ));
        // **The crowd, and the ordering is the measurement** — see
        // [`crowd::walk`], which has to write a member's placement in the same
        // window `place_entities` writes a real entity's.
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
