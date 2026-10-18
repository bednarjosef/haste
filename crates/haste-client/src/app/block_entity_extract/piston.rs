/// A moving piston's data from its NBT (vanilla `PistonMovingBlockEntity.loadAdditional`:
/// `blockState`, `facing`, `progress`, `extending`, `source`). Unreadable or missing fields
/// draw nothing (vanilla's `movedState` defaults to air, which also draws nothing).
pub(super) fn parse_piston_nbt(nbt: &[u8]) -> haste_render::PistonData {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    let Ok(doc) = haste_protocol::nbt::Nbt::read_unnamed(&mut helper) else {
        return air_piston();
    };
    let root = &doc.root_tag;
    // The state string carries its properties: `block[facing=north,...]`; keep it whole (the
    // renderer parses it into the blockstate's variants).
    let block_state = root
        .get_string("blockState")
        .map(|s| s.to_string())
        .unwrap_or_default();
    let facing = root
        .get_int("facing")
        .and_then(|i| u8::try_from(i).ok())
        .and_then(haste_core::Direction::from_ordinal)
        .unwrap_or(haste_core::Direction::Down);
    let progress = root.get_float("progress").unwrap_or(0.0);
    let extending = root.get_bool("extending").unwrap_or(false);
    let source = root.get_bool("source").unwrap_or(false);
    haste_render::PistonData {
        block_state,
        progress: haste_render::TickPair::fixed(progress),
        facing,
        extending,
        source,
    }
}

/// The piston data for a block entity without a moved block (draws nothing).
pub(super) fn air_piston() -> haste_render::PistonData {
    haste_render::PistonData {
        block_state: String::new(),
        progress: haste_render::TickPair::fixed(0.0),
        facing: haste_core::Direction::Down,
        extending: false,
        source: false,
    }
}
