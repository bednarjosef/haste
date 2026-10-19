//! Hanging decorations (item frames, paintings): the state vanilla's `ItemFrameRenderer` /
//! `PaintingRenderer` extract, built from the entity type, the spawn direction and the synched
//! entity data.
//!
//! Metadata indices come from the synched data order (minecraft.wiki "Java Edition protocol /
//! Entity metadata"): base `Entity` defines 0..=7 (shared flags, air, custom name, name visible,
//! silent, no gravity, pose, frozen ticks), then `HangingEntity.DATA_DIRECTION` (8),
//! `ItemFrame.DATA_ITEM` (9) and `DATA_ROTATION` (10); `Painting.DATA_PAINTING_VARIANT_ID` is 9.
use std::collections::HashMap;

use haste_core::entity::MetadataValue;
use haste_core::model::{
    Decoration, ExperienceOrbDecoration, FallingBlockDecoration, ItemFrameDecoration,
    LightningDecoration, PaintingDecoration, PrimedTntDecoration,
};
use haste_core::{BlockStateId, Direction, Identifier, RegistryIndex};

/// `ItemFrame.DATA_ITEM`.
pub const ITEM_FRAME_ITEM: u8 = 9;
/// `ItemFrame.DATA_ROTATION`.
pub const ITEM_FRAME_ROTATION: u8 = 10;
/// `Painting.DATA_PAINTING_VARIANT_ID`.
pub const PAINTING_VARIANT: u8 = 9;

/// One `minecraft:painting_variant` entry: the synced registry's `width`/`height` and the
/// `asset_id` sprite (vanilla `PaintingVariant`).
#[derive(Clone, Debug, PartialEq)]
pub struct PaintingVariant {
    pub width: u8,
    pub height: u8,
    pub asset: Identifier,
}

impl PaintingVariant {
    /// The front texture id of the variant's sprite: vanilla's `paintings` atlas sources the
    /// `painting/` directory, so the sprite `minecraft:kebab` is `textures/painting/kebab.png`.
    pub fn front_texture(&self) -> Identifier {
        if self.asset.path().starts_with("painting/") {
            self.asset.clone()
        } else {
            Identifier::new(
                self.asset.namespace(),
                &format!("painting/{}", self.asset.path()),
            )
        }
    }
}

/// The item frame's synched data: the framed item and its rotation (`ItemFrame.getItem` /
/// `getRotation`).
pub fn item_frame_item(metadata: &crate::metadata::Metadata) -> Option<RegistryIndex> {
    match metadata.get(ITEM_FRAME_ITEM)?.value {
        MetadataValue::ItemStack(ref s) if !s.is_empty() => s.item,
        _ => None,
    }
}

pub fn item_frame_rotation(metadata: &crate::metadata::Metadata) -> u8 {
    match metadata.get(ITEM_FRAME_ROTATION).map(|e| &e.value) {
        Some(MetadataValue::VarInt(r)) => (*r).rem_euclid(8) as u8,
        Some(MetadataValue::Byte(b)) => (*b).rem_euclid(8) as u8,
        _ => 0,
    }
}

/// The painting variant index or inline variant in `Painting.DATA_PAINTING_VARIANT_ID`, from the
/// raw `Holder<PaintingVariant>` bytes: VarInt `id + 1`, or `0` followed by the inline variant
/// (`width: i32, height: i32, asset_id: Identifier, ...`).
pub fn painting_variant(metadata: &crate::metadata::Metadata) -> Option<PaintingVariantRef> {
    let entry = metadata.get(PAINTING_VARIANT)?;
    let raw = match &entry.value {
        MetadataValue::Other { raw, .. } => raw,
        _ => return None,
    };
    let (id, rest) = read_var_int(raw)?;
    if id != 0 {
        return Some(PaintingVariantRef::Registry(RegistryIndex(id as u32 - 1)));
    }
    let mut r = rest;
    let width = read_i32(&mut r)?;
    let height = read_i32(&mut r)?;
    let asset = read_identifier(&mut r)?;
    Some(PaintingVariantRef::Inline(PaintingVariant {
        width: width.clamp(1, 16) as u8,
        height: height.clamp(1, 16) as u8,
        asset,
    }))
}

/// A painting's variant as the entity data names it: a synced-registry id, or an inline variant
/// (sent only for variants not in the registry; mods).
#[derive(Clone, Debug, PartialEq)]
pub enum PaintingVariantRef {
    Registry(RegistryIndex),
    Inline(PaintingVariant),
}

/// `TntRenderer.extractRenderState`'s `fuseRemainingInTicks`: `getFuse() - partialTicks + 1`,
/// so the rendered fuse eases from fuse+1 down to fuse across each tick.
pub fn tnt_fuse_remaining(fuse: i32, partial_tick: f32) -> f32 {
    fuse as f32 - partial_tick + 1.0
}

