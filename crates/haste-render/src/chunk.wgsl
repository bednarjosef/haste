// Chunk terrain shader (WGSL port of Pomme's chunk.vert/chunk.frag design;
// sprite-rect atlas wrapping from its atlas_sprite.glsl — see THIRD_PARTY.md).
// Light is sky/block nibbles resolved through a lightmap uniform here, so
// time-of-day darkening never triggers a remesh.

struct Camera {
    cam_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    fog_start: f32,
    fog_end: f32,
    // Sky light multiplier by time of day (1 at day, 0 at night; block light is unaffected).
    daylight: f32,
    _pad: f32,
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

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) sprite_uv: vec2<f32>,
    @location(1) light: f32,
    @location(2) tint: vec3<f32>,
    @location(3) fog: f32,
    @location(4) @interpolate(flat) sprite: u32,
    @location(5) world_dist: f32,
};

const POS_RANGE: f32 = 24.0;
const POS_BIAS: f32 = 4.0;
const UV_FIXED_SCALE: f32 = 4095.0;

fn face_shade(index: u32) -> f32 {
    // vanilla CardinalLighting: down .5, up 1, north/south .8, west/east .6,
    // 6 = unshaded.
    if (index == 0u) { return 0.5; }
    if (index == 1u) { return 1.0; }
    if (index == 2u || index == 3u) { return 0.8; }
    if (index == 4u || index == 5u) { return 0.6; }
    return 1.0;
}

fn lightmap(sky: u32, block: u32) -> f32 {
    // Two-band lightmap row lookup. Sky uses camera.light_sky (packed 4x f32
    // per row, 16 levels => expand via arrays below).
    // For simplicity the rows are passed as two vec4s each holding 4 levels
    // is not enough; instead compute the vanilla brightness curve directly:
    // brightness = 0.8^(15-level) style curve approximated by Pomme's table.
    // Pomme LIGHT_TABLE (26.2): index by level.
    let s = f32(sky) / 15.0;
    let b = f32(block) / 15.0;
    // Vanilla light texture: sky fades with time of day; here daylight=1.
    // Use the standard gamma-ish response.
    let sky_l = (pow(s, 1.55) * 0.95 + 0.05) * (0.15 + 0.85 * camera.daylight);
    let block_l = pow(b, 1.55) * 0.95 + 0.05;
    return max(sky_l, block_l);
}

@vertex
fn vs_main(in: VertexIn, @builtin(vertex_index) vi: u32) -> VertexOut {
    // Decode the packed position: u16 x/y/z, one per u32 lane of the u16x4.
    let px = f32(in.pos_packed.x);
    let py = f32(in.pos_packed.y);
    let pz = f32(in.pos_packed.z);
    let local = vec3<f32>(px, py, pz) / 65535.0 * POS_RANGE - POS_BIAS;

    let world = vec3<f32>(in.origin) + local;
    let rel = world - camera.cam_pos.xyz;

    var out: VertexOut;
    out.clip = camera.view_proj * vec4<f32>(rel, 1.0);
    out.sprite_uv = vec2<f32>(f32(in.uv_packed.x), f32(in.uv_packed.y)) / UV_FIXED_SCALE;
    out.sprite = in.sprite_packed;

    // Light nibbles -> lightmap. `light.x` = sky<<4 | block, `light.y` =
    // ao (bits 0..1) | shade index (bits 2..4).
    let packed = in.light.x;
    let sky = packed >> 4u;
    let block = packed & 0xFu;
    let ao_shade_packed = in.light.y;
    let ao_level = ao_shade_packed & 3u;
    let shade_index = (ao_shade_packed >> 2u) & 7u;
    let ao = 0.4 + 0.2 * f32(ao_level); // AO_BRIGHTNESS [0.4, 0.6, 0.8, 1.0]
    out.light = lightmap(sky, block) * ao * face_shade(shade_index);

    // Per-vertex biome/block tint. Vanilla multiplies sRGB texel by sRGB tint; the atlas is
    // sampled linear here, so the tint is linearized too (the product of the two
    // linearizations is the linearization of the product, up to the curve's linear toe).
    let tint_srgb = vec3<f32>(f32(in.light.z), f32(in.light.w), f32(in.tint_b.x)) / 255.0;
    out.tint = select(
        pow((tint_srgb + 0.055) / 1.055, vec3<f32>(2.4)),
        tint_srgb / 12.92,
        tint_srgb <= vec3<f32>(0.04045),
    );

    // Fog: spherical distance bands.
    let dist = length(rel);
    out.world_dist = dist;
    var fog = 0.0;
    if (dist > camera.fog_start) {
        fog = clamp((dist - camera.fog_start) / max(camera.fog_end - camera.fog_start, 0.001), 0.0, 1.0);
    }
    out.fog = fog;
    return out;
}

@group(1) @binding(1) var atlas_texture: texture_2d<f32>;
@group(1) @binding(2) var atlas_sampler: sampler;
@group(1) @binding(3) var<storage, read> sprite_rects: array<vec4<u32>>;

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let rect = sprite_rects[in.sprite];
    let sprite_size = vec2<f32>(rect.zw);
    let atlas_size = vec2<f32>(textureDimensions(atlas_texture));
    let origin = vec2<f32>(rect.xy);

    // Wrap the sprite-local UV inside the sprite rectangle (Pomme
    // atlas_sprite.glsl): fract for repeats, then map into the atlas.
    let uv = (origin + fract(in.sprite_uv) * sprite_size) / atlas_size;
    let color = textureSample(atlas_texture, atlas_sampler, uv);
    if (color.a < 0.5) {
        discard;
    }
    // sRGB-ish: tint and light are applied in linear-ish space like Pomme's
    // shade_chunk_surface, then fog.
    let lit = color.rgb * in.tint * in.light;
    let fogged = mix(lit, camera.fog_color.rgb, in.fog);
    return vec4<f32>(fogged, color.a);
}
