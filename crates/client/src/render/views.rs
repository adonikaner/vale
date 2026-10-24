//! The camera an entity is seen from, when a host draws a scene of its own.
//!
//! The passes that pose a rig, step a particle emitter or turn a billboard
//! test it against one camera and turn it toward that camera. In a session
//! that is the world camera, which draws every entity there is. A host app can
//! also stand client entities in a scene of its own, drawn by its own camera
//! on one of the host layers ([`super::offscreen::layers::HOST`]) and not on
//! the world's layer. An entity in such a scene is seen from that camera, and
//! culling it against the world camera's frustum, or turning its quads toward
//! the world camera, gives a picture with the rig in its bind pose and the
//! flames edge-on.
//!
//! [`HostViews`] lists the cameras that draw such scenes, gathered once a
//! frame, and [`HostViews::of`] answers which of them an entity is seen from,
//! by its render layers. An entity on the world's layer, or on no layer a host
//! camera draws, is seen from the world camera, as it always was. With no host
//! camera active the list is empty and every pass behaves as it did before
//! this module existed.

use bevy::camera::primitives::Frustum;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use super::offscreen::layers;

/// One host camera: the layers it draws, where it stands, and what it sees.
#[derive(Debug, Clone)]
pub struct HostView {
    pub layers: RenderLayers,
    pub placement: GlobalTransform,
    pub frustum: Frustum,
}

/// The active cameras that draw host layers and not the world's layer. See
/// the module comment.
#[derive(Resource, Debug, Default)]
pub struct HostViews {
    views: Vec<HostView>,
}

impl HostViews {
    /// The host camera an entity on `layers` is seen from, or `None` for the
    /// world camera.
    ///
    /// `None` for an entity with no layers or on the world's layer, which the
    /// world camera draws. Otherwise the first host camera that draws one of
    /// the entity's layers.
    pub fn of(&self, layers: Option<&RenderLayers>) -> Option<&HostView> {
        let layers = layers?;
        if layers.intersects(&RenderLayers::layer(layers::WORLD)) {
            return None;
        }
        self.views.iter().find(|view| view.layers.intersects(layers))
    }

    /// Whether any host camera is active.
    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
}

/// Whether an entity on `layers` is in the world: on no layers, which is the
/// world's, or on the world's among others. A pass that builds one world mesh
/// for every unit (the blob shadows, the names) leaves out an entity that is
/// not, because the world camera does not draw it and a host scene does not
/// draw that mesh.
pub fn in_world(layers: Option<&RenderLayers>) -> bool {
    layers.is_none_or(|layers| layers.intersects(&RenderLayers::layer(layers::WORLD)))
}

/// Whether a camera's layers are host layers only.
fn host_only(layers: &RenderLayers) -> bool {
    layers.iter().all(|layer| layers::HOST.contains(&layer)) && layers.iter().next().is_some()
}

/// Gather the active host cameras for this frame's passes.
///
/// In `PreUpdate`. A camera's `GlobalTransform` and `Frustum` are written in
/// `PostUpdate`, so the values read here are the last frame's, which is what
/// the passes read from the world camera too.
fn gather(
    cameras: Query<(&Camera, &GlobalTransform, &Frustum, &RenderLayers), With<Camera3d>>,
    mut views: ResMut<HostViews>,
) {
    views.views.clear();
    for (camera, placement, frustum, layers) in &cameras {
        if !camera.is_active || !host_only(layers) {
            continue;
        }
        views.views.push(HostView {
            layers: layers.clone(),
            placement: *placement,
            frustum: *frustum,
        });
    }
}

pub(crate) struct ViewsPlugin;

impl Plugin for ViewsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HostViews>().add_systems(PreUpdate, gather);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(layer: usize) -> HostView {
        HostView {
            layers: RenderLayers::layer(layer),
            placement: GlobalTransform::default(),
            frustum: Frustum::default(),
        }
    }

    /// An entity on the world's layer, or with no layers, is the world
    /// camera's. One on a host layer only is the host camera's that draws it.
    #[test]
    fn an_entity_is_seen_from_the_camera_that_draws_its_layer() {
        let host = layers::HOST.start;
        let views = HostViews {
            views: vec![view(host), view(host + 1)],
        };
        assert!(views.of(None).is_none());
        assert!(views.of(Some(&RenderLayers::layer(0))).is_none());
        assert!(views.of(Some(&RenderLayers::from_layers(&[0, host]))).is_none());
        let seen = views.of(Some(&RenderLayers::layer(host + 1))).expect("drawn by the second");
        assert_eq!(seen.layers, RenderLayers::layer(host + 1));
        assert!(views.of(Some(&RenderLayers::layer(host + 2))).is_none(), "no camera draws it");
        assert!(HostViews::default().of(Some(&RenderLayers::layer(host))).is_none());
        assert!(in_world(None) && in_world(Some(&RenderLayers::from_layers(&[0, host]))));
        assert!(!in_world(Some(&RenderLayers::layer(host))));
    }

    /// Only a camera on host layers alone is a host view: the portraits and
    /// the dolls are the client's own, and a camera on the world's layer is a
    /// view of the world.
    #[test]
    fn only_host_layers_make_a_host_view() {
        let host = layers::HOST.start;
        assert!(host_only(&RenderLayers::layer(host)));
        assert!(!host_only(&RenderLayers::from_layers(&[0, host])));
        assert!(!host_only(&RenderLayers::layer(1)));
        assert!(!host_only(&RenderLayers::none()));
    }
}
