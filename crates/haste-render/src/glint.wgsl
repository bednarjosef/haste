// The enchantment glint's coordinates (see glint.rs), appended to the entity and GUI shaders,
// which declare the glint textures and the `GlintParams` uniform in their own bind groups.
//
// Vanilla's item/entity vertex shaders compute `texCoordGlint = (TextureMat * vec4(UV0, 0, 1)).xy`
// with `TextureMat = translation(-offset0, offset1, 0) * rotateZ(PI / 18) * scale(s)`
// (`TextureTransform.setupGlintTexturing`); UV0 is the vertex's coordinate in vanilla's own
// texture space: the atlas for item sprites, the texture itself for entity models and armour.

struct GlintParams {
    // The texture matrix's translation (-offset0, offset1).
    translate: vec2<f32>,
    // `GlintAlpha` (the glintStrength option).
    alpha: f32,
    pad: f32,
};

// `GlintMode` in glint.rs.
const GLINT_NONE: u32 = 0u;
const GLINT_ITEMS_SHEET: u32 = 1u;
const GLINT_BLOCKS_SHEET: u32 = 2u;
const GLINT_ENTITY: u32 = 3u;
const GLINT_ARMOR: u32 = 4u;

// `uv` is the vertex's atlas texel coordinate inside `rect` (x, y, w, h in texels).
fn glint_coords(mode: u32, uv: vec2<f32>, rect: vec4<u32>, translate: vec2<f32>) -> vec2<f32> {
    let local = uv - vec2<f32>(rect.xy);
    var base: vec2<f32>;
    var scale: f32;
    switch mode {
        case 1u: {
            // An `item/` sprite in vanilla's 1024x512 items atlas, GLINT_TEXTURING.
            base = local / vec2<f32>(1024.0, 512.0);
            scale = 8.0;
        }
        case 2u: {
            // A block texture in vanilla's 2048x2048 blocks atlas, GLINT_TEXTURING.
            base = local / vec2<f32>(2048.0, 2048.0);
            scale = 8.0;
        }
        case 3u: {
            // An entity texture, ENTITY_GLINT_TEXTURING.
            base = local / vec2<f32>(rect.zw);
            scale = 0.5;
        }
        default: {
            // An armour texture, ARMOR_ENTITY_GLINT_TEXTURING.
            base = local / vec2<f32>(rect.zw);
            scale = 0.16;
        }
    }
    let s = base * scale;
    // rotateZ(PI / 18)
    let c = cos(0.17453292519943295);
    let n = sin(0.17453292519943295);
    return vec2<f32>(c * s.x - n * s.y, n * s.x + c * s.y) + translate;
}
