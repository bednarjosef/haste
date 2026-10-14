// The vanilla 16x16 light texture: one texel per (block, sky) light-level pair, computed on
// the CPU (vanilla LightTexture rasterizes the same values into a texture through its
// lightmap.fsh; the shader math is ported here exactly).
//
// Vanilla sources: net.minecraft.client.renderer.Lightmap (updateLightTexture),
// LightmapRenderStateExtractor (blockFactor, flicker tick, brightness, darkness),
// net.minecraft.client.renderer.state.LightmapRenderState (defaults), and the jar shader
// shaders/core/lightmap.fsh (the texel formula below, field for field).

/// The light texture's side length in texels.
pub const LIGHTMAP_SIZE: usize = 16;
/// The RGBA8 bytes of the light texture (16x16x4, row-major).
pub const LIGHTMAP_BYTES: usize = LIGHTMAP_SIZE * LIGHTMAP_SIZE * 4;

/// One lightmap texel, linear RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub [f32; 3]);

/// The inputs `LightmapRenderStateExtractor.extract` gathers from the world and player.
/// `sky_darken` is 1 - SKY_LIGHT_FACTOR (0 day .. 1 night, the "sky darken factor" the brief
/// names); `block_light_flicker` is `LightmapRenderStateExtractor.blockLightFlicker` (0 for a
/// steady lightmap); `gamma` is the brightness option (0..1); `night_vision` its effect
/// intensity, 0 without it. `ambient_light` is `DimensionType.ambientLight` (0 overworld, 0.1
/// Nether, 0.25 End); `has_sky_light` is false in the Nether and the End, whose fixed sky light
/// and ambient colours replace the sky term.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightmapInputs {
    pub sky_darken: f32,
    pub block_light_flicker: f32,
    pub gamma: f32,
    pub night_vision: f32,
    pub ambient_light: f32,
    pub has_sky_light: bool,
}

impl LightmapInputs {
    /// Overworld noon, no flicker, default brightness: the all-white reference frame.
    pub fn overworld_noon() -> Self {
        Self {
            sky_darken: 0.0,
            block_light_flicker: 0.0,
            gamma: 0.0,
            night_vision: 0.0,
            ambient_light: 0.0,
            has_sky_light: true,
        }
    }
}

/// `get_brightness` in lightmap.fsh: the vanilla brightness response of a light level
/// already normalized to 0..1 (the shader feeds it `floor(texCoord * 16) / 15`).
fn brightness_curve(level: f32) -> f32 {
    level / (4.0 - 3.0 * level)
}

/// `notGamma` in lightmap.fsh: vanilla's brightness-option curve.
fn not_gamma(color: [f32; 3]) -> [f32; 3] {
    let max_component = color[0].max(color[1]).max(color[2]);
    let max_inverted = 1.0 - max_component;
    let max_scaled = 1.0 - max_inverted * max_inverted * max_inverted * max_inverted;
    if max_component <= 0.0 {
        return [0.0; 3];
    }
    let k = max_scaled / max_component;
    [color[0] * k, color[1] * k, color[2] * k]
}

/// `parabolicMixFactor` in lightmap.fsh.
fn parabolic_mix_factor(level: f32) -> f32 {
    let t = 2.0 * level - 1.0;
    t * t
}

/// The overworld sky-light colour by darkness (Timelines.OVERWORLD_DAY SKY_LIGHT_COLOR track:
/// white at day, ARGB.color(255, 122, 122) = (0.48, 0.48, 1.0) at night).
fn sky_light_color(darken: f32) -> [f32; 3] {
    if darken >= 0.5 {
        NIGHT_SKY_LIGHT_COLOR
    } else {
        [1.0; 3]
    }
}

/// Timelines.NIGHT_SKY_LIGHT_COLOR (ARGB.colorFromFloat(0.48, 0.48, 1.0)).
const NIGHT_SKY_LIGHT_COLOR: [f32; 3] = [122.0 / 255.0, 122.0 / 255.0, 1.0];

/// The Nether's fixed lightmap inputs (DimensionTypes.NETHER).
pub fn nether() -> LightmapInputs {
    LightmapInputs {
        sky_darken: 1.0,
        block_light_flicker: 0.0,
        gamma: 0.0,
        night_vision: 0.0,
        ambient_light: 0.1,
        has_sky_light: false,
    }
}

/// The End's fixed lightmap inputs (DimensionTypes.END).
pub fn end() -> LightmapInputs {
    LightmapInputs {
        sky_darken: 1.0,
        block_light_flicker: 0.0,
        gamma: 0.0,
        night_vision: 0.0,
        ambient_light: 0.25,
        has_sky_light: false,
    }
}

