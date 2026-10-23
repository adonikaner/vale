//! **The one material every interface batch draws with**: a texture, a vertex
//! colour, and which of the game's blend modes combines it with what is behind.
//!
//! The blend is the reason this is a custom `Material2d` rather than
//! `ColorMaterial`: a region's `alphaMode=` is one of five
//! ([`crate::lua::widgets::regions::Paint::blend`]), and each needs its own
//! blend state. The pipeline is specialised per [`Blend`], which rides the
//! material's bind-group data so two batches in different modes cannot share
//! a pipeline.
//!
//! ## Blend colour space
//!
//! A blend also depends on the space the mixed values are in. 1.12 is
//! fixed-function: a plain `X8R8G8B8` back buffer with no
//! `D3DRS_SRGBWRITEENABLE`, so `SRCALPHA, INVSRCALPHA` mixes the bytes its
//! files state. [`crate::render::present`] puts the world in that byte space;
//! the interface is not in it. Every texture is uploaded as
//! `Rgba8UnormSrgb`, so the ROP decodes the destination to linear, mixes, and
//! re-encodes.
//!
//! The effect is that a dark translucent fill lets through more of what is
//! behind it. The spell tooltip's plate and the cooldown swirl were both
//! reported as too transparent. `Interface\Tooltips\UI-Tooltip-Background` is
//! a flat grey 148 at a uniform alpha of 187/255 (64x64 DXT3, every nibble
//! `0xB`), tinted by `GameTooltip_OnLoad` to `TOOLTIP_DEFAULT_BACKGROUND_COLOR`
//! (0.09, 0.09, 0.19), so the source is `13, 13, 28`. Over the spellbook's
//! parchment at about `150, 110, 55` that mixes, in bytes, to `50, 39, 35`.
//!
//! Values measured from a screenshot of each client, against the two
//! predictions:
//!
//! ```text
//!   1.12, between two glyphs inside the plate      47, 36, 35
//!   this client, same place                        90, 61, 36
//!   what a byte-space mix predicts                 50, 39, 35
//!   what a linear mix predicts                     89, 61, 36
//! ```
//!
//! The linear prediction matches this client's picture to within a byte on
//! all three channels, so the art, the tint and the alpha were correct and the
//! difference is the blend space.
//!
//! The alpha is therefore pre-compensated. `1 - a` is the fraction of the
//! destination that survives a mix, and the fraction that survives a linear
//! mix by the same visible amount is `srgb_to_linear(1 - a)`. That is the same
//! piecewise curve `gamma.wgsl` uses, because `Color::srgb` uses it too.
//! [`byte_space_alpha`] is the function every texture goes through. The same
//! plate over the same parchment comes out `40, 30, 30` against 1.12's
//! `47, 36, 35`: six bytes off rather than forty. The remaining gap is the
//! second limit below. The compensation:
//!
//! * is exact for a black source over any destination: a scrim, a cooldown
//!   swirl and, close to it, a tooltip plate;
//! * corrects the destination term and not the source term. The two need
//!   different numbers: the destination survives `1 - a` of a byte-space mix
//!   and the source contributes `a` of one, but the ROP takes a single alpha.
//!   The source is encoded before the mix instead of after it, which lands a
//!   dark source a few bytes low (the 40 against 47 above) and a bright one a
//!   few bytes high;
//! * compensates a texture's own alpha and an inherited `SetAlpha`
//!   separately, and the GPU multiplies them, so a partly transparent texture
//!   inside a fading frame is slightly too opaque mid-fade. Both are 1.0 in
//!   almost every draw the interface makes.
//!
//! The exact fix is to draw the interface into a byte-space target, where the
//! hardware mixes bytes and no compensation is needed. This painter does not
//! do that yet.

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
    /// which the m2 shader uses too. Three elements in the shipped directory
    /// use it.
    AlphaKey,
    /// `ADD`: the source added on, scaled by its alpha — `SRCALPHA, ONE`.
    /// 136 elements of the directory use it: the cast bar's spark, every
    /// button flash, every glow.
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

/// The alpha that makes a linear ROP mix the way 1.12's byte-space one did.
///
/// `1 - a` is the fraction of the destination a mix leaves. The interface is
/// drawn to an sRGB target, so that fraction is applied to the decoded
/// (linear) destination, which for a dark source is the whole of the visible
/// difference. The fraction that scales a linear value by the same visible
/// amount that `1 - a` scales a byte is `srgb_to_linear(1 - a)`, so the
/// compensated alpha leaves that fraction.
///
/// Exact for a black source over any destination. The module comment lists
/// the two cases where it is an approximation, and why the exact fix is a
/// byte-space render target rather than a function.
pub(crate) fn byte_space_alpha(alpha: f32) -> f32 {
    1.0 - srgb_to_linear(1.0 - alpha.clamp(0.0, 1.0))
}

/// The byte-space emulation for a coloured translucent fill, matched for the
/// case where such a fill usually sits: over a dark destination.
///
/// [`byte_space_alpha`] is exact for a black source and too opaque for a
/// bright one. For the skill bars' `[0, 0, 0.75, 0.5]` background it gives an
/// effective 0.79, so every rank bar drew as a solid saturated block instead
/// of the translucent navy the 1.12.1 client shows. This function instead
/// picks the linear alpha that reproduces the byte-space result against a
/// black destination: `a' = lin(a·m) / lin(m)`, with `m` the brightest
/// channel. That gives 0.22 for the skill bar background and 0.21 for the
/// fill. It is exact at alpha 0 and 1 and over a black destination; over a
/// bright destination it is too transparent, where [`byte_space_alpha`] is
/// too opaque. A black source uses [`byte_space_alpha`], so the backdrops that
/// function was measured on are unchanged. The exact fix is still the
/// byte-space target the module comment describes.
pub(crate) fn coloured_alpha(rgba: [f32; 4], alpha: f32) -> f32 {
    let a = (rgba[3] * alpha).clamp(0.0, 1.0);
    let m = rgba[0].max(rgba[1]).max(rgba[2]).clamp(0.0, 1.0);
    if m <= 0.0 {
        return byte_space_alpha(a);
    }
    (srgb_to_linear(a * m) / srgb_to_linear(m)).clamp(0.0, 1.0)
}

