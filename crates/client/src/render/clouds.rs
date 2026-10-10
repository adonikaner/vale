//! The cloud layer: the texture [`vale_assets::tables::light::clouds`]
//! generates, drawn on its dome over the camera.
//!
//! The generator decides every texel; this pass feeds it the light, uploads
//! the rows it rewrites, and keeps the dome on the camera. The light is the
//! resolved [`crate::render::sky::Sky`]: bands 10 to 12 colour the clouds and
//! float band 3 sets how much of the sky they cover, so a zone's clouds and a
//! storm's overcast blend in with the rest of its light.
//!
//! The dome is drawn after the sun and the moons and before any skybox model
//! (see [`crate::render::sky::CLOUDS_SORT`]), at the far plane like every sky
//! surface. It is hidden while an opaque skybox is up, while the light states
//! no cloud density, and outside the world.
//!
//! The texture's size is the `SkyCloudLOD` CVar's: 128 texels at 0, 256 at 1.

use bevy::asset::{embedded_asset, RenderAssetUsages};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;

use vale_assets::tables::light::clouds::{dome, CloudField, CloudLight};

/// How far out the dome is drawn, in yards: the radius of the sphere it is a
/// cap of. Any radius inside the gradient dome draws the same picture, since
/// the dome is centred on the camera and written at the far plane.
const CLOUD_RADIUS: f32 = 700.0;

pub struct CloudPlugin;

impl Plugin for CloudPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/clouds.wgsl");
        app.add_plugins(MaterialPlugin::<CloudMaterial>::default())
            .add_systems(Startup, spawn_dome)
            .add_systems(
                Update,
                (
                    drift
                        .after(crate::render::sky::apply)
                        .after(crate::render::skybox::follow_the_light),
                    follow_camera.after(crate::world::camera::place),
                ),
            );
    }
}

/// The dome entity.
#[derive(Component)]
pub struct CloudDome;

/// The generator and the image it writes into.
#[derive(Resource)]
pub struct CloudLayer {
    field: CloudField,
    /// The `SkyCloudLOD` the field was made for.
    lod: u32,
    image: Handle<Image>,
    material: Handle<CloudMaterial>,
}

/// The dome's material: the cloud texture, multiplied by each ring's vertex
/// alpha.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct CloudMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
}

impl Material for CloudMaterial {
    fn fragment_shader() -> ShaderRef {
        crate::render::shader::CLOUDS.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn depth_bias(&self) -> f32 {
        crate::render::sky::CLOUDS_SORT
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Seen from beneath.
        descriptor.primitive.cull_mode = None;
        crate::render::sky::behind_the_world(descriptor);
        Ok(())
    }
}

/// An empty texture of `size` texels square, in the bytes the generator
/// writes, sampled without wrapping.
fn cloud_image(size: usize) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: size as u32,
            height: size as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        ..ImageSamplerDescriptor::linear()
    });
    image
}

/// The dome's mesh, in Bevy's axes, on the unit sphere; the entity's scale
/// makes it [`CLOUD_RADIUS`].
fn dome_mesh() -> Mesh {
    let vertices = dome::vertices();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vertices
            .iter()
            .map(|v| crate::render::axes::to_bevy(v.position).to_array())
            .collect::<Vec<_>>(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vertices.iter().map(|_| [0.0, -1.0, 0.0]).collect::<Vec<_>>(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vertices.iter().map(|v| v.uv).collect::<Vec<_>>());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vertices.iter().map(|v| [1.0, 1.0, 1.0, v.alpha]).collect::<Vec<_>>(),
    );
    mesh.insert_indices(Indices::U32(dome::indices()));
    mesh
}

fn spawn_dome(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
) {
    let field = CloudField::new(0);
    let image = images.add(cloud_image(field.size()));
    let material = materials.add(CloudMaterial {
        texture: image.clone(),
    });
    commands.spawn((
        CloudDome,
        Mesh3d(meshes.add(dome_mesh())),
        MeshMaterial3d(material.clone()),
        Transform::from_scale(Vec3::splat(CLOUD_RADIUS)),
        Visibility::Hidden,
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.insert_resource(CloudLayer {
        field,
        lod: 0,
        image,
        material,
    });
}

/// Feed the generator this frame's light, upload the rows it rewrites, and show
/// or hide the dome.
fn drift(
    layer: Option<ResMut<CloudLayer>>,
    sky: Res<crate::render::sky::Sky>,
    clock: Res<crate::render::sky::WorldClock>,
    weather: Res<crate::render::weather::WeatherState>,
    tuning: Res<crate::render::tuning::WorldTuning>,
    cover: Res<crate::render::sky::SkyCover>,
    focus: Res<crate::render::focus::WorldFocus>,
    cvars: Option<Res<crate::settings::cvars::CVars>>,
    time: Res<Time>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<CloudMaterial>>,
    mut dome: Query<&mut Visibility, With<CloudDome>>,
) {
    let Some(mut layer) = layer else {
        return;
    };
    let atmosphere = &sky.current;
    let shown = focus.present
        && tuning.clouds
        && !cover.opaque
        && atmosphere.cloud_density > 0.0;
    for mut visibility in &mut dome {
        visibility.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
    if !shown {
        return;
    }

    let lod = cvars.map_or(0, |c| c.number("SkyCloudLOD").clamp(0.0, 1.0) as u32);
    if lod != layer.lod {
        layer.field = CloudField::new(lod);
        layer.lod = lod;
        let replacement = cloud_image(layer.field.size());
        let handle = images.add(replacement);
        layer.image = handle.clone();
        if let Some(mut material) = materials.get_mut(&layer.material) {
            material.texture = handle;
        }
    }

    let light = CloudLight {
        density: atmosphere.cloud_density,
        highlight: atmosphere.clouds[0],
        shade: atmosphere.clouds[1],
        base: atmosphere.clouds[2],
        storm: if tuning.weather { weather.storm() } else { 0.0 },
        half_minutes: clock.half_minutes,
    };
    let Some(rows) = layer.field.advance(time.delta_secs(), &light) else {
        return;
    };
    let size = layer.field.size();
    let texels = &layer.field.rgba()[rows.start * size..rows.end * size];
    if let Some(mut image) = images.get_mut(&layer.image) {
        if let Some(data) = image.data.as_mut() {
            let from = rows.start * size * 4;
            for (out, texel) in data[from..from + texels.len() * 4].chunks_exact_mut(4).zip(texels) {
                out.copy_from_slice(texel);
            }
        }
    }
}

/// Keep the dome centred on the camera. The camera's `Transform`, for the
/// reason `render::sky`'s `follow_camera` states.
fn follow_camera(
    camera: Query<&Transform, With<crate::world::camera::WorldCamera>>,
    mut dome: Query<&mut Transform, (With<CloudDome>, Without<crate::world::camera::WorldCamera>)>,
) {
    let Some(eye) = camera.iter().next() else {
        return;
    };
    for mut transform in &mut dome {
        transform.translation = eye.translation;
    }
}
