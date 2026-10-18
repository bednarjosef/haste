//! Tests of the channel pools (`crate::channels`).

use super::*;

#[test]
fn an_ordinary_device_splits_into_247_static_and_8_streaming() {
    // Library.init with 255 mono sources: streaming = clamp(sqrt(255) = 15, 2, 8) = 8,
    // static = clamp(255 - 8, 8, 255) = 247.
    let pools = ChannelPools::new();
    assert_eq!(pools.limit(Pool::Static), 247);
    assert_eq!(pools.limit(Pool::Streaming), 8);
}

#[test]
fn the_split_follows_library_init_for_other_device_sizes() {
    // The same clamps for other mono-source counts.
    let pools = ChannelPools::with_total_sources(30);
    assert_eq!(pools.limit(Pool::Streaming), 5, "sqrt(30) = 5.47 -> 5");
    assert_eq!(pools.limit(Pool::Static), 25);
    // A small device still gets both pools (2..=8 streaming, 8..=255 static).
    let pools = ChannelPools::with_total_sources(1);
    assert_eq!(pools.limit(Pool::Streaming), 2);
    assert_eq!(pools.limit(Pool::Static), 8);
    let pools = ChannelPools::with_total_sources(1000);
    assert_eq!(pools.limit(Pool::Streaming), 8);
    assert_eq!(pools.limit(Pool::Static), 255);
}

#[test]
fn a_full_pool_refuses_and_a_released_slot_answers_again() {
    let mut pools = ChannelPools::with_total_sources(30);
    for _ in 0..pools.limit(Pool::Streaming) {
        assert!(pools.acquire(Pool::Streaming));
    }
    assert!(!pools.acquire(Pool::Streaming), "streaming pool is full");
    // The static pool is separate: it still answers.
    assert!(pools.acquire(Pool::Static));
    // Release one streaming channel and it answers again.
    pools.release(Pool::Streaming);
    assert!(pools.acquire(Pool::Streaming));
    assert_eq!(pools.used(Pool::Streaming), pools.limit(Pool::Streaming));
}

#[test]
fn debug_string_is_vanillas_format() {
    let mut pools = ChannelPools::new();
    pools.acquire(Pool::Static);
    pools.acquire(Pool::Static);
    pools.acquire(Pool::Streaming);
    assert_eq!(pools.debug_string(), "Sounds: 2/247 + 1/8");
}
