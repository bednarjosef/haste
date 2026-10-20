// The depth pyramid of the GPU occlusion culling (see gpu_cull/pyramid.rs).

// The pyramid: level 0 holds the farthest depth of each 2x2 pixel block, each next
// level the farthest of 2x2 texels of the one before.
@group(0) @binding(0) var depth_src: texture_depth_2d;
@group(0) @binding(1) var level_dst: texture_storage_2d<r32float, write>;
@group(0) @binding(2) var level_src: texture_2d<f32>;

// The source texels a destination texel covers: its 2x2 block, plus the extra row or
// column of an odd-sized source on the last texel (mip sizes round down).
fn block(id: vec2<u32>, out: vec2<u32>, src: vec2<u32>) -> vec4<u32> {
    let p = id * 2u;
    let end = select(p + 1u, src - 1u, id == out - 1u);
    return vec4<u32>(p, min(end, src - 1u));
}

@compute @workgroup_size(8, 8)
fn reduce_depth(@builtin(global_invocation_id) id: vec3<u32>) {
    let out = textureDimensions(level_dst);
    if (id.x >= out.x || id.y >= out.y) {
        return;
    }
    let b = block(id.xy, out, textureDimensions(depth_src));
    var far = 0.0;
    for (var y = b.y; y <= b.w; y++) {
        for (var x = b.x; x <= b.z; x++) {
            far = max(far, textureLoad(depth_src, vec2<u32>(x, y), 0));
        }
    }
    textureStore(level_dst, id.xy, vec4<f32>(far, 0.0, 0.0, 0.0));
}

@compute @workgroup_size(8, 8)
fn reduce_level(@builtin(global_invocation_id) id: vec3<u32>) {
    let out = textureDimensions(level_dst);
    if (id.x >= out.x || id.y >= out.y) {
        return;
    }
    let b = block(id.xy, out, textureDimensions(level_src));
    var far = 0.0;
    for (var y = b.y; y <= b.w; y++) {
        for (var x = b.x; x <= b.z; x++) {
            far = max(far, textureLoad(level_src, vec2<u32>(x, y), 0).r);
        }
    }
    textureStore(level_dst, id.xy, vec4<f32>(far, 0.0, 0.0, 0.0));
}