/// The decoration of an entity, if its type has one. `variants` is the synced
/// `minecraft:painting_variant` table (empty when the server sent none).
/// `partial_tick` interpolates the TNT's fuse, `default_tnt_state` is the TNT block's
/// default state (what a TNT without a synced `DATA_BLOCK_STATE_ID` renders as).
#[allow(clippy::too_many_arguments)]
pub fn build(
    kind: RegistryIndex,
    data: i32,
    metadata: &crate::metadata::Metadata,
    variants: &HashMap<u32, PaintingVariant>,
    entity_id: i32,
    age: f32,
    partial_tick: f32,
    default_tnt_state: BlockStateId,
) -> Option<Decoration> {
    // The lightning bolt is not hanging: it renders at the entity's own position, its
    // geometry from a seed (the entity id stands in for vanilla's per-bolt random).
    if matches!(
        crate::registry::decoration_kind(kind),
        Some(crate::registry::DecorationKind::LightningBolt)
    ) {
        return Some(Decoration::Lightning(LightningDecoration {
            seed: entity_id as u32 as u64 | 0x5EED_0000_0000_0000,
        }));
    }
    // Falling blocks and primed TNT render as their block model (vanilla
    // `FallingBlockRenderer`/`TntRenderer`); they are not hanging either, so the client
    // keeps them at the entity's own (interpolated) position. The falling block's state is
    // the spawn packet's data field (`FallingBlockEntity.recreateFromPacket` reads
    // `Block.stateById(packet.getData())`); a negative id is malformed and draws nothing.
    // The types match by registry name (the decoration-kind table covers the hanging
    // kinds), so an unknown name is simply not one of these.
    match crate::registry::entity_type_name(kind) {
        Some("minecraft:falling_block") => {
            let block_state = BlockStateId(u32::try_from(data).unwrap_or(BlockStateId::AIR.0));
            return Some(Decoration::FallingBlock(FallingBlockDecoration {
                block_state,
                landed: false,
            }));
        }
        Some("minecraft:tnt") => {
            // `TntRenderer.extractRenderState`: `getFuse() - partialTicks + 1`; the block
            // state is `DATA_BLOCK_STATE_ID`, defaulting to the TNT block.
            return Some(Decoration::PrimedTnt(PrimedTntDecoration {
                block_state: metadata.tnt_block_state.unwrap_or(default_tnt_state),
                fuse: tnt_fuse_remaining(metadata.tnt_fuse, partial_tick),
            }));
        }
        _ => {}
    }
    // Experience orbs render as a camera-facing quad with a value-tiered icon and a colour
    // cycle; they are not hanging, so they keep the entity's own position.
    if matches!(
        crate::registry::decoration_kind(kind),
        Some(crate::registry::DecorationKind::ExperienceOrb)
    ) {
        let value = match metadata.get(8).map(|e| &e.value) {
            Some(haste_core::entity::MetadataValue::VarInt(v)) => *v,
            _ => 0,
        };
        return Some(Decoration::ExperienceOrb(ExperienceOrbDecoration {
            value,
            age,
        }));
    }
    let facing = Direction::from_ordinal(u8::try_from(data).ok()?)?;
    match crate::registry::decoration_kind(kind)? {
        crate::registry::DecorationKind::ItemFrame { glow } => {
            Some(Decoration::ItemFrame(ItemFrameDecoration {
                facing,
                rotation: item_frame_rotation(metadata),
                glow,
                item: item_frame_item(metadata),
                // Filled in by the client from the framed stack's component patch (the
                // component tables live there, not in this crate).
                map_id: None,
            }))
        }
        crate::registry::DecorationKind::Painting => {
            let variant = painting_variant(metadata);
            let (width, height, front) = match variant {
                Some(PaintingVariantRef::Registry(idx)) => match variants.get(&idx.0) {
                    Some(v) => (v.width, v.height, Some(v.front_texture())),
                    // An unknown registry id (should not happen on a synced server): draw the
                    // back of a single block, never panic.
                    None => (1, 1, None),
                },
                Some(PaintingVariantRef::Inline(v)) => (v.width, v.height, Some(v.front_texture())),
                None => (1, 1, None),
            };
            Some(Decoration::Painting(PaintingDecoration {
                facing,
                width,
                height,
                front,
            }))
        }
        // Handled before the facing is parsed (the bolt is not hanging).
        crate::registry::DecorationKind::LightningBolt => None,
        // Handled before the facing is parsed (not hanging).
        crate::registry::DecorationKind::ExperienceOrb => None,
    }
}

