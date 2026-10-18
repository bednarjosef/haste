use haste_protocol::haste_core::{BlockPos, Direction};
use haste_world::World;

use super::facing::direction_of_property;
use super::BlockEntityExtractor;

impl BlockEntityExtractor {
    /// A double chest's lid openness and light (vanilla `ChestBlock.opennessCombiner`'s max
    /// and `BrightnessCombiner`'s max light over both halves): our own lid and light, or the
    /// max with the matching other half's when there is one.
    pub(super) fn chest_open_and_light(
        &self,
        world: &World,
        blocks: &haste_data::BlockStates,
        pos: BlockPos,
        facing: Direction,
        half: haste_render::ChestHalf,
    ) -> ((f32, f32), (u8, u8)) {
        use haste_core::view::BlockView as _;
        let mut open = self.lid_map.get(&pos).copied().unwrap_or((0.0, 0.0));
        let mut light = (world.sky_light(pos), world.block_light(pos));
        if half != haste_render::ChestHalf::Single {
            let connected = connected_direction(facing, half);
            let neighbour = pos.offset(connected);
            if is_double_chest_half(blocks, world, neighbour, half, facing) {
                if let Some(n) = self.lid_map.get(&neighbour) {
                    open = (open.0.max(n.0), open.1.max(n.1));
                }
                light = (
                    light.0.max(world.sky_light(neighbour)),
                    light.1.max(world.block_light(neighbour)),
                );
            }
        }
        (open, light)
    }
}

/// Where a double chest's other half is (vanilla `ChestBlock.getConnectedDirection`:
/// the LEFT half's neighbour is clockwise of the facing, the RIGHT half's counter-clockwise).
pub(super) fn connected_direction(facing: Direction, half: haste_render::ChestHalf) -> Direction {
    let clockwise = |d: Direction| match d {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        Direction::Down | Direction::Up => d,
    };
    match half {
        haste_render::ChestHalf::Left => clockwise(facing),
        _ => clockwise(facing).opposite(),
    }
}

/// Whether `pos` holds the matching other half of our double chest: a chest/trapped chest of
/// the opposite half, the same facing, whose connected direction points back at us (vanilla's
/// `DoubleBlockCombiner` accepts exactly that neighbour).
pub(super) fn is_double_chest_half(
    blocks: &haste_data::BlockStates,
    world: &World,
    pos: BlockPos,
    our_half: haste_render::ChestHalf,
    facing: Direction,
) -> bool {
    use haste_core::view::BlockView as _;
    let state = world.block_state(pos);
    let block = blocks.block_of(state);
    if block.namespace() != "minecraft" || !matches!(block.path(), "chest" | "trapped_chest") {
        return false;
    }
    let neighbour_half = match blocks.property_value(state, "type") {
        Some("left") => haste_render::ChestHalf::Left,
        Some("right") => haste_render::ChestHalf::Right,
        _ => return false,
    };
    let Some(neighbour_facing) = blocks
        .property_value(state, "facing")
        .and_then(direction_of_property)
    else {
        return false;
    };
    neighbour_facing == facing
        && neighbour_half == our_half.other()
        && connected_direction(neighbour_facing, neighbour_half).opposite()
            == connected_direction(facing, our_half)
}
