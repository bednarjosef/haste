// End portals and end gateways: the layered starfield, a WGSL port of vanilla's
// `rendertype_end_portal` shader pair (26.3 assets, core/rendertype_end_portal.*). The vertex
// stage carries the screen-space projective coords; the fragment stage sums the end-sky base
// and the 15/16 rotated, scaled portal layers, then applies fog like the entity passes.

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
    game_time: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
// Sampler0: the End sky; Sampler1: the portal starfield.
@group(1) @binding(0) var end_sky: texture_2d<f32>;
@group(1) @binding(1) var end_portal: texture_2d<f32>;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    // The portal's layer count (15 for the end portal, 16 for the gateway; vanilla's two
    // shader defines ride per vertex here).
    @location(1) layers: f32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) tex_proj: vec4<f32>,
    @location(1) fog: f32,
    @location(2) @interpolate(flat) layers: f32,
};

// Vanilla projection.glsl `projection_from_position`.
fn projection_from_position(position: vec4<f32>) -> vec4<f32> {
    let half = position * 0.5;
    return vec4<f32>(
        half.x + position.w,
        half.y + position.w,
        position.z,
        position.w,
    );
}

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
    out.tex_proj = projection_from_position(out.clip);
    out.layers = v.layers;
    let spherical = length(v.pos);
    let cylindrical = max(length(v.pos.xz), abs(v.pos.y));
    out.fog = max(
        linear_fog_value(spherical, camera.fog_env_start, camera.fog_env_end),
        linear_fog_value(cylindrical, camera.fog_rd_start, camera.fog_rd_end),
    );
    return out;
}

// Vanilla rendertype_end_portal.fsh `COLORS`: the per-layer tints.
const COLORS = array<vec3<f32>, 16>(
    vec3<f32>(0.022087, 0.098399, 0.110818),
    vec3<f32>(0.011892, 0.095924, 0.089485),
    vec3<f32>(0.027636, 0.101689, 0.100326),
    vec3<f32>(0.046564, 0.109883, 0.114838),
    vec3<f32>(0.064901, 0.117696, 0.097189),
    vec3<f32>(0.063761, 0.086895, 0.123646),
    vec3<f32>(0.084817, 0.111994, 0.166380),
    vec3<f32>(0.097489, 0.154120, 0.091064),
    vec3<f32>(0.106152, 0.131144, 0.195191),
    vec3<f32>(0.097721, 0.110188, 0.187229),
    vec3<f32>(0.133516, 0.138278, 0.148582),
    vec3<f32>(0.070006, 0.243332, 0.235792),
    vec3<f32>(0.196766, 0.142899, 0.214696),
    vec3<f32>(0.047281, 0.315338, 0.321970),
    vec3<f32>(0.204675, 0.390010, 0.302066),
    vec3<f32>(0.080955, 0.314821, 0.661491),
);

// The portal's layer count comes per vertex (15 for the portal, 16 for the gateway).

// Vanilla end_portal_layer: the layer's translate (17/layer on x, the GameTime scroll on y),
// z rotation and scale, composed with SCALE_TRANSLATE. Ported to row-major mat4 math; GLSL
// multiplies `texProj0 * mat` (row vector), which is `transpose(mat) * v` here.
fn end_portal_layer(layer: f32, tex_proj: vec4<f32>) -> vec4<f32> {
    let translate = mat4x4<f32>(
        vec4<f32>(1.0, 0.0, 0.0, 17.0 / layer),
        vec4<f32>(0.0, 1.0, 0.0, (2.0 + layer / 1.5) * (camera.game_time * 1.5)),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0),
    );
    let angle = radians((layer * layer * 4321.0 + layer * 9.0) * 2.0);
    let rotate = mat2x2<f32>(
        vec2<f32>(cos(angle), -sin(angle)),
        vec2<f32>(sin(angle), cos(angle)),
    );
    let scale = (4.5 - layer / 4.0) * 2.0;
    let rotate_scale = mat4x4<f32>(
        vec4<f32>(rotate[0].x * scale, rotate[0].y * scale, 0.0, 0.0),
        vec4<f32>(rotate[1].x * scale, rotate[1].y * scale, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, scale, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0),
    );
    let scale_translate = mat4x4<f32>(
        vec4<f32>(0.5, 0.0, 0.0, 0.25),
        vec4<f32>(0.0, 0.5, 0.0, 0.25),
        vec4<f32>(0.0, 0.0, 1.0, 0.0),
        vec4<f32>(0.0, 0.0, 0.0, 1.0),
    );
    // mat4 composition in GLSL column-major with row-vector multiply `v * A * B` equals
    // `(B^T * A^T) * v` with WGSL's column-vector convention.
    let m = transpose(rotate_scale) * transpose(translate) * transpose(scale_translate);
    return m * tex_proj;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let uv = in.tex_proj.xy / in.tex_proj.w;
    let dims_sky = vec2<f32>(textureDimensions(end_sky));
    let dims_portal = vec2<f32>(textureDimensions(end_portal));
    let base = textureLoad(end_sky, vec2<i32>(uv * dims_sky), 0).rgb * COLORS[0];
    var color = base;
    let layers = u32(in.layers);
    for (var i: u32 = 0u; i < layers; i = i + 1u) {
        let projected = end_portal_layer(f32(i + 1u), in.tex_proj);
        let uv_layer = projected.xy / projected.w;
        let c = textureLoad(end_portal, vec2<i32>(uv_layer * dims_portal), 0).rgb;
        color = color + c * COLORS[i];
    }
    return vec4<f32>(mix(color, camera.fog_color.rgb, in.fog), 1.0);
}
