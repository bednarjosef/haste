use super::*;
/// `DyeColor.getTextureDiffuseColor` for a few dyes, as linear 0..1 triples
/// (white, orange, black, red — the exact vanilla constants, channel / 255).
#[test]
fn dye_tints_match_vanilla() {
    assert_eq!(dye_tint(0), [249.0 / 255.0, 1.0, 254.0 / 255.0]); // white 0xF9FFFE
    assert_eq!(dye_tint(1), [249.0 / 255.0, 128.0 / 255.0, 29.0 / 255.0]); // orange 0xF9801D
    assert_eq!(dye_tint(15), [29.0 / 255.0, 29.0 / 255.0, 33.0 / 255.0]); // black 0x1D1D21
    assert_eq!(dye_tint(14), [176.0 / 255.0, 46.0 / 255.0, 38.0 / 255.0]); // red 0xB02E26
                                                                           // Out-of-range ids wrap to a real dye like `DyeColor.byId`.
    assert_eq!(dye_tint(16), dye_tint(0));
    assert_eq!(dye_tint(-1), dye_tint(15));
}
