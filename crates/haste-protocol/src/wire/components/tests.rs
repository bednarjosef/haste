use crate::wire::Writer;

use super::*;
use haste_core::Identifier;

/// A patch body: the add/remove counts, the adds as (type id, data) and
/// the removes as type ids.
fn patch(components: &[(i32, &[u8])], removed: &[i32]) -> Vec<u8> {
    let mut w = Writer::new();
    w.len_prefix(components.len());
    w.len_prefix(removed.len());
    for (ty, data) in components {
        w.var_int(*ty);
        w.bytes(data);
    }
    for ty in removed {
        w.var_int(*ty);
    }
    w.into_inner()
}

/// The type ids used by these tests, with the names the tables resolve
/// them to.
fn tables() -> ProtocolTables {
    ProtocolTables::for_tests(&[
        (1, "minecraft:max_stack_size"),
        (2, "minecraft:max_damage"),
        (3, "minecraft:damage"),
        (4, "minecraft:unbreakable"),
        (5, "minecraft:repair_cost"),
        (6, "minecraft:custom_name"),
        (7, "minecraft:item_name"),
        (8, "minecraft:enchantments"),
        (9, "minecraft:stored_enchantments"),
        (10, "minecraft:tool"),
        (11, "minecraft:food"),
        (12, "minecraft:rarity"),
        (13, "minecraft:mystery"),
        (14, "minecraft:minimum_attack_charge"),
        (15, "minecraft:piercing_weapon"),
        (16, "minecraft:consumable"),
    ])
}

fn text(w: &mut Writer, s: &str) {
    w.component(&Component::text(s));
}

#[test]
fn every_typed_component_reads_its_value() {
    let tables = tables();
    let mut tool = Writer::new();
    w_tool(&mut tool);
    let mut name = Writer::new();
    text(&mut name, "Killing Blow");
    let mut item_name = Writer::new();
    text(&mut item_name, "Iron Block");
    let body = patch(
        &[
            (3, &[5]),
            (2, &[0x99, 0x0C]),
            (1, &[1]),
            (4, &[]),
            (5, &[3]),
            (6, &name.buf),
            (7, &item_name.buf),
            (8, &[1, 8, 3]), // sharpness (id 8), level 3
            // id 0 is a valid enchantment (the first in the registry)
            (9, &[2, 0, 1, 39, 5]),
            (10, &tool.buf),
            (12, &[2]),
        ],
        &[],
    );
    let parsed = read_patch(&body, &tables).unwrap();
    assert_eq!(parsed.damage(), Some(5));
    assert_eq!(parsed.max_damage(), Some(1561));
    assert_eq!(parsed.max_stack_size(), Some(1));
    assert!(parsed.unbreakable());
    assert_eq!(parsed.repair_cost(), Some(3));
    assert_eq!(parsed.custom_name(), Some(&Component::text("Killing Blow")));
    assert_eq!(parsed.item_name(), Some(&Component::text("Iron Block")));
    assert_eq!(
        parsed.enchantments(),
        Some(&[EnchantmentLevel { id: 8, level: 3 }][..])
    );
    assert_eq!(
        parsed.stored_enchantments(),
        Some(
            &[
                EnchantmentLevel { id: 0, level: 1 },
                EnchantmentLevel { id: 39, level: 5 }
            ][..]
        )
    );
    let rules = &parsed.tool().unwrap().rules;
    assert_eq!(
        rules[0].blocks,
        IdSet::Tag(Identifier::minecraft("mineable/axe"))
    );
    assert_eq!(rules[0].speed, Some(12.0));
    assert_eq!(rules[0].correct_for_drops, Some(true));
    assert_eq!(rules[1].blocks, IdSet::Ids(vec![17, 18]));
    assert_eq!(rules[1].speed, None);
    assert_eq!(rules[1].correct_for_drops, None);
    assert_eq!(parsed.tool().unwrap().default_mining_speed, 1.0);
    assert_eq!(parsed.tool().unwrap().damage_per_block, 2);
    assert!(parsed.tool().unwrap().can_destroy_blocks_in_creative);
    assert_eq!(parsed.rarity(), Some(Rarity::Rare));
}

