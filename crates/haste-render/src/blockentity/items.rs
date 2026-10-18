// The item and mob block entities (vanilla `CampfireRenderer`, `SpawnerRenderer` /
// `TrialSpawnerRenderer`, `VaultRenderer`, `ShelfRenderer`, `BrushableBlockRenderer`): their
// render payloads, per-slot and per-spin matrices, and the spawner's display entity posed
// through the entity cube emitter.

use glam::{Mat4, Vec3};
use haste_core::model::{CubeModel, ModelPart, PoseOverrides};
use haste_core::{BlockPos, Direction};

use super::{BlockEntityInstance, BlockEntityKind};
use crate::entity::{EntityVertex, TickPair};

/// The items on a campfire's grill, by slot (vanilla `CampfireBlockEntity.getItems()`, at most
/// four). Fixed size and `Copy`: no allocation per frame or per block entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CampfireItems {
    /// The item in each slot, by item registry index (`None` for an empty slot).
    pub items: [Option<haste_core::RegistryIndex>; 4],
}

impl CampfireItems {
    /// The item in `slot`, if any.
    pub fn get(&self, slot: usize) -> Option<haste_core::RegistryIndex> {
        self.items.get(slot).copied().flatten()
    }
}

/// The mob spinning inside a spawner (vanilla `SpawnerRenderState`): the display entity's
/// model (the client resolves it from `SpawnData`), the yaw spin in degrees and the scale.
#[derive(Clone, Debug, PartialEq)]
pub struct SpawnerDisplay {
    /// The display entity's cube model; `None` draws nothing (vanilla leaves the render
    /// state's `displayEntity` null when the spawn data names no spawnable entity).
    pub model: Option<std::sync::Arc<CubeModel>>,
    /// The display entity's spin as the ends of the last tick, in the units vanilla's
    /// `BaseSpawner.spin` / `TrialSpawnerStateData.spin` accumulate (see
    /// [`spawner_spin_at`] for the degrees the renderer draws).
    pub spin: TickPair,
    /// The scale vanilla derives from the display entity's bounding box
    /// (`0.53125` divided by the larger of its width and height when that exceeds 1).
    pub scale: f32,
}

/// The degrees a spawner's display mob is turned to at `partial`, vanilla
/// `TrialSpawnerRenderer.extractSpawnerData`'s `Mth.lerp(partialTicks, oSpin, spin) * 10`
/// (vanilla keeps the accumulating spin in its own units and scales it only for the pose).
pub fn spawner_spin_at(spawner: &SpawnerDisplay, partial: f32) -> f32 {
    spawner.spin.at(partial) * 10.0
}

/// A vault's displayed item (vanilla `VaultRenderState`): the item and how many copies of the
/// stack to draw, plus the spin in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VaultDisplay {
    pub item: haste_core::RegistryIndex,
    /// `ItemClusterRenderState.getRenderedAmount` of the stack's count.
    pub count: u32,
    pub spin: TickPair,
}

impl VaultDisplay {
    /// How many copies of the stack vanilla draws (1 for a single item, then one more per
    /// full 16 of the count, up to 5): `ItemClusterRenderState.getRenderedAmount`.
    pub fn copies(&self) -> u32 {
        match self.count {
            ..=0 => 0,
            1 => 1,
            2..=16 => 2,
            17..=32 => 3,
            33..=48 => 4,
            _ => 5,
        }
    }

    /// The yaw the displayed item is turned to at `partial`, vanilla `VaultRenderer`'s
    /// `Mth.rotLerp(partialTicks, clientData.previousSpin(), clientData.currentSpin())`: the
    /// lerp wraps at 180 degrees so the item keeps turning the short way when the spin rolls
    /// over from its 10 degrees a tick (`VaultClientData.updateDisplayItemSpin`).
    pub fn spin_at(&self, partial: f32) -> f32 {
        let from = self.spin.start;
        let mut delta = (self.spin.end - from) % 360.0;
        if delta >= 180.0 {
            delta -= 360.0;
        }
        if delta < -180.0 {
            delta += 360.0;
        }
        from + partial * delta
    }
}

