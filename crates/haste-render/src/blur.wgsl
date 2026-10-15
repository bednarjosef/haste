// Menu background blur: vanilla's post/box_blur.fsh as WGSL. Six passes (three separable
// rounds of horizontal + vertical) over the stored sRGB bytes like vanilla's RGBA8_UNORM
// main target; bilinear sampling halves the sample count (step of two texels).

struct BlurUniforms {
    dir: vec2<f32>,
    radius: f32,
    _pad: f32,
    in_size: vec2<f32>,
};

@group(0) @binding(0) var in_tex: texture_2d<f32>;
@group(0) @binding(1) var in_sampler: sampler;
@group(0) @binding(2) var<uniform> u: BlurUniforms;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VertexOut {
    // Full-screen triangle.
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VertexOut;
    out.clip = vec4<f32>(p[i], 0.0, 1.0);
    out.uv = vec2<f32>(p[i].x * 0.5 + 0.5, 0.5 - p[i].y * 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let one_texel = vec2<f32>(1.0, 1.0) / u.in_size;
    let step = one_texel * u.dir;
    let radius = floor(u.radius + 0.5);
    var blurred = vec4<f32>(0.0);
    var a = -radius + 0.5;
    loop {
        if (a > radius) {
            break;
        }
        blurred = blurred + textureSampleLevel(in_tex, in_sampler, in.uv + step * a, 0.0);
        a = a + 2.0;
    }
    blurred = blurred + textureSampleLevel(in_tex, in_sampler, in.uv + step * radius, 0.0) * 0.5;
    return blurred / (radius + 0.5);
}
