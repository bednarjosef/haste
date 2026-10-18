// Fancy clouds (vanilla CloudRenderer + clouds.vsh/clouds.fsh): the extruded cell mesh
// around the camera, camera-cell-relative positions with the per-frame offset inside the
// cell from the uniform, face shades in the vertex colour, distance fog in the fragment.

struct Clouds {
    view_proj: mat4x4<f32>,
    // The mesh's offset inside the camera's cell: (-xInCell, cloud bottom relative to
    // the camera, -zInCell) — vanilla's CloudOffset.
    offset: vec3<f32>,
    _pad0: f32,
    // The cloud colour (sRGB; the shade multiplies it before linearising) and alpha.
    color: vec4<f32>,
    // The cloud fog end distance.
    fog_end: f32,
    _pad1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> clouds: Clouds;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) shade: f32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // The camera-relative position, for the spherical fog distance.
    @location(0) pos: vec3<f32>,
    // The face shade.
    @location(1) shade: f32,
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

// sRGB to linear (the target is sRGB; the colour chain runs in sRGB like vanilla's).
fn srgb_to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    let pos = v.pos + clouds.offset;
    out.clip = clouds.view_proj * vec4<f32>(pos, 1.0);
    out.pos = pos;
    out.shade = v.shade;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Vanilla clouds.vsh: vertexColor = faceColors[direction] * CloudColor; clouds.fsh:
    // alpha *= 1 - linear_fog_value(spherical distance, 0, cloud end). The shade
    // multiplies the sRGB colour before linearising (vanilla writes both straight into
    // its sRGB target).
    let c = in.shade * clouds.color.rgb;
    let rgb = vec3<f32>(
        srgb_to_linear(c.r),
        srgb_to_linear(c.g),
        srgb_to_linear(c.b),
    );
    let fade = 1.0 - linear_fog_value(length(in.pos), 0.0, clouds.fog_end);
    return vec4<f32>(rgb, clouds.color.a * fade);
}