/// The items on a shelf block (vanilla `ShelfBlockEntity`): three slots plus the
/// `align_items_to_bottom` flag the board's NBT carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ShelfItems {
    pub items: [Option<haste_core::RegistryIndex>; 3],
    pub align_to_bottom: bool,
}

impl ShelfItems {
    /// The item in `slot`, if any.
    pub fn get(&self, slot: usize) -> Option<haste_core::RegistryIndex> {
        self.items.get(slot).copied().flatten()
    }
}

/// The item a brushable block reveals (vanilla `BrushableBlockRenderState`): the item, the
/// block face it is brushed from, and the `dusted` state (0..3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrushableItem {
    pub item: haste_core::RegistryIndex,
    /// `BrushableBlockEntity.getHitDirection()`; `None` while the block has not been brushed
    /// (vanilla then draws nothing).
    pub hit_direction: Option<Direction>,
    /// The block state's `dusted` property (vanilla's completion state, 0..3).
    pub dusted: u8,
}

/// Vanilla `TrialSpawnerRenderer.extractSpawnerData`'s base scale before the bounding-box
/// correction (`state.scale = 0.53125F`).
pub const SPAWNER_SCALE: f32 = 0.53125;

/// Vanilla `CampfireRenderer.SIZE`: the scale an item on a campfire's grill is drawn at.
pub const CAMPFIRE_ITEM_SIZE: f32 = 0.375;

/// Vanilla `ShelfRenderer.ITEM_SIZE` and `ALIGN_ITEMS_TO_BOTTOM`.
pub const SHELF_ITEM_SIZE: f32 = 0.25;
pub const SHELF_ALIGN_OFFSET: f32 = -0.25;

/// The extent of a cube model's geometry at rest (model units), as the bounding box of every
/// cube transformed by its part pivots and rotations (the same matrices `emit_part` composes,
/// with no pose). Used to size a spawner's display entity.
pub(crate) fn cube_model_extent(model: &CubeModel) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut see = |p: Vec3| {
        min = min.min(p);
        max = max.max(p);
    };
    extent_of(&model.root, Mat4::IDENTITY, &mut see);
    if min.x.is_finite() {
        (min, max)
    } else {
        (Vec3::ZERO, Vec3::ZERO)
    }
}

fn extent_of(part: &ModelPart, parent: Mat4, see: &mut impl FnMut(Vec3)) {
    let m = parent
        * Mat4::from_translation(part.pivot)
        * Mat4::from_rotation_z(part.rotation.z)
        * Mat4::from_rotation_y(part.rotation.y)
        * Mat4::from_rotation_x(part.rotation.x);
    for cube in &part.cubes {
        let o = cube.origin + cube.inflate;
        let s = cube.size + 2.0 * cube.inflate;
        for x in [o.x, o.x + s.x] {
            for y in [o.y, o.y + s.y] {
                for z in [o.z, o.z + s.z] {
                    see(m.transform_point3(Vec3::new(x, y, z)));
                }
            }
        }
    }
    for child in &part.children {
        extent_of(child, m, see);
    }
}

/// Vanilla `TrialSpawnerRenderer.extractSpawnerData`'s scale for `model`: `0.53125`, divided by
/// the larger of the display entity's bounding box width and height when that exceeds one
/// block. Haste has no per-type bounding box table yet (haste-entity's `EntityDimensions`
/// falls back to its default box, `crates/haste-entity/src/dims.rs:49`), so the box comes from
/// the display model's own geometry, which vanilla's mob models are built to match.
pub fn spawner_scale(model: &CubeModel) -> f32 {
    let (min, max) = cube_model_extent(model);
    let longest = (max.x - min.x).max(max.y - min.y) / 16.0;
    if longest > 1.0 {
        SPAWNER_SCALE / longest
    } else {
        SPAWNER_SCALE
    }
}