/// The piecewise IEC 61966-2-1 curve, not a 2.2 power: the same one
/// `render/shaders/gamma.wgsl` and `Color::srgb` use. A different transfer
/// function here would leave the interface and the world a few bytes apart.
fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`coloured_alpha`]'s fixed points: exact at 0 and 1, the black-source
    /// fallback unchanged, and the skill bars' background (the case the
    /// function was written for) near its byte-space weight rather than the
    /// effective 0.79 [`byte_space_alpha`] gave it.
    #[test]
    fn the_coloured_fold_keeps_its_end_points() {
        let bar_background = [0.0, 0.0, 0.75, 0.5];
        assert_eq!(coloured_alpha(bar_background, 0.0), 0.0);
        assert_eq!(coloured_alpha([0.0, 0.0, 0.75, 1.0], 1.0), 1.0);
        let folded = coloured_alpha(bar_background, 1.0);
        assert!(
            (0.15..0.30).contains(&folded),
            "the translucent blue lands near its byte-space weight, got {folded}"
        );
        // Black uses `byte_space_alpha`, so the backdrops are unchanged.
        assert_eq!(coloured_alpha([0.0, 0.0, 0.0, 0.5], 1.0), byte_space_alpha(0.5));
    }

    /// Alpha 0 and 1 are unchanged, and every value between them becomes more
    /// opaque.
    ///
    /// Opaque and clear must stay exact: an alpha of 1 that came back as 0.999
    /// would put a seam between two adjacent opaque quads, and an alpha of 0
    /// that came back as anything else would make every hidden texture in the
    /// directory faintly visible.
    #[test]
    fn the_blend_compensation_pins_both_ends() {
        assert_eq!(byte_space_alpha(1.0), 1.0);
        assert_eq!(byte_space_alpha(0.0), 0.0);
        for a in [0.1, 0.25, 0.5, 0.6, 0.733, 0.9] {
            let out = byte_space_alpha(a);
            assert!(out > a, "{a} -> {out}");
            assert!(out < 1.0, "{a} -> {out}");
        }
        // Out of range on either side is clamped rather than producing a NaN
        // through `powf` of a negative.
        assert_eq!(byte_space_alpha(-1.0), 0.0);
        assert_eq!(byte_space_alpha(2.0), 1.0);
    }

    /// The two cases this correction was written for: the tooltip plate and
    /// the cooldown swirl.
    ///
    /// `UI-Tooltip-Background` is a flat grey at 187/255 tinted to
    /// `TOOLTIP_DEFAULT_BACKGROUND_COLOR`, and `cooldown.blp`'s dark half is
    /// black at about 0.6. Both are near-black sources, the case the
    /// compensation is exact for. The test checks the whole round trip: what a
    /// linear ROP leaves of the destination, against what 1.12's byte-space
    /// one leaves of it.
    #[test]
    fn a_dark_scrim_lands_where_the_games_own_blend_would() {
        // What survives a mix, as a fraction of the destination byte.
        let survives = |alpha: f32, destination: f32| {
            let linear = srgb_to_linear(destination) * (1.0 - byte_space_alpha(alpha));
            // Encoded back to a byte, which is what the sRGB target stores.
            let encoded = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            encoded / destination
        };
        for (alpha, destination) in [(187.0 / 255.0, 200.0 / 255.0), (0.6, 150.0 / 255.0)] {
            let got = survives(alpha, destination);
            // 1.12 leaves exactly `1 - a` of the destination. The compensation
            // is exact under a power transfer function, and the sRGB curve has
            // a linear toe, so the result is two or three percent short;
            // without the compensation it is 60% too much.
            assert!(
                (got - (1.0 - alpha)).abs() < 0.03,
                "alpha {alpha} over {destination}: {got} against {}",
                1.0 - alpha
            );
            // Uncompensated, the destination survives at least 1.4 times as
            // much as it should; this is the too-transparent result that was
            // reported.
            let raw = {
                let linear = srgb_to_linear(destination) * (1.0 - alpha);
                let encoded = 1.055 * linear.powf(1.0 / 2.4) - 0.055;
                encoded / destination
            };
            assert!(raw > (1.0 - alpha) * 1.4, "{raw} against {}", 1.0 - alpha);
        }
    }

    /// The shader's `ALPHA_KEY_CUTOFF` is the reference's 224/255 pushed
    /// through the same byte-space compensation every upload's alpha takes —
    /// the test is against compensated texels, so an uncompensated threshold
    /// would pass texels the reference culls. Pinned here because the WGSL
    /// holds it as a literal.
    #[test]
    fn the_alpha_key_cutoff_is_the_compensated_224() {
        let expected = byte_space_alpha(224.0 / 255.0);

        assert!(
            (expected - 0.9863).abs() < 5e-4,
            "the shader's literal 0.9863 drifted from byte_space_alpha(224/255) = {expected}"
        );
    }
}
