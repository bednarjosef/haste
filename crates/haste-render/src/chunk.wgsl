// Chunk terrain shader (WGSL port of Pomme's chunk.vert/chunk.frag design;
// sprite-rect atlas wrapping from its atlas_sprite.glsl — see THIRD_PARTY.md).
// Light is sky/block nibbles resolved through the vanilla light texture
// (group 2, see lightmap.rs), so time-of-day darkening never triggers a remesh.

struct Camera {
    cam_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    // Vanilla fog bands: environmental (spherical distance) and render distance
    // (cylindrical: max of horizontal distance and |y|), then the sky and cloud fades.
    fog_env_start: f32,
    fog_env_end: f32,
    fog_rd_start: f32,
    fog_rd_end: f32,
    fog_sky_end: f32,
    fog_cloud_end: f32,
    // Sky light multiplier by time of day (1 at day, 0 at night; block light is unaffected).
    daylight: f32,
    // Outline stroke (pixels / target height) and aspect: only the overlay shader reads
    // them. Every Camera declaration must stay layout-identical (renderer.rs asserts
    // the Rust and WGSL sizes are equal, a uniform struct's size is rounded up to its
    // alignment in WGSL, so the explicit pad below is load-bearing).
    line_width: f32,
    aspect: f32,
    // Vanilla `Globals.GameTime` (the end portal layers scroll with it; see endportal.wgsl).
    game_time: f32,
    // 1 when the target view is sRGB and the hardware encodes on store (the GL fallback
    // without non-sRGB views): the fragment then emits linear colour like it used to.
    // 0 with the normal non-sRGB view: the fragment writes vanilla's sRGB-encoded bytes
    // raw, so the translucent blends act on them like vanilla's plain UNORM target.
    hw_srgb: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;

struct VertexIn {
    // location 0: pos as u16x4 (x, y, z, pad) — WGSL unpacks via bit ops below.
    @location(0) pos_packed: vec4<u32>,
    // location 1: uv as u16x2.
    @location(1) uv_packed: vec2<u32>,
    // location 2: sprite u16.
    @location(2) sprite_packed: u32,
    // location 3: light [u8; 4] (sky|block, ao|shade, tint red, tint green), raw bytes.
    @location(3) light: vec4<u32>,
    // location 5: (tint blue, pad).
    @location(5) tint_b: vec2<u32>,
    // Instance-rate: section origin in world blocks.
    @location(4) origin: vec3<i32>,
};

// Three outputs per vertex (vanilla terrain.vsh has five, fewer components): the pass is
// bound by vertex and primitive throughput, and on RDNA every output is one more vec4 the
// vertex exports, so the colour and the fog share one.
struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) sprite_uv: vec2<f32>,
    // rgb: vanilla's `vertexColor` (tint, face shade and AO times the lightmap texel, all per
    // vertex as terrain.vsh does); a: the fog factor.
    @location(1) color_fog: vec4<f32>,
    @location(2) @interpolate(flat) sprite: u32,
};

const POS_RANGE: f32 = 24.0;
const POS_BIAS: f32 = 4.0;
const UV_FIXED_SCALE: f32 = 4095.0;

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

fn face_shade(index: u32) -> f32 {
    // vanilla CardinalLighting: down .5, up 1, north/south .8, west/east .6,
    // 6 = unshaded.
    if (index == 0u) { return 0.5; }
    if (index == 1u) { return 1.0; }
    if (index == 2u || index == 3u) { return 0.8; }
    if (index == 4u || index == 5u) { return 0.6; }
    return 1.0;
}

// The vanilla light texture (LightTexture), uploaded by the renderer when its inputs
// change: block light on u, sky light on v, sampled at texel centres like vanilla's
// sample_lightmap ((uv + 0.5) / 16). Vertex stage, so explicit LOD 0 (levels are per
// vertex anyway, like vanilla's vertex-shader lightmap sample).
@group(2) @binding(0) var lightmap_texture: texture_2d<f32>;
@group(2) @binding(1) var lightmap_sampler: sampler;

