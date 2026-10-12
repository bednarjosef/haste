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
    // xyz: camera position, w: fog start.
    cam_pos: vec4<f32>,
    // x: fog end, y: cloud offset (blocks), z: cloud height, w: sky kind (0 overworld, 1 none, 2 end).
    params: vec4<f32>,
    // x: cloud brightness, y: cloud alpha.
    clouds: vec4<f32>,
    // End sky texture rectangle in the atlas, clouds rectangle.
    end_rect: vec4<u32>,
    cloud_rect: vec4<u32>,
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

fn fog_factor(dist: f32) -> f32 {
    if (dist <= sky.cam_pos.w) {
        return 0.0;
    }
    return clamp((dist - sky.cam_pos.w) / max(sky.params.x - sky.cam_pos.w, 0.001), 0.0, 1.0);
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
    if (kind == 1u) {
        return vec4<f32>(sky.fog_color.rgb, 1.0);
    }
    // Overworld: sky colour overhead fading into the fog colour at the horizon, the sunrise
    // glow low in the sun's direction.
    let up = smoothstep(-0.02, 0.45, dir.y);
    var color = mix(sky.fog_color.rgb, sky.sky_color.rgb, up);
    let glow = sky.sunrise_color.a * pow(max(dot(normalize(vec3<f32>(dir.x, 0.0, dir.z) + vec3<f32>(0.0, 0.0001, 0.0)), sky.sunrise_dir.xyz), 0.0), 3.0)
        * (1.0 - smoothstep(0.0, 0.55, abs(dir.y)));
    color = mix(color, sky.sunrise_color.rgb, clamp(glow, 0.0, 1.0));
    return vec4<f32>(color, 1.0);
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
        // Stars: plain quads at the star brightness.
        let b = sky.sunrise_dir.w;
        return vec4<f32>(in.color.rgb * b, 1.0);
    }
    let c = fetch(in.uv, in.rect);
    return vec4<f32>(c.rgb * in.color.rgb * in.color.a, 1.0);
}

// ---- world-space: rain and snow columns (alpha blended, depth tested) ----

@vertex
fn vs_world(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = sky.view_proj * vec4<f32>(v.pos, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    out.fog = fog_factor(length(v.pos - sky.cam_pos.xyz));
    return out;
}

@fragment
fn fs_world(in: VertexOut) -> @location(0) vec4<f32> {
    let c = fetch(in.uv, in.rect) * in.color;
    if (c.a < 0.01) {
        discard;
    }
    return vec4<f32>(mix(c.rgb, sky.fog_color.rgb, in.fog), c.a);
}

// ---- clouds: one plane at cloud height, each clouds.png texel a 12x12 block cell ----

struct CloudOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_clouds(@builtin(vertex_index) i: u32) -> CloudOut {
    let corner = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    )[i];
    let extent = max(sky.params.x * 2.0, 256.0);
    let world = vec3<f32>(sky.cam_pos.x + corner.x * extent, sky.params.z, sky.cam_pos.z + corner.y * extent);
    var out: CloudOut;
    out.clip = sky.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    return out;
}

@fragment
fn fs_clouds(in: CloudOut) -> @location(0) vec4<f32> {
    let cell = floor(vec2<f32>(in.world.x + sky.params.y, in.world.z) / 12.0);
    let c = fetch(vec2<f32>(sky.cloud_rect.xy) + cell + vec2<f32>(0.5, 0.5), sky.cloud_rect);
    if (c.a < 0.5) {
        discard;
    }
    let dist = length(in.world - sky.cam_pos.xyz);
    // Clouds fade out over twice the fog distance (they sit far beyond the terrain).
    let fade = clamp(1.0 - (dist - sky.params.x) / max(sky.params.x, 1.0), 0.0, 1.0);
    let rgb = mix(sky.fog_color.rgb, vec3<f32>(sky.clouds.x), fade);
    return vec4<f32>(rgb, sky.clouds.y * fade);
}
