use super::*;
use crate::wire::{DecodeError, IdOr, IdSet, ResolvableProfile, SoundEvent};

#[test]
fn empty_and_plain_stacks_round_trip() {
    let empty = ItemStackData::default();
    let plain = ItemStackData {
        count: 3,
        item: Some(RegistryIndex(42)),
        components_raw: Bytes::new(),
    };
    let mut w = Writer::new();
    w.write(&empty);
    w.write(&plain);
    assert_eq!(w.buf, vec![0, 3, 42, 0, 0]);
    let mut r = Reader::new(&w.buf);
    assert!(r.read::<ItemStackData>().unwrap().is_empty());
    let back = r.read::<ItemStackData>().unwrap();
    assert_eq!(back.count, 3);
    assert_eq!(back.item, Some(RegistryIndex(42)));
    assert_eq!(&back.components_raw[..], &[0, 0]);
    r.finish().unwrap();
}

#[test]
fn components_need_tables_and_known_layouts() {
    let tables = crate::packets::ids::ProtocolTables::for_tests(&[
        (3, "minecraft:damage"),
        (60, "minecraft:mystery"),
    ]);
    // count 1, item 7, 1 add, 1 remove, damage=5, remove type 4
    let bytes = [1u8, 7, 1, 1, 3, 5, 4];
    let mut r = Reader::new(&bytes).with_tables(&tables);
    let s = r.read::<ItemStackData>().unwrap();
    assert_eq!(&s.components_raw[..], &[1, 1, 3, 5, 4]);
    r.finish().unwrap();
    assert!(Reader::new(&bytes).read::<ItemStackData>().is_err());
    let unknown = [1u8, 7, 1, 0, 60, 0];
    assert!(Reader::new(&unknown)
        .with_tables(&tables)
        .read::<ItemStackData>()
        .is_err());
}