/// Vanilla smooth light: channels in 16th-of-a-level units (0..240); the
/// lightmap sample is `minecraft_sample_lightmap`'s `(value + 8) / 256`
/// (a level's texel centre plus the fraction between levels, bilinear).
fn light_color16(sky16: u32, block16: u32) -> vec3<f32> {
    let uv = (vec2<f32>(f32(block16), f32(sky16)) + vec2<f32>(8.0)) / 256.0;
    return textureSampleLevel(lightmap_texture, lightmap_sampler, uv, 0.0).rgb;
}

@vertex
fn vs_main(in: VertexIn, @builtin(vertex_index) vi: u32) -> VertexOut {
    // Decode the packed position: u16 x/y/z, one per u32 lane of the u16x4.
    let px = f32(in.pos_packed.x);
    let py = f32(in.pos_packed.y);
    let pz = f32(in.pos_packed.z);
    let local = vec3<f32>(px, py, pz) / 65535.0 * POS_RANGE - POS_BIAS;

    // Light in vanilla smooth units (16 per level, 0..240): `light.x` = sky,
    // `tint_b.y` = block, `light.y` = ao occluder count (bits 0..2) | shade
    // index (bits 3..5).
    let sky16 = u32(in.light.x);
    let block16 = u32(in.tint_b.y);
    let ao_shade_packed = in.light.y;
    let ao_count = ao_shade_packed & 7u;
    let shade_index = (ao_shade_packed >> 3u) & 7u;

    return finish_vertex(
        local,
        vec2<f32>(f32(in.uv_packed.x), f32(in.uv_packed.y)) / UV_FIXED_SCALE,
        sky16,
        block16,
        ao_count,
        shade_index,
        // Per-vertex biome/block tint, kept as the raw sRGB-encoded bytes: vanilla
        // multiplies the encoded texel by the encoded tint (see `shade`).
        vec3<f32>(f32(in.light.z), f32(in.light.w), f32(in.tint_b.x)) / 255.0,
        in.sprite_packed,
        in.origin,
    );
}

/// The vertex tail shared by both terrain formats: camera transform, lightmap
/// sample with AO and face shade, tint and the two fog bands (all the math
/// `vs_main` used to inline).
fn finish_vertex(
    local: vec3<f32>,
    sprite_uv: vec2<f32>,
    sky16: u32,
    block16: u32,
    ao_count: u32,
    shade_index: u32,
    tint: vec3<f32>,
    sprite: u32,
    origin: vec3<i32>,
) -> VertexOut {
    let world = vec3<f32>(origin) + local;
    let rel = world - camera.cam_pos.xyz;

    var out: VertexOut;
    out.clip = camera.view_proj * vec4<f32>(rel, 1.0);
    out.sprite_uv = sprite_uv;
    out.sprite = sprite;

    // Vanilla's per-vertex AO brightness: the average of four shade samples
    // (two sides, the diagonal, the face cell), each 1.0 or 0.2 — i.e.
    // 1 - 0.2 * occluders.
    let ao = 1.0 - 0.2 * f32(ao_count);
    let color = tint * light_color16(sky16, block16) * ao * face_shade(shade_index);

    // Fog: vanilla's two linear bands, both per-vertex like its vertex-shader varyings —
    // the environmental band on the spherical distance, the render distance band on the
    // cylindrical distance (max of horizontal and |y|); the fragment takes their max.
    let spherical = length(rel);
    let cylindrical = max(length(rel.xz), abs(rel.y));
    let fog = max(
        linear_fog_value(spherical, camera.fog_env_start, camera.fog_env_end),
        linear_fog_value(cylindrical, camera.fog_rd_start, camera.fog_rd_end),
    );
    out.color_fog = vec4<f32>(color, fog);
    return out;
}

// ---------------------------------------------------------------------------
// Quad pulling (`--terrain-format quad`): the vertex shader reads one compact
// record per quad from the arena's storage buffers (group 3, bound once per
// arena) and expands the corners itself from `vertex_index`; the only index
// buffer is the static corner pattern shared by every draw. See
// quad_format.rs for the record layouts.