/// Vanilla `CampfireRenderer.submit`'s per-slot matrix: the block's centre, turned so the slot
/// looks outwards (`Direction.from2DDataValue((slot + facing.get2DDataValue()) % 4)`'s
/// `-toYRot()`), laid flat (the +90 degrees about X) and offset to its own quadrant of the
/// grill, at vanilla's `SIZE` scale. The item's own `display.fixed` transform is the
/// renderer's job (`emit_item`).
pub(crate) fn campfire_item_matrix(
    pos: BlockPos,
    facing: Direction,
    slot: usize,
    camera: glam::DVec3,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    // The slot's direction around the block (`from2DDataValue` of `(slot + facing) % 4`): the
    // 2D order is south 0, west 1, north 2, east 3, so the item turns by `-toYRot` of it.
    let two_d = (slot + facing.to_2d_data_value() as usize) % 4;
    let y_rot = -(two_d as f32 * 90.0);
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.44921875, 0.5))
        * Mat4::from_rotation_y(y_rot.to_radians())
        * Mat4::from_rotation_x(90.0f32.to_radians())
        * Mat4::from_translation(Vec3::new(-0.3125, -0.3125, 0.0))
        * Mat4::from_scale(Vec3::splat(CAMPFIRE_ITEM_SIZE))
}

/// Vanilla `SpawnerRenderer.submitEntityInSpawner` followed by
/// `LivingEntityRenderer.submit`'s transform for the static display entity: the block's centre
/// turned by the spawner's spin, the mob dropped a little and tipped back 30 degrees, then
/// vanilla's `setupRotations` yaw (the display entity never turns its body, so it is the
/// fixed `180 - bodyRot`), the model's y/z flip and the /16 that turns the model's pixel units
/// into blocks. Haste's entity models place the feet at their y 0, so vanilla's extra
/// `-1.501` block translate is not needed here.
pub(crate) fn spawner_root(pos: BlockPos, spin: f32, scale: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.4, 0.5))
        * Mat4::from_rotation_y(spin.to_radians())
        * Mat4::from_translation(Vec3::new(0.0, -0.2, 0.0))
        * Mat4::from_rotation_x((-30.0f32).to_radians())
        * Mat4::from_scale(Vec3::splat(scale))
        * Mat4::from_rotation_y(180.0f32.to_radians())
        * Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0) / 16.0)
}

/// Poses a spawner's display entity model into `out` (the entity motion is not animated: the
/// mob only turns with the spawner's spin, like vanilla's `EntityRenderDispatcher.submit` on
/// the static display entity).
pub(crate) fn emit_spawner(
    out: &mut Vec<EntityVertex>,
    inst: &BlockEntityInstance,
    model: &CubeModel,
    rect: [u32; 4],
    camera: glam::DVec3,
    partial: f32,
) {
    let BlockEntityKind::Spawner(ref spawner) = inst.kind else {
        return;
    };
    use crate::entity::emit_part_tinted;
    let (sky, block) = inst.light;
    let root = spawner_root(
        inst.pos,
        spawner_spin_at(spawner, partial),
        spawner.scale,
        camera,
    );
    emit_part_tinted(
        out,
        model,
        &model.root,
        &PoseOverrides::default(),
        root,
        rect,
        (block, sky),
        0.0,
        [1.0; 3],
    );
}

/// Vanilla `VaultRenderer.submit`: the block's centre at 0.4, turned by the vault's spin. The
/// item cluster's copies come from the caller.
pub(crate) fn vault_root(pos: BlockPos, spin: f32, camera: glam::DVec3) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.4, 0.5))
        * Mat4::from_rotation_y(spin.to_radians())
}

