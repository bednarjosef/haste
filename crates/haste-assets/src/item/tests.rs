//! The third-person display entries of the item models (`ItemTransforms.getTransform`):
//! the left hand's own entry and its per-file fallback to the right one.
use super::*;

/// `item/handheld` defines its own `thirdperson_lefthand` ([0, 90, -55], translation
/// [0, 4, 0.5]/16, scale 0.85) — written so the left-hand fix at draw time cancels it back
/// to the right entry's angle — and `item/generated` names none, so a flat item's left
/// entry is its right one (`ItemTransforms.getTransform(THIRD_PERSON_LEFT_HAND)`).
#[test]
#[ignore = "needs the client jar in the cache"]
fn handhelds_left_entry_and_generateds_fallback() {
    let jar = haste_core::paths::client_jar();
    let mut a = Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let sword = a
        .item_model(&Identifier::minecraft("diamond_sword"))
        .unwrap();
    assert_eq!(sword.parts[0].thirdperson.rotation, [0.0, -90.0, 55.0]);
    assert_eq!(sword.parts[0].thirdperson_left.rotation, [0.0, 90.0, -55.0]);
    assert_eq!(
        sword.parts[0].thirdperson_left.translation,
        [0.0, 0.25, 0.5 / 16.0]
    );
    assert_eq!(sword.parts[0].thirdperson_left.scale, [0.85; 3]);
    let bread = a.item_model(&Identifier::minecraft("bread")).unwrap();
    assert_eq!(bread.parts[0].thirdperson_left, bread.parts[0].thirdperson);
    assert_eq!(bread.parts[0].thirdperson.scale, [0.55; 3]);
}
