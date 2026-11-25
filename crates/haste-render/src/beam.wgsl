// Beacon and end-gateway beams: the entity vertex format over the entity atlas, but with the
// lightmap left out — vanilla's `rendertype_beacon_beam` shader is texture * vertex colour +
// fog (vanilla `RenderPipelines.BEACON_BEAM_OPAQUE`/`BEACON_BEAM_TRANSLUCENT`, whose snippet
// binds no lighting).

struct Camera {
    cam_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    fog_env_start: f32,
    fog_env_end: f32,
    fog_rd_start: f32,
    fog_rd_end: f32,
    fog_sky_end: f32,
    fog_cloud_end: f32,
    daylight: f32,
    line_width: f32,
    aspect: f32,
    game_time: f32,
    // 1 when the target view is sRGB and the hardware encodes (GL fallback); see entity.wgsl.
    hw_srgb: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var atlas: texture_2d<f32>;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // Linear RGB multiplier (the section colour) and alpha (255 inner, 32 glow).
    @location(2) color: vec4<f32>,
    @location(3) rect: vec4<u32>,
    // Unused here (the beam ignores the lightmap); kept for the shared vertex format.
    @location(4) light: vec4<u32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) rect: vec4<u32>,
    @location(3) fog: f32,
};

// Vanilla fog.glsl `linear_fog_value` (see entity.wgsl).
fn linear_fog_value(dist: f32, start: f32, end: f32) -> f32 {
    if (dist <= start) {
        return 0.0;
    }
    if (dist >= end) {
        return 1.0;
    }
    return (dist - start) / (end - start);
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = camera.view_proj * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    let spherical = length(v.pos);
    let cylindrical = max(length(v.pos.xz), abs(v.pos.y));
    out.fog = max(
        linear_fog_value(spherical, camera.fog_env_start, camera.fog_env_end),
        linear_fog_value(cylindrical, camera.fog_rd_start, camera.fog_rd_end),
    );
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let origin = vec2<i32>(in.rect.xy);
    let size = vec2<i32>(in.rect.zw);
    let texel = clamp(vec2<i32>(floor(in.uv)), origin, origin + size - vec2<i32>(1, 1));
    let c = textureLoad(atlas, texel, 0);
    // Vanilla `rendertype_beacon_beam.fsh`: texture * vertex colour (the section's tint
    // times the pass alpha — 1.0 inner, 32/255 glow) + fog. Encoded for the non-sRGB view
    // like entity.wgsl; the GL fallback's sRGB view gets the linear value.
    let lit = c.rgb * in.color.rgb;
    let rgb = mix(lit, camera.fog_color.rgb, in.fog);
    let out = select(srgb_encode(rgb), rgb, camera.hw_srgb == 1.0);
    return vec4<f32>(out, c.a * in.color.a);
}

// The sRGB transfer function (IEC 61966-2-1), the one the sRGB texture formats apply.
fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}
