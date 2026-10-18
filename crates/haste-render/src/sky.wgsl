// Sky, celestial bodies, stars, clouds and rain. One environment atlas (sun, moon phases, clouds,
// rain, snow, end sky) fetched by texel with per-vertex wrap rectangles. Colours in linear space
// (the target is sRGB).

struct Sky {
    view_proj: mat4x4<f32>,
    rot_view_proj: mat4x4<f32>,
    inv_rot_view_proj: mat4x4<f32>,
    celestial: mat4x4<f32>,
    sky_color: vec4<f32>,
    fog_color: vec4<f32>,
    sunrise_color: vec4<f32>,
    // xyz: horizontal direction of the sunrise glow, w: star brightness.
    sunrise_dir: vec4<f32>,
    // x: the sky kind (0 overworld, 1 none, 2 end); the other slots are unused.
    params: vec4<f32>,
    // Vanilla fog bands for the sky dome and weather: environmental start/end,
    // render distance start/end.
    fog_bands: vec4<f32>,
    // x: sky fog end, y: cloud fog end.
    fog_bands2: vec4<f32>,
    // The sunrise/sunset fan's rotation (camera rotation frame).
    sunrise_matrix: mat4x4<f32>,
    // The End flash (vanilla `SkyRenderer.renderEndFlash`): intensity in x (0 = none),
    // the x/y direction angles in degrees in y/z.
    end_flash: vec4<f32>,
    // The End flash quad's pose (camera rotation frame).
    end_flash_matrix: mat4x4<f32>,
    // End sky texture rectangle in the atlas.
    end_rect: vec4<u32>,
};

@group(0) @binding(0) var<uniform> sky: Sky;
@group(0) @binding(1) var atlas: texture_2d<f32>;

fn fetch(uv: vec2<f32>, rect: vec4<u32>) -> vec4<f32> {
    let origin = vec2<f32>(rect.xy);
    let size = vec2<f32>(rect.zw);
    let local = uv - origin;
    let wrapped = local - size * floor(local / size);
    let texel = clamp(vec2<i32>(floor(origin + wrapped)), vec2<i32>(rect.xy), vec2<i32>(rect.xy + rect.zw) - vec2<i32>(1, 1));
    return textureLoad(atlas, texel, 0);
}

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

// The terrain fog bands (vanilla `total_fog_value`) for the weather columns.
fn fog_factor(pos: vec3<f32>) -> f32 {
    let spherical = length(pos);
    let cylindrical = max(length(pos.xz), abs(pos.y));
    return max(
        linear_fog_value(spherical, sky.fog_bands.x, sky.fog_bands.y),
        linear_fog_value(cylindrical, sky.fog_bands.z, sky.fog_bands.w),
    );
}

// ---- background: full-screen triangle ----

struct BackgroundOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_background(@builtin(vertex_index) i: u32) -> BackgroundOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var out: BackgroundOut;
    out.clip = vec4<f32>(p, 1.0, 1.0);
    out.ndc = p;
    return out;
}

@fragment
fn fs_background(in: BackgroundOut) -> @location(0) vec4<f32> {
    let far = sky.inv_rot_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let near = sky.inv_rot_view_proj * vec4<f32>(in.ndc, 0.0, 1.0);
    let dir = normalize(far.xyz / far.w - near.xyz / near.w);
    let kind = u32(sky.params.w);
    if (kind == 2u) {
        // The End: the end sky texture tiled on the faces of a cube around the camera.
        let a = abs(dir);
        var uv: vec2<f32>;
        if (a.y >= a.x && a.y >= a.z) {
            uv = dir.xz / a.y;
        } else if (a.x >= a.z) {
            uv = dir.zy / a.x;
        } else {
            uv = dir.xy / a.z;
        }
        let texel = (uv * 0.5 + 0.5) * 16.0 * vec2<f32>(sky.end_rect.zw);
        let c = fetch(vec2<f32>(sky.end_rect.xy) + texel, sky.end_rect);
        return vec4<f32>(c.rgb * sky.sky_color.rgb, 1.0);
    }
    // Skyless dimensions: flat fog colour (the overworld dome is the disc fan).
    return vec4<f32>(sky.fog_color.rgb, 1.0);
}

