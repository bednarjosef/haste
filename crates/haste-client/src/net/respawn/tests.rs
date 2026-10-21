//! Respawn tests: the vanilla default point, and a `set_default_spawn_position` packet replacing
//! it.

use std::sync::Arc;

use haste_core::{BlockPos, EntityId, Identifier};
use haste_protocol::haste_core::game::{DimensionInfo, GameMode};
use haste_protocol::packets::play::clientbound as cb;

use super::*;
use crate::net::Session;

fn session() -> Session {
    Session::new(
        EntityId(7),
        GameMode::Creative,
        DimensionInfo::overworld(),
        Arc::new(Vec::new()),
    )
}

#[test]
fn the_default_point_is_the_overworld_at_the_origin() {
    let d = RespawnData::default();
    assert_eq!(d.dimension, Identifier::minecraft("overworld"));
    assert_eq!(d.pos, BlockPos::new(0, 0, 0));
    assert_eq!((d.yaw, d.pitch), (0.0, 0.0));
}

#[test]
fn a_fresh_session_starts_at_the_default_point() {
    assert_eq!(session().respawn, RespawnData::default());
}

#[test]
fn a_spawn_packet_replaces_the_stored_point() {
    let mut s = session();
    let packet = cb::SetDefaultSpawnPosition {
        dimension: Identifier::minecraft("the_nether"),
        pos: BlockPos::new(-12, 70, 33),
        yaw: 90.0,
        pitch: -5.0,
    };
    s.set_respawn_data(&packet);
    assert_eq!(
        s.respawn,
        RespawnData {
            dimension: Identifier::minecraft("the_nether"),
            pos: BlockPos::new(-12, 70, 33),
            yaw: 90.0,
            pitch: -5.0,
        }
    );
}
