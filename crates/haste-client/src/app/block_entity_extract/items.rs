/// Reads a block entity's NBT payload (the network variant `read_unnamed` reads). `None` when
/// the payload does not parse, like vanilla's empty stack.
pub(super) fn read_nbt(nbt: &[u8]) -> Option<haste_protocol::nbt::Nbt> {
    use haste_protocol::nbt::deserializer::NbtReadHelperJava;
    let mut cursor = std::io::Cursor::new(nbt);
    let mut helper = NbtReadHelperJava::new(&mut cursor);
    haste_protocol::nbt::Nbt::read_unnamed(&mut helper).ok()
}

/// An item stack NBT's registry index and count: vanilla `ItemStack`'s `id` (a registry id in
/// the namespace's own form) and `count`. `None` for an id we cannot resolve.
pub(super) fn parse_item_stack(
    c: &haste_protocol::nbt::NbtCompound,
    registries: &haste_data::Registries,
) -> Option<(haste_core::RegistryIndex, u32)> {
    let id = haste_core::Identifier::parse(c.get_string("id")?).ok()?;
    let item = registries.index("minecraft:item", &id)?;
    let count = c.get_int("count").unwrap_or(1).max(1) as u32;
    Some((item, count))
}

/// The item stack under a chain of NBT keys (`shared_data.display_item` for a vault, `item`
/// for a brushable block), as its registry index and count.
pub(super) fn parse_item_stack_path(
    nbt: &[u8],
    path: &[&str],
    registries: &haste_data::Registries,
) -> Option<(haste_core::RegistryIndex, u32)> {
    let doc = read_nbt(nbt)?;
    let mut current = &doc.root_tag;
    for key in path {
        current = current.get_compound(key)?;
    }
    parse_item_stack(current, registries)
}

/// The items of a slot-addressed container NBT (`Items`, vanilla `ContainerHelper
/// .loadAllItems`), written into the caller's slot array; an item whose `Slot` is outside the
/// array is dropped rather than panicking.
pub(super) fn parse_items_into(
    nbt: &[u8],
    registries: &haste_data::Registries,
    items: &mut [Option<haste_core::RegistryIndex>],
) {
    let Some(doc) = read_nbt(nbt) else {
        return;
    };
    let Some(list) = doc.root_tag.get_list("Items") else {
        return;
    };
    for entry in list {
        let haste_protocol::nbt::NbtTag::Compound(c) = entry else {
            continue;
        };
        let slot = c.get_byte("Slot").unwrap_or(-1);
        if slot < 0 || slot as usize >= items.len() {
            continue;
        }
        if let Some((item, _)) = parse_item_stack(c, registries) {
            items[slot as usize] = Some(item);
        }
    }
}

/// The entity id a spawner's NBT names (`BaseSpawner`'s `SpawnData.entity.id`; a trial
/// spawner's `spawn_data.entity.id`), as a registry id.
pub(super) fn spawner_entity_id(nbt: &[u8], trial: bool) -> Option<haste_core::Identifier> {
    let doc = read_nbt(nbt)?;
    let data = if trial {
        doc.root_tag.get_compound("spawn_data")?
    } else {
        &doc.root_tag
    };
    let spawn = if trial {
        data
    } else {
        data.get_compound("SpawnData")?
    };
    let entity = spawn.get_compound("entity")?;
    haste_core::Identifier::parse(entity.get_string("id")?).ok()
}