/// An axe: two rules, one with a speed and drop override, one bare.
fn w_tool(w: &mut Writer) {
    w.var_int(2);
    w.write(&IdSet::Tag(Identifier::minecraft("mineable/axe")));
    w.option(&Some(12.0f32), |w, v| w.f32(*v));
    w.option(&Some(true), |w, v| w.bool(*v));
    w.write(&IdSet::Ids(vec![17, 18]));
    w.bool(false);
    w.bool(false);
    w.f32(1.0);
    w.var_int(2);
    w.bool(true);
}

/// The spear's `piercing_weapon` (knockback, dismounts, an optional draw sound with no
/// fixed range, no hit sound) and `minimum_attack_charge`.
#[test]
fn spear_components_read_their_values() {
    let tables = tables();
    let mut spear = Writer::new();
    spear.bool(true); // deals_knockback
    spear.bool(false); // dismounts
    let sound = SoundEvent {
        name: Identifier::minecraft("item.spear.attack"),
        fixed_range: None,
    };
    spear.option(&Some(sound), |w, v| {
        w.identifier(&v.name);
        w.option(&v.fixed_range, |w, r| w.f32(*r));
    });
    spear.option(&None::<SoundEvent>, |_, _| {});
    let charge = 0.75f32;
    let body = patch(
        &[(15, &spear.buf), (14, charge.to_be_bytes().as_slice())],
        &[],
    );
    let parsed = read_patch(&body, &tables).unwrap();
    let weapon = parsed.piercing_weapon().unwrap();
    assert!(weapon.deals_knockback);
    assert!(!weapon.dismounts);
    assert_eq!(
        weapon.sound,
        Some(SoundEvent {
            name: Identifier::minecraft("item.spear.attack"),
            fixed_range: None,
        })
    );
    assert_eq!(weapon.hit_sound, None);
    assert_eq!(parsed.minimum_attack_charge(), Some(0.75));
}

#[test]
fn other_components_keep_their_bytes() {
    let tables = tables();
    let food = [4u8, 0, 0, 32, 64, 0]; // nutrition 4, saturation 2.5, always edible
    let body = patch(&[(11, &food)], &[]);
    let parsed = read_patch(&body, &tables).unwrap();
    assert_eq!(
        parsed.added,
        vec![ItemComponent::Other {
            name: "minecraft:food".to_string(),
            raw: Bytes::copy_from_slice(&food),
        }]
    );
    assert_eq!(parsed.damage(), None);
    assert_eq!(parsed.tool(), None);
    assert_eq!(parsed.enchantments(), None);
}

#[test]
fn removals_name_the_removed_types() {
    let tables = tables();
    let body = patch(&[(3, &[7])], &[4, 12]);
    let parsed = read_patch(&body, &tables).unwrap();
    assert_eq!(parsed.damage(), Some(7));
    assert_eq!(
        parsed.removed,
        vec!["minecraft:unbreakable", "minecraft:rarity"]
    );
}

#[test]
fn empty_patch_is_empty_and_exact() {
    let tables = tables();
    let parsed = read_patch(&[0, 0], &tables).unwrap();
    assert_eq!(parsed, ComponentPatch::default());
    assert!(parsed.added.is_empty() && parsed.removed.is_empty());
}

/// Every typed variant consumes exactly its bytes and no more.
#[test]
fn typed_components_consume_exactly_their_bytes() {
    let tables = tables();
    let mut tool = Writer::new();
    w_tool(&mut tool);
    let mut name = Writer::new();
    text(&mut name, "n");
    let mut enchantments = Writer::new();
    enchantments.var_int(1);
    enchantments.var_int(9);
    enchantments.var_int(3);
    let samples: &[(&str, &[u8])] = &[
        ("minecraft:max_stack_size", &[16]),
        ("minecraft:max_damage", &[0x99, 0x0C]),
        ("minecraft:damage", &[5]),
        ("minecraft:unbreakable", &[]),
        ("minecraft:repair_cost", &[3]),
        ("minecraft:rarity", &[3]),
        ("minecraft:custom_name", &name.buf),
        ("minecraft:item_name", &name.buf),
        ("minecraft:enchantments", &enchantments.buf),
        ("minecraft:stored_enchantments", &enchantments.buf),
        ("minecraft:tool", &tool.buf),
        // Not modelled, but a known layout: kept raw.
        ("minecraft:food", &[4, 0, 0, 32, 64, 0]),
    ];
    for (name, bytes) in samples {
        let mut wire = bytes.to_vec();
        wire.push(0xAB);
        let mut r = Reader::new(&wire).with_tables(&tables);
        read_component(&mut r, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            r.peek_rest(),
            &[0xAB],
            "{name} did not consume exactly its bytes"
        );
    }
}

