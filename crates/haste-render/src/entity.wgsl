// Entity cube models: CPU-posed triangles in world space over one entity atlas (sRGB, so
// textureLoad returns linear colour like the chunk atlas samples), with the chunk camera and fog.

struct Camera {
    cam_pos: vec4<f32>,
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    // Vanilla fog bands (see chunk.wgsl).
    fog_env_start: f32,
    fog_env_end: f32,
    fog_rd_start: f32,
    fog_rd_end: f32,
    fog_sky_end: f32,
    fog_cloud_end: f32,
    daylight: f32,
    // Outline stroke (pixels / target height) and aspect: only the overlay shader reads
    // them (see chunk.wgsl for why the pad is load-bearing).
    line_width: f32,
    aspect: f32,
    // Vanilla `Globals.GameTime` (the end portal layers scroll with it; see endportal.wgsl).
    game_time: f32,
    // 1 when the target view is sRGB and the hardware encodes (GL fallback): emit linear
    // colour there; 0 with the non-sRGB view, where the sRGB-encoded result is written raw
    // so translucent blends act on the encoded bytes like vanilla's UNORM target.
    hw_srgb: f32,
};

@group(0) @binding(0) var<uniform> camera: Camera;
@group(1) @binding(0) var atlas: texture_2d<f32>;
// The vanilla light texture (LightTexture), shared with the terrain: block light on u,
// sky light on v, sampled at texel centres like vanilla's sample_lightmap.
@group(2) @binding(0) var lightmap_texture: texture_2d<f32>;
@group(2) @binding(1) var lightmap_sampler: sampler;
// The enchantment glint (glint.rs, glint.wgsl): only the `vs_glint`/`fs_glint` entry points
// use these, so pipelines over the atlas alone (particles) stay valid.
@group(1) @binding(1) var glint_item: texture_2d<f32>;
@group(1) @binding(2) var glint_armor: texture_2d<f32>;
@group(1) @binding(3) var glint_sampler: sampler;
@group(1) @binding(4) var<uniform> glint: GlintParams;

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // Linear RGB multiplier (shade x tint) and alpha.
    @location(2) color: vec4<f32>,
    @location(3) rect: vec4<u32>,
    // Light levels for the light texture: block, sky; then the glint mode (`GlintMode`).
    @location(4) light: vec4<u32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) rect: vec4<u32>,
    @location(3) fog: f32,
    @location(4) light_color: vec3<f32>,
    // The glint mode (0: none) and coordinates (`vs_glint` only).
    @location(5) @interpolate(flat) glint_mode: u32,
    @location(6) glint_uv: vec2<f32>,
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

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    return lit_vertex(v);
}

fn lit_vertex(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = camera.view_proj * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    // The vanilla light texture at the entity's (block, sky) light; vertex stage, so an
    // explicit LOD 0 (the level is constant over the entity like vanilla's UV2).
    let light_uv = (vec2<f32>(f32(v.light.x), f32(v.light.y)) + 0.5) / 16.0;
    out.light_color = textureSampleLevel(lightmap_texture, lightmap_sampler, light_uv, 0.0).rgb;
    // Positions are camera-relative; vanilla's two fog bands (see chunk.wgsl).
    let spherical = length(v.pos);
    let cylindrical = max(length(v.pos.xz), abs(v.pos.y));
    out.fog = max(
        linear_fog_value(spherical, camera.fog_env_start, camera.fog_env_end),
        linear_fog_value(cylindrical, camera.fog_rd_start, camera.fog_rd_end),
    );
    out.glint_mode = 0u;
    out.glint_uv = vec2<f32>(0.0);
    return out;
}

// The entity pass with the enchantment glint: vanilla's `GLINT` vertex shader adds
// `texCoordGlint = (TextureMat * vec4(UV0, 0, 1)).xy`; vertices without glint carry mode 0.
@vertex
fn vs_glint(v: VertexIn) -> VertexOut {
    var out = lit_vertex(v);
    out.glint_mode = v.light.z;
    if (v.light.z != GLINT_NONE) {
        out.glint_uv = glint_coords(v.light.z, v.uv, v.rect, glint.translate);
    }
    return out;
}

// The see-through text pass (vanilla `TEXT_SEE_THROUGH`, the `core/text` shader without the
// world snippet): the vertex colour unlit by the light texture and never fogged.
@vertex
fn vs_unlit(v: VertexIn) -> VertexOut {
    var out = lit_vertex(v);
    out.light_color = vec3<f32>(1.0);
    out.fog = 0.0;
    return out;
}

// The atlas texel under `in` (the alpha cutout discards below 0.1, vanilla's ALPHA_CUTOUT).
fn atlas_texel(in: VertexOut) -> vec4<f32> {
    let origin = vec2<i32>(in.rect.xy);
    let size = vec2<i32>(in.rect.zw);
    let texel = clamp(vec2<i32>(floor(in.uv)), origin, origin + size - vec2<i32>(1, 1));
    return textureLoad(atlas, texel, 0);
}

// The fogged, encoded output of a lit linear colour.
fn entity_output(lit: vec3<f32>, fog: f32, alpha: f32) -> vec4<f32> {
    // The target view is non-sRGB, so the sRGB-encoded result is stored raw (the linear
    // math is unchanged; only its encoding moves from the hardware into the shader) and
    // translucent pixels blend on the encoded bytes like vanilla. The GL fallback's sRGB
    // view still gets the hardware encode, so the linear value goes out there.
    let rgb = mix(lit, camera.fog_color.rgb, fog);
    let out = select(srgb_encode(rgb), rgb, camera.hw_srgb == 1.0);
    return vec4<f32>(out, alpha);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let c = atlas_texel(in);
    if (c.a < 0.1) {
        discard;
    }
    let lit = c.rgb * in.color.rgb * in.light_color;
    return entity_output(lit, in.fog, c.a * in.color.a);
}

// `fs_main` plus the glint (vanilla `item.fsh`/`entity.fsh` with `GLINT`): the glint texel
// scaled by `GlintAlpha` is squared and added to the lit colour before the fog, in vanilla's
// stored (sRGB-encoded) space, and the alpha is raised to at least `GlintAlpha`.
@fragment
fn fs_glint(in: VertexOut) -> @location(0) vec4<f32> {
    let c = atlas_texel(in);
    if (c.a < 0.1) {
        discard;
    }
    var lit = c.rgb * in.color.rgb * in.light_color;
    var alpha = c.a * in.color.a;
    if (in.glint_mode != GLINT_NONE) {
        var g: vec4<f32>;
        if (in.glint_mode == GLINT_ARMOR) {
            g = textureSampleLevel(glint_armor, glint_sampler, in.glint_uv, 0.0);
        } else {
            g = textureSampleLevel(glint_item, glint_sampler, in.glint_uv, 0.0);
        }
        g = g * glint.alpha;
        lit = srgb_decode(srgb_encode(lit) + g.rgb * g.rgb);
        alpha = max(alpha, glint.alpha);
    }
    return entity_output(lit, in.fog, alpha);
}

// The inverse sRGB transfer function (IEC 61966-2-1).
fn srgb_decode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(pow((x + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
}

// The sRGB transfer function (IEC 61966-2-1), the one the sRGB texture formats apply.
fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let x = max(c, vec3<f32>(0.0));
    return select(1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055, x * 12.92, x <= vec3<f32>(0.0031308));
}
