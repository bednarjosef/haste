//! Part names interned to small ids, so per-frame pose state compares ids instead of strings.
//!
//! Vanilla's `ModelPart` trees key their parts by name (`MeshDefinition`'s child map), and the
//! client resolves pose overrides by that name once per part per frame. Interning turns each
//! name into a `PartId` at model build time (a cold path); the frame path then matches ids.

use std::collections::HashMap;
use std::sync::Mutex;

/// An interned part name. Ids are stable for the process; parts of one model have distinct
/// names (vanilla's `MeshDefinition` keys its children by name), so an id never matches two
/// parts of the same model.
///
/// `Default` exists for serde's `skip` on `ModelPart::id` (nothing in the tree serializes
/// models today); a tree that was deserialized must re-derive its ids (`intern(name)` per
/// part) before rendering, or every part matches `ROOT` at worst.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PartId(u32);

/// The well-known parts, interned as constants so hot paths reference them directly instead
/// of calling [`intern`]. Every name a Haste-authored or dumped model uses still goes through
/// `intern` at model build time; unknown names get ids from `FIRST_DYNAMIC` up and behave
/// exactly the same. Each constant's value is its entry's index in `KNOWN` (the tests pin
/// that).
pub const HEAD: PartId = PartId(0);
pub const BODY: PartId = PartId(1);
pub const RIGHT_ARM: PartId = PartId(2);
pub const LEFT_ARM: PartId = PartId(3);
pub const RIGHT_LEG: PartId = PartId(4);
pub const LEFT_LEG: PartId = PartId(5);
pub const RIGHT_FRONT_LEG: PartId = PartId(6);
pub const LEFT_FRONT_LEG: PartId = PartId(7);
pub const RIGHT_HIND_LEG: PartId = PartId(8);
pub const LEFT_HIND_LEG: PartId = PartId(9);
pub const RIGHT_MIDDLE_FRONT_LEG: PartId = PartId(10);
pub const LEFT_MIDDLE_FRONT_LEG: PartId = PartId(11);
pub const RIGHT_MIDDLE_HIND_LEG: PartId = PartId(12);
pub const LEFT_MIDDLE_HIND_LEG: PartId = PartId(13);
pub const RIGHT_WING: PartId = PartId(14);
pub const LEFT_WING: PartId = PartId(15);
pub const ARMS: PartId = PartId(16);
pub const HAT: PartId = PartId(17);
pub const JACKET: PartId = PartId(18);
pub const LEFT_PANTS: PartId = PartId(19);
pub const RIGHT_PANTS: PartId = PartId(20);
pub const LEFT_SLEEVE: PartId = PartId(21);
pub const RIGHT_SLEEVE: PartId = PartId(22);
pub const CAPE: PartId = PartId(23);
pub const LID: PartId = PartId(24);
pub const LOCK: PartId = PartId(25);
pub const LEFT_LID: PartId = PartId(26);
pub const RIGHT_LID: PartId = PartId(27);
pub const ROOT: PartId = PartId(28);

/// The known names, in the order of their constants (`intern` matches these before the
/// dynamic table). Order: animation targets, player skin layers, block entity parts, root.
const KNOWN: &[&str] = &[
    "head",
    "body",
    "right_arm",
    "left_arm",
    "right_leg",
    "left_leg",
    "right_front_leg",
    "left_front_leg",
    "right_hind_leg",
    "left_hind_leg",
    "right_middle_front_leg",
    "left_middle_front_leg",
    "right_middle_hind_leg",
    "left_middle_hind_leg",
    "right_wing",
    "left_wing",
    "arms",
    "hat",
    "jacket",
    "left_pants",
    "right_pants",
    "left_sleeve",
    "right_sleeve",
    "cape",
    "lid",
    "lock",
    "left_lid",
    "right_lid",
    "root",
];

/// The first id the dynamic interner may hand out; the constants above own `0..FIRST_DYNAMIC`.
pub const FIRST_DYNAMIC: u32 = KNOWN.len() as u32;

#[cfg(test)]
mod known_order_tests {
    use super::*;

