//! GUI blending: vanilla's GUI blend functions (`BlendFunction`) as wgpu blend states, and the
//! byte-level blend maths the tests check them against.

use haste_core::gui::GuiBlend;

/// Whether the GUI's shader has to emit linear colour for `format`: only when the target itself is
/// sRGB-encoded (no non-sRGB view on this backend), because then the hardware encodes whatever
/// the GUI writes. On a non-sRGB view nothing converts and the GUI's colours are written raw,
/// which is what vanilla does (its main target and GUI atlas are both RGBA8_UNORM).
pub fn gui_writes_linear(format: wgpu::TextureFormat) -> bool {
    format.is_srgb()
}

/// The sRGB transfer function (WGSL `to_linear`), byte in, linear in 0..1.
pub(super) fn srgb_to_linear(byte: u8) -> f32 {
    let c = byte as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The inverse of [`srgb_to_linear`], linear in 0..1 to a byte.
pub(super) fn linear_to_srgb(x: f32) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let c = if x <= 0.0031308 {
        x * 12.92
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Vanilla's GUI blend of the sprite pixel `src` over the background pixel `dst`, in 0..255
/// bytes: the GUI_TEXTURED pipeline's BlendFunction.TRANSLUCENT (colour SRC_ALPHA,
/// ONE_MINUS_SRC_ALPHA; alpha ONE, ONE_MINUS_SRC_ALPHA) applied to the *stored* values, which is
/// what the GUI pipelines do on the non-sRGB view of the target.
pub fn blend_gui_pixel(src: [u8; 4], dst: [u8; 3]) -> [u8; 4] {
    let a = src[3] as u32;
    let ia = 255 - a;
    let rgb: [u8; 3] =
        std::array::from_fn(|i| ((a * src[i] as u32 + ia * dst[i] as u32 + 127) / 255) as u8);
    [rgb[0], rgb[1], rgb[2], 255]
}

/// Vanilla's GUI alpha blend (alpha equation ONE, ONE_MINUS_SRC_ALPHA) of the sprite alpha `src`
/// over the background alpha `dst`, both 0..255. Colour-space independent: alpha is not encoded.
pub fn blend_gui_alpha(src: u8, dst: u8) -> u8 {
    let a = src as u32;
    ((255 * a + (255 - a) * dst as u32 + 127) / 255) as u8
}

/// What the same blend produces when the GUI's target is sRGB-encoded (the GL fallback): the
/// hardware decodes the destination, blends in linear space and encodes the result. Opaque pixels
/// are unchanged (encode(decode(x)) == x), translucent ones drift towards the destination's own
/// encoding; multiplicative and inverting pipelines are worse still, so `fs_main_multiply` and
/// `fs_main_invert` do not linearize.
pub fn blend_gui_pixel_linear(src: [u8; 4], dst: [u8; 3]) -> [u8; 4] {
    let a = src[3] as f32 / 255.0;
    let ia = 1.0 - a;
    let mix = |i: usize| linear_to_srgb(a * srgb_to_linear(src[i]) + ia * srgb_to_linear(dst[i]));
    [mix(0), mix(1), mix(2), 255]
}

/// The wgpu blend state of a GUI blend mode. Normal is vanilla's `gui_textured` (straight alpha),
/// Multiply the vignette's (dst * (1 - src colour)), Invert the crosshair's (the source inverts the
/// colour behind it) and Additive the text selection highlight's (src ONE / dst ONE, both equations:
/// `BlendFunction.ADDITIVE`).
pub(super) fn blend_state(mode: GuiBlend) -> wgpu::BlendState {
    let equation =
        |src_factor: wgpu::BlendFactor, dst_factor: wgpu::BlendFactor| wgpu::BlendComponent {
            src_factor,
            dst_factor,
            operation: wgpu::BlendOperation::Add,
        };
    match mode {
        GuiBlend::Normal => wgpu::BlendState {
            color: equation(
                wgpu::BlendFactor::SrcAlpha,
                wgpu::BlendFactor::OneMinusSrcAlpha,
            ),
            alpha: equation(wgpu::BlendFactor::One, wgpu::BlendFactor::OneMinusSrcAlpha),
        },
        GuiBlend::Multiply => wgpu::BlendState {
            color: equation(wgpu::BlendFactor::Zero, wgpu::BlendFactor::OneMinusSrc),
            alpha: equation(wgpu::BlendFactor::Zero, wgpu::BlendFactor::One),
        },
        GuiBlend::Invert => wgpu::BlendState {
            color: equation(
                wgpu::BlendFactor::OneMinusDst,
                wgpu::BlendFactor::OneMinusSrc,
            ),
            // vanilla BlendFunction.INVERT: alpha ONE, ZERO
            alpha: equation(wgpu::BlendFactor::One, wgpu::BlendFactor::Zero),
        },
        GuiBlend::Additive => wgpu::BlendState {
            color: equation(wgpu::BlendFactor::One, wgpu::BlendFactor::One),
            alpha: equation(wgpu::BlendFactor::One, wgpu::BlendFactor::One),
        },
    }
}
