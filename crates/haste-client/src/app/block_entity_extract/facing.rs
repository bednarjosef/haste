use haste_protocol::haste_core::Direction;

/// A block state `facing` property value as a `Direction`.
pub(super) fn direction_of_property(v: &str) -> Option<Direction> {
    Direction::ALL.iter().find(|d| d.name() == v).copied()
}

/// Vanilla `Direction.toYRot`: south 0, west 90, north 180, east 270.
pub(super) fn to_y_rot(d: Direction) -> f32 {
    match d {
        Direction::South => 0.0,
        Direction::West => 90.0,
        Direction::North => 180.0,
        Direction::East => 270.0,
        Direction::Down | Direction::Up => 0.0,
    }
}

/// The yaw a sign/banner turns by: the block state's `rotation` segment
/// (`RotationSegment.convertToDegrees`: `segment * 22.5`, wrapped from 180 to -180), or the
/// wall `facing`'s `toYRot()` when there is no rotation property.
pub(super) fn rotation_or_facing(
    blocks: &haste_data::BlockStates,
    state: haste_core::BlockStateId,
    facing: Direction,
) -> f32 {
    match blocks.property_value(state, "rotation") {
        Some(rot) => rot.parse::<i32>().map_or(0.0, |r| {
            let d = r.rem_euclid(16) as f32 * 22.5;
            if d >= 180.0 {
                d - 360.0
            } else {
                d
            }
        }),
        None => to_y_rot(facing),
    }
}
