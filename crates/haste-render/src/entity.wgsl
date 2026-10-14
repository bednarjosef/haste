// Entity cube models: CPU-posed triangles in world space over one entity atlas (sRGB, so
// textureLoad returns linear colour like the chunk atlas samples), with the chunk camera and fog.

struct Camera {
    cam_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    // Vanilla fog bands (see chunk.wgsl).
    fog_env_start: f32,
    fog_env_end: f32,
    fog_rd_start: f32,
    fog_rd_end: f32,
    fog_sky_end: f32,
    fog_cloud_end: f32,
    daylight: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var atlas: texture_2d<f32>;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // Linear RGB multiplier (light x shade x tint) and alpha.
    @location(2) color: vec4<f32>,
    @location(3) rect: vec4<u32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) rect: vec4<u32>,
    @location(3) fog: f32,
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
    out.clip = camera.view_proj * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    // Positions are camera-relative; vanilla's two fog bands (see chunk.wgsl).
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
    if (c.a < 0.1) {
        discard;
    }
    let lit = c.rgb * in.color.rgb;
    return vec4<f32>(mix(lit, camera.fog_color.rgb, in.fog), c.a * in.color.a);
}
