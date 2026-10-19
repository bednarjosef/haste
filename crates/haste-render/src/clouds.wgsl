// Fancy clouds (vanilla CloudRenderer + clouds.vsh/clouds.fsh): one 3-byte face
// record per drawn face (vanilla's encodeFace layout, padded to a u32 in the instance
// buffer), decoded here into the quad's corners — no vertex data on the GPU, like
// vanilla's texel buffer. Camera-cell-relative positions with the per-frame offset
// inside the cell from the uniform, face shades in the vertex colour, distance fog in
// the fragment.

struct Clouds {
    view_proj: mat4x4<f32>,
    // The mesh's offset inside the camera's cell: (-xInCell, cloud bottom relative to
    // the camera, -zInCell) — vanilla's CloudOffset.
    offset: vec3<f32>,
    // 1 when the target view is sRGB and the hardware encodes (GL fallback); see
    // entity.wgsl. 0 with the non-sRGB view, where the sRGB-encoded bytes are written raw.
    hw_srgb: f32,
    // The cloud colour (sRGB; the shade multiplies it) and alpha.
    color: vec4<f32>,
    // The cloud fog end distance.
    fog_end: f32,
    _pad1: vec4<f32>,
};

@group(0) @binding(0) var<uniform> clouds: Clouds;

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

// sRGB to linear, for the GL fallback's sRGB view (the colour chain runs in sRGB like
// vanilla's; the normal non-sRGB view takes the encoded value raw).
fn srgb_to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

// The record's flag bits (vanilla clouds.vsh): the direction in the low three, the
// inside-face and top-colour flags, and the cell coordinates' low bits.
const FLAG_MASK_DIR: u32 = 7u;
const FLAG_INSIDE_FACE: u32 = 16u;
const FLAG_USE_TOP_COLOR: u32 = 32u;
const FLAG_EXTRA_Z: u32 = 64u;
const FLAG_EXTRA_X: u32 = 128u;

// Vanilla clouds.vsh's bit-packed corner table: `vertex(index)` returns the unit corner
// (0/1 per axis) of face corner `index` = direction * 4 + corner.
const PACKED_X: u32 = 0xF03CC3;
const PACKED_Y: u32 = 0x6666F0;
const PACKED_Z: u32 = 0xC3F066;

fn unit_vertex(index: u32) -> vec3<f32> {
    return vec3<f32>(
        f32((PACKED_X >> index) & 1u),
        f32((PACKED_Y >> index) & 1u),
        f32((PACKED_Z >> index) & 1u),
    );
}

// The record's signed half-resolution byte, sign-extended.
fn as_i8(byte: u32) -> i32 {
    let v = i32(byte & 0xFFu);
    return select(v - 256, v, v < 128);
}

// Vanilla clouds.vsh's faceColors: bottom 0.7, top 1.0, north/south 0.8, west/east 0.9.
fn face_shade(direction: u32) -> f32 {
    if (direction == 0u) {
        return 0.7;
    }
    if (direction == 1u) {
        return 1.0;
    }
    if (direction == 2u || direction == 3u) {
        return 0.8;
    }
    return 0.9;
}

@vertex
fn vs_main(
    @builtin(vertex_index) vid: u32,
    @location(0) record: u32,
) -> VertexOut {
    // The two triangles of the face's quad: (0, 1, 2) and (0, 2, 3), the corner order
    // vanilla's sequential quad index buffer gives; inside faces visit the quad's
    // corners in the opposite order (vanilla's `3 - quadVertex`).
    var corner_order = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    // The record's three bytes: x >> 1, z >> 1, dir | flags.
    let dir_and_flags = (record >> 16) & 0xFFu;
    if ((dir_and_flags & FLAG_INSIDE_FACE) != 0u) {
        corner_order = array<u32, 6>(3u, 2u, 1u, 3u, 1u, 0u);
    }
    let direction = dir_and_flags & FLAG_MASK_DIR;
    let use_top_color = (dir_and_flags & FLAG_USE_TOP_COLOR) != 0u;
    let cell_x = as_i8(record) * 2 + i32((dir_and_flags & FLAG_EXTRA_X) >> 7);
    let cell_z = as_i8(record >> 8) * 2 + i32((dir_and_flags & FLAG_EXTRA_Z) >> 6);
    let corner = corner_order[vid % 6u];
    // Vanilla clouds.vsh: faceVertex * CellSize + vec3(cellX, 0, cellZ) * CellSize +
    // CloudOffset, CellSize being (12, 4, 12).
    let unit = unit_vertex(direction * 4u + corner);
    var out: VertexOut;
    let pos = vec3<f32>(f32(cell_x), 0.0, f32(cell_z)) * vec3<f32>(12.0, 4.0, 12.0)
        + unit * vec3<f32>(12.0, 4.0, 12.0)
        + clouds.offset;
    out.clip = clouds.view_proj * vec4<f32>(pos, 1.0);
    out.pos = pos;
    out.shade = select(face_shade(direction), 1.0, use_top_color);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Vanilla clouds.vsh: vertexColor = faceColors[direction] * CloudColor; clouds.fsh:
    // alpha *= 1 - linear_fog_value(spherical distance, 0, cloud end). The shade
    // multiplies the sRGB colour, and the encoded result is written raw into the
    // non-sRGB view so the translucent cloud pixels blend on the sRGB bytes like
    // vanilla's plain UNORM target (the GL fallback's sRGB view gets the linear value
    // for its hardware encode instead).
    let c = in.shade * clouds.color.rgb;
    let rgb = select(c, vec3<f32>(srgb_to_linear(c.r), srgb_to_linear(c.g), srgb_to_linear(c.b)), clouds.hw_srgb == 1.0);
    let fade = 1.0 - linear_fog_value(length(in.pos), 0.0, clouds.fog_end);
    return vec4<f32>(rgb, clouds.color.a * fade);
}
