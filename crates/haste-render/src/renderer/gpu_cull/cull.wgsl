// GPU occlusion culling of terrain draws (see gpu_cull.rs): a depth pyramid built
// from the depth of the sections drawn early, and a test of every draw's section
// box against it. Haste's own; vanilla culls only on the CPU.

struct Params {
    view_proj: mat4x4<f32>,
    // Target size in pixels.
    size: vec2<f32>,
    // Pyramid levels.
    levels: u32,
    // Draws in the list.
    draws: u32,
    // Runs in the list: the late phase's counts follow the early phase's.
    runs: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

// One draw's section: its mesh bounds relative to the camera, its slab slot, and
// where its run of the multi-draw starts.
struct Draw {
    lo: vec3<f32>,
    slot: u32,
    hi: vec3<f32>,
    run: u32,
    run_start: u32,
    translucent: u32,
    _pad0: u32,
    _pad1: u32,
}

// `DrawIndexedIndirectArgs`.
struct Args {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> draws: array<Draw>;
@group(0) @binding(2) var<storage, read> args_in: array<Args>;
@group(0) @binding(3) var<storage, read_write> args_early: array<Args>;
@group(0) @binding(4) var<storage, read_write> args_late: array<Args>;
@group(0) @binding(5) var<storage, read_write> counts: array<atomic<u32>>;
@group(0) @binding(6) var<storage, read> vis_prev: array<u32>;
@group(0) @binding(7) var<storage, read_write> vis_cur: array<u32>;
@group(0) @binding(8) var pyramid: texture_2d<f32>;

// Early phase: the opaque draws of sections visible last frame, compacted per run.
@compute @workgroup_size(64)
fn select_early(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= params.draws) {
        return;
    }
    let d = draws[i];
    if (d.translucent != 0u || vis_prev[d.slot] == 0u) {
        return;
    }
    let j = atomicAdd(&counts[d.run], 1u);
    args_early[d.run_start + j] = args_in[i];
}

// Whether the box is hidden behind the early depth: its nearest depth lies behind
// the farthest depth the pyramid holds over its screen rectangle.
fn occluded(d: Draw) -> bool {
    var lo_px = vec2<f32>(1e9, 1e9);
    var hi_px = vec2<f32>(-1e9, -1e9);
    var near = 1.0;
    for (var c = 0u; c < 8u; c++) {
        let p = vec3<f32>(
            select(d.lo.x, d.hi.x, (c & 1u) != 0u),
            select(d.lo.y, d.hi.y, (c & 2u) != 0u),
            select(d.lo.z, d.hi.z, (c & 4u) != 0u),
        );
        let clip = params.view_proj * vec4<f32>(p, 1.0);
        // A corner at or behind the eye: the box reaches the camera, keep it.
        if (clip.w <= 1e-4) {
            return false;
        }
        let ndc = clip.xyz / clip.w;
        let px = vec2<f32>((ndc.x * 0.5 + 0.5) * params.size.x, (0.5 - ndc.y * 0.5) * params.size.y);
        lo_px = min(lo_px, px);
        hi_px = max(hi_px, px);
        near = min(near, ndc.z);
    }
    let lo = clamp(lo_px, vec2<f32>(0.0), params.size - 1.0);
    let hi = clamp(hi_px, vec2<f32>(0.0), params.size - 1.0);
    // The level where the rectangle spans at most two texels per axis: level L's texel
    // covers 2^(L + 1) pixels.
    let extent = max(max(hi.x - lo.x, hi.y - lo.y), 1.0);
    let level = min(u32(max(ceil(log2(extent)) - 1.0, 0.0)), params.levels - 1u);
    let texel = f32(1u << (level + 1u));
    // Clamped: the last texel of a level also covers the pixels past an odd edge.
    let last = vec2<i32>(textureDimensions(pyramid, level)) - 1;
    let t0 = min(vec2<i32>(floor(lo / texel)), last);
    let t1 = min(vec2<i32>(floor(hi / texel)), last);
    var far = 0.0;
    for (var y = t0.y; y <= t1.y; y++) {
        for (var x = t0.x; x <= t1.x; x++) {
            far = max(far, textureLoad(pyramid, vec2<i32>(x, y), i32(level)).r);
        }
    }
    // One step of the 24-bit depth buffer of slack, against its rounding.
    return near > far + 1.2e-7;
}

// Late phase, after the pyramid: every draw is tested. Opaque draws not drawn early
// are compacted into the late runs; translucent draws keep their place (their order is
// the blend order) and get zero instances when hidden.
@compute @workgroup_size(64)
fn test_late(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if (i >= params.draws) {
        return;
    }
    let d = draws[i];
    let visible = !occluded(d);
    if (visible) {
        vis_cur[d.slot] = 1u;
    }
    if (d.translucent != 0u) {
        var a = args_in[i];
        a.instance_count = select(0u, 1u, visible);
        args_late[i] = a;
        return;
    }
    if (visible && vis_prev[d.slot] == 0u) {
        let j = atomicAdd(&counts[params.runs + d.run], 1u);
        args_late[d.run_start + j] = args_in[i];
    }
}
