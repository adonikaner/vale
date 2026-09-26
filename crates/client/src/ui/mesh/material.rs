//! **The one material every interface batch draws with**: a texture, a vertex
//! colour, and which of the game's blend modes combines it with what is behind.
//!
//! The blend is the whole reason this is a custom `Material2d` rather than
//! `ColorMaterial`: a region's `alphaMode=` is one of five
//! ([`crate::lua::widgets::regions::Paint::blend`]), egui has exactly one, and
//! the egui painter's stated approximation — additive art preprocessed so alpha
//! blending looks right — is what this pass exists to stop making. The pipeline
//! is specialised per [`Blend`], which rides the material's bind-group data so
//! two batches in different modes can never share a pipeline by accident.

use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState,
    RenderPipelineDescriptor, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey};

/// The URI [`super::UiMeshPlugin`]'s `embedded_asset!` registers the shader
/// under. Asserted in [`super::tests`], the way every embedded shader here is —
/// a mismatch fails at runtime as a batch that never draws.
pub const SHADER: &str = "embedded://vale_client/ui/mesh/shaders/interface.wgsl";

/// How a batch combines with what is already on the screen — the game's own
/// `alphaMode=` vocabulary, one variant per distinct pipeline it makes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Blend {
    /// `BLEND`: src over dst by the source's alpha.
    #[default]
    Alpha,
    /// `ALPHAKEY`: no blend at all — a fragment is kept or discarded by the
    /// reference's own per-blend cutoff table (**224** for the alpha key),
    /// which the m2 shader uses too. Three
    /// elements in the shipped directory, and the egui painter drew them as
    /// `BLEND`; this is the honest form.
    AlphaKey,
    /// `ADD`: the source added on, scaled by its alpha — `SRCALPHA, ONE`,
    /// which is the mode the egui painter could only approximate.
    Additive,
    /// `MOD`: the destination multiplied by the source.
    Modulate,
    /// `DISABLE`: the source written outright. Spelled as a replace blend
    /// rather than `AlphaMode2d::Opaque` so every batch stays in the one
    /// z-sorted transparent phase — an opaque 2d batch would be drawn in a
    /// different pass from the quads under it, and 2d has no depth buffer to
    /// put them back in order.
    Opaque,
}

impl Blend {
    /// The game's word for it, from [`crate::lua::widgets::regions::Paint::blend`].
    pub fn of(word: &str) -> Blend {
        match word {
            "ADD" => Blend::Additive,
            "ALPHAKEY" => Blend::AlphaKey,
            "MOD" => Blend::Modulate,
            "DISABLE" => Blend::Opaque,
            _ => Blend::Alpha,
        }
    }

    fn state(self) -> BlendState {
        let alpha_over = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        };
        match self {
            Blend::Alpha => BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::SrcAlpha,
                    dst_factor: BlendFactor::OneMinusSrcAlpha,
                    operation: BlendOperation::Add,
                },
                alpha: alpha_over,
            },
            Blend::Additive => BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::SrcAlpha,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
                alpha: alpha_over,
            },
            Blend::Modulate => BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::Zero,
                    dst_factor: BlendFactor::Src,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent {
                    src_factor: BlendFactor::Zero,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
            },
            // The reference disables blending outright for an alpha key —
            // what varies per fragment is the discard, in the shader.
            Blend::AlphaKey | Blend::Opaque => BlendState::REPLACE,
        }
    }
}

/// A texture and the mode it lands in. One instance per distinct pair, cached
/// by [`super::rebuild`] so a bind group survives across rebuilds.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
#[bind_group_data(Blend)]
pub struct InterfaceMaterial {
    #[texture(0)]
    #[sampler(1)]
    pub texture: Handle<Image>,
    pub blend: Blend,
}

impl From<&InterfaceMaterial> for Blend {
    fn from(material: &InterfaceMaterial) -> Blend {
        material.blend
    }
}

impl Material2d for InterfaceMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    /// `Blend` for every mode — see [`Blend::Opaque`] for why even the opaque
    /// one stays in the sorted phase.
    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: Material2dKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(key.bind_group_data.state());
            }
            if key.bind_group_data == Blend::AlphaKey {
                fragment.shader_defs.push("ALPHA_KEY".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// The shader's `ALPHA_KEY_CUTOFF` is the reference's 224/255 pushed
    /// through the same byte-space compensation every upload's alpha takes —
    /// the test is against compensated texels, so an uncompensated threshold
    /// would pass texels the reference culls. Pinned here because the WGSL
    /// holds it as a literal.
    #[test]
    fn the_alpha_key_cutoff_is_the_compensated_224() {
        let expected = crate::ui::framexml::byte_space_alpha(224.0 / 255.0);
        assert!(
            (expected - 0.9863).abs() < 5e-4,
            "the shader's literal 0.9863 drifted from byte_space_alpha(224/255) = {expected}"
        );
    }
}
