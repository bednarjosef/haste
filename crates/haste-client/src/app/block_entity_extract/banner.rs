use std::sync::Arc;

/// A banner's patterns from its NBT (`BannerBlockEntity`'s `patterns`: a list of
/// `{pattern: <registry id>, color: <dye name>}`) plus the base colour from the block id
/// (`<colour>_banner`, vanilla `BannerBlock.getColor`). `None` when the NBT does not parse;
/// a banner without a `patterns` tag is the plain base colour.
pub(super) fn parse_banner_nbt(
    nbt: &[u8],
    block: &haste_core::Identifier,
) -> Option<Arc<haste_render::blockentity::BannerData>> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    use haste_protocol::nbt::NbtTag;
    const DYES: [&str; 16] = [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ];
    let dye = |name: &str| {
        DYES.iter()
            .position(|d| *d == name)
            .map(|i| i as u8)
            .unwrap_or(15)
    };
    // The base colour: `<colour>_banner` / `<colour>_wall_banner` -> the dye id.
    let base = block
        .path()
        .strip_suffix("_wall_banner")
        .or_else(|| block.path().strip_suffix("_banner"))
        .and_then(|c| DYES.iter().position(|d| *d == c))
        .map_or(15, |i| i as u8);
    let mut data = haste_render::blockentity::BannerData {
        base,
        layers: Vec::new(),
    };
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let doc = haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()?;
    if let Some(patterns) = doc.root_tag.get_list("patterns") {
        for layer in patterns {
            let NbtTag::Compound(c) = layer else {
                continue;
            };
            // The pattern is the registry id (a string) or an inline `{asset_id: ...}` (a
            // compound under `pattern`, or at the layer's top level for leniency).
            let pattern = match c.get("pattern") {
                Some(NbtTag::String(s)) => haste_core::Identifier::parse(s).ok()?,
                Some(NbtTag::Compound(p)) => {
                    haste_core::Identifier::parse(p.get_string("asset_id")?).ok()?
                }
                _ => c
                    .get_string("asset_id")
                    .and_then(|s| haste_core::Identifier::parse(s).ok())?,
            };
            let color = c.get_string("color").map(dye).unwrap_or(15);
            data.layers
                .push(haste_render::blockentity::BannerLayer { pattern, color });
        }
    }
    Some(Arc::new(data))
}