#[test]
fn unknown_layouts_and_ids_fail() {
    let tables = tables();
    // A component whose layout is unknown cannot be kept raw.
    let body = patch(&[(13, &[1])], &[]);
    assert!(read_patch(&body, &tables).is_err());
    // A type id the tables do not know.
    let body = patch(&[(99, &[])], &[]);
    assert!(read_patch(&body, &tables).is_err());
    // Trailing bytes after the patch are a layout mismatch.
    let mut body = patch(&[(3, &[1])], &[]);
    body.push(0);
    assert!(read_patch(&body, &tables).is_err());
    // A truncated enchantment list (count 2, one entry).
    let body = patch(&[(8, &[2, 0, 3])], &[]);
    assert!(read_patch(&body, &tables).is_err());
}

/// A `minecraft:consumable` body as vanilla's `Consumable.STREAM_CODEC` writes it:
/// seconds, animation, the sound holder, the particle flag, then the effect count.
fn w_consumable(
    w: &mut Writer,
    seconds: f32,
    animation: i32,
    sound: &IdOr<SoundEvent>,
    particles: bool,
) {
    w.f32(seconds);
    w.var_int(animation);
    w.write(sound);
    w.bool(particles);
    w.len_prefix(0); // no on_consume_effects
}

/// A consumable whose sound is a registry id, with no consume effects.
#[test]
fn consumable_reads_the_patch_component() {
    let tables = tables();
    let mut c = Writer::new();
    w_consumable(&mut c, 3.2, 2, &IdOr::Id(15), false);
    let body = patch(&[(16, &c.buf)], &[]);
    let parsed = read_patch(&body, &tables).unwrap();
    let consumable = parsed.consumable().unwrap();
    assert_eq!(consumable.consume_seconds, 3.2);
    assert_eq!(consumable.animation, 2); // drink
    assert_eq!(consumable.sound, IdOr::Id(15));
    assert!(!consumable.has_consume_particles);
    assert!(!parsed.removes_consumable());
}

/// A consumable with an inline sound (fixed range present), two `on_consume_effects`
/// (an apply_effects and a play_sound), a component before it and one after: the effects
/// are consumed by their codecs without desyncing the stream.
#[test]
fn consumable_inline_sound_and_consume_effects() {
    let tables = tables();
    let mut c = Writer::new();
    c.f32(0.1);
    c.var_int(1); // eat
    c.var_int(0); // inline sound event
    c.identifier(&Identifier::minecraft("mob.goat.cheer"));
    c.bool(true);
    c.f32(64.0); // fixed range
    c.bool(true); // has_consume_particles
    c.len_prefix(2); // two on_consume_effects
    c.var_int(0); // apply_effects
    c.len_prefix(1); // one MobEffectInstance
    c.var_int(3); // effect id
    c.var_int(0); // amplifier
    c.var_int(200); // duration
    c.bool(false); // ambient
    c.bool(true); // show particles
    c.bool(true); // show icon
    c.bool(false); // no hidden effect
    c.f32(1.0); // probability
    c.var_int(4); // play_sound
    c.identifier(&Identifier::minecraft("entity.generic.drink"));
    c.bool(false); // no fixed range
    let body = patch(&[(3, &[9]), (16, &c.buf), (5, &[3])], &[]);
    let parsed = read_patch(&body, &tables).unwrap();
    assert_eq!(parsed.damage(), Some(9));
    let consumable = parsed.consumable().unwrap();
    assert_eq!(consumable.consume_seconds, 0.1);
    assert_eq!(
        consumable.sound,
        IdOr::Inline(SoundEvent {
            name: Identifier::minecraft("mob.goat.cheer"),
            fixed_range: Some(64.0),
        })
    );
    assert!(consumable.has_consume_particles);
    // The component after the consumable read too: the effects consumed their bytes.
    assert_eq!(parsed.repair_cost(), Some(3));
}

/// A removed `minecraft:consumable` makes the stack not consumable.
#[test]
fn removed_consumable_is_reported() {
    let tables = tables();
    let body = patch(&[], &[16]);
    let parsed = read_patch(&body, &tables).unwrap();
    assert!(parsed.removes_consumable());
    assert_eq!(parsed.consumable(), None);
}