/// The render origin vanilla's `recalculateBoundingBox` gives a hanging entity, from the block
/// position the spawn packet sent. Item frames: `atCenterOf(pos) - dir * 0.46875`. Paintings:
/// shifted by `left * horizontalOffset + up * verticalOffset` (vanilla's even-size 0.5 offset).
/// `size` is `Some((width, height))` for a painting, `None` for an item frame.
pub fn hanging_render_center(
    facing: Direction,
    block_pos: haste_core::DVec3,
    size: Option<(u8, u8)>,
) -> haste_core::DVec3 {
    let center = block_pos + haste_core::DVec3::splat(0.5);
    let wall = center - facing.offset().as_dvec3() * 0.46875;
    match size {
        None => wall,
        Some((w, h)) => {
            let horizontal = if w % 2 == 0 { 0.5 } else { 0.0 };
            let vertical = if h % 2 == 0 { 0.5 } else { 0.0 };
            wall + facing.counter_clock_wise().offset().as_dvec3() * horizontal
                + haste_core::DVec3::Y * vertical
        }
    }
}

/// The render origin for the entity's decoration, resolving a painting's size from the variant
/// table; falls back to the wall center when the type is not a known decoration.
pub fn render_center(
    kind: RegistryIndex,
    data: i32,
    block_pos: haste_core::DVec3,
    metadata: &crate::metadata::Metadata,
    variants: &HashMap<u32, PaintingVariant>,
) -> haste_core::DVec3 {
    // The lightning bolt and the experience orb hang on nothing: their render origin is their
    // own position (the spawn packet's / entity position). Arrows are the same (their
    // decoration is built by `crate::arrow`, not from a facing).
    if matches!(
        crate::registry::decoration_kind(kind),
        Some(
            crate::registry::DecorationKind::LightningBolt
                | crate::registry::DecorationKind::ExperienceOrb
        )
    ) || crate::arrow::is_arrow(kind)
    {
        return block_pos;
    }
    let Some(facing) = u8::try_from(data).ok().and_then(Direction::from_ordinal) else {
        return block_pos + haste_core::DVec3::splat(0.5);
    };
    let size = match crate::registry::decoration_kind(kind) {
        Some(crate::registry::DecorationKind::Painting) => Some(match painting_variant(metadata) {
            Some(PaintingVariantRef::Registry(idx)) => variants
                .get(&idx.0)
                .map(|v| (v.width, v.height))
                .unwrap_or((1, 1)),
            Some(PaintingVariantRef::Inline(v)) => (v.width, v.height),
            None => (1, 1),
        }),
        _ => None,
    };
    hanging_render_center(facing, block_pos, size)
}