struct QuadIn {
    // Instance-rate, one per draw: the section origin and the span kind
    // (0 = rect records, 1 = general records). The span's record base rides
    // the draw's base_vertex: the static index pattern (0..3 per quad) plus
    // base_vertex makes vertex_index >> 2 the record and vertex_index & 3
    // the corner.
    @location(0) origin: vec3<i32>,
    @location(1) kind: u32,
};

@group(3) @binding(0) var<storage, read> quad_rects: array<u32>;
@group(3) @binding(1) var<storage, read> quad_generals: array<u32>;

/// A decoded corner, whichever record kind it came from.
struct Decoded {
    local: vec3<f32>,
    sprite_uv: vec2<f32>,
    sky16: u32,
    block16: u32,
    ao_count: u32,
    shade_index: u32,
    tint: vec3<f32>,
    sprite: u32,
};

/// Position component `i` of corner `c` of a `PackedGeneral`: u16 `3*c + i`,
/// split across the record's first six words.
fn general_pos_component(base_word: u32, c: u32, i: u32) -> u32 {
    let idx = c * 3u + i;
    let w = quad_generals[base_word + idx / 2u];
    return select(w & 0xFFFFu, w >> 16u, (idx & 1u) == 1u);
}

/// One `PackedGeneral` (64 bytes, sixteen u32 words) from the general-record
/// buffer, decoded to corner `c`. Word layout mirrors quad_format.rs: words
/// 0..6 are the four corner positions (three u16 each), words 6..10 the
/// corner UVs (two u16 each — one word per corner), words 10..12 the packed
/// per-corner light, word 12 the sprite and tint's red/green, word 13 the
/// tint's blue.
fn pull_general(base_word: u32, c: u32) -> Decoded {
    let local = vec3<f32>(
        f32(general_pos_component(base_word, c, 0u)),
        f32(general_pos_component(base_word, c, 1u)),
        f32(general_pos_component(base_word, c, 2u)),
    ) / 65535.0 * POS_RANGE - POS_BIAS;
    let uv_word = quad_generals[base_word + 6u + c];
    let uv = vec2<f32>(f32(uv_word & 0xFFFFu), f32(uv_word >> 16u)) / UV_FIXED_SCALE;
    let w10 = quad_generals[base_word + 10u];
    let w11 = quad_generals[base_word + 11u];
    let w12 = quad_generals[base_word + 12u];
    let w13 = quad_generals[base_word + 13u];
    var bits = 0u;
    if (c == 0u) {
        bits = w10 & 0x7FFFu;
    } else if (c == 1u) {
        bits = w10 >> 15u;
    } else if (c == 2u) {
        bits = ((w10 >> 30u) | (w11 << 2u)) & 0x7FFFu;
    } else {
        bits = (w11 >> 13u) & 0x7FFFu;
    }
    return Decoded(
        local,
        uv,
        (bits & 0x3Fu) * 4u,
        ((bits >> 6u) & 0x3Fu) * 4u,
        (bits >> 12u) & 7u,
        (w11 >> 28u) & 7u,
        vec3<f32>(
            f32((w12 >> 16u) & 0xFFu),
            f32((w12 >> 24u) & 0xFFu),
            f32(w13 & 0xFFu),
        ) / 255.0,
        w12 & 0xFFFFu,
    );
}

