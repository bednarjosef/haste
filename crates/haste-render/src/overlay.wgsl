// World overlays: the block selection outline (edges drawn as screen-space quads, vanilla
// `rendertype_lines` style: wgpu has no wide lines, so the vertex shader expands each edge
// into a quad `line_width` framebuffer pixels wide) and block-break cracks (decal quads over
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
    // Outline stroke: framebuffer pixels divided by the target height, and the
    // target's aspect (width / height). Together they give the NDC offset of the
    // stroke: one pixel is 2 / height in NDC y, 2 / (aspect * height) in NDC x.
    line_width: f32,
    aspect: f32,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

@group(1) @binding(0) var cracks: texture_2d<f32>;

struct CrackVertexIn {
    @location(0) pos: vec3<f32>,
    // uv in the crack strip (texels) in xy, stage column origin in z.
    @location(1) data: vec4<f32>,
};

struct LineVertexIn {
    @location(0) pos: vec3<f32>,
    // xyz: the edge's other endpoint relative to `pos` (the screen-space expansion
    // direction); w: which side of the edge this vertex lies on (-1 or 1).
    @location(1) to: vec4<f32>,
    @location(2) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) data: vec4<f32>,
};

// Vanilla `rendertype_lines` expansion: the edge's direction on screen, offset
// perpendicular to it by half the stroke width on each side of the edge.
@vertex
fn vs_line(v: LineVertexIn) -> VertexOut {
    let start = camera.view_proj * vec4<f32>(v.pos, 1.0);
    let end = camera.view_proj * vec4<f32>(v.pos + v.to.xyz, 1.0);
    // The edge's direction in screen pixels: NDC delta with x scaled by the aspect.
    var dir = (end.xy / end.w) - (start.xy / start.w);
    dir = normalize(vec2<f32>(dir.x * camera.aspect, dir.y));
    // Perpendicular, back in NDC units, half the stroke on each side.
    let perp = vec2<f32>(-dir.y / camera.aspect, dir.x) * camera.line_width;
    var out: VertexOut;
    out.clip = vec4<f32>(start.xy + v.to.w * perp * start.w, start.zw);
    out.data = v.color;
    return out;
}

@vertex
fn vs_main(v: CrackVertexIn) -> VertexOut {
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