    /// Each constant's value is its name's index in `KNOWN` (the invariant `intern` relies on).
    #[test]
    fn constants_match_the_known_table() {
        assert_eq!(KNOWN[HEAD.0 as usize], "head");
        assert_eq!(KNOWN[BODY.0 as usize], "body");
        assert_eq!(KNOWN[RIGHT_ARM.0 as usize], "right_arm");
        assert_eq!(KNOWN[LEFT_ARM.0 as usize], "left_arm");
        assert_eq!(KNOWN[RIGHT_LEG.0 as usize], "right_leg");
        assert_eq!(KNOWN[LEFT_LEG.0 as usize], "left_leg");
        assert_eq!(KNOWN[RIGHT_FRONT_LEG.0 as usize], "right_front_leg");
        assert_eq!(KNOWN[LEFT_FRONT_LEG.0 as usize], "left_front_leg");
        assert_eq!(KNOWN[RIGHT_HIND_LEG.0 as usize], "right_hind_leg");
        assert_eq!(KNOWN[LEFT_HIND_LEG.0 as usize], "left_hind_leg");
        assert_eq!(
            KNOWN[RIGHT_MIDDLE_FRONT_LEG.0 as usize],
            "right_middle_front_leg"
        );
        assert_eq!(
            KNOWN[LEFT_MIDDLE_FRONT_LEG.0 as usize],
            "left_middle_front_leg"
        );
        assert_eq!(
            KNOWN[RIGHT_MIDDLE_HIND_LEG.0 as usize],
            "right_middle_hind_leg"
        );
        assert_eq!(
            KNOWN[LEFT_MIDDLE_HIND_LEG.0 as usize],
            "left_middle_hind_leg"
        );
        assert_eq!(KNOWN[RIGHT_WING.0 as usize], "right_wing");
        assert_eq!(KNOWN[LEFT_WING.0 as usize], "left_wing");
        assert_eq!(KNOWN[ARMS.0 as usize], "arms");
        assert_eq!(KNOWN[HAT.0 as usize], "hat");
        assert_eq!(KNOWN[JACKET.0 as usize], "jacket");
        assert_eq!(KNOWN[LEFT_PANTS.0 as usize], "left_pants");
        assert_eq!(KNOWN[RIGHT_PANTS.0 as usize], "right_pants");
        assert_eq!(KNOWN[LEFT_SLEEVE.0 as usize], "left_sleeve");
        assert_eq!(KNOWN[RIGHT_SLEEVE.0 as usize], "right_sleeve");
        assert_eq!(KNOWN[CAPE.0 as usize], "cape");
        assert_eq!(KNOWN[LID.0 as usize], "lid");
        assert_eq!(KNOWN[LOCK.0 as usize], "lock");
        assert_eq!(KNOWN[LEFT_LID.0 as usize], "left_lid");
        assert_eq!(KNOWN[RIGHT_LID.0 as usize], "right_lid");
        assert_eq!(KNOWN[ROOT.0 as usize], "root");
    }
}

/// The dynamic table (names past the known set): name -> id and id -> name, grown lazily.
/// Part names come from the model dump and hand-written models, a few hundred in practice;
/// each name is leaked once.
struct Table {
    by_name: HashMap<&'static str, PartId>,
    by_id: Vec<&'static str>,
}

static TABLE: Mutex<Option<Table>> = Mutex::new(None);

/// Interns `name`: its constant id when it is a known part, else the next dynamic id. Ids are
/// only ever assigned, never reused, so a `PartId` stays valid for the whole process.
pub fn intern(name: &str) -> PartId {
    if let Some(i) = KNOWN.iter().position(|k| *k == name) {
        return PartId(i as u32);
    }
    let mut guard = TABLE.lock().unwrap();
    let table = guard.get_or_insert_with(|| Table {
        by_name: HashMap::new(),
        by_id: Vec::new(),
    });
    if let Some(id) = table.by_name.get(name) {
        return *id;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    let id = PartId(FIRST_DYNAMIC + table.by_id.len() as u32);
    table.by_id.push(leaked);
    table.by_name.insert(leaked, id);
    id
}

/// The name an id interns (`"?"` for an id this process never assigned; every id in a live
/// model was assigned here or is a constant).
pub fn name_of(id: PartId) -> &'static str {
    let i = id.0 as usize;
    if i < KNOWN.len() {
        return KNOWN[i];
    }
    let guard = TABLE.lock().unwrap();
    guard
        .as_ref()
        .and_then(|t| t.by_id.get(i - KNOWN.len()))
        .copied()
        .unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intern_is_stable_and_injective() {
        let a = intern("test_part_a");
        let b = intern("test_part_b");
        assert_eq!(a, intern("test_part_a"));
        assert_ne!(a, b);
        assert_eq!(name_of(a), "test_part_a");
        assert_eq!(name_of(b), "test_part_b");
    }

    #[test]
    fn known_names_intern_to_their_constants() {
        assert_eq!(intern("head"), HEAD);
        assert_eq!(intern("root"), ROOT);
        assert_eq!(name_of(HEAD), "head");
        assert_eq!(name_of(ROOT), "root");
        // Unknown names come after the constants.
        assert!(intern("test_unknown_part").0 >= FIRST_DYNAMIC);
    }
}
