// The world border's forcefield wall (vanilla `world_border.vsh` / `.fsh` through
// `RenderPipelines.WORLD_BORDER`): the wall's quads are camera-relative (the offset is
// vanilla's ModelOffset), the forcefield texture scrolls by the texture matrix, and the
// fragment multiplies the texel by the status tint and the distance alpha (ColorModulator).

struct BorderUniforms {
    view_proj: mat4x4<f32>,
    /// The wall's origin relative to the camera (vanilla's ModelOffset).
    offset: vec3<f32>,
    /// 1 when the target view is sRGB and the hardware encodes (GL fallback).
    hw_srgb: f32,
    /// The status tint (sRGB) and the distance alpha (ColorModulator).
    color: vec4<f32>,
    /// The texture matrix's translation along u and v (the scroll).
    tex_offset: f32,
    _pad: vec3<f32>,
};

@group(0) @binding(0) var<uniform> border: BorderUniforms;
@group(0) @binding(1) var forcefield: texture_2d<f32>;
@group(0) @binding(2) var forcefield_sampler: sampler;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = border.view_proj * vec4<f32>(in.pos + border.offset, 1.0);
    out.uv = in.uv + vec2<f32>(border.tex_offset, border.tex_offset);
    return out;
}

fn srgb_to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // The texel times the tint in sRGB, written raw into the non-sRGB view (or linearised
    // for the GL fallback's hardware encode), as the clouds do.
    let texel = textureSample(forcefield, forcefield_sampler, in.uv);
    let c = texel * border.color;
    let rgb = select(
        c.rgb,
        vec3<f32>(srgb_to_linear(c.r), srgb_to_linear(c.g), srgb_to_linear(c.b)),
        border.hw_srgb == 1.0,
    );
    return vec4<f32>(rgb, c.a);
}
