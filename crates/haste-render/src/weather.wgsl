// Rain and snow columns (vanilla WeatherEffectRenderer + particle shader): camera-relative
// quads over the sky atlas, tinted by the lightmap and fogged by the terrain bands.

struct Weather {
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    fog_env_start: f32,
    fog_env_end: f32,
    fog_rd_start: f32,
    fog_rd_end: f32,
    // 1 when the target view is sRGB and the hardware encodes (GL fallback); see
    // entity.wgsl. 0 with the non-sRGB view, where the sRGB-encoded bytes are written raw.
    hw_srgb: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> weather: Weather;
// The sky atlas (sRGB, textureLoad returns linear colour).
@group(0) @binding(1) var atlas: texture_2d<f32>;
// The vanilla light texture, sampled at texel centres like the entity pass.
@group(0) @binding(2) var lightmap_texture: texture_2d<f32>;
@group(0) @binding(3) var lightmap_sampler: sampler;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) rect: vec4<u32>,
    // Light levels for the light texture: block, sky.
    @location(4) light: vec2<u32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) rect: vec4<u32>,
    @location(3) fog: f32,
    @location(4) light_color: vec3<f32>,
};

// Vanilla fog.glsl `linear_fog_value`.
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
    out.clip = weather.view_proj * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    // The light texture at the column's (block, sky) light; vertex stage like the entity
    // pass (the level is constant over the column like vanilla's UV2).
    let light_uv = (vec2<f32>(f32(v.light.x), f32(v.light.y)) + 0.5) / 16.0;
    out.light_color = textureSampleLevel(lightmap_texture, lightmap_sampler, light_uv, 0.0).rgb;
    // Camera-relative; vanilla's two fog bands (see entity.wgsl).
    let spherical = length(v.pos);
    let cylindrical = max(length(v.pos.xz), abs(v.pos.y));
    out.fog = max(
        linear_fog_value(spherical, weather.fog_env_start, weather.fog_env_end),
        linear_fog_value(cylindrical, weather.fog_rd_start, weather.fog_rd_end),
    );
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Vanilla's weather textures repeat (the columns' V run many tiles through the
    // texture); the atlas cannot rely on the sampler's REPEAT, so wrap the UV into the
    // sprite's rect like the sky pass's fetch.
    let origin = vec2<f32>(in.rect.xy);
    let size = vec2<f32>(in.rect.zw);
    let local = in.uv - origin;
    let wrapped = local - size * floor(local / size);
    let texel = vec2<i32>(floor(origin + wrapped));
    let c = textureLoad(atlas, texel, 0);
    if (c.a < 0.1) {
        discard;
    }
    let lit = c.rgb * in.light_color;
    // Encoded for the non-sRGB view like entity.wgsl; the GL fallback's sRGB view gets
    // the linear value.
    let rgb = mix(lit, weather.fog_color.rgb, in.fog);
    let out = select(srgb_encode(rgb), rgb, weather.hw_srgb == 1.0);
    return vec4<f32>(out, c.a * in.color.a);
}

// ---- lightning bolts (vanilla rendertype_lightning): additive, fogged vertex colour ----

struct LightningIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LightningOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) fog: f32,
};

@vertex
fn vs_lightning(v: LightningIn) -> LightningOut {
    var out: LightningOut;
    out.clip = weather.view_proj * vec4<f32>(v.pos, 1.0);
    out.color = v.color;
    let spherical = length(v.pos);
    let cylindrical = max(length(v.pos.xz), abs(v.pos.y));
    out.fog = max(
        linear_fog_value(spherical, weather.fog_env_start, weather.fog_env_end),
        linear_fog_value(cylindrical, weather.fog_rd_start, weather.fog_rd_end),
    );
    return out;
}

@fragment
fn fs_lightning(in: LightningOut) -> @location(0) vec4<f32> {
    // Vanilla: color * (1 - total_fog_value), additive blend. Encoded for the non-sRGB
    // view like entity.wgsl, so the additive blend lands on the target's sRGB bytes like
    // vanilla's; the GL fallback's sRGB view gets the linear value.
    let rgb = in.color.rgb * (1.0 - in.fog);
    return vec4<f32>(select(srgb_encode(rgb), rgb, weather.hw_srgb == 1.0), in.color.a * (1.0 - in.fog));
}

// The sRGB transfer function (IEC 61966-2-1), the one the sRGB texture formats apply.
fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}