/// One `PackedQuad` (32 bytes, eight u32 words) from the quad arena, expanded
/// to corner `c` of quad `g`. Layout mirrors quad_format.rs: the header (top
/// byte of word 7) carries the normal axis, the uv drivers and the corner
/// ring; the light word pair packs 15 bits per corner plus the shade index at
/// bit 60.
fn pull_rect(base_word: u32, c: u32) -> Decoded {
    let w0 = quad_rects[base_word];
    let w1 = quad_rects[base_word + 1u];
    let w2 = quad_rects[base_word + 2u];
    let w3 = quad_rects[base_word + 3u];
    let w4 = quad_rects[base_word + 4u];
    let w5 = quad_rects[base_word + 5u];
    let w6 = quad_rects[base_word + 6u];
    let w7 = quad_rects[base_word + 7u];
    let header = w7 >> 24u;
    let n = header & 3u;
    // The corner ring: grid corner of vertex 0 and the walk direction.
    let start = (header >> 4u) & 3u;
    let step = select(1u, 3u, ((header >> 6u) & 1u) == 1u);
    let grid = (start + step * c) & 3u;
    let sa = grid == 1u || grid == 2u;
    let sb = grid == 2u || grid == 3u;
    // Tangent bounds (a, b are the two non-normal axes in ascending order).
    let u = select(w0 >> 16u, w1 & 0xFFFFu, sa);
    let v = select(w1 >> 16u, w2 & 0xFFFFu, sb);
    let plane = w0 & 0xFFFFu;
    // The corner's (a, b, plane) mapped onto x/y/z by the normal axis.
    var pos3 = vec3<f32>(f32(plane), f32(u), f32(v));
    if (n == 1u) {
        pos3 = vec3<f32>(f32(u), f32(plane), f32(v));
    } else if (n == 2u) {
        pos3 = vec3<f32>(f32(u), f32(v), f32(plane));
    }
    let local = pos3 / 65535.0 * POS_RANGE - POS_BIAS;
    // UV: each component follows one corner sign (its driver), 0- or 1-bound.
    let du = ((header >> 2u) & 1u) == 1u;
    let dv = ((header >> 3u) & 1u) == 1u;
    let uv = vec2<f32>(
        f32(select(w2 >> 16u, w3 & 0xFFFFu, select(sa, sb, du))),
        f32(select(w3 >> 16u, w4 & 0xFFFFu, select(sa, sb, dv))),
    ) / UV_FIXED_SCALE;
    // Light: corner `c`'s 15 bits out of the packed pair, shade at bit 60.
    var bits = 0u;
    if (c == 0u) {
        bits = w5 & 0x7FFFu;
    } else if (c == 1u) {
        bits = w5 >> 15u;
    } else if (c == 2u) {
        bits = ((w5 >> 30u) | (w6 << 2u)) & 0x7FFFu;
    } else {
        bits = (w6 >> 13u) & 0x7FFFu;
    }
    return Decoded(
        local,
        uv,
        (bits & 0x3Fu) * 4u,
        ((bits >> 6u) & 0x3Fu) * 4u,
        (bits >> 12u) & 7u,
        (w6 >> 28u) & 7u,
        vec3<f32>(f32(w7 & 0xFFu), f32((w7 >> 8u) & 0xFFu), f32((w7 >> 16u) & 0xFFu)) / 255.0,
        w4 >> 16u,
    );
}

@vertex
fn vs_quad(in: QuadIn, @builtin(vertex_index) vi: u32) -> VertexOut {
    var d: Decoded;
    if (in.kind == 0u) {
        // Rect records: the index pattern makes vi = 4 * record + corner.
        d = pull_rect((vi >> 2u) * 8u, vi & 3u);
    } else {
        // General records: vi = 4 * record + corner.
        d = pull_general((vi >> 2u) * 16u, vi & 3u);
    }
    return finish_vertex(
        d.local, d.sprite_uv, d.sky16, d.block16, d.ao_count, d.shade_index, d.tint, d.sprite,
        in.origin,
    );
}



@group(1) @binding(1) var atlas_texture: texture_2d<f32>;
@group(1) @binding(2) var atlas_sampler: sampler;
@group(1) @binding(3) var<storage, read> sprite_rects: array<vec4<u32>>;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in, 0.0);
}

// Cutout terrain (leaves, plants) discards below vanilla CUTOUT_TERRAIN's
// ALPHA_CUTOUT 0.5 (RenderPipelines.CUTOUT_TERRAIN).
@fragment
fn fs_cutout(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in, 0.5);
}

// Translucent terrain (water, glass) keeps vanilla's lower cutout: 26.3's
// TRANSLUCENT_TERRAIN pipeline defines ALPHA_CUTOUT 0.1 where CUTOUT_TERRAIN
// uses 0.5.
@fragment
fn fs_translucent(in: VertexOut) -> @location(0) vec4<f32> {
    return shade(in, 0.1);
}

