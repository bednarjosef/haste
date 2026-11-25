//! Vanilla's `DyeColor` tints for dyed overlay layers.
/// `DyeColor.getTextureDiffuseColor` (the third constructor argument), linear RGB in 0..1:
/// the tint vanilla multiplies into dyed overlay layers. Order is the dye id (white 0 ..
/// black 15).
pub const DYE_COLORS: [[f32; 3]; 16] = {
    // The vanilla constants are sRGB 0xRRGGBB; the vertex colour is a linear multiplier
    // sampled against an sRGB atlas, so the raw 8-bit channels are what vanilla passes on.
    let hex = [
        0xF9FFFE, 0xF9801D, 0xC74EBD, 0x3AB3DA, 0xFED83D, 0x80C71F, 0xF38BAA, 0x474F52, 0x9D9D97,
        0x169C9C, 0x8932B8, 0x3C44AA, 0x835432, 0x5E7C16, 0xB02E26, 0x1D1D21,
    ];
    let mut out = [[0.0f32; 3]; 16];
    let mut i = 0;
    while i < 16 {
        out[i] = [
            ((hex[i] >> 16) & 0xFF) as f32 / 255.0,
            ((hex[i] >> 8) & 0xFF) as f32 / 255.0,
            (hex[i] & 0xFF) as f32 / 255.0,
        ];
        i += 1;
    }
    out
};

/// The dye colour's linear tint (see [`DYE_COLORS`]); ids wrap like `DyeColor.byId`
/// (`ByIdMap.continuous` with the zero strategy, so any id outside 0..15 is white).
pub fn dye_tint(id: i32) -> [f32; 3] {
    DYE_COLORS[id.rem_euclid(16) as usize]
}
