use haste_protocol::haste_core::BlockPos;

use super::BlockEntityExtractor;

impl BlockEntityExtractor {
    /// The item or mob payload of a block entity (campfire items, shelf items, a vault's
    /// displayed item, a brushable block's item, a spawner's display mob), parsed from the
    /// block entity's NBT only when the payload changes and cached per position (the same
    /// (pointer, length) scheme as the sign text). Kinds with no payload are handed back
    /// unchanged.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn item_block_kind(
        &mut self,
        pos: BlockPos,
        be: &haste_core::chunk::BlockEntityData,
        kind: haste_render::BlockEntityKind,
        type_name: &haste_core::Identifier,
        blocks: &haste_data::BlockStates,
        state: haste_core::BlockStateId,
        registries: &haste_data::Registries,
        spin: (f32, f32),
    ) -> haste_render::BlockEntityKind {
        use haste_render::blockentity::{BrushableItem, CampfireItems, ShelfItems, VaultDisplay};
        let payload = matches!(
            kind,
            haste_render::BlockEntityKind::Campfire(_)
                | haste_render::BlockEntityKind::Spawner(_)
                | haste_render::BlockEntityKind::Vault(_)
                | haste_render::BlockEntityKind::Shelf(_)
                | haste_render::BlockEntityKind::Brushable(_)
        );
        if !payload {
            return kind;
        }
        let key = (be.nbt.as_ptr() as usize, be.nbt.len());
        if let Some((k, len, cached)) = self.item_blocks.get(&pos) {
            if (*k, *len) == key {
                return cached.clone();
            }
        }
        let parsed = match kind {
            // The campfire's and the shelf's items, by slot (`ContainerHelper.loadAllItems`).
            haste_render::BlockEntityKind::Campfire(_) => {
                let mut items = [None; 4];
                parse_items_into(&be.nbt, registries, &mut items);
                haste_render::BlockEntityKind::Campfire(CampfireItems { items })
            }
            haste_render::BlockEntityKind::Shelf(_) => {
                let mut items = [None; 3];
                parse_items_into(&be.nbt, registries, &mut items);
                let align = read_nbt(&be.nbt)
                    .and_then(|doc| doc.root_tag.get_bool("align_items_to_bottom"))
                    .unwrap_or(false);
                haste_render::BlockEntityKind::Shelf(ShelfItems {
                    items,
                    align_to_bottom: align,
                })
            }
            // The vault's displayed item lives under `shared_data.display_item`; it draws only
            // when the shared data has one (vanilla `VaultBlockEntity.Client
            // .shouldDisplayActiveEffects` is `hasDisplayItem`).
            haste_render::BlockEntityKind::Vault(_) => {
                match parse_item_stack_path(&be.nbt, &["shared_data", "display_item"], registries) {
                    Some((item, count)) => haste_render::BlockEntityKind::Vault(VaultDisplay {
                        item,
                        count,
                        spin: haste_render::TickPair {
                            start: spin.0,
                            end: spin.1,
                        },
                    }),
                    None => kind,
                }
            }
            // The brushable block's item (its `item` tag) and the block state's `dusted`; the
            // face it is brushed from is not in the NBT until it has been brushed, so it stays
            // `None` and the item is drawn only while that face is known.
            haste_render::BlockEntityKind::Brushable(_) => {
                let dusted = blocks
                    .property_value(state, "dusted")
                    .and_then(|v| v.parse::<u8>().ok())
                    .unwrap_or(0);
                match (
                    parse_item_stack_path(&be.nbt, &["item"], registries),
                    // Vanilla stores `hit_direction` as `Direction.LEGACY_ID_CODEC`, a byte of
                    // the direction's 3D data value (`BrushableBlockEntity.getUpdateTag`); read
                    // it as any numeric, like vanilla's lenient `ValueInput`.
                    read_nbt(&be.nbt)
                        .and_then(|doc| doc.root_tag.get_numeric_short("hit_direction"))
                        .and_then(|v| haste_core::Direction::from_ordinal(v as u8)),
                ) {
                    (Some((item, _)), hit_direction) => {
                        haste_render::BlockEntityKind::Brushable(BrushableItem {
                            item,
                            hit_direction,
                            dusted,
                        })
                    }
                    (None, _) => kind,
                }
            }
            // The spawner's display mob: the model from `SpawnData`'s (a trial spawner's
            // `spawn_data`'s) entity id, and the spin the game tick keeps.
            haste_render::BlockEntityKind::Spawner(_) => {
                let trial =
                    type_name.namespace() == "minecraft" && type_name.path() == "trial_spawner";
                // A trial spawner has no display entity outside the states with a spinning mob
                // (vanilla `TrialSpawnerStateData.getOrCreateDisplayEntity`).
                let spinning = !trial
                    || matches!(
                        blocks
                            .property_value(state, "trial_spawner_state")
                            .unwrap_or(""),
                        "waiting_for_players" | "active"
                    );
                let entity = spinning
                    .then(|| spawner_entity_id(&be.nbt, trial))
                    .flatten()
                    .and_then(|id| registries.index("minecraft:entity_type", &id))
                    .and_then(haste_entity::registry::model_for);
                match entity {
                    Some(model) => haste_render::BlockEntityKind::Spawner(
                        haste_render::blockentity::SpawnerDisplay {
                            model: Some(model.clone()),
                            spin: haste_render::TickPair {
                                start: spin.0,
                                end: spin.1,
                            },
                            scale: haste_render::blockentity::spawner_scale(&model),
                        },
                    ),
                    None => kind,
                }
            }
            other => other,
        };
        self.item_blocks.insert(pos, (key.0, key.1, parsed.clone()));
        parsed
    }
}

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