fn read_var_int(bytes: &[u8]) -> Option<(i32, &[u8])> {
    let mut value: u32 = 0;
    let mut shift = 0;
    for (i, b) in bytes.iter().enumerate() {
        value |= ((b & 0x7F) as u32) << shift;
        if b & 0x80 == 0 {
            return Some((value as i32, &bytes[i + 1..]));
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
    None
}

fn read_i32(bytes: &mut &[u8]) -> Option<i32> {
    if bytes.len() < 4 {
        return None;
    }
    let v = i32::from_be_bytes(bytes[..4].try_into().ok()?);
    *bytes = &bytes[4..];
    Some(v)
}

fn read_identifier(bytes: &mut &[u8]) -> Option<Identifier> {
    let (len, rest) = read_var_int(bytes)?;
    let len = usize::try_from(len).ok()?;
    if rest.len() < len {
        return None;
    }
    let s = std::str::from_utf8(&rest[..len]).ok()?;
    *bytes = &rest[len..];
    Identifier::parse(s).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::Metadata;
    use haste_core::entity::{MetadataEntry, MetadataValue};

    fn metadata(entries: Vec<MetadataEntry>) -> Metadata {
        let mut m = Metadata::default();
        m.apply(&entries);
        m
    }

    #[test]
    fn item_frame_reads_item_and_rotation() {
        let m = metadata(vec![
            MetadataEntry {
                index: ITEM_FRAME_ITEM,
                value: MetadataValue::ItemStack(haste_core::entity::ItemStackData {
                    count: 1,
                    item: Some(RegistryIndex(42)),
                    components_raw: Default::default(),
                }),
            },
            MetadataEntry {
                index: ITEM_FRAME_ROTATION,
                value: MetadataValue::VarInt(1),
            },
        ]);
        assert_eq!(item_frame_item(&m), Some(RegistryIndex(42)));
        assert_eq!(item_frame_rotation(&m), 1);
        // Out of range wraps like vanilla's `rotation % 8`.
        let m = metadata(vec![MetadataEntry {
            index: ITEM_FRAME_ROTATION,
            value: MetadataValue::VarInt(9),
        }]);
        assert_eq!(item_frame_rotation(&m), 1);
    }

    #[test]
    fn painting_registry_id_is_varint_minus_one() {
        // `minecraft:kebab` is registry id 24, sent as the single byte 25.
        let m = metadata(vec![MetadataEntry {
            index: PAINTING_VARIANT,
            value: MetadataValue::Other {
                type_id: 34,
                raw: vec![25].into(),
            },
        }]);
        assert_eq!(
            painting_variant(&m),
            Some(PaintingVariantRef::Registry(RegistryIndex(24)))
        );
    }

    #[test]
    fn painting_inline_variant_parses_width_height_asset() {
        // 0 (inline), width 2, height 2, "minecraft:kebab".
        let mut raw = vec![0u8];
        raw.extend_from_slice(&2i32.to_be_bytes());
        raw.extend_from_slice(&2i32.to_be_bytes());
        let asset = b"minecraft:kebab";
        raw.push(asset.len() as u8);
        raw.extend_from_slice(asset);
        let m = metadata(vec![MetadataEntry {
            index: PAINTING_VARIANT,
            value: MetadataValue::Other {
                type_id: 34,
                raw: raw.into(),
            },
        }]);
        let Some(PaintingVariantRef::Inline(v)) = painting_variant(&m) else {
            panic!("inline variant");
        };
        assert_eq!((v.width, v.height), (2, 2));
        assert_eq!(v.asset, Identifier::minecraft("kebab"));
        assert_eq!(v.front_texture(), Identifier::minecraft("painting/kebab"));
    }

    #[test]
    fn painting_variant_sizes_from_the_table() {
        let mut variants = HashMap::new();
        variants.insert(
            24,
            PaintingVariant {
                width: 2,
                height: 2,
                asset: Identifier::minecraft("kebab"),
            },
        );
        let m = metadata(vec![MetadataEntry {
            index: PAINTING_VARIANT,
            value: MetadataValue::Other {
                type_id: 34,
                raw: vec![25].into(),
            },
        }]);
        // Unknown kind index: no decoration.
        assert!(build(
            RegistryIndex(999),
            2,
            &m,
            &variants,
            1,
            0.0,
            0.5,
            BlockStateId::AIR
        )
        .is_none());
    }

    #[test]
    fn tnt_metadata_reads_fuse_and_block_state() {
        // The default fuse is vanilla's `PrimedTnt.DEFAULT_FUSE_TIME` (80), and no block
        // state means the TNT block's default (the caller resolves it).
        let m = Metadata::default();
        assert_eq!(m.tnt_fuse, crate::metadata::DEFAULT_TNT_FUSE);
        assert_eq!(m.tnt_block_state, None);
        // `DATA_FUSE_ID` (index 8, int) and `DATA_BLOCK_STATE_ID` (index 9).
        let m = metadata(vec![
            MetadataEntry {
                index: 8,
                value: MetadataValue::VarInt(37),
            },
            MetadataEntry {
                index: 9,
                value: MetadataValue::BlockState(BlockStateId(12)),
            },
        ]);
        assert_eq!(m.tnt_fuse, 37);
        assert_eq!(m.tnt_block_state, Some(BlockStateId(12)));
    }

    #[test]
    fn tnt_fuse_eases_from_one_past_the_synced_value() {
        // `getFuse() - partialTicks + 1`: 10 renders 11 -> 10 across the tick, so the
        // countdown stays continuous across tick boundaries.
        assert!((tnt_fuse_remaining(10, 0.0) - 11.0).abs() < 1e-6);
        assert!((tnt_fuse_remaining(10, 1.0) - 10.0).abs() < 1e-6);
        assert!((tnt_fuse_remaining(10, 0.25) - 10.75).abs() < 1e-6);
    }

    #[test]
    fn painting_center_offsets_even_sizes() {
        // NORTH painting on block (0,0,0): center = (0.5,0.5,0.5) - (0,0,-1)*0.46875
        // + WEST*0.5 + UP*0.5 = (0.0, 1.0, 0.96875).
        let c = hanging_render_center(Direction::North, haste_core::DVec3::ZERO, Some((2, 2)));
        assert_eq!(c, haste_core::DVec3::new(0.0, 1.0, 0.96875));
        // An odd-size painting only moves to the wall.
        let c = hanging_render_center(Direction::North, haste_core::DVec3::ZERO, Some((1, 3)));
        assert_eq!(c, haste_core::DVec3::new(0.5, 0.5, 0.96875));
        // An item frame never takes the size offsets.
        let c = hanging_render_center(Direction::North, haste_core::DVec3::ZERO, None);
        assert_eq!(c, haste_core::DVec3::new(0.5, 0.5, 0.96875));
    }
}
