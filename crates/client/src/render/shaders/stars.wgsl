// The star dome: `Environments\Stars\Stars.m2`, drawn inside the sky.
//
// **The second surface in the world that is not fogged**, for the same reason
// the gradient dome is not (see `sky.wgsl`): this *is* sky, and fogging it would
// paint the horizon haze over the zenith. Nothing here is lit either — a star is
// its own light, the model's batches all declare `unlit`, and a sun term over
// them would put the daylight sun on the night sky.
//
// So the whole fragment is one texel times one colour. `tint.rgb` is white
// today and exists because the client's own star object carries a colour word
// beside its alpha; `tint.a` is the batch's own constant opacity (the model
// states five of them, 0.25..0.75, which is what gives the field its depth)
// multiplied by the hour's fade, which is `light::celestial::STARS`.

//
// **Written at the far plane, like the gradient dome under it** — see
// `sky.wgsl` and `render::sky::SkyMaterial::specialize`. The dome the stars sit
// on is 900 yards out and the loaded terrain reaches half as far again, so a
// star field that kept its own depth would be drawn *over* the far mountains.

#import bevy_pbr::forward_io::VertexOutput
#import vale::gamma::to_frame

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> tint: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var star_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var star_sampler: sampler;

struct StarOutput {
    @location(0) colour: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fragment(in: VertexOutput) -> StarOutput {
    let texel = textureSample(star_texture, star_sampler, in.uv);
    var out: StarOutput;
    // Encoded on the way out — see `to_frame`. The star field is alpha-blended
    // over the dome, so this one *is* a blend the space applies to: a 0.25-alpha
    // star mixed in linear and a 0.25-alpha star mixed in bytes are different
    // stars, and the byte one is the client's.
    out.colour = to_frame(vec4<f32>(texel.rgb * tint.rgb, texel.a * tint.a));
    out.depth = 0.0;
    return out;
}