/// Vanilla `ShelfRenderer.submitItem`: the block's centre turned by the shelf's facing (a
/// vertical facing uses vanilla's 180 degrees), then the slot's x offset and the standing
/// plane's -0.25, at vanilla's `ITEM_SIZE` scale. `offset_y` is vanilla's bounding-box
/// alignment (see [`shelf_item_offset_y`]); it is applied in the same scaled space, after the
/// scale, exactly like the renderer's `poseStack.translate(0, offsetY, 0)`.
pub(crate) fn shelf_item_matrix(
    pos: BlockPos,
    facing: Direction,
    slot: usize,
    align_to_bottom: bool,
    offset_y: f32,
    camera: glam::DVec3,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    let y_rot = if facing.is_horizontal() {
        -facing.to_y_rot()
    } else {
        180.0
    };
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.5, 0.5, 0.5))
        * Mat4::from_rotation_y(y_rot.to_radians())
        * Mat4::from_translation(Vec3::new(
            (slot as f32 - 1.0) * 0.3125,
            if align_to_bottom {
                SHELF_ALIGN_OFFSET
            } else {
                0.0
            },
            -0.25,
        ))
        * Mat4::from_scale(Vec3::splat(SHELF_ITEM_SIZE))
        * Mat4::from_translation(Vec3::new(0.0, offset_y, 0.0))
}

/// Vanilla `ShelfRenderer.submitItem`'s vertical alignment: the model's box is lifted by its
/// min y, and centred on the board when the shelf does not align its items to the bottom.
/// `bbox` is the model's bounding box through its `on_shelf` display transform.
pub fn shelf_item_offset_y(bbox: (Vec3, Vec3), align_to_bottom: bool) -> f32 {
    let (min, max) = bbox;
    let mut offset = -min.y;
    if !align_to_bottom {
        offset += -(max.y - min.y) / 2.0;
    }
    offset
}

/// Vanilla `BrushableBlockRenderer.submit`'s matrix: half a block up, translated out of the
/// brushed face by the completion state, then vanilla's fixed 75 + 11 (90 more for a face on
/// the x axis) degrees of yaw, at half scale. The item's own `display.fixed` transform is the
/// renderer's job.
pub(crate) fn brushable_item_matrix(
    pos: BlockPos,
    hit_direction: Direction,
    dusted: u8,
    camera: glam::DVec3,
) -> Mat4 {
    let p = pos.as_dvec3() - camera;
    // Vanilla `translations`: each face's own resting translation, plus the completion offset
    // out of it (`completionState / 10 * 0.75`).
    let mut t = Vec3::new(0.5, 0.0, 0.5);
    let offset = dusted as f32 / 10.0 * 0.75;
    match hit_direction {
        Direction::East => t.x = 0.73 + offset,
        Direction::West => t.x = 0.25 - offset,
        Direction::Up => t.y = 0.25 + offset,
        Direction::Down => t.y = -0.23 - offset,
        Direction::North => t.z = 0.25 - offset,
        Direction::South => t.z = 0.73 + offset,
    }
    let east_west = matches!(hit_direction, Direction::East | Direction::West);
    Mat4::from_translation(p.as_vec3() + Vec3::new(0.0, 0.5, 0.0))
        * Mat4::from_translation(t)
        * Mat4::from_rotation_y(75.0f32.to_radians())
        * Mat4::from_rotation_y((if east_west { 90.0f32 } else { 0.0 } + 11.0).to_radians())
        * Mat4::from_scale(Vec3::splat(0.5))
}
/// The block entity kinds that draw an item or a stack of items: they go through the entity
/// renderer's item emitters (same models as held and dropped items) rather than the block
/// entity's own texture, so the emitter splits them out first.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ItemBlockEntity {
    Campfire(CampfireItems),
    Shelf(ShelfItems),
    Vault(VaultDisplay),
    Brushable(BrushableItem),
}

/// The item-drawing payload of a block entity kind, when it has one.
pub(crate) fn item_kind(kind: &BlockEntityKind) -> Option<ItemBlockEntity> {
    match kind {
        BlockEntityKind::Campfire(items) => Some(ItemBlockEntity::Campfire(*items)),
        BlockEntityKind::Shelf(items) => Some(ItemBlockEntity::Shelf(*items)),
        BlockEntityKind::Vault(vault) => Some(ItemBlockEntity::Vault(*vault)),
        BlockEntityKind::Brushable(b) => Some(ItemBlockEntity::Brushable(*b)),
        _ => None,
    }
}
