// The title panorama (vanilla core/panorama shaders): the unit cube around the camera, each
// fragment sampling the cube texture in the direction of its vertex position.

@group(0) @binding(0) var<uniform> mvp: mat4x4<f32>;
@group(0) @binding(1) var cube: texture_cube<f32>;
@group(0) @binding(2) var cube_sampler: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) dir: vec3<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> VsOut {
    var out: VsOut;
    out.clip = mvp * vec4<f32>(pos, 1.0);
    out.dir = pos;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(cube, cube_sampler, in.dir);
}