fn shade(in: VertexOut, alpha_cutoff: f32) -> vec4<f32> {
    let rect = sprite_rects[in.sprite];
    let sprite_size = vec2<f32>(rect.zw);
    let atlas_size = vec2<f32>(textureDimensions(atlas_texture));
    let origin = vec2<f32>(rect.xy);

    // Wrap the sprite-local UV inside the sprite rectangle (Pomme
    // atlas_sprite.glsl): fract for repeats, then map into the atlas.
    // The derivatives for mip selection and the texel snap come from the
    // continuous sprite-local UV, not from the wrapped one: fract() makes the
    // mapped UV jump a whole sprite at every repeat boundary (a greedy-merged
    // quad repeats the sprite per block), and a derivative taken across that
    // jump clamps textureSampleGrad to the atlas's coarsest mip — the
    // atlas-average grey, drawn as a thin dashed line on every block edge
    // (the user bug this fixes; vanilla never merges, so it has no wraps).
    let uv = (origin + fract(in.sprite_uv) * sprite_size) / atlas_size;
    let atlas_scale = sprite_size / atlas_size;
    let color = sample_nearest(
        uv,
        1.0 / atlas_size,
        dpdx(in.sprite_uv) * atlas_scale,
        dpdy(in.sprite_uv) * atlas_scale,
    );
    // Vanilla's SOLID_TERRAIN defines no ALPHA_CUTOUT (RenderPipelines.SOLID_TERRAIN):
    // solid terrain never discards, so the GPU keeps early-Z for the whole pass.
    if (alpha_cutoff > 0.0 && color.a < alpha_cutoff) {
        discard;
    }
    // Vanilla's terrain shader (terrain.vsh/terrain.fsh) does all of its colour math on
    // the encoded (sRGB) values, into a plain RGBA8 target: texel * vertex colour (tint,
    // AO and face shade times the lightmap texel, multiplied per vertex), then the fog
    // mix with the encoded fog colour.
    // The atlas here is an UNORM texture like vanilla's (`TextureAtlas.createTexture`),
    // so the sample is already the encoded texel (and the GPU filters encoded values,
    // like vanilla's). The target view is non-sRGB, so the encoded result is written raw
    // and the translucent blend acts on the encoded bytes exactly like vanilla's; on the
    // GL fallback's sRGB view the hardware would encode, so the linear value goes out
    // there instead. Doing the multiplies on linear values instead lifts every
    // factor below 1 to roughly its 1/2.4th power (a 0.6 side face or a two-occluder AO
    // corner draws ~25% too bright).
    let lit = color.rgb * in.color_fog.rgb;
    let fogged = mix(lit, srgb_encode(camera.fog_color.rgb), in.color_fog.a);
    return vec4<f32>(select(fogged, srgb_decode(fogged), camera.hw_srgb == 1.0), color.a);
}

// The sRGB transfer function (IEC 61966-2-1), the one the sRGB texture formats apply.
fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}

// Vanilla texture_sampling.glsl `sampleNearest` (UseRgss == 0, the default): snap
// the UV toward the containing texel's centre by how much of the texel one screen
// pixel covers, then sample with explicit screen derivatives (the atlas sampler is
// LINEAR/LINEAR trilinear; the snap keeps near terrain crisp). The derivatives are
// passed in by the caller: they must be continuous (see `shade`).
fn sample_nearest(uv: vec2<f32>, pixel_size: vec2<f32>, du: vec2<f32>, dv: vec2<f32>) -> vec4<f32> {
    let texel_screen_size = sqrt(du * du + dv * dv);
    let uv_texel_coords = uv / pixel_size;
    let texel_center = round(uv_texel_coords) - vec2<f32>(0.5);
    var texel_offset = uv_texel_coords - texel_center;
    texel_offset = (texel_offset - vec2<f32>(0.5)) * pixel_size / texel_screen_size + vec2<f32>(0.5);
    texel_offset = clamp(texel_offset, vec2<f32>(0.0), vec2<f32>(1.0));
    return textureSampleGrad(atlas_texture, atlas_sampler, (texel_center + texel_offset) * pixel_size, du, dv);
}
