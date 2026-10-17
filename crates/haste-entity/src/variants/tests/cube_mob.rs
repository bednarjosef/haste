use super::*;
/// The slime and magma cube scale: `AbstractCubeMob.ID_SIZE` at index 18, everything
/// else 1.0.
#[test]
#[ignore = "needs cache/datagen (registries.json; HASTE_CACHE overrides)"]
fn cube_size_scales_the_model() {
    let Some(slime) = kind_of("minecraft:slime") else {
        eprintln!("skipped: no registries.json in the cache");
        return;
    };
    let size = |i: i32| int_metadata(&[(metadata_index::CUBE_SIZE, i)]);
    assert_eq!(scale_for(slime, &Metadata::default()), 1.0);
    assert_eq!(scale_for(slime, &size(4)), 4.0);
    // Non-positive sizes render at the minimum (vanilla never sends them).
    assert_eq!(scale_for(slime, &size(0)), 1.0);
    if let Some(pig) = kind_of("minecraft:pig") {
        assert_eq!(scale_for(pig, &size(4)), 1.0);
    }
}
