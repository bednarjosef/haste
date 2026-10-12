// GUI batch: coloured and textured triangles in GUI pixels, one atlas, nearest texel fetch with
// per-vertex wrap rectangles (tiling and nine-slice edges need repeat inside an atlas region).
// Written to a non-sRGB view of the target so blending happens on sRGB values like vanilla.

struct Uniforms {
    // GUI size in GUI pixels (framebuffer / scale).
    size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var atlas: texture_2d<f32>;

struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) rect: vec4<u32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) rect: vec4<u32>,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = vec4<f32>(v.pos.x / u.size.x * 2.0 - 1.0, 1.0 - v.pos.y / u.size.y * 2.0, 0.0, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if (in.rect.z == 0u) {
        return in.color;
    }
    let origin = vec2<f32>(in.rect.xy);
    let size = vec2<f32>(in.rect.zw);
    let local = in.uv - origin;
    let wrapped = local - size * floor(local / size);
    let texel = clamp(vec2<i32>(floor(origin + wrapped)), vec2<i32>(in.rect.xy), vec2<i32>(in.rect.xy + in.rect.zw) - vec2<i32>(1, 1));
    let c = textureLoad(atlas, texel, 0) * in.color;
    if (c.a <= 0.0) {
        discard;
    }
    return c;
}
