use haste_protocol::haste_core::{BlockPos, Direction};
use haste_world::World;

use super::facing::direction_of_property;

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