// ---- sky disc: the dome fan, fogged by spherical distance over the sky fog end ----

struct DiscOut {
    @builtin(position) clip: vec4<f32>,
    // The model-space position: vanilla computes the fog per FRAGMENT (sky.fsh
    // `fog_spherical_distance(Position)`), and the disc's triangles span center-to-ring
    // (distance 16 to 512) — interpolating vertex fog values across them is far off.
    @location(0) pos: vec3<f32>,
};

@vertex
fn vs_disc(v: VertexIn) -> DiscOut {
    var out: DiscOut;
    // Positions are in the camera's rotation frame already (radius 512 around it).
    let clip = sky.rot_view_proj * vec4<f32>(v.pos, 1.0);
    out.clip = vec4<f32>(clip.xy, clip.w * 0.99999, clip.w);
    out.pos = v.pos;
    return out;
}

@fragment
fn fs_disc(in: DiscOut) -> @location(0) vec4<f32> {
    // Vanilla sky.fsh: max(linear(spherical, 0, skyEnd), linear(cylindrical, skyEnd,
    // skyEnd)); the cylindrical term is 1 only past the ring where the spherical is too.
    let fog = linear_fog_value(length(in.pos), 0.0, sky.fog_bands2.x);
    return vec4<f32>(mix(sky.sky_color.rgb, sky.fog_color.rgb, fog), 1.0);
}

@fragment
fn fs_dark_disc(in: DiscOut) -> @location(0) vec4<f32> {
    // Vanilla renderDarkDisc: the sky pipeline coloured black, fogged like the sky disc.
    let fog = linear_fog_value(length(in.pos), 0.0, sky.fog_bands2.x);
    return vec4<f32>(mix(vec3<f32>(0.0), sky.fog_color.rgb, fog), 1.0);
}

// ---- sunrise/sunset fan: translucent timeline colour, alpha ramping center to ring ----

@vertex
fn vs_sunrise(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    let world = sky.sunrise_matrix * vec4<f32>(v.pos, 1.0);
    let clip = sky.rot_view_proj * vec4<f32>(world.xyz, 1.0);
    out.clip = vec4<f32>(clip.xy, clip.w * 0.99999, clip.w);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    out.fog = 0.0;
    return out;
}

@fragment
fn fs_sunrise(in: VertexOut) -> @location(0) vec4<f32> {
    let a = in.color.a * sky.sunrise_color.a;
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(sky.sunrise_color.rgb, a);
}

// ---- celestial: sun, moon, stars in the camera's rotation frame, additive ----

struct VertexIn {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) rect: vec4<u32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) rect: vec4<u32>,
    @location(3) fog: f32,
};

@vertex
fn vs_celestial(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    let world = sky.celestial * vec4<f32>(v.pos, 1.0);
    let clip = sky.rot_view_proj * vec4<f32>(world.xyz, 1.0);
    // Pinned to the far plane: behind everything.
    out.clip = vec4<f32>(clip.xy, clip.w * 0.99999, clip.w);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    out.fog = 0.0;
    return out;
}

@fragment
fn fs_celestial(in: VertexOut) -> @location(0) vec4<f32> {
    if (in.rect.z == 0u) {
        // Stars: plain quads coloured (b, b, b, b) with the star brightness b, blended
        // src-alpha/one (vanilla `RenderPipelines.STARS`, `BlendFunction.OVERLAY`): b * b.
        let b = sky.sunrise_dir.w;
        return vec4<f32>(in.color.rgb * b * b, 1.0);
    }
    // Additive with the texture's alpha (src alpha, one), like vanilla's sun and moon.
    let c = fetch(in.uv, in.rect);
    return vec4<f32>(c.rgb * c.a * in.color.rgb * in.color.a, 1.0);
}

// ---- the End flash: a celestial quad in the flash's own pose (vanilla renderEndFlash) ----

@vertex
fn vs_end_flash(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    let world = sky.end_flash_matrix * vec4<f32>(v.pos, 1.0);
    let clip = sky.rot_view_proj * vec4<f32>(world.xyz, 1.0);
    // Pinned to the far plane: behind everything, like the other celestial bodies.
    out.clip = vec4<f32>(clip.xy, clip.w * 0.99999, clip.w);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    out.fog = 0.0;
    return out;
}
