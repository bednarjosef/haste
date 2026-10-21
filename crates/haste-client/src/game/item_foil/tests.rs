//! The patch side of `ItemStack.hasFoil`.

use super::*;
use haste_protocol::wire::components::EnchantmentLevel;

fn other(name: &str, raw: &[u8]) -> ItemComponent {
    ItemComponent::Other {
        name: name.to_owned(),
        raw: bytes::Bytes::copy_from_slice(raw),
    }
}

#[test]
fn an_empty_patch_changes_nothing() {
    assert_eq!(foil_patch(&ComponentPatch::default()), FoilPatch::default());
}

#[test]
fn enchantments_set_by_list_emptiness() {
    let enchanted = ComponentPatch {
        added: vec![ItemComponent::Enchantments(vec![EnchantmentLevel {
            id: 3,
            level: 5,
        }])],
        removed: vec![],
    };
    assert_eq!(foil_patch(&enchanted).enchantments, Patched::Set(true));
    let empty = ComponentPatch {
        added: vec![ItemComponent::Enchantments(vec![])],
        removed: vec![],
    };
    assert_eq!(foil_patch(&empty).enchantments, Patched::Set(false));
}

#[test]
fn stored_enchantments_are_not_enchantments() {
    let book = ComponentPatch {
        added: vec![ItemComponent::StoredEnchantments(vec![EnchantmentLevel {
            id: 3,
            level: 5,
        }])],
        removed: vec![],
    };
    assert_eq!(foil_patch(&book), FoilPatch::default());
}

#[test]
fn glint_override_reads_its_bool_and_removals_are_seen() {
    let on = ComponentPatch {
        added: vec![other(GLINT_OVERRIDE, &[1])],
        removed: vec![],
    };
    assert_eq!(foil_patch(&on).glint_override, Patched::Set(true));
    let off = ComponentPatch {
        added: vec![other(GLINT_OVERRIDE, &[0])],
        removed: vec![],
    };
    assert_eq!(foil_patch(&off).glint_override, Patched::Set(false));
    let removed = ComponentPatch {
        added: vec![other(LODESTONE_TRACKER, &[0])],
        removed: vec![GLINT_OVERRIDE.to_owned(), ENCHANTMENTS.to_owned()],
    };
    let p = foil_patch(&removed);
    assert_eq!(p.glint_override, Patched::Removed);
    assert_eq!(p.enchantments, Patched::Removed);
    assert_eq!(p.lodestone_tracker, Patched::Set(()));
}