/// The sky-light factor for `sky_darken` with the ambient-light lerp vanilla's
/// `LightTexture.getBrightness` uses in its own callers: the factor itself is simply
/// 1 - sky_darken here (the environment attribute), with the dimension's fixed sky light
/// replacing it when the dimension has none.
fn sky_factor(inputs: &LightmapInputs) -> f32 {
    if inputs.has_sky_light {
        1.0 - inputs.sky_darken
    } else {
        0.0
    }
}

/// The fixed light environments of skyless dimensions (DimensionTypes attributes).
fn fixed_sky_color(inputs: &LightmapInputs) -> [f32; 3] {
    // Nether SKY_LIGHT_COLOR = Timelines.NIGHT_SKY_LIGHT_COLOR; End = rgb(172, 96, 205).
    // Both are used with a zero sky factor, so the value only matters for tests'
    // bookkeeping; kept for the lightmap's ambient term documentation.
    if inputs.ambient_light >= 0.25 {
        [172.0 / 255.0, 96.0 / 255.0, 205.0 / 255.0]
    } else {
        NIGHT_SKY_LIGHT_COLOR
    }
}

/// The linear light texture for the inputs (RGBA8, row-major). `out` must be
/// [`LIGHTMAP_BYTES`] long.
pub fn compute_lightmap(inputs: &LightmapInputs, out: &mut [u8]) {
    assert_eq!(out.len(), LIGHTMAP_BYTES, "lightmap output buffer size");
    let sky_factor = sky_factor(inputs);
    let block_factor = inputs.block_light_flicker + 1.4;
    let sky_light_color = if inputs.has_sky_light {
        sky_light_color(inputs.sky_darken)
    } else {
        fixed_sky_color(inputs)
    };
    for y in 0..LIGHTMAP_SIZE {
        let sky_level = y as f32 / 15.0;
        for x in 0..LIGHTMAP_SIZE {
            let block_level = x as f32 / 15.0;
            let texel = texel(
                inputs,
                block_level,
                sky_level,
                block_factor,
                sky_factor,
                sky_light_color,
            );
            let i = (y * LIGHTMAP_SIZE + x) * 4;
            for c in 0..3 {
                out[i + c] = (texel[c].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
            out[i + 3] = 255;
        }
    }
}

/// One lightmap texel: the body of lightmap.fsh `main`, in linear-light floats.
fn texel(
    inputs: &LightmapInputs,
    block_level: f32,
    sky_level: f32,
    block_factor: f32,
    sky_factor: f32,
    sky_light_color: [f32; 3],
) -> [f32; 3] {
    let block_brightness = brightness_curve(block_level) * block_factor;
    let sky_brightness = brightness_curve(sky_level) * sky_factor;

    // The ambient colour: the dimension's AMBIENT_LIGHT_COLOR. The overworld's is near-black
    // (rgb 10, 10, 25 => 0.04, 0.04, 0.1); the Nether's rgb 48, 25, 15 and the End's rgb 102,
    // 68, 12 take over where the sky contributes nothing.
    let ambient_color = ambient_color(inputs);
    let night_vision_color = [
        NIGHT_VISION_COLOR[0] * inputs.night_vision,
        NIGHT_VISION_COLOR[1] * inputs.night_vision,
        NIGHT_VISION_COLOR[2] * inputs.night_vision,
    ];
    let mut color = [
        ambient_color[0].max(night_vision_color[0]),
        ambient_color[1].max(night_vision_color[1]),
        ambient_color[2].max(night_vision_color[2]),
    ];

    // Sky light.
    for c in 0..3 {
        color[c] += sky_light_color[c] * sky_brightness;
    }

    // Block light, tinted warm at low levels (BLOCK_LIGHT_TINT default rgb 0.75, 0.66, 0.51).
    let parabolic = 0.9 * parabolic_mix_factor(block_level);
    let block_light_tint = BLOCK_LIGHT_TINT;
    let block_light_color = [
        block_light_tint[0] + (1.0 - block_light_tint[0]) * parabolic,
        block_light_tint[1] + (1.0 - block_light_tint[1]) * parabolic,
        block_light_tint[2] + (1.0 - block_light_tint[2]) * parabolic,
    ];
    for c in 0..3 {
        color[c] += block_light_color[c] * block_brightness;
    }

    // (Skyless dimensions carry their fixed brightness in the ambient colour alone;
    // `DimensionType.ambientLight` itself feeds vanilla's `LightTexture.getBrightness`, not
    // this shader.)

    // Brightness option (the gamma mix) on the clamped colour.
    let brightness = inputs.gamma.max(0.0);
    let clamped = [
        color[0].clamp(0.0, 1.0),
        color[1].clamp(0.0, 1.0),
        color[2].clamp(0.0, 1.0),
    ];
    let lifted = not_gamma(clamped);
    [
        clamped[0] + (lifted[0] - clamped[0]) * brightness,
        clamped[1] + (lifted[1] - clamped[1]) * brightness,
        clamped[2] + (lifted[2] - clamped[2]) * brightness,
    ]
}

/// The ambient colour for the dimension class (DimensionTypes.AMBIENT_LIGHT_COLOR).
fn ambient_color(inputs: &LightmapInputs) -> [f32; 3] {
    if inputs.has_sky_light {
        // Overworld: ARGB.vector3fFromRGB24(-16119286) = rgb(10, 10, 10).
        [10.0 / 255.0; 3]
    } else if inputs.ambient_light >= 0.25 {
        // End: ARGB.vector3fFromRGB24(-12630209) = rgb(63, 71, 63).
        [63.0 / 255.0, 71.0 / 255.0, 63.0 / 255.0]
    } else {
        // Nether: ARGB.vector3fFromRGB24(-13621215) = rgb(48, 40, 33).
        [48.0 / 255.0, 40.0 / 255.0, 33.0 / 255.0]
    }
}

/// NIGHT_VISION_COLOR default (ARGB.vector3fFromRGB24(-6710887) = rgb 153, 153, 153).
const NIGHT_VISION_COLOR: [f32; 3] = [153.0 / 255.0; 3];
/// BLOCK_LIGHT_TINT default (ARGB.vector3fFromRGB24(-10100) = rgb 255, 216, 140).
const BLOCK_LIGHT_TINT: [f32; 3] = [1.0, 216.0 / 255.0, 140.0 / 255.0];

#[cfg(test)]
mod tests {
    use super::*;

    /// texel (block, sky) of a freshly computed map, as linear f32 triple.
    fn tex(inputs: &LightmapInputs, block: usize, sky: usize) -> [f32; 3] {
        let mut data = [0u8; LIGHTMAP_BYTES];
        compute_lightmap(inputs, &mut data);
        let i = (sky * LIGHTMAP_SIZE + block) * 4;
        [
            data[i] as f32 / 255.0,
            data[i + 1] as f32 / 255.0,
            data[i + 2] as f32 / 255.0,
        ]
    }

    #[test]
    fn noon_full_sky_light_is_white() {
        // Noon: sky factor 1, sky level 15 => brightness 1, colour white + overworld ambient.
        // block 0, sky 15: ambient(10,10,10)/255 + white*1 clamps to 1.
        let t = tex(&LightmapInputs::overworld_noon(), 0, 15);
        assert!((t[0] - 1.0).abs() < 1e-3, "{t:?}");
        assert!((t[1] - 1.0).abs() < 1e-3);
        assert!((t[2] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn noon_block0_sky0_is_ambient_dark() {
        // No light at all at noon: only the overworld ambient colour, no sky or block term
        // (sky factor 1 but sky brightness of level 0 is 0). rgb(10,10,10)/255 = 0.0392.
        let t = tex(&LightmapInputs::overworld_noon(), 0, 0);
        for c in 0..3 {
            assert!((t[c] - 10.0 / 255.0).abs() < 1.0 / 255.0, "{t:?}");
        }
    }

    #[test]
    fn midnight_full_sky_is_dim() {
        // Midnight: sky darken 1, sky factor 0, night sky colour applies to nothing.
        // Sky 15 still contributes 0; the texel is the ambient colour.
        let night = LightmapInputs {
            sky_darken: 1.0,
            ..LightmapInputs::overworld_noon()
        };
        let t = tex(&night, 0, 15);
        for c in 0..3 {
            assert!((t[c] - 10.0 / 255.0).abs() < 1.0 / 255.0, "{t:?}");
        }
    }

    #[test]
    fn torch_block14_at_night_is_warm_and_bright() {
        // Block 14, sky 0 at night: block brightness = curve(14/15) * 1.4, block light colour
        // mixed towards white by 0.9 * (2*14/15 - 1)^2.
        let night = LightmapInputs {
            sky_darken: 1.0,
            ..LightmapInputs::overworld_noon()
        };
        let t = tex(&night, 14, 0);
        let level: f32 = 14.0 / 15.0;
        // curve(14) = (14/15) / (4 - 3*14/15) = 0.93333/1.2 = 0.77778 (hand).
        let curve = level / (4.0 - 3.0 * level);
        assert!((curve - 0.77778).abs() < 1e-4, "curve {curve}");
        // Hand-computed R channel: block tint red is 1.0, so texel_r = 10/255 (ambient) +
        // 1.0 * curve * 1.4 = 0.0392 + 1.0889 > 1, which clamps to 1.0.
        let expected_r = 10.0 / 255.0 + curve * 1.4;
        assert!(expected_r > 1.0, "expected {expected_r}");
        assert!((t[0] - 1.0).abs() < 1.0 / 255.0, "r {}", t[0]);
        // Warmth: the tint's blue (140/255) keeps blue below red at lower block light.
        let dim_torch = tex(&night, 4, 0);
        assert!(
            dim_torch[0] > dim_torch[2] + 0.02,
            "not warm: {dim_torch:?}"
        );
    }

    #[test]
    fn nether_is_dim_and_reddish() {
        // The Nether: no sky term, ambient rgb(48,40,33)/255, block light still applies.
        let t = tex(&nether(), 0, 15);
        // Sky contributes nothing (factor 0): texel is the ambient colour.
        // rgb(48,40,33)/255 = (0.188, 0.157, 0.129).
        assert!((t[0] - 48.0 / 255.0).abs() < 1.0 / 255.0, "{t:?}");
        assert!((t[1] - 40.0 / 255.0).abs() < 1.0 / 255.0);
        assert!((t[2] - 33.0 / 255.0).abs() < 1.0 / 255.0);
        // Warmer at high block light.
        let lit = tex(&nether(), 14, 0);
        assert!(lit[0] > t[0] + 0.1);
    }

    #[test]
    fn end_is_dim_green_grey() {
        let t = tex(&end(), 0, 15);
        // rgb(63,71,63)/255 = (0.247, 0.278, 0.247).
        assert!((t[0] - 63.0 / 255.0).abs() < 1.0 / 255.0, "{t:?}");
        assert!((t[1] - 71.0 / 255.0).abs() < 1.0 / 255.0);
        assert!((t[2] - 63.0 / 255.0).abs() < 1.0 / 255.0);
    }

    #[test]
    fn gamma_lifts_shadows() {
        // gamma 1 applies notGamma fully: a dark texel moves up, a white one stays.
        let mut bright = LightmapInputs::overworld_noon();
        bright.gamma = 1.0;
        let dark_inputs = LightmapInputs {
            sky_darken: 1.0,
            ..bright
        };
        let plain = tex(
            &LightmapInputs {
                sky_darken: 1.0,
                ..LightmapInputs::overworld_noon()
            },
            0,
            0,
        );
        let lifted = tex(&dark_inputs, 0, 0);
        assert!(lifted[2] > plain[2], "{lifted:?} vs {plain:?}");
        let full = tex(&bright, 0, 15);
        assert!((full[0] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn night_vision_flattens_the_map() {
        // With the effect at full strength the darkest texel rises to the night-vision colour
        // rgb(153,153,153)/255 = 0.6.
        let nv = LightmapInputs {
            night_vision: 1.0,
            ..LightmapInputs::overworld_noon()
        };
        let t = tex(&nv, 0, 0);
        for c in 0..3 {
            assert!((t[c] - 153.0 / 255.0).abs() < 1.0 / 255.0, "{t:?}");
        }
    }

    #[test]
    fn block_light_flicker_darkens_block_light_only() {
        // A negative flicker value (the extractor's random walk can go either way) scales the
        // block term: texel(block 14) shrinks, texel(sky 15) does not.
        let base = LightmapInputs::overworld_noon();
        let flickered = LightmapInputs {
            block_light_flicker: -0.1,
            ..base
        };
        let torch_base = tex(&base, 14, 0);
        let torch_flick = tex(&flickered, 14, 0);
        // Red clamps at 1.0; the blue channel (tint 140/255) shows the drop.
        assert!(
            torch_flick[2] < torch_base[2],
            "{torch_flick:?} vs {torch_base:?}"
        );
        let sky_base = tex(&base, 0, 15);
        let sky_flick = tex(&flickered, 0, 15);
        assert!((sky_flick[2] - sky_base[2]).abs() < 1e-4);
    }

    #[test]
    fn lightmap_layout_is_block_x_sky_y() {
        // Vanilla samples with UV2.x the block axis and UV2.y the sky axis; the computed map
        // must be brighter along +x at fixed y than along +y at fixed x at night (block light
        // unaffected by darkness).
        let night = LightmapInputs {
            sky_darken: 1.0,
            ..LightmapInputs::overworld_noon()
        };
        let right = tex(&night, 14, 0);
        let up = tex(&night, 0, 14);
        assert!(right[0] > up[0] + 0.1, "{right:?} vs {up:?}");
    }
}