/// Hand-built values for every documented component; each must be
/// consumed exactly.
fn samples() -> Vec<(&'static str, Vec<u8>)> {
    use haste_core::{BlockPos, Component, Identifier};
    let w = |f: &dyn Fn(&mut Writer)| {
        let mut w = Writer::new();
        f(&mut w);
        w.into_inner()
    };
    let nbt = |w: &mut Writer| {
        let mut c = crate::nbt::NbtCompound::new();
        c.put_int("x", 1);
        w.nbt_compound(Some(&c));
    };
    let text = |w: &mut Writer| w.component(&Component::text("t"));
    let sound = |w: &mut Writer| {
        w.write(&SoundEvent {
            name: Identifier::minecraft("s"),
            fixed_range: Some(1.0),
        })
    };
    let id_set = |w: &mut Writer| w.write(&IdSet::Ids(vec![1, 2]));
    // Components hold item stack templates: item 300 (two VarInt bytes),
    // then count 2, then an empty patch.
    let stack = |w: &mut Writer| {
        w.write_as::<_, ItemStackTemplate>(&ItemStackData {
            count: 2,
            item: Some(RegistryIndex(300)),
            components_raw: Bytes::new(),
        })
    };
    let effect = |w: &mut Writer| {
        w.var_int(1); // type
        w.var_int(0);
        w.var_int(200);
        w.bool(false);
        w.bool(true);
        w.bool(true);
        w.bool(true); // hidden effect present
        w.var_int(0);
        w.var_int(100);
        w.bool(false);
        w.bool(true);
        w.bool(true);
        w.bool(false);
    };
    let consume_effects = move |w: &mut Writer| {
        w.var_int(5);
        w.var_int(0);
        w.var_int(1);
        effect(w);
        w.f32(1.0);
        w.var_int(1);
        id_set(w);
        w.var_int(2);
        w.var_int(3);
        w.f32(16.0);
        w.var_int(4);
        sound(w);
    };
    let firework = |w: &mut Writer| {
        w.var_int(1);
        w.write(&vec![0xff0000i32]);
        w.write(&Vec::<i32>::new());
        w.bool(true);
        w.bool(false);
    };
    let predicate = move |w: &mut Writer| {
        w.bool(true);
        id_set(w);
        w.bool(true);
        w.var_int(2);
        w.string("facing");
        w.bool(true);
        w.string("north");
        w.string("age");
        w.bool(false);
        w.string("0");
        w.string("3");
        w.bool(true);
        nbt(w);
        w.var_int(1); // exact matcher: damage = 4
        w.var_int(3);
        w.var_int(4);
        w.var_int(1); // partial predicate
        w.var_int(0);
        nbt(w);
    };
    vec![
        ("custom_data", w(&nbt)),
        ("max_stack_size", w(&|w| w.var_int(16))),
        ("unbreakable", vec![]),
        (
            "use_effects",
            w(&|w| {
                w.bool(true);
                w.bool(false);
                w.f32(0.5);
            }),
        ),
        ("custom_name", w(&text)),
        ("minimum_attack_charge", w(&|w| w.f32(0.5))),
        ("damage_type", w(&|w| w.var_int(3))),
        (
            "lore",
            w(&|w| {
                w.var_int(2);
                text(w);
                text(w);
            }),
        ),
        (
            "enchantments",
            w(&|w| {
                w.var_int(1);
                w.var_int(5);
                w.var_int(3);
            }),
        ),
        (
            "can_place_on",
            w(&|w| {
                w.var_int(1);
                predicate(w);
            }),
        ),
        (
            "custom_model_data",
            w(&|w| {
                w.write(&vec![1.0f32]);
                w.write(&vec![true]);
                w.write(&vec!["a".to_string()]);
                w.write(&vec![7i32]);
            }),
        ),
        (
            "tooltip_display",
            w(&|w| {
                w.bool(false);
                w.write_as::<_, crate::wire::List<crate::wire::VarInt>>(&vec![1i32, 2]);
            }),
        ),
        (
            "food",
            w(&|w| {
                w.var_int(4);
                w.f32(2.4);
                w.bool(false);
            }),
        ),
        (
            "consumable",
            w(&|w| {
                w.f32(1.6);
                w.var_int(1);
                w.write(&IdOr::<SoundEvent>::Id(10));
                w.bool(true);
                consume_effects(w);
            }),
        ),
        ("use_remainder", w(&stack)),
        (
            "use_cooldown",
            w(&|w| {
                w.f32(1.0);
                w.write(&Some(Identifier::minecraft("g")));
            }),
        ),
        ("damage_resistant", w(&id_set)),
        ("attack_range", w(&|w| (0..6).for_each(|_| w.f32(1.0)))),
        (
            "tool",
            w(&|w| {
                w.var_int(1);
                id_set(w);
                w.write(&Some(8.0f32));
                w.write(&Some(true));
                w.f32(1.0);
                w.var_int(1);
                w.bool(true);
            }),
        ),
        (
            "weapon",
            w(&|w| {
                w.var_int(1);
                w.f32(5.0);
            }),
        ),
        (
            "equippable",
            w(&|w| {
                w.var_int(4);
                w.write(&IdOr::Inline(SoundEvent {
                    name: Identifier::minecraft("e"),
                    fixed_range: None,
                }));
                w.write(&Some(Identifier::minecraft("m")));
                w.write(&None::<Identifier>);
                w.write(&Some(IdSet::Tag(Identifier::minecraft("t"))));
                w.bool(true);
                w.bool(true);
                w.bool(true);
                w.bool(false);
                w.bool(false);
                w.write(&IdOr::<SoundEvent>::Id(1));
            }),
        ),
        ("repairable", w(&id_set)),
        ("death_protection", w(&consume_effects)),
        (
            "blocks_attacks",
            w(&|w| {
                w.f32(0.25);
                w.f32(1.0);
                w.var_int(1);
                w.f32(90.0);
                w.bool(true);
                id_set(w);
                w.f32(0.0);
                w.f32(1.0);
                w.f32(3.0);
                w.f32(1.0);
                w.f32(1.0);
                w.bool(false);
                w.bool(true);
                w.write(&IdOr::<SoundEvent>::Id(2));
                w.bool(false);
            }),
        ),
        (
            "piercing_weapon",
            w(&|w| {
                w.bool(true);
                w.bool(false);
                w.bool(true);
                sound(w);
                w.bool(false);
            }),
        ),
        (
            "kinetic_weapon",
            w(&|w| {
                w.var_int(10);
                w.var_int(5);
                w.bool(true);
                w.var_int(20);
                w.f32(0.1);
                w.f32(0.2);
                w.bool(false);
                w.bool(false);
                w.f32(1.0);
                w.f32(2.0);
                w.bool(false);
                w.bool(true);
                sound(w);
            }),
        ),
        (
            "charged_projectiles",
            w(&|w| {
                w.var_int(1);
                stack(w);
            }),
        ),
        (
            "potion_contents",
            w(&|w| {
                w.option(&Some(0i32), |w, p| w.var_int(*p));
                w.write(&Some(0xff00ffi32));
                w.var_int(1);
                effect(w);
                w.write(&Some("n".to_string()));
            }),
        ),
        ("potion_duration_scale", w(&|w| w.f32(0.5))),
        (
            "suspicious_stew_effects",
            w(&|w| {
                w.var_int(1);
                w.var_int(2);
                w.var_int(160);
            }),
        ),
        (
            "writable_book_content",
            w(&|w| {
                w.var_int(1);
                w.string("p");
                w.write(&None::<String>);
            }),
        ),
        (
            "written_book_content",
            w(&|w| {
                w.string("title");
                w.write(&None::<String>);
                w.string("author");
                w.var_int(0);
                w.var_int(1);
                text(w);
                w.bool(true);
                text(w);
                w.bool(true);
            }),
        ),
        (
            "trim",
            w(&|w| {
                w.var_int(0);
                w.string("_gold");
                w.var_int(1);
                w.identifier(&Identifier::minecraft("iron"));
                w.string("gold_darker");
                text(w);
                w.var_int(0);
                w.string("coast");
                w.var_int(5);
                text(w);
                w.bool(false);
            }),
        ),
        ("debug_stick_state", w(&nbt)),
        (
            "entity_data",
            w(&|w| {
                w.var_int(5);
                nbt(w);
            }),
        ),
        (
            "instrument",
            w(&|w| {
                w.var_int(0);
                w.write(&IdOr::<SoundEvent>::Id(1));
                w.f32(7.0);
                w.f32(256.0);
                text(w);
            }),
        ),
        (
            "jukebox_playable",
            w(&|w| {
                w.var_int(0);
                w.write(&IdOr::<SoundEvent>::Id(1));
                text(w);
                w.f32(100.0);
                w.var_int(5);
            }),
        ),
        (
            "lodestone_tracker",
            w(&|w| {
                w.bool(true);
                w.identifier(&Identifier::minecraft("overworld"));
                w.block_pos(&BlockPos::new(1, 2, 3));
                w.bool(true);
            }),
        ),
        ("firework_explosion", w(&firework)),
        (
            "fireworks",
            w(&|w| {
                w.var_int(2);
                w.var_int(1);
                firework(w);
            }),
        ),
        (
            "profile",
            w(&|w| {
                w.write(&ResolvableProfile {
                    profile: crate::wire::ResolvableProfileKind::Partial {
                        name: Some("Steve".into()),
                        uuid: None,
                        properties: vec![],
                    },
                    body: None,
                    cape: None,
                    elytra: None,
                    model: Some(1),
                })
            }),
        ),
        (
            "banner_patterns",
            w(&|w| {
                w.var_int(2);
                w.var_int(3);
                w.var_int(14);
                w.var_int(0);
                w.identifier(&Identifier::minecraft("stripe"));
                w.string("block.minecraft.banner.stripe");
                w.var_int(0);
            }),
        ),
        (
            "pot_decorations",
            // Observed bytes: back and right empty, left and front sherds.
            vec![0, 1, 0xC9, 0x0C, 1, 0, 0, 0, 1, 0xC5, 0x0C, 1, 0, 0],
        ),
        (
            "container",
            w(&|w| {
                w.var_int(2);
                w.bool(true);
                stack(w);
                w.bool(false);
            }),
        ),
        (
            "block_state",
            w(&|w| {
                w.var_int(1);
                w.string("lit");
                w.string("true");
            }),
        ),
        (
            "bees",
            w(&|w| {
                w.var_int(1);
                w.var_int(9);
                nbt(w);
                w.var_int(10);
                w.var_int(600);
            }),
        ),
        (
            "lock",
            w(&|w| w.nbt_tag(Some(&crate::nbt::NbtTag::String("key".into())))),
        ),
        ("break_sound", w(&|w| w.write(&IdOr::<SoundEvent>::Id(3)))),
        ("sulfur_cube_content", w(&stack)),
        ("dyed_color", w(&|w| w.i32(0x00ff00))),
        (
            "provides_banner_patterns",
            w(&|w| w.write(&IdSet::Tag(Identifier::minecraft("pattern_item/flower")))),
        ),
        ("provides_trim_material", w(&|w| w.var_int(4))),
        ("sheep/color", w(&|w| w.var_int(14))),
        // Observed on the 26.3 vanilla server: `iron_sword[attribute_modifiers=
        // [{type:"attack_damage",amount:5,id:"x:y",operation:"add_value"}]]`.
        (
            "attribute_modifiers",
            vec![
                1, 3, 3, b'x', b':', b'y', 0x40, 0x14, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        ),
        ("painting/variant", vec![25]),
        (
            "attack_animation",
            w(&|w| {
                w.var_int(1);
                w.var_int(6);
            }),
        ),
    ]
}

/// Bytes observed from the 26.3 vanilla server for a shulker box holding
/// five diamonds (`container` = list of optional templates).
#[test]
fn container_holds_optional_templates() {
    let t = crate::packets::ids::ProtocolTables::for_tests(&[(77, "minecraft:container")]);
    let wire = [1u8, 0x90, 0x05, 1, 0, 77, 1, 1, 0xF4, 0x07, 5, 0, 0];
    let mut r = Reader::new(&wire).with_tables(&t);
    let s = ItemStackData::read(&mut r).unwrap();
    r.finish().unwrap();
    assert_eq!(s.item, Some(RegistryIndex(656)));
    let mut r = Reader::new(&s.components_raw[3..]);
    let inner = r.list(256, OptionalItemStackTemplate::read).unwrap();
    assert_eq!(inner[0].item, Some(RegistryIndex(1012)));
    assert_eq!(inner[0].count, 5);
}

#[test]
fn every_documented_component_skips_exactly() {
    let tables = crate::packets::ids::ProtocolTables::for_tests(&[(3, "minecraft:damage")]);
    for (name, bytes) in samples() {
        let mut with_tail = bytes.clone();
        with_tail.push(0xAB);
        let mut r = Reader::new(&with_tail).with_tables(&tables);
        skip_component(&mut r, &format!("minecraft:{name}"))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            r.peek_rest(),
            &[0xAB],
            "{name} did not consume exactly its bytes"
        );
    }
}

#[test]
#[ignore = "needs cache/datagen/26.3/reports (run with --ignored); prints unsupported components"]
fn report_unsupported_components() {
    let t = crate::packets::ids::ProtocolTables::load().expect("reports");
    let mut missing = Vec::new();
    for id in 0..200 {
        let Some(name) = t.data_component_name(id) else {
            continue;
        };
        let mut r = Reader::new(&[]);
        if let Err(DecodeError::Invalid(m)) = skip_component(&mut r, name) {
            if m.contains("not implemented") {
                missing.push(name.to_string());
            }
        }
    }
    println!("unsupported data components: {missing:?}");
}
