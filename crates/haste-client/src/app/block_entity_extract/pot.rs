/// The pottery pattern a sherd item names (vanilla's item
/// `minecraft:provides_pottery_pattern` component against `DecoratedPotPattern`'s asset id):
/// `minecraft:angler_pottery_sherd` -> `minecraft:angler`. `minecraft:brick` (the blank sherd)
/// has none. Items from mods whose names do not follow the vanilla pattern draw the blank side.
pub(super) fn sherd_pattern(item: &haste_core::Identifier) -> Option<haste_core::Identifier> {
    if item.namespace() != "minecraft" {
        return None;
    }
    let path = item.path().strip_suffix("_pottery_sherd")?;
    Some(haste_core::Identifier::minecraft(path))
}

/// A decorated pot's four sherds from its NBT (`DecoratedPotBlockEntity`'s `sherds`, a record
/// of optional item stack templates keyed `back`, `left`, `right`, `front`; the item is either
/// an `{id, count}` compound or a plain id string). Sides without a sherd are `None`.
pub(super) fn parse_pot_sherds(nbt: &[u8]) -> haste_render::blockentity::PotSherds {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    use haste_protocol::nbt::NbtTag;
    let side = |c: &haste_protocol::nbt::NbtCompound, key: &str| match c.get(key) {
        // Vanilla writes each sherd as an item stack template; older payloads (and lenient
        // readers) allow the plain item id.
        Some(NbtTag::Compound(item)) => item.get_string("id").and_then(|s| {
            haste_core::Identifier::parse(s)
                .ok()
                .as_ref()
                .and_then(sherd_pattern)
        }),
        Some(NbtTag::String(s)) => haste_core::Identifier::parse(s)
            .ok()
            .as_ref()
            .and_then(sherd_pattern),
        _ => None,
    };
    let mut out = haste_render::blockentity::PotSherds::default();
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let Ok(doc) = haste_protocol::nbt::Nbt::read_unnamed(&mut helper) else {
        return out;
    };
    if let Some(c) = doc.root_tag.get_compound("sherds") {
        out.front = side(c, "front");
        out.back = side(c, "back");
        out.left = side(c, "left");
        out.right = side(c, "right");
    }
    out
}
