// World overlays: the block selection outline (lines) and block-break cracks (decal quads over
// the block's model, multiplied into the terrain: 2 * src * dst like vanilla's crack blend).
// Camera-relative positions, the chunk camera uniform.

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
    // Vanilla `Globals.GameTime` (unused by the overlays; see endportal.wgsl).
    game_time: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var cracks: texture_2d<f32>;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    // Line: rgba colour; crack: uv in the crack strip (texels) in xy.
    @location(1) data: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) data: vec4<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = camera.view_proj * vec4<f32>(v.pos, 1.0);
    out.data = v.data;
    return out;
}

@fragment
fn fs_line(in: VertexOut) -> @location(0) vec4<f32> {
    return in.data;
}

@fragment
fn fs_crack(in: VertexOut) -> @location(0) vec4<f32> {
    // data.xy: texel coordinates, data.z: stage column origin; wrap inside the 16x16 stage.
    let local = in.data.xy - 16.0 * floor(in.data.xy / 16.0);
    let texel = vec2<i32>(i32(in.data.z) + i32(local.x), i32(local.y));
    let c = textureLoad(cracks, clamp(texel, vec2<i32>(0, 0), vec2<i32>(159, 15)), 0);
    if (c.a < 0.01) {
        discard;
    }
    return vec4<f32>(c.rgb, c.a);
}
