// The boats' water patch (vanilla `rendertype_water_mask`): position-only triangles that
// write depth and no colour, drawn before the translucent water so the water surface inside
// a floating hull disappears. See entity/boat.rs.

struct Camera {
    cam_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VertexIn {
    @location(0) pos: vec3<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> @builtin(position) vec4<f32> {
    // Camera-relative like the entity vertices.
    return camera.view_proj * vec4<f32>(v.pos, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Masked off by the pipeline (no colour writes); only the depth write matters.
    return vec4<f32>(0.0, 0.0, 0.0, 0.0);
}
