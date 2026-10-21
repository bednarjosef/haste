//! Difficulty tests: the ids wrap around the four values, a fresh login starts at normal and
//! unlocked, and a `change_difficulty` packet replaces both.

use std::sync::Arc;

use haste_core::EntityId;
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
fn ids_wrap_around_the_four_difficulties() {
    assert_eq!(Difficulty::from_id(0), Difficulty::Peaceful);
    assert_eq!(Difficulty::from_id(1), Difficulty::Easy);
    assert_eq!(Difficulty::from_id(2), Difficulty::Normal);
    assert_eq!(Difficulty::from_id(3), Difficulty::Hard);
    assert_eq!(Difficulty::from_id(4), Difficulty::Peaceful);
    assert_eq!(Difficulty::from_id(7), Difficulty::Hard);
}

#[test]
fn a_fresh_session_starts_at_normal_and_unlocked() {
    assert_eq!(
        session().level_difficulty,
        LevelDifficulty {
            difficulty: Difficulty::Normal,
            locked: false,
        }
    );
}

#[test]
fn a_change_difficulty_packet_replaces_the_difficulty_and_the_lock() {
    let mut s = session();
    s.set_difficulty(&cb::ChangeDifficulty {
        difficulty: 3,
        locked: true,
    });
    assert_eq!(
        s.level_difficulty,
        LevelDifficulty {
            difficulty: Difficulty::Hard,
            locked: true,
        }
    );
}
