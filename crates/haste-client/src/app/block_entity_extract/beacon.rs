/// A stained-glass block's (or the beacon's own) beam colour (vanilla `BeaconBeamBlock.getColor`
/// -> `DyeColor.getTextureDiffuseColor`), or `None` for other blocks.
pub(super) fn beacon_beam_color(block: &haste_core::Identifier) -> Option<u32> {
    const GLASS: [(&str, u32); 16] = [
        ("white_stained_glass", 0),
        ("orange_stained_glass", 1),
        ("magenta_stained_glass", 2),
        ("light_blue_stained_glass", 3),
        ("yellow_stained_glass", 4),
        ("lime_stained_glass", 5),
        ("pink_stained_glass", 6),
        ("gray_stained_glass", 7),
        ("light_gray_stained_glass", 8),
        ("cyan_stained_glass", 9),
        ("purple_stained_glass", 10),
        ("blue_stained_glass", 11),
        ("brown_stained_glass", 12),
        ("green_stained_glass", 13),
        ("red_stained_glass", 14),
        ("black_stained_glass", 15),
    ];
    if block.namespace() != "minecraft" {
        return None;
    }
    // The beacon block itself is a white beam block (vanilla `BeaconBlock.getColor`).
    let color = if block.path() == "beacon" {
        0
    } else {
        GLASS
            .iter()
            .find(|(name, _)| *name == block.path())
            .map(|(_, dye)| *dye)?
    };
    Some(haste_render::blockentity::DYE_DIFFUSE_COLOR[color as usize])
}

/// Vanilla `ARGB.average`: the per-channel mean of two colours.
pub(super) fn average_argb(a: u32, b: u32) -> u32 {
    let ch = |s: u32| (((a >> s) & 0xFF).wrapping_add((b >> s) & 0xFF)) / 2;
    0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0)
}
