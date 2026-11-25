// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See docs/THIRD_PARTY.md.
//! sRGB <-> linear channel tables (vanilla `ARGB` over its 10-bit table).
/// sRGB channel value (0..=255) as a linear float in [0, 1] (vanilla
/// `ARGB.srgbToLinearChannel` over the 10-bit table).
pub(super) fn srgb_to_linear_f32(channel: u8) -> f32 {
    f32::from(srgb_to_linear_10(channel)) / 1023.0
}

/// Linear float back to an sRGB byte (vanilla `ARGB.linearToSrgbChannel`).
pub(super) fn linear_to_srgb_f32(linear: f32) -> u8 {
    let index = (linear * 1023.0).floor().clamp(0.0, 1023.0) as u16;
    linear_10_to_srgb(index)
}

pub(super) fn srgb_to_linear_10(channel: u8) -> u16 {
    static TABLE: std::sync::OnceLock<[u16; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let x = index as f32 / 255.0;
            let linear = if x >= 0.04045 {
                (((f64::from(x) + 0.055) / 1.055).powf(2.4)) as f32
            } else {
                x / 12.92
            };
            (linear * 1023.0).round() as u16
        })
    })[channel as usize]
}

pub(super) fn linear_10_to_srgb(channel: u16) -> u8 {
    static TABLE: std::sync::OnceLock<[u8; 1024]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let x = index as f32 / 1023.0;
            let srgb = if x >= 0.0031308 {
                (1.055 * f64::from(x).powf(1.0 / 2.4) - 0.055) as f32
            } else {
                12.92 * x
            };
            (srgb * 255.0).round().clamp(0.0, 255.0) as u8
        })
    })[usize::from(channel)]
}
