// GUI batch: coloured and textured triangles in GUI pixels, one atlas, nearest texel fetch with
// per-vertex wrap rectangles (tiling and nine-slice edges need repeat inside an atlas region).
//
// Colour space: vanilla's GUI is composited onto its main target with BlendFunction.TRANSLUCENT
// on stored (sRGB-encoded) bytes -- vanilla's MainTarget and its GUI atlas are both RGBA8_UNORM
// and its GUI shaders emit the encoded value, so what vanilla writes is what it blends. The
// GUI here draws through a non-sRGB view of the target, which leaves the stored bytes raw and
// blends them the same way, so `fs_main` writes its colour unchanged in that case.
//
// On backends whose surface has no non-sRGB view (GL) the target *is* sRGB-encoded: the hardware
// then decodes the destination, blends in linear space and encodes the result. `fs_main` converts
// its colour to linear so opaque pixels still land on vanilla's value (encode(decode(x)) == x),
// while translucent pixels blend in the wrong space -- far from vanilla for Multiply and Invert,
// where the raw value is measurably closer, so only `fs_main` converts.

struct Uniforms {
    // GUI size in GUI pixels (framebuffer / scale).
    size: vec2<f32>,
    // x: 1 when the target is sRGB-encoded by the hardware (no non-sRGB view on this backend),
    // so the normal blend needs its colour in linear space.
    flags: vec2<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var atlas: texture_2d<f32>;
// The enchantment glint on items (glint.rs, glint.wgsl; the armour texture is bound but
// unused here).
@group(0) @binding(2) var glint_item: texture_2d<f32>;
@group(0) @binding(3) var glint_armor: texture_2d<f32>;
@group(0) @binding(4) var glint_sampler: sampler;
@group(0) @binding(5) var<uniform> glint: GlintParams;

struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) rect: vec4<u32>,
    // The glint mode (`GlintMode`, 0: none).
    @location(4) glint: u32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) rect: vec4<u32>,
    @location(3) @interpolate(flat) glint_mode: u32,
    @location(4) glint_uv: vec2<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = vec4<f32>(v.pos.x / u.size.x * 2.0 - 1.0, 1.0 - v.pos.y / u.size.y * 2.0, 0.0, 1.0);
    out.uv = v.uv;
    out.color = v.color;
    out.rect = v.rect;
    out.glint_mode = v.glint;
    out.glint_uv = vec2<f32>(0.0);
    if (v.glint != GLINT_NONE) {
        out.glint_uv = glint_coords(v.glint, v.uv, v.rect, glint.translate);
    }
    return out;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

/// The GUI fragment colour, `linearize` converting it for an sRGB-encoded target. `discard` on a
/// fully transparent texel keeps an empty glyph or sprite hole from writing anything.
fn gui_color(in: VertexOut, linearize: bool) -> vec4<f32> {
    var c: vec4<f32>;
    if (in.rect.z == 0u) {
        c = in.color;
    } else {
        let origin = vec2<f32>(in.rect.xy);
        let size = vec2<f32>(in.rect.zw);
        let local = in.uv - origin;
        let wrapped = local - size * floor(local / size);
        let texel = clamp(vec2<i32>(floor(origin + wrapped)), vec2<i32>(in.rect.xy), vec2<i32>(in.rect.xy + in.rect.zw) - vec2<i32>(1, 1));
        c = textureLoad(atlas, texel, 0) * in.color;
        if (c.a <= 0.0) {
            discard;
        }
        // An item with foil (vanilla's GUI items draw through the item pipelines' `GLINT`
        // define): the glint texel scaled by `GlintAlpha`, squared, added to the stored colour;
        // the alpha raised to at least `GlintAlpha`.
        if (in.glint_mode != GLINT_NONE) {
            let g = textureSampleLevel(glint_item, glint_sampler, in.glint_uv, 0.0) * glint.alpha;
            c = vec4<f32>(c.rgb + g.rgb * g.rgb, max(c.a, glint.alpha));
        }
    }
    if (linearize) {
        return vec4<f32>(to_linear(c.rgb), c.a);
    }
    return c;
}

// Vanilla's GUI_TEXTURED (BlendFunction.TRANSLUCENT: src SRC_ALPHA, dst ONE_MINUS_SRC_ALPHA,
// alpha ONE, ONE_MINUS_SRC_ALPHA): the normal blend, linearized on an sRGB target.
@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return gui_color(in, u.flags.x > 0.5);
}

// Vanilla's VIGNETTE (src ZERO, dst ONE_MINUS_SRC_COLOR, alpha ZERO, ONE): a product of the
// destination, so the raw texel stays closest to vanilla's stored-value product.
@fragment
fn fs_main_multiply(in: VertexOut) -> @location(0) vec4<f32> {
    return gui_color(in, false);
}

// Vanilla's CROSSHAIR (BlendFunction.INVERT: src ONE_MINUS_DST_COLOR, dst ONE_MINUS_SRC_COLOR,
// alpha ONE, ZERO): the colour inverse of the destination.
@fragment
fn fs_main_invert(in: VertexOut) -> @location(0) vec4<f32> {
    return gui_color(in, false);
}

// Vanilla's GUI_TEXT_HIGHLIGHT (BlendFunction.ADDITIVE: ONE, ONE): adds the stored value.
@fragment
fn fs_main_additive(in: VertexOut) -> @location(0) vec4<f32> {
    return gui_color(in, false);
}
