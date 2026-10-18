use super::*;

#[test]
fn stack_encoding_round_trips() {
    let s = ItemStackData {
        count: 64,
        item: Some(RegistryIndex(300)),
        components_raw: bytes::Bytes::from_static(&[1, 0, 5, 9]),
    };
    let b = encode_stack(&s);
    assert_eq!(b[..3], [64, 0xAC, 0x02]);
    assert_eq!(decode_stack(&b), Some(s));
    assert_eq!(decode_stack(&[0]), Some(empty_stack()));
    assert_eq!(encode_stack(&empty_stack()), vec![0]);
    assert_eq!(decode_stack(&[0x80]), None, "truncated VarInt");
    let plain = ItemStackData {
        count: 1,
        item: Some(RegistryIndex(1)),
        components_raw: bytes::Bytes::new(),
    };
    assert_eq!(encode_stack(&plain), vec![1, 1, 0, 0]);
}

#[test]
fn mirror_slots() {
    let mut s = GameState::default();
    s.set_mirror(
        3,
        ItemStackData {
            count: 5,
            item: Some(RegistryIndex(1)),
            components_raw: bytes::Bytes::new(),
        },
    );
    assert_eq!(s.slot(3).map(|x| x.count), Some(5));
    assert!(s.slot(4).is_none());
    assert!(s.slot(999).is_none());
    s.set_mirror(50, empty_stack());
    assert_eq!(s.inventory.len(), 51);
    s.selected_slot = 3;
    assert_eq!(s.selected().map(|x| x.count), Some(5));
    s.sent.push(ChatMessage {
        text: "hi".into(),
        command: false,
    });
    assert_eq!(s.take_sent().len(), 1);
    assert!(s.sent.is_empty());
    // Every mirrored change is queued for the client to drain, oldest first.
    assert_eq!(
        s.take_inventory_changes(),
        vec![(3, s.slot(3).cloned().unwrap()), (50, empty_stack())]
    );
    s.set_mirror(3, empty_stack());
    assert_eq!(
        s.take_inventory_changes(),
        vec![(3, empty_stack())],
        "clearing a slot is a change too (a creative pickup empties the hotbar slot)"
    );
    assert!(s.take_inventory_changes().is_empty(), "drained");
}
